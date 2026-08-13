//! The macOS window. Linux builds its own windows instead; see `wayland.rs`
//! and `x11.rs`.
//!
//! The macOS window stays mapped for the lifetime of the process. Mapping a
//! transparent native window can briefly show its backing clear colour and
//! macOS applies the normal window-show animation, neither of which belongs in
//! an overlay that appears on every layer press. "Hidden" therefore means
//! drawing no texture into a fully transparent, click-through window.
//!
//! Everything that is not specific to macOS lives in `eframe_common.rs`, which
//! this file hands its one platform difference: the activation policy.

use anyhow::Result;
use std::path::PathBuf;
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

use crate::ui::eframe_common::{self, PlatformHooks, base_viewport};

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    eframe_common::run(
        assets_dir,
        PlatformHooks {
            native_options,
            // Nothing to do before a logic pass, and hiding is exactly dropping
            // the texture; both of those are the shared behaviour.
            before_logic: |_| {},
            after_hide: |_| {},
        },
    )
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: base_viewport(),
        // An overlay is not an application you switch to: it belongs beside
        // the keyboard, not in the Dock or the menu bar. macOS gives an
        // unbundled binary the regular policy, which parks an icon in the Dock
        // for as long as the login service runs. Accessory keeps the windows
        // and drops the rest.
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        ..Default::default()
    }
}
