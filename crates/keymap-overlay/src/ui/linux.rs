//! Linux HID daemon and renderer state service.
//!
//! Linux renderers live outside this process: Qt owns its own event loop, and
//! GNOME/Cinnamon extensions run inside their desktop shell. The daemon keeps
//! the authoritative reduced state and publishes final display models over the
//! user's D-Bus session.

use anyhow::{Context as _, Result};
use keymap_overlay_linux_protocol::{
    BUS_NAME, OBJECT_PATH, RENDERER_INTERFACE, RendererService, RendererStateStore,
};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::{SystemTime, UNIX_EPOCH};
use zbus::blocking::Connection;

use crate::{
    LayerEventSink, ListenerEvent, ModelCache, PendingTransition, Transition, compose_model,
    load_model_cache, spawn_raw_hid_listener,
};

#[derive(Clone)]
struct ChannelSink(Sender<ListenerEvent>);

impl LayerEventSink for ChannelSink {
    fn send(&self, event: ListenerEvent) -> bool {
        self.0.send(event).is_ok()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RendererState {
    generation: u64,
    visible: bool,
    model_json: String,
}

#[derive(Debug, Eq, PartialEq)]
struct UpdateOutcome {
    changed: bool,
    missing_model: bool,
}

impl RendererState {
    fn for_process() -> Result<Self> {
        let generation = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("The system clock is before the Unix epoch")?
            .as_micros()
            .try_into()
            .context("The system clock cannot fit in the renderer generation")?;
        Ok(Self {
            generation,
            ..Self::default()
        })
    }

    fn update(&mut self, transition: &Transition, models: &ModelCache) -> Result<UpdateOutcome> {
        match transition {
            Transition::Show {
                keyboard_id,
                layers,
            } => {
                let Some(model) = compose_model(models, *keyboard_id, layers) else {
                    return Ok(UpdateOutcome {
                        changed: self.hide(),
                        missing_model: true,
                    });
                };
                let model_json = serde_json::to_string(&model)
                    .context("Failed to serialize an overlay model")?;
                Ok(UpdateOutcome {
                    changed: self.replace_content(true, model_json),
                    missing_model: false,
                })
            }
            Transition::Hide => Ok(UpdateOutcome {
                changed: self.hide(),
                missing_model: false,
            }),
            Transition::Ignore => Ok(UpdateOutcome {
                changed: false,
                missing_model: false,
            }),
        }
    }

    fn replace_content(&mut self, visible: bool, model_json: String) -> bool {
        let next = Self {
            generation: self.generation,
            visible,
            model_json,
        };
        if *self == next {
            return false;
        }
        *self = Self {
            generation: self.generation.wrapping_add(1),
            ..next
        };
        true
    }

    fn hide(&mut self) -> bool {
        self.replace_content(false, String::new())
    }

    fn tuple(&self) -> (u64, bool, String) {
        (self.generation, self.visible, self.model_json.clone())
    }
}

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let models = load_model_cache(&assets_dir)?;
    // Seed generations with wall time so a renderer can distinguish a daemon
    // restart from an old queued signal without any persistent state.
    let mut state = RendererState::for_process()?;
    let state_store = RendererStateStore::new(state.tuple());
    let connection =
        Connection::session().context("Failed to connect to the user D-Bus session")?;
    connection
        .object_server()
        .at(OBJECT_PATH, RendererService::new(state_store.clone()))
        .context("Failed to register the renderer state object")?;
    connection
        .request_name(BUS_NAME)
        .context("Failed to own the keymap overlay D-Bus name")?;

    let (sender, receiver) = mpsc::channel();
    spawn_raw_hid_listener(ChannelSink(sender));
    let mut pending = PendingTransition::default();

    for event in receiver {
        pending.push(event);
        let transition = pending.take();
        let outcome = state.update(&transition, &models)?;
        if outcome.missing_model
            && let Transition::Show {
                keyboard_id,
                layers,
            } = &transition
        {
            log::warn!(
                "Overlay model composition is unavailable for keyboard {keyboard_id}, layers {layers:?}"
            );
        }
        if !outcome.changed {
            continue;
        }
        let snapshot = state.tuple();
        state_store.set(snapshot.clone());
        connection
            .emit_signal(
                None::<&str>,
                OBJECT_PATH,
                RENDERER_INTERFACE,
                "StateChanged",
                &snapshot,
            )
            .context("Failed to publish renderer state")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OverlayModel;
    use std::collections::HashMap;

    fn model(layer: u8) -> OverlayModel {
        OverlayModel {
            version: 1,
            layer,
            width: 100,
            height: 50,
            header_font_size: 14.0,
            key_font_size: 10.0,
            encoder_font_size: 9.0,
            keys: vec![],
            encoders: vec![],
        }
    }

    #[test]
    fn renderer_state_serializes_visible_models_and_hides() {
        let models = HashMap::from([((2, 0), model(0)), ((2, 3), model(3))]);
        let mut state = RendererState::default();

        let show = state
            .update(
                &Transition::Show {
                    keyboard_id: 2,
                    layers: vec![3],
                },
                &models,
            )
            .expect("show model");
        assert_eq!(
            show,
            UpdateOutcome {
                changed: true,
                missing_model: false,
            }
        );
        assert!(state.visible);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&state.model_json).expect("model JSON")["layer"],
            3
        );

        assert!(
            state
                .update(&Transition::Hide, &models)
                .expect("hide model")
                .changed
        );
        assert!(!state.visible);
        assert!(state.model_json.is_empty());
        assert_eq!(state.generation, 2);
    }

    #[test]
    fn unchanged_states_are_not_published() {
        let models = HashMap::from([((2, 0), model(0)), ((2, 3), model(3))]);
        let mut state = RendererState::default();
        let show = Transition::Show {
            keyboard_id: 2,
            layers: vec![3],
        };

        assert!(state.update(&show, &models).expect("first show").changed);
        let generation = state.generation;
        assert!(!state.update(&show, &models).expect("repeated show").changed);
        assert_eq!(state.generation, generation);
        assert!(
            state
                .update(&Transition::Hide, &models)
                .expect("first hide")
                .changed
        );
        let generation = state.generation;
        assert!(
            !state
                .update(&Transition::Hide, &models)
                .expect("repeated hide")
                .changed
        );
        assert_eq!(state.generation, generation);
    }

    #[test]
    fn missing_models_hide_visible_state_once() {
        let models = HashMap::from([((2, 0), model(0)), ((2, 3), model(3))]);
        let mut state = RendererState::default();
        state
            .update(
                &Transition::Show {
                    keyboard_id: 2,
                    layers: vec![3],
                },
                &models,
            )
            .expect("show model");

        let missing = state
            .update(
                &Transition::Show {
                    keyboard_id: 1,
                    layers: vec![9],
                },
                &models,
            )
            .expect("missing model");
        assert_eq!(
            missing,
            UpdateOutcome {
                changed: true,
                missing_model: true,
            }
        );
        assert!(!state.visible);
        assert_eq!(state.generation, 2);

        let repeated = state
            .update(
                &Transition::Show {
                    keyboard_id: 1,
                    layers: vec![9],
                },
                &models,
            )
            .expect("repeated missing model");
        assert_eq!(
            repeated,
            UpdateOutcome {
                changed: false,
                missing_model: true,
            }
        );
        assert_eq!(state.generation, 2);
    }
}
