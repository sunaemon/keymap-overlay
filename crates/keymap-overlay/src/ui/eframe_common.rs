//! The shared eframe implementation used by the Windows backend.

use anyhow::Result;
use eframe::egui::{self, ColorImage, Pos2, TextureHandle, Vec2, ViewportCommand};
use log::warn;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::{
    LayerEventSink, ListenerEvent, PendingTransition, Transition, image_path, load_image_cache,
    spawn_raw_hid_listener,
};

/// The size of a hidden overlay: one transparent pixel. Neither window can be
/// unmapped, so this is as absent as a mapped window gets.
pub(crate) const IDLE_SIZE: f32 = 1.0;

pub(crate) struct PlatformHooks {
    pub(crate) native_options: fn() -> eframe::NativeOptions,
    /// Runs at the top of every logic pass, before events are drained.
    pub(crate) before_logic: fn(&egui::Context),
}

/// The viewport properties an overlay needs on either system.
///
/// Undecorated, transparent, above everything and invisible to the pointer, and
/// built inactive so that the first show does not take focus.
pub(crate) fn base_viewport() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_mouse_passthrough(true)
        .with_active(false)
        .with_inner_size([IDLE_SIZE, IDLE_SIZE])
}

pub(crate) fn run(assets_dir: PathBuf, hooks: PlatformHooks) -> Result<()> {
    let (sender, receiver) = mpsc::channel();

    eframe::run_native(
        "Keymap Overlay",
        (hooks.native_options)(),
        Box::new(move |creation| {
            // The listener needs the egui context so it can wake the UI thread
            // on a layer event; without it the app would have to poll.
            spawn_raw_hid_listener(RepaintSink {
                sender,
                context: creation.egui_ctx.clone(),
            });
            Ok(Box::new(OverlayApp::new(
                creation.egui_ctx.clone(),
                assets_dir,
                receiver,
                hooks,
            )?))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Failed to start the keymap overlay: {error}"))
}

#[derive(Clone)]
struct RepaintSink {
    sender: Sender<ListenerEvent>,
    context: egui::Context,
}

impl LayerEventSink for RepaintSink {
    fn send(&self, event: ListenerEvent) -> bool {
        if self.sender.send(event).is_err() {
            return false;
        }
        self.context.request_repaint();
        true
    }
}

struct OverlayApp {
    assets_dir: PathBuf,
    receiver: Receiver<ListenerEvent>,
    pending: PendingTransition,
    textures: HashMap<(u8, u8), TextureHandle>,
    texture: Option<(u8, u8)>,
    hooks: PlatformHooks,
}

impl OverlayApp {
    fn new(
        context: egui::Context,
        assets_dir: PathBuf,
        receiver: Receiver<ListenerEvent>,
        hooks: PlatformHooks,
    ) -> Result<Self> {
        let textures = load_image_cache(&assets_dir)?
            .into_iter()
            .map(|(key, image)| {
                let size = [image.width() as usize, image.height() as usize];
                let image = ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                let texture = context.load_texture(
                    format!("{}_L{}", key.0, key.1),
                    image,
                    Default::default(),
                );
                (key, texture)
            })
            .collect();
        Ok(Self {
            assets_dir,
            receiver,
            pending: PendingTransition::default(),
            textures,
            texture: None,
            hooks,
        })
    }

    fn process_events(&mut self, context: &egui::Context) {
        for event in self.receiver.try_iter() {
            self.pending.push(event);
        }
        match self.pending.take() {
            Transition::Show { keyboard_id, layer } => {
                self.show_layer(context, keyboard_id, layer);
            }
            Transition::Hide => self.hide(context),
            Transition::Ignore => {}
        }
    }

    fn show_layer(&mut self, context: &egui::Context, keyboard_id: u8, layer: u8) {
        let path = image_path(&self.assets_dir, keyboard_id, layer);
        match self.textures.get(&(keyboard_id, layer)) {
            Some(texture) => {
                let size = texture.size();
                let size_changed = self
                    .texture
                    .and_then(|key| self.textures.get(&key))
                    .is_none_or(|current| current.size() != size);
                self.texture = Some((keyboard_id, layer));
                if size_changed {
                    let size = Vec2::new(size[0] as f32, size[1] as f32);
                    context.send_viewport_cmd(ViewportCommand::InnerSize(size));
                    center_on_monitor(context, size);
                }
                context.request_repaint();
            }
            None => {
                warn!("Overlay image is unavailable: {}", path.display());
                // Stay hidden rather than leaving the previous layer on screen.
                self.hide(context);
            }
        }
    }

    /// Drops the image and shrinks the window back to [`IDLE_SIZE`].
    ///
    /// Neither system can unmap this window, so hiding is drawing nothing. The
    /// shrink is what keeps that from leaving a full-size invisible window on
    /// top of everything: mouse passthrough is the only thing stopping such a
    /// window from taking clicks across its whole rectangle, and a single pixel
    /// is a much smaller thing to be wrong about.
    ///
    /// It costs nothing on the next show. `show_layer` derives `size_changed`
    /// from `texture`, which is cleared here, so the show after a hide re-sends
    /// the size and re-centres either way.
    fn hide(&mut self, context: &egui::Context) {
        self.texture = None;
        context.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(IDLE_SIZE, IDLE_SIZE)));
        context.request_repaint();
    }
}

/// Places a newly sized overlay in the middle of the primary monitor.
///
/// `ViewportCommand::center_on_screen` would be the obvious call, but it centres
/// on the window's *current* `outer_rect`, which at this point is still the
/// previous layer's size — the resize above has not been applied yet. The size
/// is known here, so the position is computed from it directly.
///
/// Unlike `x11.rs::centered_position`, which adds `MonitorHandle::position()`,
/// this cannot centre on the monitor the window happens to be on: `ViewportInfo`
/// carries `monitor_size` but no monitor origin, while `OuterPosition` is in
/// virtual-desktop coordinates. On a multi-monitor desktop the overlay therefore
/// lands on the primary monitor. Fixing it needs the current monitor's rect from
/// outside egui.
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
    /// Fully transparent, unlike eframe's translucent-grey default, because both
    /// windows stay mapped while idle: anything opaque here would sit on screen
    /// for the whole session.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    /// Events are drained in the logic pass, not in `ui`: eframe runs no egui
    /// pass while a viewport is hidden, which is where the macOS overlay sits
    /// between key holds, so work put in `ui` would never run when the press
    /// that shows it arrives.
    fn logic(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        (self.hooks.before_logic)(context);
        self.process_events(context);
    }

    /// The `Ui` eframe hands over already has no margin and no background,
    /// which is all the central panel with a transparent `Frame::NONE` was
    /// there for, so the image is drawn straight into it.
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        if let Some(texture) = self.texture.and_then(|key| self.textures.get(&key)) {
            ui.image(texture);
        }
        // No periodic repaint: the Raw HID listener wakes the UI thread when a
        // layer event arrives, so an idle overlay costs nothing.
    }
}
