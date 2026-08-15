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
    LayerEventSink, ListenerEvent, ModelCache, OverlayModel, PendingTransition, Transition,
    compose_model, load_model_cache, spawn_raw_hid_listener,
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

    fn update(&mut self, transition: &Transition, models: &ModelCache) -> Result<bool> {
        match transition {
            Transition::Show {
                keyboard_id,
                layers,
            } => {
                let Some(model) = compose_model(models, *keyboard_id, layers) else {
                    self.hide();
                    return Ok(true);
                };
                self.generation = self.generation.wrapping_add(1);
                self.visible = true;
                self.model_json = serde_json::to_string(&model)
                    .context("Failed to serialize an overlay model")?;
                Ok(true)
            }
            Transition::Hide => {
                self.hide();
                Ok(true)
            }
            Transition::Ignore => Ok(false),
        }
    }

    fn hide(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.visible = false;
        self.model_json.clear();
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
        let missing_model = match &transition {
            Transition::Show {
                keyboard_id,
                layers,
            } => compose_model(&models, *keyboard_id, layers).is_none(),
            Transition::Hide | Transition::Ignore => false,
        };
        let changed = state.update(&transition, &models)?;
        let snapshot = state.tuple();
        if missing_model
            && let Transition::Show {
                keyboard_id,
                layers,
            } = &transition
        {
            log::warn!(
                "Overlay model composition is unavailable for keyboard {keyboard_id}, layers {layers:?}"
            );
        }
        if !changed {
            continue;
        }
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

        assert!(
            state
                .update(
                    &Transition::Show {
                        keyboard_id: 2,
                        layers: vec![3],
                    },
                    &models,
                )
                .expect("show model")
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
        );
        assert!(!state.visible);
        assert!(state.model_json.is_empty());
        assert_eq!(state.generation, 2);
    }

    #[test]
    fn missing_models_publish_a_hidden_state() {
        let mut state = RendererState::default();

        assert!(
            state
                .update(
                    &Transition::Show {
                        keyboard_id: 1,
                        layers: vec![9],
                    },
                    &HashMap::new(),
                )
                .expect("missing model")
        );
        assert!(!state.visible);
        assert_eq!(state.generation, 1);
    }
}
