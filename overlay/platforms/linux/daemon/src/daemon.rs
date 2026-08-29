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
use keymap_overlay_runtime::{
    LayerEvent, LayerEventSink, LayerEventSourceHandle, ModelCache, PendingTransition,
    SimulatedLayer, StartupModels, Transition, compose_model, spawn_layer_event_source,
};
use log::{info, warn};
use rustix::event::{PollFd, PollFlags, poll};
use std::os::fd::AsFd;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use zbus::blocking::Connection;

#[derive(Clone)]
struct ChannelSink(Sender<LayerEvent>);

impl LayerEventSink for ChannelSink {
    fn send(&self, event: LayerEvent) -> bool {
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

pub(crate) fn run(startup: StartupModels, simulated: Option<SimulatedLayer>) -> Result<()> {
    let StartupModels {
        models,
        raw_hid_devices,
    } = startup;
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
    let source = spawn_layer_event_source(
        ChannelSink(sender),
        simulated,
        raw_hid_devices,
        models.keys().map(|(keyboard_id, _)| *keyboard_id),
    );
    if source.uses_raw_hid() {
        spawn_device_watcher(source);
    }
    let mut pending = PendingTransition::default();

    for event in &receiver {
        let transition = reduce_queued_events(event, &receiver, &mut pending);
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

fn spawn_device_watcher(listener: LayerEventSourceHandle) {
    thread::spawn(move || {
        if let Err(error) = watch_for_arrivals(&listener) {
            // Not fatal: without it, keyboards are still picked up whenever
            // one of the active readers ends.
            warn!("Stopped watching for keyboards: {error:#}");
        }
    });
}

/// Blocks on udev events, so an idle overlay costs nothing.
fn watch_for_arrivals(listener: &LayerEventSourceHandle) -> Result<()> {
    let socket = udev::MonitorBuilder::new()
        .context("Failed to open a udev monitor")?
        .match_subsystem("hidraw")
        .context("Failed to match the hidraw subsystem")?
        .listen()
        .context("Failed to listen for udev events")?;

    loop {
        wait_readable(&socket)?;
        // Events are drained in a batch: plugging in one keyboard emits
        // several, and they should cost one enumeration between them.
        let arrived = socket.iter().fold(false, |arrived, event| {
            arrived | (event.event_type() == udev::EventType::Add)
        });
        if arrived && listener.device_arrived() {
            info!("A Raw HID device appeared; enumerating again");
        }
    }
}

fn wait_readable(socket: &udev::MonitorSocket) -> Result<()> {
    let descriptor = socket.as_fd();
    let mut fds = [PollFd::new(&descriptor, PollFlags::IN)];
    poll(&mut fds, None).context("Failed to wait on the udev monitor")?;
    Ok(())
}

fn reduce_queued_events(
    first: LayerEvent,
    receiver: &Receiver<LayerEvent>,
    pending: &mut PendingTransition,
) -> Transition {
    pending.push(first);
    for event in receiver.try_iter() {
        pending.push(event);
    }
    pending.take()
}

#[cfg(test)]
mod tests {
    use super::*;
    use keymap_overlay_runtime::{OverlayModel, RawLayerEvent};
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
    fn queued_events_are_reduced_before_the_daemon_publishes() {
        let (sender, receiver) = mpsc::channel();
        let mut pending = PendingTransition::default();
        let first = LayerEvent::Report(RawLayerEvent {
            keyboard_id: 1,
            layer: 2,
            pressed: true,
        });
        sender
            .send(LayerEvent::Report(RawLayerEvent {
                keyboard_id: 1,
                layer: 3,
                pressed: true,
            }))
            .expect("queue layer press");
        sender
            .send(LayerEvent::Report(RawLayerEvent {
                keyboard_id: 1,
                layer: 3,
                pressed: false,
            }))
            .expect("queue layer release");

        assert_eq!(
            reduce_queued_events(first, &receiver, &mut pending),
            Transition::Show {
                keyboard_id: 1,
                layers: vec![2],
            }
        );

        let (sender, receiver) = mpsc::channel();
        let second_keyboard = LayerEvent::Report(RawLayerEvent {
            keyboard_id: 2,
            layer: 4,
            pressed: true,
        });
        sender
            .send(LayerEvent::Disconnected {
                keyboard_id: Some(2),
            })
            .expect("queue keyboard disconnect");

        assert_eq!(
            reduce_queued_events(second_keyboard, &receiver, &mut pending),
            Transition::Show {
                keyboard_id: 1,
                layers: vec![2],
            }
        );
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
