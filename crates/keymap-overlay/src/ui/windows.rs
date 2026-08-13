//! The Windows window, an eframe/egui window like the macOS one.
//!
//! It differs from `eframe_window.rs` in one structural way: **this window is
//! mapped once and never hidden again.** Hiding it would be the natural thing
//! to do, and it is what every other backend does, but on Windows it steals
//! focus. `ViewportCommand::Visible(true)` reaches `winit`'s
//! `WindowFlags::VISIBLE`, and winit only issues `SW_SHOWNOACTIVATE` for the
//! *first* show of a window built with `with_active(false)`; it flips the
//! marker and every later show is a plain `SW_SHOW`, which activates. The
//! overlay shows and hides on every layer key hold, so from the second press
//! onward the window would take focus and swallow the very keystrokes the
//! layer key was held for — the failure `doc/design.md` describes for X11.
//!
//! So "hidden" here means *drawing nothing*: the window keeps its place in the
//! stack, transparent and click-through, and `after_hide` shrinks it back to a
//! single pixel. Two things follow, and both are load-bearing:
//!
//! - `clear_color` must be fully transparent. eframe's default is a translucent
//!   dark grey, which no other backend ever shows because they all unmap; here
//!   it would be a permanent grey rectangle over the screen. That is shared
//!   behaviour in `eframe_common.rs`, because macOS stays mapped too.
//! - resizing must not activate either, which holds: winit's resize path passes
//!   `SWP_NOACTIVATE`.
//!
//! Everything not specific to Windows lives in `eframe_common.rs`; the three
//! differences are the hooks below.

use anyhow::Result;
use eframe::egui::{self, Vec2, ViewportCommand};
use std::path::PathBuf;

use crate::ui::eframe_common::{self, IDLE_SIZE, PlatformHooks, base_viewport};

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    eframe_common::run(
        assets_dir,
        PlatformHooks {
            native_options,
            before_logic: pin_pixels_per_point,
            after_hide: shrink_to_idle,
        },
    )
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: base_viewport()
            // An overlay is not an application you switch to, so it belongs in
            // neither the taskbar nor the alt-tab list.
            .with_taskbar(false),
        ..Default::default()
    }
}

/// Layer images are presented at their own pixel size, as on the other systems
/// — `DPI` in the Makefile is where an image is sized for a screen. Windows
/// reports a scale factor for the display, which egui would otherwise apply to
/// both the image and the viewport sizes the shared code sends, so it is pinned
/// to 1:1 here instead.
///
/// Every frame, not once: this sets a *zoom factor* of 1/scale_factor, which
/// egui multiplies by the live scale factor on each pass. Pinning once would
/// come undone the moment the overlay met a monitor scaled differently from the
/// one it started on. The call returns early when the value is already 1:1, so
/// repeating it is free.
fn pin_pixels_per_point(context: &egui::Context) {
    context.set_pixels_per_point(1.0);
}

/// The window cannot be unmapped, so a hidden overlay is shrunk instead: it has
/// to stay mapped, and it may as well cover as little as possible until the next
/// layer image gives it a size.
fn shrink_to_idle(context: &egui::Context) {
    context.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(IDLE_SIZE, IDLE_SIZE)));
}
