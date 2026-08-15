//! Native Qt Quick overlay for KDE Plasma.
//!
//! The Raw HID readers and model composition stay in Rust. A Unix socket pair
//! carries the final model or hide transition to Qt's main loop, where a
//! `QSocketNotifier` wakes the window without polling. LayerShellQt gives the
//! `QQuickWindow` its overlay layer, active-screen placement, and no-input
//! semantics on Wayland.

use anyhow::{Context as _, Result};
use std::os::fd::IntoRawFd as _;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::{
    LayerEventSink, ListenerEvent, ModelCache, PendingTransition, Transition, compose_model,
    load_model_cache, spawn_raw_hid_listener,
};

const HIDE: u8 = 2;

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let (reader, writer) = UnixDatagram::pair().context("Failed to create the Qt event socket")?;
    let models = load_model_cache(&assets_dir)?;
    spawn_raw_hid_listener(SocketSink {
        state: Arc::new(Mutex::new(SocketState {
            socket: writer,
            pending: PendingTransition::default(),
            models,
        })),
    });

    keymap_overlay_qt_bridge::run_qt_overlay(reader.into_raw_fd())
        .context("The Qt overlay event loop failed")
}

#[derive(Clone)]
struct SocketSink {
    state: Arc<Mutex<SocketState>>,
}

struct SocketState {
    socket: UnixDatagram,
    pending: PendingTransition,
    models: ModelCache,
}

impl LayerEventSink for SocketSink {
    fn send(&self, event: ListenerEvent) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        state.pending.push(event);
        let packet = match state.pending.take() {
            Transition::Show {
                keyboard_id,
                layers,
            } => match compose_model(&state.models, keyboard_id, &layers) {
                Some(model) => match serde_json::to_vec(&model) {
                    Ok(packet) => packet,
                    Err(_) => return false,
                },
                None => vec![HIDE],
            },
            Transition::Hide => vec![HIDE],
            Transition::Ignore => return true,
        };
        state.socket.send(&packet).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keymap_core::RawLayerEvent;

    fn layer(pressed: bool) -> ListenerEvent {
        ListenerEvent::Layer(RawLayerEvent {
            keyboard_id: 2,
            layer: 3,
            pressed,
        })
    }

    #[test]
    fn qt_hides_for_an_unavailable_model_and_a_release() {
        let (reader, writer) = UnixDatagram::pair().expect("socket pair");
        let sink = SocketSink {
            state: Arc::new(Mutex::new(SocketState {
                socket: writer,
                pending: PendingTransition::default(),
                models: ModelCache::new(),
            })),
        };
        let mut packet = [0; 3];

        assert!(sink.send(layer(true)));
        assert_eq!(reader.recv(&mut packet).expect("show packet"), 1);
        assert_eq!(packet[0], HIDE);

        assert!(sink.send(layer(false)));
        assert_eq!(reader.recv(&mut packet).expect("hide packet"), 1);
        assert_eq!(packet[0], HIDE);
    }
}
