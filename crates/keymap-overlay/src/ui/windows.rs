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
//! stack, transparent and click-through, and `hide` drops the texture. Two
//! things follow, and both are load-bearing:
//!
//! - `clear_color` must be fully transparent. eframe's default is a translucent
//!   dark grey, which no other backend ever shows because they all unmap; here
//!   it would be a permanent grey rectangle over the screen.
//! - resizing must not activate either, which holds: winit's resize path passes
//!   `SWP_NOACTIVATE`.

use anyhow::Result;
use eframe::egui::{self, ColorImage, Pos2, TextureHandle, Vec2, ViewportCommand};
use keymap_core::RawLayerEvent;
use log::warn;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::{
    LayerEventSink, Transition, image_path, load_image, spawn_raw_hid_listener, transition_for,
};

/// The idle window is one transparent pixel: it has to stay mapped, so it may
/// as well cover as little as possible until a layer image gives it a size.
const IDLE_SIZE: f32 = 1.0;

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let (sender, receiver) = mpsc::channel();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            // An overlay is not an application you switch to, so it belongs in
            // neither the taskbar nor the alt-tab list.
            .with_taskbar(false)
            // The one show this window ever gets must not take focus; see the
            // module comment for why this is a one-shot on Windows.
            .with_active(false)
            .with_inner_size([IDLE_SIZE, IDLE_SIZE]),
        ..Default::default()
    };
    eframe::run_native(
        "Keymap Overlay",
        options,
        Box::new(move |creation| {
            // The listener needs the egui context so it can wake the UI thread
            // on a layer event; without it the app would have to poll.
            spawn_raw_hid_listener(RepaintSink {
                sender,
                context: creation.egui_ctx.clone(),
            });
            Ok(Box::new(OverlayApp::new(assets_dir, receiver)))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Failed to start the keymap overlay: {error}"))
}

#[derive(Clone)]
struct RepaintSink {
    sender: Sender<RawLayerEvent>,
    context: egui::Context,
}

impl LayerEventSink for RepaintSink {
    fn send(&self, event: RawLayerEvent) -> bool {
        if self.sender.send(event).is_err() {
            return false;
        }
        self.context.request_repaint();
        true
    }
}

struct OverlayApp {
    assets_dir: PathBuf,
    receiver: Receiver<RawLayerEvent>,
    held_keys: Vec<(u8, u8)>,
    texture: Option<TextureHandle>,
}

impl OverlayApp {
    fn new(assets_dir: PathBuf, receiver: Receiver<RawLayerEvent>) -> Self {
        Self {
            assets_dir,
            receiver,
            held_keys: Vec::new(),
            texture: None,
        }
    }

    fn process_events(&mut self, context: &egui::Context) {
        while let Ok(event) = self.receiver.try_recv() {
            match transition_for(&mut self.held_keys, event) {
                Transition::Show { keyboard_id, layer } => {
                    self.show_layer(context, keyboard_id, layer);
                }
                Transition::Hide => self.hide(context),
                Transition::Ignore => {}
            }
        }
    }

    fn show_layer(&mut self, context: &egui::Context, keyboard_id: u8, layer: u8) {
        let path = image_path(&self.assets_dir, keyboard_id, layer);
        match load_image(&path) {
            Ok(image) => {
                let size = [image.width() as usize, image.height() as usize];
                let image = ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                self.texture = Some(context.load_texture(
                    path.display().to_string(),
                    image,
                    Default::default(),
                ));
                let size = Vec2::new(size[0] as f32, size[1] as f32);
                context.send_viewport_cmd(ViewportCommand::InnerSize(size));
                center_on_monitor(context, size);
                context.request_repaint();
            }
            Err(error) => {
                warn!("Failed to load overlay image {}: {error:#}", path.display());
                // Stay hidden rather than leaving the previous layer on screen.
                self.hide(context);
            }
        }
    }

    /// Drops the image rather than hiding the window; see the module comment.
    fn hide(&mut self, context: &egui::Context) {
        self.texture = None;
        context.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(IDLE_SIZE, IDLE_SIZE)));
        context.request_repaint();
    }
}

/// Places the window in the middle of the primary monitor.
///
/// `ViewportCommand::center_on_screen` would be the obvious call, but it
/// centres on the window's *current* `outer_rect`, which at this point is still
/// the previous layer's size — the resize above has not been applied yet. The
/// size is known here, so the position is computed from it directly.
///
/// Unlike `x11.rs::centered_position`, which adds `MonitorHandle::position()`,
/// this cannot centre on the monitor the window happens to be on:
/// `ViewportInfo` carries `monitor_size` but no monitor origin, while
/// `OuterPosition` is in virtual-desktop coordinates. On a multi-monitor
/// desktop the overlay therefore lands on the primary monitor. Fixing it needs
/// the current monitor's rect from outside egui.
fn center_on_monitor(context: &egui::Context, size: Vec2) {
    let Some(monitor) = context.input(|input| input.viewport().monitor_size) else {
        // Nothing to centre against; the window stays where it was, which is
        // better than moving it to a corner.
        return;
    };
    let position = Pos2::new(
        ((monitor.x - size.x) / 2.0).max(0.0),
        ((monitor.y - size.y) / 2.0).max(0.0),
    );
    context.send_viewport_cmd(ViewportCommand::OuterPosition(position));
}

impl eframe::App for OverlayApp {
    /// Fully transparent, unlike eframe's translucent-grey default, because
    /// this window is always mapped: anything opaque here would sit on screen
    /// for the whole session.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    /// Events are drained here to stay in step with `eframe_window.rs`, where
    /// draining from `ui` would not work at all. This window is never hidden,
    /// so an egui pass does run either way, but there is nothing to gain from
    /// the two backends disagreeing about where the events are handled.
    fn logic(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        // Layer images are presented at their own pixel size, as on the other
        // systems — `DPI` in the Makefile is where an image is sized for a
        // screen. Windows reports a scale factor for the display, which egui
        // would otherwise apply to both the image and the sizes sent above, so
        // it is pinned to 1:1 here instead.
        //
        // Every frame, not once: this sets a *zoom factor* of
        // 1/scale_factor, which egui multiplies by the live scale factor on
        // each pass. Pinning once would come undone the moment the overlay met
        // a monitor scaled differently from the one it started on. The call
        // returns early when the value is already 1:1, so repeating it is free.
        context.set_pixels_per_point(1.0);
        self.process_events(context);
    }

    /// The `Ui` eframe hands over already has no margin and no background,
    /// which is all the central panel with a transparent `Frame::NONE` was
    /// there for, so the image is drawn straight into it.
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        if let Some(texture) = &self.texture {
            ui.image(texture);
        }
        // No periodic repaint: the Raw HID listener wakes the UI thread when a
        // layer event arrives, so an idle overlay costs nothing.
    }
}
