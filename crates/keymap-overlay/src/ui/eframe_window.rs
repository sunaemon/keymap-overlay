//! Linux builds its own windows instead; see `wayland.rs` and `x11.rs`.

use anyhow::Result;
use eframe::egui::{self, ColorImage, TextureHandle, Vec2, ViewportCommand};
use log::warn;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};

use crate::{
    LayerEventSink, ListenerEvent, Transition, image_path, load_image, spawn_raw_hid_listener,
    transition_for_event,
};

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let (sender, receiver) = mpsc::channel();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_visible(false),
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
            Ok(Box::new(OverlayApp::new(assets_dir, receiver)))
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
    texture: Option<TextureHandle>,
    viewport_initialized: bool,
}

impl OverlayApp {
    fn new(assets_dir: PathBuf, receiver: Receiver<ListenerEvent>) -> Self {
        Self {
            assets_dir,
            receiver,
            held_keys: Vec::new(),
            texture: None,
            viewport_initialized: false,
        }
    }

    fn process_events(&mut self, context: &egui::Context) {
        while let Ok(event) = self.receiver.try_recv() {
            let transition = transition_for_event(&mut self.held_keys, event);
            match transition {
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
                context.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(
                    size[0] as f32,
                    size[1] as f32,
                )));
                context.send_viewport_cmd(ViewportCommand::Visible(true));
                context.request_repaint();
            }
            Err(error) => {
                warn!("Failed to load overlay image {}: {error:#}", path.display());
                // Stay hidden rather than leaving the previous layer on screen.
                self.hide(context);
            }
        }
    }

    fn hide(&mut self, context: &egui::Context) {
        context.send_viewport_cmd(ViewportCommand::Visible(false));
    }
}

impl eframe::App for OverlayApp {
    /// The events are drained here rather than in `ui` because eframe runs no
    /// egui pass at all while the viewport is hidden: it calls `logic` and
    /// stops. The overlay is hidden for all but the moment a layer key is
    /// held, so the press that has to bring it back arrives exactly then, and
    /// handling it from `ui` would leave the window hidden for good.
    fn logic(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        if !self.viewport_initialized {
            // macOS can show the native window even when its initial visibility is false.
            // Hide it explicitly on the first pass, before any layer notification arrives.
            context.send_viewport_cmd(ViewportCommand::Visible(false));
            self.viewport_initialized = true;
        }
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
