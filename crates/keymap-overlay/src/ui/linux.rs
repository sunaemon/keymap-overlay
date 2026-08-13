//! Picks which of the two Linux windows to open.
//!
//! A layer surface is the only kind of Wayland window that can behave the way
//! this overlay has to, so it wins whenever the compositor offers one. Where it
//! does not — GNOME above all, which has no `zwlr_layer_shell_v1` — the X11
//! window is the fallback, reached through XWayland in a Wayland session and
//! directly in an X11 one. It is a fallback rather than an equal: it is
//! override-redirect, so nothing places it or keeps it stacked for it, and in a
//! Wayland session it needs XWayland to be running at all. `x11.rs` has the
//! reasoning for going unmanaged rather than asking for `_NET_WM_STATE_ABOVE`.
//!
//! `KEYMAP_OVERLAY_BACKEND` overrides the choice, which is also the only way to
//! exercise the fallback on a machine whose compositor does support layer-shell.

use anyhow::{Result, bail};
use log::{info, warn};
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::ui::{wayland, x11};

const BACKEND_ENV: &str = "KEYMAP_OVERLAY_BACKEND";

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    match resolve_backend(env::var_os(BACKEND_ENV))? {
        Backend::LayerShell => run_layer_shell(assets_dir),
        Backend::X11 => run_x11(assets_dir),
        Backend::Auto => {
            if wayland::is_available() {
                run_layer_shell(assets_dir)
            } else {
                warn!(
                    "This compositor does not implement zwlr_layer_shell_v1, so the overlay falls back to an X11 window; it may not stay above other windows"
                );
                run_x11(assets_dir)
            }
        }
    }
}

/// Which window to open, before asking the compositor what it supports.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    Auto,
    LayerShell,
    X11,
}

fn resolve_backend(configured: Option<OsString>) -> Result<Backend> {
    let Some(configured) = configured else {
        return Ok(Backend::Auto);
    };
    match configured.to_str() {
        Some("auto") => Ok(Backend::Auto),
        Some("layer-shell") => Ok(Backend::LayerShell),
        Some("x11") => Ok(Backend::X11),
        _ => bail!("{BACKEND_ENV} must be auto, layer-shell or x11, not {configured:?}"),
    }
}

fn run_layer_shell(assets_dir: PathBuf) -> Result<()> {
    info!("Opening the Wayland layer-shell overlay");
    wayland::run(assets_dir)
}

fn run_x11(assets_dir: PathBuf) -> Result<()> {
    // winit would fail on its own, but with an error about the display
    // connection that says nothing about why an X11 window was wanted here.
    if env::var_os("DISPLAY").is_none() {
        bail!(
            "The X11 overlay needs DISPLAY, which is not set; in a Wayland session without zwlr_layer_shell_v1 the fallback needs XWayland running"
        );
    }
    info!("Opening the X11 overlay");
    x11::run(assets_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_variable_leaves_the_choice_to_the_compositor() {
        assert_eq!(
            resolve_backend(None).expect("unset is valid"),
            Backend::Auto
        );
    }

    #[test]
    fn each_backend_can_be_demanded_by_name() {
        for (value, expected) in [
            ("auto", Backend::Auto),
            ("layer-shell", Backend::LayerShell),
            ("x11", Backend::X11),
        ] {
            assert_eq!(
                resolve_backend(Some(OsString::from(value))).expect("a documented value"),
                expected
            );
        }
    }

    /// A typo must not silently open the wrong window, which on Wayland would
    /// look like the overlay being broken rather than misconfigured.
    #[test]
    fn an_unknown_backend_is_an_error() {
        assert!(resolve_backend(Some(OsString::from("wayland"))).is_err());
        assert!(resolve_backend(Some(OsString::from(""))).is_err());
    }
}
