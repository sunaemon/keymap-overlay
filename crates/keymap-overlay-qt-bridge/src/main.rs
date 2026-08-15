//! Qt renderer client for the Linux keymap overlay daemon.

#[cfg(target_os = "linux")]
mod linux {
    use anyhow::{Context as _, Result};
    use keymap_overlay_linux_protocol::{RendererProxyBlocking, RendererState, decode_state};
    use std::env;
    use std::os::fd::IntoRawFd as _;
    use std::os::unix::net::UnixDatagram;
    use std::thread;
    use zbus::blocking::Connection;

    const HIDE: u8 = 2;
    const RELAY_FAILED: u8 = 3;

    pub(super) fn run() -> Result<()> {
        if is_gnome_desktop() && env::var_os("KEYMAP_OVERLAY_FORCE_QT").is_none() {
            return Ok(());
        }

        let (reader, writer) =
            UnixDatagram::pair().context("Failed to create the Qt event socket")?;
        thread::spawn(move || {
            if let Err(error) = relay_renderer_state(&writer) {
                eprintln!("Qt renderer stopped: {error:#}");
                let _ = writer.send(&[RELAY_FAILED]);
            }
        });
        keymap_overlay_qt_bridge::run_qt_overlay(reader.into_raw_fd())
            .context("The Qt overlay event loop failed")
    }

    fn is_gnome_desktop() -> bool {
        env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
            desktop
                .split(':')
                .any(|part| part.eq_ignore_ascii_case("gnome"))
        })
    }

    fn relay_renderer_state(socket: &UnixDatagram) -> Result<()> {
        let connection =
            Connection::session().context("Failed to connect to the user D-Bus session")?;
        let proxy = RendererProxyBlocking::new(&connection)
            .context("Failed to create the renderer state proxy")?;
        let signals = proxy
            .receive_state_changed()
            .context("Failed to subscribe to renderer state")?;
        let initial = proxy
            .get_state()
            .context("Failed to read the initial renderer state")?;
        let mut generation = initial.0;
        send_state(socket, &initial)?;

        for message in signals {
            let state = decode_state(&message).context("Failed to decode renderer state")?;
            if state.0 <= generation {
                continue;
            }
            generation = state.0;
            send_state(socket, &state)?;
        }
        Ok(())
    }

    fn send_state(socket: &UnixDatagram, state: &RendererState) -> Result<()> {
        let packet = if state.1 { state.2.as_bytes() } else { &[HIDE] };
        socket
            .send(packet)
            .context("Failed to forward renderer state to Qt")?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn desktop_names_match_only_gnome_components() {
            assert!(
                "ubuntu:GNOME"
                    .split(':')
                    .any(|part| part.eq_ignore_ascii_case("gnome"))
            );
            assert!(
                !"X-Cinnamon"
                    .split(':')
                    .any(|part| part.eq_ignore_ascii_case("gnome"))
            );
        }

        #[test]
        fn state_packets_carry_json_or_hide() {
            let (reader, writer) = UnixDatagram::pair().expect("socket pair");
            let mut packet = [0; 32];

            send_state(&writer, &(1, true, "{\"version\":1}".into())).expect("show state");
            let count = reader.recv(&mut packet).expect("show packet");
            assert_eq!(&packet[..count], br#"{"version":1}"#);

            send_state(&writer, &(2, false, String::new())).expect("hide state");
            let count = reader.recv(&mut packet).expect("hide packet");
            assert_eq!(&packet[..count], &[HIDE]);
        }

        #[test]
        fn relay_failure_packet_is_distinct_from_state_packets() {
            assert_ne!(RELAY_FAILED, HIDE);
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {}
