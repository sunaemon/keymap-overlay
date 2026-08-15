//! Native macOS overlay.
//!
//! AppKit owns the complete view hierarchy. An `NSGlassEffectView` supplies
//! the adaptive background and its content is built from native boxes and text
//! fields described by the installed JSON model. No key label is rasterized
//! into an intermediate image.

use anyhow::{Context, Result, bail};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBox, NSBoxType, NSColor,
    NSFont, NSGlassEffectView, NSGlassEffectViewStyle, NSMainMenuWindowLevel, NSScreen,
    NSTextAlignment, NSTextField, NSView, NSViewController, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize, NSString};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::{LayerEventSink, ListenerEvent, PendingTransition, Transition, spawn_raw_hid_listener};

const IDLE_SIZE: f64 = 1.0;
const GLASS_RADIUS: f64 = 22.0;
const KEY_RADIUS: f64 = 11.0;

#[derive(Clone)]
struct ChannelSink(Sender<ListenerEvent>);

impl LayerEventSink for ChannelSink {
    fn send(&self, event: ListenerEvent) -> bool {
        self.0.send(event).is_ok()
    }
}

#[derive(Deserialize)]
struct OverlayModel {
    version: u8,
    layer: u8,
    width: u32,
    height: u32,
    header_font_size: f64,
    key_font_size: f64,
    encoder_font_size: f64,
    keys: Vec<DisplayKey>,
    encoders: Vec<DisplayEncoder>,
}

#[derive(Deserialize)]
struct DisplayKey {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    label: Vec<String>,
    held: bool,
}

#[derive(Deserialize)]
struct DisplayEncoder {
    x: u32,
    y: u32,
    size: u32,
    counter_clockwise: Vec<String>,
    clockwise: Vec<String>,
    press: String,
    held: bool,
}

struct NativeLayer {
    view: Retained<NSView>,
    size: NSSize,
}

struct OverlayApp {
    assets_dir: PathBuf,
    receiver: Receiver<ListenerEvent>,
    pending: PendingTransition,
    layers: HashMap<(u8, u8), NativeLayer>,
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

    let layers = load_native_layers(&assets_dir, mtm)?;
    let (sender, receiver) = mpsc::channel();
    spawn_raw_hid_listener(ChannelSink(sender));

    let mut overlay = OverlayApp {
        assets_dir,
        receiver,
        pending: PendingTransition::default(),
        layers,
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

fn load_native_layers(
    assets_dir: &Path,
    mtm: MainThreadMarker,
) -> Result<HashMap<(u8, u8), NativeLayer>> {
    let mut layers = HashMap::new();
    for entry in fs::read_dir(assets_dir)
        .with_context(|| format!("Failed to read asset directory {}", assets_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(key) = model_key(&path) else {
            continue;
        };
        let model: OverlayModel = serde_json::from_reader(
            fs::File::open(&path).with_context(|| format!("Failed to open {}", path.display()))?,
        )
        .with_context(|| format!("Failed to parse {}", path.display()))?;
        if model.version != 1 {
            bail!(
                "Unsupported overlay model version {} in {}",
                model.version,
                path.display()
            );
        }
        if model.layer != key.1 {
            bail!("Layer in {} does not match its filename", path.display());
        }
        layers.insert(key, build_native_layer(&model, mtm));
    }
    Ok(layers)
}

fn model_key(path: &Path) -> Option<(u8, u8)> {
    if path.extension()?.to_str()? != "json" {
        return None;
    }
    let (keyboard_id, layer) = path.file_stem()?.to_str()?.split_once("_L")?;
    Some((keyboard_id.parse().ok()?, layer.parse().ok()?))
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
    surface.setBorderWidth(0.0);
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

    let half = f64::from(encoder.size) / 2.0;
    let y = frame.origin.y;
    let x = frame.origin.x;
    add_label(
        root,
        &encoder_text("↶", &encoder.counter_clockwise),
        NSRect::new(NSPoint::new(x, y + half - 5.0), NSSize::new(half, half)),
        model.encoder_font_size,
        NSTextAlignment::Center,
        mtm,
    );
    add_label(
        root,
        &encoder_text("↷", &encoder.clockwise),
        NSRect::new(
            NSPoint::new(x + half, y + half - 5.0),
            NSSize::new(half, half),
        ),
        model.encoder_font_size,
        NSTextAlignment::Center,
        mtm,
    );
    if !encoder.press.is_empty() {
        add_label(
            root,
            &format!("P {}", encoder.press),
            NSRect::new(
                NSPoint::new(x, y + 5.0),
                NSSize::new(f64::from(encoder.size), half),
            ),
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

fn top_left_frame(x: u32, y: u32, width: u32, height: u32, canvas_height: u32) -> NSRect {
    NSRect::new(
        NSPoint::new(f64::from(x), f64::from(canvas_height - y - height)),
        NSSize::new(f64::from(width), f64::from(height)),
    )
}

fn key_color() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        232.0 / 255.0,
        235.0 / 255.0,
        240.0 / 255.0,
        248.0 / 255.0,
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
        loop {
            let deadline = NSDate::dateWithTimeIntervalSinceNow(0.01);
            run_loop.runUntilDate(&deadline);

            for event in self.receiver.try_iter() {
                self.pending.push(event);
            }
            match self.pending.take() {
                Transition::Show { keyboard_id, layer } => self.show(keyboard_id, layer),
                Transition::Hide => self.hide(),
                Transition::Ignore => {}
            }
        }
    }

    fn show(&self, keyboard_id: u8, layer: u8) {
        let Some(native) = self.layers.get(&(keyboard_id, layer)) else {
            log::warn!(
                "Overlay model is unavailable: {}",
                self.assets_dir
                    .join(format!("{keyboard_id}_L{layer}.json"))
                    .display()
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
