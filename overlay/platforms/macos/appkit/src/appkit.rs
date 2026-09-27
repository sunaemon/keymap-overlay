//! Native macOS overlay.
//!
//! AppKit owns the complete view hierarchy. Liquid Glass supplies the adaptive
//! background on macOS 26 and newer, with `NSVisualEffectView` on earlier
//! releases. Content is built from native boxes and text fields described by
//! the in-memory model. No key label is rasterized into an intermediate image.

use anyhow::{Context, Result};
use block2::StackBlock;
use dispatch::Queue;
use iohidmanager::async_api::ManagerDeviceMatchingStream;
use iohidmanager::{HidManager, HidUsage};
#[cfg(test)]
use keymap_overlay_runtime::DisplayKey;
use keymap_overlay_runtime::{
    DisplayEncoder, LayerEvent, LayerEventSink, LayerEventSourceHandle, ModelStore, OverlayModel,
    OverlayPosition, OverlayPreferences, PendingTransition, RAW_USAGE_ID, RAW_USAGE_PAGE,
    SimulatedLayer, StartupModels, Transition, desktop_tray::DesktopTray,
    desktop_tray::TrayCommand, spawn_layer_event_source,
};
use log::{info, warn};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, extern_methods, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSApplication, NSApplicationActivationPolicy,
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSButtonType,
    NSColor, NSControlStateValueOff, NSControlStateValueOn, NSEvent, NSFont, NSGlassEffectView,
    NSGlassEffectViewStyle, NSMainMenuWindowLevel, NSScreen, NSScrollView, NSTabView,
    NSTabViewItem, NSTextAlignment, NSTextField, NSView, NSViewController,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSPointInRect, NSProcessInfo, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

const ARRIVAL_BUFFER_SIZE: usize = 16;
const IDLE_SIZE: f64 = 1.0;
const GLASS_RADIUS: f64 = 22.0;
const KEY_RADIUS: f64 = 11.0;
const SETTINGS_WIDTH: f64 = 920.0;
const SETTINGS_HEIGHT: f64 = 700.0;
static APPEARANCE_CHANGED: AtomicBool = AtomicBool::new(false);
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct AppearanceView;

    impl AppearanceView {
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            log::info!(
                "macOS effective appearance changed to {}",
                self.effectiveAppearance().name()
            );
            APPEARANCE_CHANGED.store(true, Ordering::Release);
            Queue::main().exec_async(process_appearance_change);
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn view_did_change_backing_properties(&self) {
            Queue::main().exec_async(process_screen_change);
        }
    }
);

impl AppearanceView {
    extern_methods!(
        #[unsafe(method(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self>;
    );
}

define_class!(
    #[unsafe(super(NSButton))]
    #[thread_kind = MainThreadOnly]
    struct SettingsButton;

    impl SettingsButton {
        #[unsafe(method(settingsAction:))]
        fn settings_action(&self, _sender: &NSButton) {
            OVERLAY_APP.with(|app| {
                if let Some(app) = app.borrow_mut().as_mut() {
                    app.apply_settings_action(self.tag());
                }
            });
        }
    }
);

impl SettingsButton {
    extern_methods!(
        #[unsafe(method(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self>;

        #[unsafe(method(setTarget:))]
        fn set_settings_target(&self, target: Option<&SettingsButton>);

        #[unsafe(method(setAction:))]
        fn set_settings_action(&self, action: Option<Sel>);
    );
}

enum AppEvent {
    Layer(LayerEvent),
    Tray(TrayCommand),
}

#[derive(Clone)]
struct ChannelSink(Sender<AppEvent>);

impl LayerEventSink for ChannelSink {
    fn send(&self, event: LayerEvent) -> bool {
        if self.0.send(AppEvent::Layer(event)).is_err() {
            return false;
        }
        Queue::main().exec_async(process_listener_events);
        true
    }
}

struct NativeLayer {
    view: Retained<NSView>,
    size: NSSize,
}

struct SettingsWindow {
    window: Retained<NSWindow>,
    keyboard_id: Option<u8>,
    layer: Option<u8>,
    tab_view: Option<Retained<NSTabView>>,
}

struct OverlayApp {
    receiver: Receiver<AppEvent>,
    pending: PendingTransition,
    models: ModelStore,
    listener: LayerEventSourceHandle,
    layers: HashMap<(u8, Vec<u8>), NativeLayer>,
    visible_layer: Option<(u8, Vec<u8>)>,
    window: Retained<NSWindow>,
    appearance_root: Retained<NSView>,
    background: Retained<NSView>,
    content_host: Retained<NSView>,
    screen_frame: Option<NSRect>,
    e2e_state_file: Option<PathBuf>,
    e2e_shows_remaining: Option<u32>,
    preferences: OverlayPreferences,
    launch_at_login: bool,
    tray: Option<DesktopTray>,
    settings: Option<SettingsWindow>,
}

thread_local! {
    static OVERLAY_APP: RefCell<Option<OverlayApp>> = const { RefCell::new(None) };
}

pub(crate) fn run(startup: StartupModels, simulated: Option<SimulatedLayer>) -> Result<()> {
    let StartupModels {
        models,
        raw_hid_devices,
    } = startup;
    let models = ModelStore::new(models);
    let mtm = MainThreadMarker::new().context("AppKit must run on the main thread")?;
    let application = NSApplication::sharedApplication(mtm);
    application.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let preferences = OverlayPreferences::load()?;

    let appearance_root = appearance_view(idle_rect(), mtm);
    let content_host = NSView::initWithFrame(mtm.alloc(), idle_rect());
    let background = build_background(idle_rect(), &content_host, mtm);
    appearance_root.addSubview(&background);

    let controller = NSViewController::new(mtm);
    controller.setView(&appearance_root);
    let window = NSWindow::windowWithContentViewController(&controller);
    configure_window(&window);

    let (sender, receiver) = mpsc::channel();
    let source = spawn_layer_event_source(
        ChannelSink(sender.clone()),
        simulated,
        raw_hid_devices,
        models.clone(),
    );
    if source.uses_raw_hid() {
        spawn_device_watcher(source.clone());
    }

    let launch_at_login = launch_at_login_enabled().unwrap_or_else(|error| {
        log::warn!("Failed to read the launch-at-login setting: {error:#}");
        true
    });
    let overlay = OverlayApp {
        receiver,
        pending: PendingTransition::default(),
        models,
        listener: source,
        layers: HashMap::new(),
        visible_layer: None,
        window,
        appearance_root,
        background,
        content_host,
        screen_frame: current_screen_frame(),
        e2e_state_file: std::env::var_os("KEYMAP_OVERLAY_E2E_STATE_FILE").map(PathBuf::from),
        e2e_shows_remaining: std::env::var("KEYMAP_OVERLAY_E2E_EXIT_AFTER_SHOWS")
            .ok()
            .and_then(|value| value.parse().ok()),
        preferences,
        launch_at_login,
        tray: None,
        settings: None,
    };

    application.finishLaunching();
    overlay.window.orderFrontRegardless();
    OVERLAY_APP.with(|app| app.replace(Some(overlay)));
    if std::env::var_os("KEYMAP_OVERLAY_E2E_EXERCISE_SETTINGS").is_some_and(|value| value == "1") {
        OVERLAY_APP.with(|app| {
            if let Some(app) = app.borrow_mut().as_mut() {
                app.exercise_settings_for_e2e();
            }
        });
    }
    let tray_sender = sender.clone();
    Queue::main().exec_async(move || install_desktop_tray(tray_sender));
    application.run();
    OVERLAY_APP.with(|app| app.take());
    Ok(())
}

fn spawn_device_watcher(listener: LayerEventSourceHandle) {
    thread::spawn(move || {
        if let Err(error) = watch_for_arrivals(&listener) {
            // Not fatal: reader failures still request enumeration. Only a
            // later arrival alongside another healthy keyboard is missed.
            warn!("Stopped watching for keyboards: {error:#}");
        }
    });
}

/// Blocks on IOHIDManager callbacks, so an idle overlay costs nothing.
fn watch_for_arrivals(listener: &LayerEventSourceHandle) -> Result<()> {
    let manager = HidManager::new().context("Failed to create an IOHIDManager")?;
    manager
        .set_device_matching(Some(HidUsage::Custom(
            u32::from(RAW_USAGE_PAGE),
            u32::from(RAW_USAGE_ID),
        )))
        .context("Failed to match the Raw HID usage")?;

    // Registering the callback reports every device already present. Those
    // devices are part of the listener's initial enumeration, not arrivals.
    // Match identities instead of waiting for a callback count: a device can
    // disappear while the watcher starts, and that must not stall it.
    let mut existing_devices = manager.devices();
    let arrivals = ManagerDeviceMatchingStream::subscribe(&manager, ARRIVAL_BUFFER_SIZE);
    while let Some(arrival) = pollster::block_on(arrivals.next()) {
        let info = arrival.device.info();
        if let Some(index) = existing_devices
            .iter()
            .position(|existing| *existing == info)
        {
            existing_devices.swap_remove(index);
            continue;
        }
        if listener.device_arrived() {
            info!("A Raw HID device appeared; enumerating again");
        }
    }
    anyhow::bail!("The IOHIDManager arrival stream ended")
}

fn process_listener_events() {
    OVERLAY_APP.with(|app| {
        if let Some(app) = app.borrow_mut().as_mut() {
            app.process_listener_events();
        }
    });
}

fn install_desktop_tray(sender: Sender<AppEvent>) {
    OVERLAY_APP.with(|app| {
        let mut app_ref = app.borrow_mut();
        let Some(app) = app_ref.as_mut() else {
            return;
        };
        match DesktopTray::new(app.preferences, app.launch_at_login, move |command| {
            if sender.send(AppEvent::Tray(command)).is_ok() {
                Queue::main().exec_async(process_listener_events);
            }
        }) {
            Ok(tray) => app.tray = Some(tray),
            Err(error) => log::error!("Failed to install the menu bar icon: {error:#}"),
        }
    });
}

fn process_appearance_change() {
    if !APPEARANCE_CHANGED.swap(false, Ordering::AcqRel) {
        return;
    }
    OVERLAY_APP.with(|app| {
        if let Some(app) = app.borrow_mut().as_mut() {
            app.refresh_appearance();
        }
    });
}

fn process_screen_change() {
    OVERLAY_APP.with(|app| {
        if let Some(app) = app.borrow_mut().as_mut() {
            app.update_screen_frame();
        }
    });
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

fn appearance_view(frame: NSRect, mtm: MainThreadMarker) -> Retained<NSView> {
    AppearanceView::init_with_frame(mtm.alloc(), frame).into_super()
}

fn build_background(
    frame: NSRect,
    content_host: &NSView,
    mtm: MainThreadMarker,
) -> Retained<NSView> {
    if supports_liquid_glass() {
        return build_glass(frame, content_host, mtm).into_super();
    }
    build_visual_effect(frame, content_host, mtm).into_super()
}

fn supports_liquid_glass() -> bool {
    if std::env::var_os("KEYMAP_OVERLAY_E2E_FORCE_VISUAL_EFFECT").is_some_and(|value| value == "1")
    {
        return false;
    }
    supports_liquid_glass_major_version(
        NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion,
    )
}

fn supports_liquid_glass_major_version(major_version: isize) -> bool {
    major_version >= 26
}

fn build_glass(
    frame: NSRect,
    content_host: &NSView,
    mtm: MainThreadMarker,
) -> Retained<NSGlassEffectView> {
    let glass = NSGlassEffectView::initWithFrame(mtm.alloc(), frame);
    glass.setStyle(NSGlassEffectViewStyle::Regular);
    glass.setCornerRadius(GLASS_RADIUS);
    glass.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    glass.setContentView(Some(content_host));
    glass
}

fn build_visual_effect(
    frame: NSRect,
    content_host: &NSView,
    mtm: MainThreadMarker,
) -> Retained<NSVisualEffectView> {
    let background = NSVisualEffectView::initWithFrame(mtm.alloc(), frame);
    background.setMaterial(NSVisualEffectMaterial::HUDWindow);
    background.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    background.setState(NSVisualEffectState::Active);
    background.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    content_host.setFrame(background.bounds());
    content_host.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    background.addSubview(content_host);
    background
}

fn build_native_layer(
    model: &OverlayModel,
    appearance: &NSAppearance,
    mtm: MainThreadMarker,
) -> NativeLayer {
    let native = RefCell::new(None);
    let block = StackBlock::new(|| {
        native.replace(Some(build_native_layer_for_current_appearance(model, mtm)));
    });
    appearance.performAsCurrentDrawingAppearance(&block);
    native
        .into_inner()
        .expect("AppKit must execute the appearance drawing block")
}

fn build_native_layer_for_current_appearance(
    model: &OverlayModel,
    mtm: MainThreadMarker,
) -> NativeLayer {
    let size = NSSize::new(f64::from(model.width), f64::from(model.height));
    let root = NSView::initWithFrame(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), size));
    let glass_text_color = resolved_color(NSColor::labelColor());

    add_label(
        &root,
        &format!("L{}", model.layer),
        NSRect::new(
            NSPoint::new(20.0, size.height - 43.0),
            NSSize::new(80.0, 24.0),
        ),
        model.header_font_size,
        NSTextAlignment::Left,
        &glass_text_color,
        mtm,
    );

    for key in &model.keys {
        let frame = top_left_frame(key.x, key.y, key.width, key.height, model.height);
        add_key_surface(&root, frame, key.held, KEY_RADIUS, mtm);
        let text_color = key_text_color(key.held);
        add_label(
            &root,
            &key.label.join("\n"),
            frame,
            model.key_font_size,
            NSTextAlignment::Center,
            &text_color,
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
    text_color: &NSColor,
    mtm: MainThreadMarker,
) {
    if text.is_empty() {
        return;
    }
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setAlignment(alignment);
    label.setFont(Some(&NSFont::systemFontOfSize(font_size)));
    label.setTextColor(Some(text_color));
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

fn add_settings_label(
    root: &NSView,
    text: &str,
    frame: NSRect,
    font_size: f64,
    mtm: MainThreadMarker,
) {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    label.setFont(Some(&NSFont::systemFontOfSize(font_size)));
    root.addSubview(&label);
}

fn add_settings_button(
    root: &NSView,
    title: &str,
    tag: isize,
    frame: NSRect,
    mtm: MainThreadMarker,
) {
    let button = SettingsButton::init_with_frame(mtm.alloc(), frame);
    button.setTitle(&NSString::from_str(title));
    button.setTag(tag);
    button.set_settings_target(Some(&button));
    button.set_settings_action(Some(sel!(settingsAction:)));
    root.addSubview(&button);
}

fn add_settings_checkbox(
    root: &NSView,
    title: &str,
    checked: bool,
    tag: isize,
    frame: NSRect,
    mtm: MainThreadMarker,
) {
    let button = SettingsButton::init_with_frame(mtm.alloc(), frame);
    button.setButtonType(NSButtonType::Switch);
    button.setTitle(&NSString::from_str(title));
    button.setState(if checked {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    button.setTag(tag);
    button.set_settings_target(Some(&button));
    button.set_settings_action(Some(sel!(settingsAction:)));
    root.addSubview(&button);
}

fn add_settings_radio(
    root: &NSView,
    title: &str,
    selected: bool,
    tag: isize,
    frame: NSRect,
    mtm: MainThreadMarker,
) {
    let button = SettingsButton::init_with_frame(mtm.alloc(), frame);
    button.setButtonType(NSButtonType::Radio);
    button.setTitle(&NSString::from_str(title));
    button.setState(if selected {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
    button.setTag(tag);
    button.set_settings_target(Some(&button));
    button.set_settings_action(Some(sel!(settingsAction:)));
    root.addSubview(&button);
}

fn add_scrolling_choices(
    root: &NSView,
    choices: &[(String, isize, bool)],
    y: f64,
    button_width: f64,
    mtm: MainThreadMarker,
) {
    let visible_width = 748.0;
    let spacing = 6.0;
    let content_width = choice_content_width(choices.len(), button_width, visible_width, spacing);
    let document = NSView::initWithFrame(
        mtm.alloc(),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(content_width, 34.0)),
    );
    let mut x = 0.0;
    for (title, tag, selected) in choices {
        add_settings_button(
            &document,
            &format!("{}{title}", if *selected { "✓ " } else { "" }),
            *tag,
            NSRect::new(NSPoint::new(x, 2.0), NSSize::new(button_width, 30.0)),
            mtm,
        );
        x += button_width + spacing;
    }
    let scroll = NSScrollView::initWithFrame(
        mtm.alloc(),
        NSRect::new(NSPoint::new(120.0, y), NSSize::new(visible_width, 42.0)),
    );
    scroll.setHasHorizontalScroller(content_width > visible_width);
    scroll.setDocumentView(Some(&document));
    root.addSubview(&scroll);
}

fn choice_content_width(count: usize, item_width: f64, visible_width: f64, spacing: f64) -> f64 {
    ((item_width + spacing) * count as f64).max(visible_width) - spacing
}

fn add_choice_row(
    root: &NSView,
    label: &str,
    x: f64,
    y: f64,
    choices: &[(&str, isize, bool)],
    mtm: MainThreadMarker,
) {
    add_settings_label(
        root,
        label,
        NSRect::new(NSPoint::new(x, y + 4.0), NSSize::new(68.0, 24.0)),
        13.0,
        mtm,
    );
    let mut button_x = x + 70.0;
    for (title, tag, selected) in choices {
        add_settings_radio(
            root,
            title,
            *selected,
            *tag,
            NSRect::new(NSPoint::new(button_x, y), NSSize::new(86.0, 32.0)),
            mtm,
        );
        button_x += 90.0;
    }
}

fn add_owned_choice_row<const N: usize>(
    root: &NSView,
    label: &str,
    x: f64,
    y: f64,
    choices: &[(String, isize, bool); N],
    mtm: MainThreadMarker,
) {
    add_settings_label(
        root,
        label,
        NSRect::new(NSPoint::new(x, y + 4.0), NSSize::new(60.0, 24.0)),
        13.0,
        mtm,
    );
    let mut button_x = x + 65.0;
    for (title, tag, selected) in choices {
        add_settings_radio(
            root,
            title,
            *selected,
            *tag,
            NSRect::new(NSPoint::new(button_x, y), NSSize::new(78.0, 32.0)),
            mtm,
        );
        button_x += 82.0;
    }
}

fn add_encoder(
    root: &NSView,
    encoder: &DisplayEncoder,
    model: &OverlayModel,
    mtm: MainThreadMarker,
) {
    let glass_text_color = resolved_color(NSColor::labelColor());
    let key_text_color = key_text_color(encoder.held);
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
        &glass_text_color,
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
        &glass_text_color,
        mtm,
    );
    if !encoder.press.is_empty() {
        add_label(
            root,
            &format!("P {}", encoder.press),
            frame,
            model.encoder_font_size,
            NSTextAlignment::Center,
            &key_text_color,
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
    resolved_color(NSColor::controlBackgroundColor())
}

fn key_border_color() -> Retained<NSColor> {
    resolved_color(NSColor::separatorColor())
}

fn held_color() -> Retained<NSColor> {
    resolved_color(NSColor::selectedControlColor())
}

fn key_text_color(held: bool) -> Retained<NSColor> {
    let color = if held {
        NSColor::selectedControlTextColor()
    } else {
        NSColor::controlTextColor()
    };
    resolved_color(color)
}

fn resolved_color(color: Retained<NSColor>) -> Retained<NSColor> {
    NSColor::colorWithCGColor(&color.CGColor()).unwrap_or(color)
}

impl OverlayApp {
    fn exercise_settings_for_e2e(&mut self) {
        self.rebuild_settings();
        self.apply_tray_command(TrayCommand::OpenSettings);
        if let Some(tabs) = self
            .settings
            .as_ref()
            .and_then(|settings| settings.tab_view.as_ref())
        {
            tabs.selectTabViewItemAtIndex(1);
        }
        self.rebuild_settings();
        self.apply_settings_action(11);
        self.apply_settings_action(12);
        self.apply_settings_action(10);
        self.apply_settings_action(175);
        self.apply_settings_action(425);
        self.apply_settings_action(1_001);
        self.apply_settings_action(2_002);
        self.apply_settings_action(-1);
        self.apply_tray_command(TrayCommand::Reload);
        let state = self.settings.as_ref().map(|settings| {
            let selected_tab = settings
                .tab_view
                .as_ref()
                .and_then(|tabs| tabs.selectedTabViewItem())
                .map(|item| item.label().to_string())
                .unwrap_or_default();
            format!(
                "settings tab={selected_tab} keyboard={:?} layer={:?} position={:?} opacity={} scale={}",
                settings.keyboard_id,
                settings.layer,
                self.preferences.position,
                self.preferences.opacity_percent,
                self.preferences.scale_percent,
            )
        });
        if let Some(state) = state {
            self.record_e2e_state(&state);
        }
    }

    fn process_listener_events(&mut self) {
        self.update_screen_frame();

        for event in self.receiver.try_iter().collect::<Vec<_>>() {
            match event {
                AppEvent::Layer(event) => self.pending.push(event),
                AppEvent::Tray(command) => self.apply_tray_command(command),
            }
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

    fn update_screen_frame(&mut self) {
        let screen_frame = current_screen_frame();
        if self.screen_frame != screen_frame {
            self.recenter_visible_layer(screen_frame);
            self.screen_frame = screen_frame;
        }
    }

    fn show(&mut self, keyboard_id: u8, layers: &[u8]) {
        let key = (keyboard_id, layers.to_vec());
        if !self.layers.contains_key(&key) {
            let Some(mut model) = self.models.compose(keyboard_id, layers) else {
                log::warn!(
                    "Overlay model is unavailable for keyboard {keyboard_id}, layers {layers:?}"
                );
                self.hide();
                return;
            };
            scale_model(&mut model, self.preferences.scale_percent);
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            // This view owns the appearance-change callback and remains in the
            // hierarchy while a layer is detached. Its effective appearance is
            // therefore authoritative for colors resolved while the overlay is
            // in its one-pixel idle state.
            let appearance = self.appearance_root.effectiveAppearance();
            self.layers
                .insert(key.clone(), build_native_layer(&model, &appearance, mtm));
        }
        self.detach_visible_layer();
        let Some(native) = self.layers.get(&key) else {
            log::warn!(
                "Overlay model is unavailable for keyboard {keyboard_id}, layers {layers:?}"
            );
            self.hide();
            return;
        };
        self.content_host.addSubview(&native.view);
        let native_subviews = native.view.subviews().len();
        self.window
            .setAlphaValue(f64::from(self.preferences.opacity_percent) / 100.0);
        self.window.setFrame_display(
            positioned_frame(native.size, self.preferences.position),
            true,
        );
        self.window.orderFrontRegardless();
        self.visible_layer = Some(key);
        let frame = self.window.frame();
        self.record_e2e_state(&format!(
            "show keyboard={keyboard_id} layers={layers:?} size={}x{} subviews={} native_subviews={native_subviews}",
            frame.size.width,
            frame.size.height,
            self.content_host.subviews().len()
        ));
        let should_exit = self
            .e2e_shows_remaining
            .as_mut()
            .is_some_and(|shows_remaining| {
                *shows_remaining = shows_remaining.saturating_sub(1);
                *shows_remaining == 0
            });
        if should_exit && let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).terminate(None);
        }
    }

    fn hide(&mut self) {
        self.conceal();
        self.visible_layer = None;
    }

    fn conceal(&self) {
        self.detach_visible_layer();
        self.window.setFrame_display(idle_rect(), false);
        let frame = self.window.frame();
        self.record_e2e_state(&format!(
            "hide size={}x{} subviews={}",
            frame.size.width,
            frame.size.height,
            self.content_host.subviews().len()
        ));
    }

    fn rebuild_layers(&mut self) {
        let visible_layer = self.visible_layer.clone();
        self.detach_visible_layer();
        self.layers.clear();
        if let Some((keyboard_id, layers)) = visible_layer {
            self.show(keyboard_id, &layers);
        }
    }

    fn apply_tray_command(&mut self, command: TrayCommand) {
        match command {
            TrayCommand::OpenSettings => {
                self.open_settings();
                return;
            }
            TrayCommand::ToggleLaunchAtLogin => {
                let enabled = !self.launch_at_login;
                if let Err(error) = set_launch_at_login(enabled) {
                    log::error!("Failed to change the launch-at-login setting: {error:#}");
                } else {
                    self.launch_at_login = enabled;
                }
                if let Some(tray) = &mut self.tray {
                    tray.sync(self.preferences, self.launch_at_login);
                }
                self.rebuild_settings();
                return;
            }
            TrayCommand::SetPosition(_) | TrayCommand::SetOpacity(_) | TrayCommand::SetScale(_) => {
            }
            TrayCommand::Reload => {
                if self.listener.reload_keyboards() {
                    info!("Reloading connected keyboard models");
                }
                return;
            }
            TrayCommand::Quit => {
                if let Some(mtm) = MainThreadMarker::new() {
                    NSApplication::sharedApplication(mtm).terminate(None);
                }
                return;
            }
        }
        let next = command
            .updated_preferences(self.preferences)
            .expect("presentation commands update preferences");
        if let Err(error) = next.save() {
            log::error!("Failed to save overlay preferences: {error:#}");
            if let Some(tray) = &mut self.tray {
                tray.sync(self.preferences, self.launch_at_login);
            }
            return;
        }
        self.preferences = next;
        if let Some(tray) = &mut self.tray {
            tray.sync(self.preferences, self.launch_at_login);
        }
        self.rebuild_layers();
        self.rebuild_settings();
    }

    fn open_settings(&mut self) {
        if self.settings.is_none() {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let choices = self.models.preview_choices();
            let selection = choices.first().copied();
            let frame = NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(SETTINGS_WIDTH, SETTINGS_HEIGHT),
            );
            let content = NSViewController::new(mtm);
            content.setView(&NSView::initWithFrame(mtm.alloc(), frame));
            let window = NSWindow::windowWithContentViewController(&content);
            window.setStyleMask(
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable,
            );
            window.setTitle(&NSString::from_str("Keymap Overlay Settings"));
            window.center();
            self.settings = Some(SettingsWindow {
                window,
                keyboard_id: selection.map(|choice| choice.0),
                layer: selection.map(|choice| choice.1),
                tab_view: None,
            });
            self.rebuild_settings();
        }
        self.rebuild_settings();
        if let Some(settings) = &self.settings {
            settings.window.makeKeyAndOrderFront(None);
        }
        if let Some(mtm) = MainThreadMarker::new() {
            let application = NSApplication::sharedApplication(mtm);
            if uses_cooperative_activation(
                NSProcessInfo::processInfo()
                    .operatingSystemVersion()
                    .majorVersion,
            ) {
                application.activate();
            } else {
                activate_legacy(&application);
            }
        }
    }

    fn apply_settings_action(&mut self, tag: isize) {
        match tag {
            1 => self.apply_tray_command(TrayCommand::ToggleLaunchAtLogin),
            10 => self.apply_tray_command(TrayCommand::SetPosition(OverlayPosition::Top)),
            11 => self.apply_tray_command(TrayCommand::SetPosition(OverlayPosition::Center)),
            12 => self.apply_tray_command(TrayCommand::SetPosition(OverlayPosition::Bottom)),
            100..=200 => self.apply_tray_command(TrayCommand::SetOpacity((tag - 100) as u8)),
            300..=500 => self.apply_tray_command(TrayCommand::SetScale((tag - 300) as u16)),
            1_000..=1_255 => {
                let keyboard_id = (tag - 1_000) as u8;
                if let Some(settings) = &mut self.settings {
                    settings.keyboard_id = Some(keyboard_id);
                    settings.layer = self
                        .models
                        .preview_choices()
                        .into_iter()
                        .find(|choice| choice.0 == keyboard_id)
                        .map(|choice| choice.1);
                }
                self.rebuild_settings();
            }
            2_000..=2_255 => {
                if let Some(settings) = &mut self.settings {
                    settings.layer = Some((tag - 2_000) as u8);
                }
                self.rebuild_settings();
            }
            _ => {}
        }
    }

    fn rebuild_settings(&mut self) {
        let Some(settings) = self.settings.as_ref() else {
            return;
        };
        let keyboard_id = settings.keyboard_id;
        let layer = settings.layer;
        let preview_selected = settings
            .tab_view
            .as_ref()
            .and_then(|tabs| tabs.selectedTabViewItem())
            .is_some_and(|item| item.label().to_string() == "Preview");
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let root = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(SETTINGS_WIDTH, SETTINGS_HEIGHT),
            ),
        );
        let tabs = NSTabView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(8.0, 16.0), NSSize::new(904.0, 668.0)),
        );
        let settings_root = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(904.0, 630.0)),
        );
        self.add_settings_controls(&settings_root, mtm);
        let settings_item = NSTabViewItem::new();
        settings_item.setLabel(&NSString::from_str("Settings"));
        settings_item.setView(Some(&settings_root));
        tabs.addTabViewItem(&settings_item);

        let preview_root = NSView::initWithFrame(
            mtm.alloc(),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(904.0, 630.0)),
        );
        add_settings_label(
            &preview_root,
            "Preview",
            NSRect::new(NSPoint::new(24.0, 600.0), NSSize::new(120.0, 24.0)),
            18.0,
            mtm,
        );
        let choices = self.models.preview_choices();
        add_settings_label(
            &preview_root,
            "Keyboard",
            NSRect::new(NSPoint::new(24.0, 562.0), NSSize::new(90.0, 26.0)),
            13.0,
            mtm,
        );
        let mut keyboards = choices.iter().map(|choice| choice.0).collect::<Vec<_>>();
        keyboards.dedup();
        let keyboard_choices = keyboards
            .into_iter()
            .map(|choice| {
                (
                    format!("Keyboard {choice}"),
                    1_000 + isize::from(choice),
                    keyboard_id == Some(choice),
                )
            })
            .collect::<Vec<_>>();
        add_scrolling_choices(&preview_root, &keyboard_choices, 552.0, 110.0, mtm);
        add_settings_label(
            &preview_root,
            "Layer",
            NSRect::new(NSPoint::new(24.0, 524.0), NSSize::new(90.0, 26.0)),
            13.0,
            mtm,
        );
        if let Some(keyboard_id) = keyboard_id {
            let layer_choices = choices
                .iter()
                .copied()
                .filter(|choice| choice.0 == keyboard_id)
                .map(|(_, choice)| {
                    (
                        format!("Layer {choice}"),
                        2_000 + isize::from(choice),
                        layer == Some(choice),
                    )
                })
                .collect::<Vec<_>>();
            add_scrolling_choices(&preview_root, &layer_choices, 510.0, 88.0, mtm);
        }
        let preview_frame = NSRect::new(NSPoint::new(24.0, 60.0), NSSize::new(872.0, 440.0));
        let preview_box = NSBox::initWithFrame(mtm.alloc(), preview_frame);
        preview_box.setBoxType(NSBoxType::Custom);
        preview_box.setCornerRadius(12.0);
        preview_box.setBorderColor(&key_border_color());
        preview_box.setFillColor(&NSColor::windowBackgroundColor());
        preview_root.addSubview(&preview_box);
        if let (Some(keyboard_id), Some(layer)) = (keyboard_id, layer)
            && let Some(mut model) = self.models.compose(keyboard_id, &[layer])
        {
            let fit = (820.0 / f64::from(model.width))
                .min(388.0 / f64::from(model.height))
                .min(1.0);
            scale_model(&mut model, (fit * 100.0).round() as u16);
            let native = build_native_layer(&model, &preview_root.effectiveAppearance(), mtm);
            native.view.setFrameOrigin(NSPoint::new(
                (preview_frame.size.width - native.size.width) / 2.0,
                (preview_frame.size.height - native.size.height) / 2.0,
            ));
            preview_box.addSubview(&native.view);
        } else {
            add_settings_label(
                &preview_box,
                "No keyboard models are loaded. Connect a keyboard and choose Reload Keyboards.",
                NSRect::new(NSPoint::new(40.0, 204.0), NSSize::new(792.0, 30.0)),
                14.0,
                mtm,
            );
        }
        let preview_item = NSTabViewItem::new();
        preview_item.setLabel(&NSString::from_str("Preview"));
        preview_item.setView(Some(&preview_root));
        tabs.addTabViewItem(&preview_item);
        if preview_selected {
            tabs.selectTabViewItemAtIndex(1);
        }
        root.addSubview(&tabs);
        if let Some(settings) = &mut self.settings {
            settings.tab_view = Some(tabs);
            settings.window.setContentView(Some(&root));
        }
    }

    fn add_settings_controls(&self, root: &NSView, mtm: MainThreadMarker) {
        add_settings_label(
            root,
            "General",
            NSRect::new(NSPoint::new(32.0, 560.0), NSSize::new(240.0, 26.0)),
            17.0,
            mtm,
        );
        add_settings_checkbox(
            root,
            "Launch at Login",
            self.launch_at_login,
            1,
            NSRect::new(NSPoint::new(32.0, 510.0), NSSize::new(180.0, 32.0)),
            mtm,
        );
        add_settings_label(
            root,
            "Overlay Appearance",
            NSRect::new(NSPoint::new(32.0, 440.0), NSSize::new(240.0, 26.0)),
            17.0,
            mtm,
        );
        add_choice_row(
            root,
            "Position",
            32.0,
            390.0,
            &[
                ("Top", 10, self.preferences.position == OverlayPosition::Top),
                (
                    "Center",
                    11,
                    self.preferences.position == OverlayPosition::Center,
                ),
                (
                    "Bottom",
                    12,
                    self.preferences.position == OverlayPosition::Bottom,
                ),
            ],
            mtm,
        );
        let opacity = OverlayPreferences::OPACITY_CHOICES.map(|value| {
            (
                format!("{value}%"),
                100 + isize::from(value),
                self.preferences.opacity_percent == value,
            )
        });
        add_owned_choice_row(root, "Opacity", 32.0, 330.0, &opacity, mtm);
        let scale = OverlayPreferences::SCALE_CHOICES.map(|value| {
            (
                format!("{value}%"),
                300 + value as isize,
                self.preferences.scale_percent == value,
            )
        });
        add_owned_choice_row(root, "Scale", 32.0, 270.0, &scale, mtm);
        add_settings_label(
            root,
            &format!("Keymap Overlay {}", env!("CARGO_PKG_VERSION")),
            NSRect::new(NSPoint::new(32.0, 24.0), NSSize::new(300.0, 24.0)),
            12.0,
            mtm,
        );
    }

    fn refresh_appearance(&mut self) {
        self.rebuild_layers();
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let background = build_background(self.appearance_root.bounds(), &self.content_host, mtm);
        self.background.removeFromSuperview();
        self.appearance_root.addSubview(&background);
        self.background = background;
    }

    fn detach_visible_layer(&self) {
        let Some(key) = &self.visible_layer else {
            return;
        };
        let Some(native) = self.layers.get(key) else {
            return;
        };
        native.view.removeFromSuperview();
    }

    fn recenter_visible_layer(&self, screen_frame: Option<NSRect>) {
        let Some(screen_frame) = screen_frame else {
            return;
        };
        let Some(key) = &self.visible_layer else {
            return;
        };
        let Some(native) = self.layers.get(key) else {
            return;
        };
        self.window.setFrame_display(
            positioned_frame_on_screen(native.size, screen_frame, self.preferences.position),
            true,
        );
    }

    fn record_e2e_state(&self, state: &str) {
        let Some(path) = &self.e2e_state_file else {
            return;
        };
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| writeln!(file, "{state}"));
        if let Err(error) = result {
            warn!(
                "Failed to record macOS E2E state in {}: {error}",
                path.display()
            );
        }
    }
}

fn uses_cooperative_activation(major_version: isize) -> bool {
    major_version >= 14
}

#[allow(deprecated)]
fn activate_legacy(application: &NSApplication) {
    application.activateIgnoringOtherApps(true);
}

fn positioned_frame(size: NSSize, position: OverlayPosition) -> NSRect {
    let Some(screen) = current_screen_frame() else {
        return NSRect::new(NSPoint::new(0.0, 0.0), size);
    };
    positioned_frame_on_screen(size, screen, position)
}

#[cfg(test)]
fn centered_frame_on_screen(size: NSSize, screen: NSRect) -> NSRect {
    positioned_frame_on_screen(size, screen, OverlayPosition::Center)
}

fn positioned_frame_on_screen(size: NSSize, screen: NSRect, position: OverlayPosition) -> NSRect {
    let y = match position {
        OverlayPosition::Top => screen.origin.y + screen.size.height - size.height,
        OverlayPosition::Center => screen.origin.y + (screen.size.height - size.height) / 2.0,
        OverlayPosition::Bottom => screen.origin.y,
    };
    NSRect::new(
        NSPoint::new(screen.origin.x + (screen.size.width - size.width) / 2.0, y),
        size,
    )
}

fn scale_model(model: &mut OverlayModel, percent: u16) {
    let scale = f64::from(percent) / 100.0;
    let dimension = |value: u32| (f64::from(value) * scale).round() as u32;
    model.width = dimension(model.width);
    model.height = dimension(model.height);
    model.header_font_size *= scale;
    model.key_font_size *= scale;
    model.encoder_font_size *= scale;
    for key in &mut model.keys {
        key.x = dimension(key.x);
        key.y = dimension(key.y);
        key.width = dimension(key.width);
        key.height = dimension(key.height);
    }
    for encoder in &mut model.encoders {
        encoder.x = dimension(encoder.x);
        encoder.y = dimension(encoder.y);
        encoder.size = dimension(encoder.size);
    }
}

fn launch_at_login_enabled() -> Result<bool> {
    let domain = launchd_domain()?;
    let output = Command::new("launchctl")
        .args(["print-disabled", &domain])
        .output()
        .context("Failed to read launchd overrides")?;
    anyhow::ensure!(output.status.success(), "launchctl print-disabled failed");
    let overrides = String::from_utf8_lossy(&output.stdout);
    Ok(launchd_overrides_allow_login(&overrides))
}

fn launchd_overrides_allow_login(overrides: &str) -> bool {
    !overrides.lines().any(|line| {
        line.contains("\"com.sunaemon.keymap-overlay\"") && line.contains("=> disabled")
    })
}

fn set_launch_at_login(enabled: bool) -> Result<()> {
    let target = format!("{}/com.sunaemon.keymap-overlay", launchd_domain()?);
    let action = if enabled { "enable" } else { "disable" };
    let status = Command::new("launchctl")
        .args([action, &target])
        .status()
        .with_context(|| format!("Failed to run launchctl {action}"))?;
    anyhow::ensure!(status.success(), "launchctl {action} failed");
    Ok(())
}

fn launchd_domain() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("Failed to read the current user ID")?;
    anyhow::ensure!(output.status.success(), "id -u failed");
    Ok(format!(
        "gui/{}",
        String::from_utf8_lossy(&output.stdout).trim()
    ))
}

fn current_screen_frame() -> Option<NSRect> {
    let mtm = MainThreadMarker::new()?;
    let mouse_location = NSEvent::mouseLocation();
    visible_frame_containing_point(
        mouse_location,
        NSScreen::screens(mtm)
            .iter()
            .map(|screen| (screen.frame(), screen.visibleFrame())),
    )
    .or_else(|| NSScreen::mainScreen(mtm).map(|screen| screen.visibleFrame()))
}

fn visible_frame_containing_point(
    point: NSPoint,
    frames: impl IntoIterator<Item = (NSRect, NSRect)>,
) -> Option<NSRect> {
    frames
        .into_iter()
        .find_map(|(full, visible)| NSPointInRect(point, full).then_some(visible))
}

#[cfg(test)]
fn frame_containing_point(
    point: NSPoint,
    frames: impl IntoIterator<Item = NSRect>,
) -> Option<NSRect> {
    frames
        .into_iter()
        .find(|frame| NSPointInRect(point, *frame))
}

fn idle_rect() -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(IDLE_SIZE, IDLE_SIZE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_a_layer_after_the_screen_geometry_changes() {
        let frame = centered_frame_on_screen(
            NSSize::new(400.0, 200.0),
            NSRect::new(NSPoint::new(100.0, 50.0), NSSize::new(1_200.0, 800.0)),
        );

        assert_eq!(frame.origin, NSPoint::new(500.0, 350.0));
        assert_eq!(frame.size, NSSize::new(400.0, 200.0));
    }

    #[test]
    fn places_a_layer_at_the_selected_screen_edge() {
        let screen = NSRect::new(NSPoint::new(100.0, 50.0), NSSize::new(1_200.0, 800.0));
        let size = NSSize::new(400.0, 200.0);

        assert_eq!(
            positioned_frame_on_screen(size, screen, OverlayPosition::Top).origin,
            NSPoint::new(500.0, 650.0)
        );
        assert_eq!(
            positioned_frame_on_screen(size, screen, OverlayPosition::Bottom).origin,
            NSPoint::new(500.0, 50.0)
        );
    }

    #[test]
    fn selects_the_screen_containing_the_pointer() {
        let left = NSRect::new(NSPoint::new(-1_200.0, 0.0), NSSize::new(1_200.0, 800.0));
        let right = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1_600.0, 900.0));

        assert_eq!(
            frame_containing_point(NSPoint::new(-300.0, 400.0), [left, right]),
            Some(left)
        );
        assert_eq!(
            frame_containing_point(NSPoint::new(500.0, 400.0), [left, right]),
            Some(right)
        );
    }

    #[test]
    fn liquid_glass_requires_macos_26() {
        assert!(!supports_liquid_glass_major_version(25));
        assert!(supports_liquid_glass_major_version(26));
        assert!(supports_liquid_glass_major_version(27));
    }

    #[test]
    fn cooperative_activation_requires_macos_14() {
        assert!(!uses_cooperative_activation(13));
        assert!(uses_cooperative_activation(14));
    }

    #[test]
    fn overflowing_preview_choices_get_scrollable_content_width() {
        assert_eq!(choice_content_width(2, 110.0, 748.0, 6.0), 742.0);
        assert!(choice_content_width(8, 110.0, 748.0, 6.0) > 748.0);
    }

    #[test]
    fn screen_selection_returns_the_usable_frame() {
        let full = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1_600.0, 900.0));
        let visible = NSRect::new(NSPoint::new(0.0, 40.0), NSSize::new(1_600.0, 820.0));

        assert_eq!(
            visible_frame_containing_point(NSPoint::new(800.0, 880.0), [(full, visible)]),
            Some(visible)
        );
    }

    #[test]
    fn reads_the_launchd_disabled_override() {
        assert!(!launchd_overrides_allow_login(
            r#""com.sunaemon.keymap-overlay" => disabled"#
        ));
        assert!(launchd_overrides_allow_login(
            r#""com.sunaemon.keymap-overlay" => enabled"#
        ));
        assert!(launchd_overrides_allow_login("disabled services = {}"));
    }

    #[test]
    fn scales_keys_and_encoders_with_the_model() {
        let mut model = OverlayModel {
            version: 2,
            layer: 1,
            width: 200,
            height: 100,
            header_font_size: 16.0,
            key_font_size: 12.0,
            encoder_font_size: 10.0,
            keys: vec![DisplayKey {
                x: 20,
                y: 10,
                width: 40,
                height: 30,
                label: vec!["A".to_owned()],
                held: false,
                transparent: false,
                momentary_layer: None,
            }],
            encoders: vec![DisplayEncoder {
                x: 100,
                y: 40,
                size: 20,
                counter_clockwise: vec!["Left".to_owned()],
                clockwise: vec!["Right".to_owned()],
                press: "Press".to_owned(),
                held: false,
                counter_clockwise_transparent: false,
                clockwise_transparent: false,
                press_transparent: false,
                momentary_layer: None,
            }],
        };

        scale_model(&mut model, 150);

        assert_eq!((model.width, model.height), (300, 150));
        assert_eq!((model.keys[0].x, model.keys[0].height), (30, 45));
        assert_eq!((model.encoders[0].x, model.encoders[0].size), (150, 30));
        assert_eq!(model.header_font_size, 24.0);
    }
}
