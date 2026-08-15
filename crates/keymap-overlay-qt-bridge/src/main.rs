//! Qt renderer client for the Linux keymap overlay daemon.

#[cfg(target_os = "linux")]
mod linux {
    use anyhow::{Context as _, Result};
    use std::env;

    pub(super) fn run() -> Result<()> {
        if is_gnome_desktop() && env::var_os("KEYMAP_OVERLAY_FORCE_QT").is_none() {
            return Ok(());
        }

        keymap_overlay_qt_bridge::run_qt_overlay().context("The Qt overlay event loop failed")
    }

    fn is_gnome_desktop() -> bool {
        env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
            desktop
                .split(':')
                .any(|part| part.eq_ignore_ascii_case("gnome"))
        })
    }

    #[cfg(test)]
    mod tests {
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
    }
}

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {}
