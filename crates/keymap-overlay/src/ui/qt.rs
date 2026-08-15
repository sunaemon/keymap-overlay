//! Native Qt Quick overlay for KDE Plasma.
//!
//! The Raw HID readers stay in Rust. A Unix socket pair carries only reduced
//! show/hide transitions to Qt's main loop, where a `QSocketNotifier` wakes the
//! window without polling. LayerShellQt gives the `QQuickWindow` its overlay
//! layer, active-screen placement, and no-input semantics on Wayland.

use anyhow::{Context as _, Result};
use std::os::fd::IntoRawFd as _;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::{LayerEventSink, ListenerEvent, PendingTransition, Transition, spawn_raw_hid_listener};

const SHOW: u8 = 1;
const HIDE: u8 = 2;

#[derive(Clone)]
struct SocketSink {
    state: Arc<Mutex<SocketState>>,
}

struct SocketState {
    socket: UnixDatagram,
    pending: PendingTransition,
}

impl LayerEventSink for SocketSink {
    fn send(&self, event: ListenerEvent) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        state.pending.push(event);
        let packet = match state.pending.take() {
            Transition::Show { keyboard_id, layer } => [SHOW, keyboard_id, layer],
            Transition::Hide => [HIDE, 0, 0],
            Transition::Ignore => return true,
        };
        state.socket.send(&packet).is_ok()
    }
}

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let (reader, writer) = UnixDatagram::pair().context("Failed to create the Qt event socket")?;
    spawn_raw_hid_listener(SocketSink {
        state: Arc::new(Mutex::new(SocketState {
            socket: writer,
            pending: PendingTransition::default(),
        })),
    });

    let assets_dir = assets_dir
        .to_str()
        .context("The Qt asset directory is not valid UTF-8")?;
    keymap_overlay_qt_bridge::run_qt_overlay(assets_dir, reader.into_raw_fd())
        .context("The Qt overlay event loop failed")
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
    fn qt_receives_reduced_show_and_hide_datagrams() {
        let (reader, writer) = UnixDatagram::pair().expect("socket pair");
        let sink = SocketSink {
            state: Arc::new(Mutex::new(SocketState {
                socket: writer,
                pending: PendingTransition::default(),
            })),
        };
        let mut packet = [0; 3];

        assert!(sink.send(layer(true)));
        assert_eq!(reader.recv(&mut packet).expect("show packet"), 3);
        assert_eq!(packet, [SHOW, 2, 3]);

        assert!(sink.send(layer(false)));
        assert_eq!(reader.recv(&mut packet).expect("hide packet"), 3);
        assert_eq!(packet, [HIDE, 0, 0]);
    }
}
