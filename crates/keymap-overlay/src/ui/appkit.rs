//! Native macOS overlay.
//!
//! AppKit owns the complete view hierarchy. An `NSGlassEffectView` supplies
//! the adaptive background and its content is built from native boxes and text
//! fields described by the installed JSON model. No key label is rasterized
//! into an intermediate image.

use anyhow::{Context, Result};
use dispatch::Queue;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBox, NSBoxType, NSColor,
    NSFont, NSGlassEffectView, NSGlassEffectViewStyle, NSMainMenuWindowLevel, NSScreen,
    NSTextAlignment, NSTextField, NSView, NSViewController, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize, NSString};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::{
    DisplayEncoder, LayerEventSink, ListenerEvent, ModelCache, OverlayModel, PendingTransition,
    Transition, compose_model, load_model_cache, spawn_raw_hid_listener,
};

const IDLE_SIZE: f64 = 1.0;
const GLASS_RADIUS: f64 = 22.0;
const KEY_RADIUS: f64 = 11.0;

#[derive(Clone)]
struct ChannelSink(Sender<ListenerEvent>);

impl LayerEventSink for ChannelSink {
    fn send(&self, event: ListenerEvent) -> bool {
        if self.0.send(event).is_err() {
            return false;
        }
        Queue::main().exec_async(|| {});
        true
    }
}

struct NativeLayer {
    view: Retained<NSView>,
    size: NSSize,
}

struct OverlayApp {
    receiver: Receiver<ListenerEvent>,
    pending: PendingTransition,
    models: ModelCache,
    layers: HashMap<(u8, Vec<u8>), NativeLayer>,
    window: Retained<NSWindow>,
    glass: Retained<NSGlassEffectView>,
    empty_view: Retained<NSView>,
}

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let mtm = MainThreadMarker::new().context("AppKit must run on the main thread")?;
    let application = NSApplication::sharedApplication(mtm);
    application.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let empty_view = NSView::initWithFrame(mtm.alloc(), idle_rect());
    let glass = NSGlassEffectView::initWithFrame(mtm.alloc(), idle_rect());
    glass.setStyle(NSGlassEffectViewStyle::Regular);
    glass.setCornerRadius(GLASS_RADIUS);
    glass.setContentView(Some(&empty_view));

    let controller = NSViewController::new(mtm);
    controller.setView(&glass);
    let window = NSWindow::windowWithContentViewController(&controller);
    configure_window(&window);

    let models = load_model_cache(&assets_dir)?;
    let (sender, receiver) = mpsc::channel();
    spawn_raw_hid_listener(ChannelSink(sender));

    let mut overlay = OverlayApp {
        receiver,
        pending: PendingTransition::default(),
        models,
        layers: HashMap::new(),
        window,
        glass,
        empty_view,
    };

    application.finishLaunching();
    overlay.window.orderFrontRegardless();
    overlay.run_event_loop()
}

fn configure_window(window: &NSWindow) {
    window.setStyleMask(NSWindowStyleMask::Borderless);
    window.setBackingType(NSBackingStoreType::Buffered);
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true);
    window.setLevel(NSMainMenuWindowLevel + 1);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::Stationary,
    );
    window.setFrame_display(idle_rect(), false);
}

fn build_native_layer(model: &OverlayModel, mtm: MainThreadMarker) -> NativeLayer {
    let size = NSSize::new(f64::from(model.width), f64::from(model.height));
    let root = NSView::initWithFrame(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), size));

    add_label(
        &root,
        &format!("L{}", model.layer),
        NSRect::new(
            NSPoint::new(20.0, size.height - 43.0),
            NSSize::new(80.0, 24.0),
        ),
        model.header_font_size,
        NSTextAlignment::Left,
        mtm,
    );

    for key in &model.keys {
        let frame = top_left_frame(key.x, key.y, key.width, key.height, model.height);
        add_key_surface(&root, frame, key.held, KEY_RADIUS, mtm);
        add_label(
            &root,
            &key.label.join("\n"),
            frame,
            model.key_font_size,
            NSTextAlignment::Center,
            mtm,
        );
    }

    for encoder in &model.encoders {
        add_encoder(&root, encoder, model, mtm);
    }

    NativeLayer { view: root, size }
}

fn add_key_surface(root: &NSView, frame: NSRect, held: bool, radius: f64, mtm: MainThreadMarker) {
    let surface = NSBox::initWithFrame(mtm.alloc(), frame);
    surface.setBoxType(NSBoxType::Custom);
    surface.setBorderWidth(0.75);
    surface.setBorderColor(&key_border_color());
    surface.setCornerRadius(radius);
    let fill = if held { held_color() } else { key_color() };
    surface.setFillColor(&fill);
    root.addSubview(&surface);
}

fn add_label(
    root: &NSView,
    text: &str,
    frame: NSRect,
    font_size: f64,
    alignment: NSTextAlignment,
    mtm: MainThreadMarker,
) {
    if text.is_empty() {
        return;
    }
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setAlignment(alignment);
    label.setFont(Some(&NSFont::systemFontOfSize(font_size)));
    label.setTextColor(Some(&text_color()));
    label.setMaximumNumberOfLines(3);
    // NSTextField vertically aligns its cell contents at the top of a tall
    // frame. Measure the native text first, then give the field only that
    // height and centre the field itself inside the requested area.
    label.sizeToFit();
    let text_height = label.frame().size.height.min(frame.size.height);
    label.setFrame(NSRect::new(
        NSPoint::new(
            frame.origin.x,
            frame.origin.y + (frame.size.height - text_height) / 2.0,
        ),
        NSSize::new(frame.size.width, text_height),
    ));
    root.addSubview(&label);
}

fn add_encoder(
    root: &NSView,
    encoder: &DisplayEncoder,
    model: &OverlayModel,
    mtm: MainThreadMarker,
) {
    let frame = top_left_frame(
        encoder.x,
        encoder.y,
        encoder.size,
        encoder.size,
        model.height,
    );
    add_key_surface(
        root,
        frame,
        encoder.held,
        f64::from(encoder.size) / 2.0,
        mtm,
    );

    let size = f64::from(encoder.size);
    let half = size / 2.0;
    let center_x = frame.origin.x + half;
    let label_half_width = size * 0.75;
    let label_gap = 3.0;
    let label_y = frame.origin.y + size + 2.0;
    add_label(
        root,
        &encoder_text("←", &encoder.counter_clockwise),
        NSRect::new(
            NSPoint::new(center_x - label_half_width, label_y),
            NSSize::new(label_half_width - label_gap, 18.0),
        ),
        model.encoder_font_size,
        NSTextAlignment::Right,
        mtm,
    );
    add_label(
        root,
        &encoder_text_trailing(&encoder.clockwise, "→"),
        NSRect::new(
            NSPoint::new(center_x + label_gap, label_y),
            NSSize::new(label_half_width - label_gap, 18.0),
        ),
        model.encoder_font_size,
        NSTextAlignment::Left,
        mtm,
    );
    if !encoder.press.is_empty() {
        add_label(
            root,
            &format!("P {}", encoder.press),
            frame,
            model.encoder_font_size,
            NSTextAlignment::Center,
            mtm,
        );
    }
}

fn encoder_text(arrow: &str, lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{arrow} {}", lines.join(" "))
    }
}

fn encoder_text_trailing(lines: &[String], arrow: &str) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{} {arrow}", lines.join(" "))
    }
}

fn top_left_frame(x: u32, y: u32, width: u32, height: u32, canvas_height: u32) -> NSRect {
    NSRect::new(
        NSPoint::new(f64::from(x), f64::from(canvas_height - y - height)),
        NSSize::new(f64::from(width), f64::from(height)),
    )
}

fn key_color() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        220.0 / 255.0,
        224.0 / 255.0,
        231.0 / 255.0,
        246.0 / 255.0,
    )
}

fn key_border_color() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        32.0 / 255.0,
        36.0 / 255.0,
        44.0 / 255.0,
        31.0 / 255.0,
    )
}

fn held_color() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 221.0 / 255.0, 221.0 / 255.0, 1.0)
}

fn text_color() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(32.0 / 255.0, 36.0 / 255.0, 44.0 / 255.0, 1.0)
}

impl OverlayApp {
    fn run_event_loop(&mut self) -> Result<()> {
        let run_loop = NSRunLoop::mainRunLoop();
        let distant_future = NSDate::distantFuture();
        let default_mode = NSString::from_str("kCFRunLoopDefaultMode");
        loop {
            run_loop.runMode_beforeDate(&default_mode, &distant_future);

            for event in self.receiver.try_iter() {
                self.pending.push(event);
            }
            match self.pending.take() {
                Transition::Show {
                    keyboard_id,
                    layers,
                } => self.show(keyboard_id, &layers),
                Transition::Hide => self.hide(),
                Transition::Ignore => {}
            }
        }
    }

    fn show(&mut self, keyboard_id: u8, layers: &[u8]) {
        let key = (keyboard_id, layers.to_vec());
        if !self.layers.contains_key(&key) {
            let Some(model) = compose_model(&self.models, keyboard_id, layers) else {
                log::warn!(
                    "Overlay model is unavailable for keyboard {keyboard_id}, layers {layers:?}"
                );
                self.hide();
                return;
            };
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            self.layers
                .insert(key.clone(), build_native_layer(&model, mtm));
        }
        let Some(native) = self.layers.get(&key) else {
            log::warn!(
                "Overlay model is unavailable for keyboard {keyboard_id}, layers {layers:?}"
            );
            self.hide();
            return;
        };
        self.glass.setContentView(Some(&native.view));
        self.window
            .setFrame_display(centered_frame(native.size), true);
        self.window.orderFrontRegardless();
    }

    fn hide(&self) {
        self.glass.setContentView(Some(&self.empty_view));
        self.window.setFrame_display(idle_rect(), false);
    }
}

fn centered_frame(size: NSSize) -> NSRect {
    let Some(screen) = MainThreadMarker::new().and_then(NSScreen::mainScreen) else {
        return NSRect::new(NSPoint::new(0.0, 0.0), size);
    };
    let screen = screen.frame();
    NSRect::new(
        NSPoint::new(
            screen.origin.x + (screen.size.width - size.width) / 2.0,
            screen.origin.y + (screen.size.height - size.height) / 2.0,
        ),
        size,
    )
}

fn idle_rect() -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(IDLE_SIZE, IDLE_SIZE))
}
