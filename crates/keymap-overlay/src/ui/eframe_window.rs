//! Linux builds its own windows instead; see `wayland.rs` and `x11.rs`.
//!
//! The macOS window stays mapped for the lifetime of the process. Mapping a
//! transparent native window can briefly show its backing clear colour and
//! macOS applies the normal window-show animation, neither of which belongs in
//! an overlay that appears on every layer press. "Hidden" therefore means
//! drawing no texture into a fully transparent, click-through window.

use anyhow::Result;
use eframe::egui::{self, ColorImage, Pos2, TextureHandle, Vec2, ViewportCommand};
use log::warn;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

use crate::{
    LayerEventSink, ListenerEvent, Transition, image_path, load_image_cache,
    spawn_raw_hid_listener, transition_for_events,
};

/// Keep the always-mapped window effectively absent before its first image.
const IDLE_SIZE: f32 = 1.0;

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let (sender, receiver) = mpsc::channel();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_active(false)
            .with_inner_size([IDLE_SIZE, IDLE_SIZE]),
        // An overlay is not an application you switch to: it belongs beside
        // the keyboard, not in the Dock or the menu bar. macOS gives an
        // unbundled binary the regular policy, which parks an icon in the Dock
        // for as long as the login service runs. Accessory keeps the windows
        // and drops the rest.
        event_loop_builder: Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
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
            Ok(Box::new(OverlayApp::new(
                creation.egui_ctx.clone(),
                assets_dir,
                receiver,
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
    held_keys: Vec<(u8, u8)>,
    textures: HashMap<(u8, u8), TextureHandle>,
    texture: Option<(u8, u8)>,
}

impl OverlayApp {
    fn new(
        context: egui::Context,
        assets_dir: PathBuf,
        receiver: Receiver<ListenerEvent>,
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
            held_keys: Vec::new(),
            textures,
            texture: None,
        })
    }

    fn process_events(&mut self, context: &egui::Context) {
        match transition_for_events(&mut self.held_keys, self.receiver.try_iter()) {
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

    fn hide(&mut self, context: &egui::Context) {
        self.texture = None;
        context.request_repaint();
    }
}

/// Places a newly sized overlay in the middle of the primary monitor.
fn center_on_monitor(context: &egui::Context, size: Vec2) {
    let Some(monitor) = context.input(|input| input.viewport().monitor_size) else {
        return;
    };
    let position = Pos2::new(
        ((monitor.x - size.x) / 2.0).max(0.0),
        ((monitor.y - size.y) / 2.0).max(0.0),
    );
    context.send_viewport_cmd(ViewportCommand::OuterPosition(position));
}

impl eframe::App for OverlayApp {
    /// Fully transparent because the native window remains mapped while idle.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    /// Events are drained in the logic pass so listener wakeups remain
    /// independent of painting.
    fn logic(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
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
