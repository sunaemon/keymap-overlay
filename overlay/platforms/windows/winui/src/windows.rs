//! Windows frontend implemented in Rust with windows-rs.

#[allow(unsafe_code)]
mod native;

use anyhow::{Context, Result};
use keymap_overlay_runtime::{
    Arguments, LayerEvent, LayerEventSink, LogDestination, ModelCache, OverlayModel, Parser as _,
    PendingTransition, SimulatedLayer, StartupRawHidDevice, Transition, compose_model,
    default_log_file, initialize_logging, spawn_layer_event_source, startup_models, write_notice,
};
use std::env;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use windows_reactor::{
    App, Border, Canvas, CanvasChildExt, ChildrenControl, Color, Component, ComponentContext,
    ContentControl, CornerRadius, Ellipse, HorizontalAlignment, KeyedView, LayoutControl,
    LocalSender, TextBlock, TextWrapping, ThemeBrush, Thickness, VerticalAlignment, View,
    ViewContext, WindowSize, WindowVisuals,
};

const TRANSPARENT: Color = Color::transparent();
const KEY_FILL: Color = Color::argb(0xE0, 0xF1, 0xF4, 0xF8);
const HELD_FILL: Color = Color::rgb(0xFF, 0xDD, 0xDD);
const ENCODER_STROKE: Color = Color::argb(0x60, 0x20, 0x24, 0x2C);
const OVERLAY_FILL: Color = Color::argb(0xE8, 0xD8, 0xE0, 0xEA);
const OVERLAY_STROKE: Color = Color::argb(0x70, 0x60, 0x67, 0x73);

#[derive(Clone)]
struct OverlayInput {
    models: Arc<ModelCache>,
    raw_hid_devices: Arc<Mutex<Vec<StartupRawHidDevice>>>,
    simulated: Option<SimulatedLayer>,
}

impl PartialEq for OverlayInput {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.models, &other.models)
            && Arc::ptr_eq(&self.raw_hid_devices, &other.raw_hid_devices)
            && self.simulated == other.simulated
    }
}

struct OverlayComponent {
    models: Arc<ModelCache>,
    pending: Arc<Mutex<PendingTransition>>,
    sender: LocalSender<Transition>,
    transition: Transition,
    window: Arc<AtomicIsize>,
}

impl Component for OverlayComponent {
    type Input = OverlayInput;
    type Message = Transition;

    fn create(input: &Self::Input, context: &ComponentContext<Self>) -> Self {
        let pending = Arc::new(Mutex::new(PendingTransition::default()));
        let window = Arc::new(AtomicIsize::new(0));
        let startup_devices = std::mem::take(
            &mut *input
                .raw_hid_devices
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        let listener = spawn_layer_event_source(
            WinUiSink {
                pending: Arc::clone(&pending),
                window: Arc::clone(&window),
            },
            input.simulated,
            startup_devices,
            input.models.keys().map(|(keyboard_id, _)| *keyboard_id),
        );
        native::install_listener(listener);
        Self {
            models: Arc::clone(&input.models),
            pending,
            sender: context.sender(),
            transition: Transition::Hide,
            window,
        }
    }

    fn update(&mut self, transition: Transition, _context: &ComponentContext<Self>) {
        if transition != Transition::Ignore {
            self.transition = transition;
        }
    }

    fn view(&self, _input: &Self::Input, context: &mut ViewContext<Self>) -> View {
        context.window_title(native::WINDOW_TITLE);
        let sender = self.sender.clone();
        let pending = Arc::clone(&self.pending);
        let window = Arc::clone(&self.window);
        context.use_effect("native-window", (), move || {
            if let Err(error) = native::configure_window(sender, pending, window) {
                log::error!("Failed to configure the native Windows window: {error}");
            }
            None
        });

        let model = match &self.transition {
            Transition::Show {
                keyboard_id,
                layers,
            } => {
                let model = compose_model(&self.models, *keyboard_id, layers);
                if model.is_none() {
                    log_composition_failure(&self.models, *keyboard_id, layers);
                }
                model
            }
            Transition::Hide | Transition::Ignore => None,
        };
        let window_size = model
            .as_ref()
            .map(|model| WindowSize {
                width: f64::from(model.width),
                height: f64::from(model.height),
            })
            .unwrap_or(WindowSize {
                width: 1.0,
                height: 1.0,
            });
        write_e2e_state(&self.transition, model.as_ref());
        context.window_visuals(
            WindowVisuals::new().client_size(window_size.width, window_size.height),
        );
        context.use_effect("window-size", window_size, move || {
            native::request_window(window_size);
            None
        });

        model.map_or_else(hidden_view, model_view)
    }
}

/// Records presentation transitions when the Windows E2E harness requests it.
fn write_e2e_state(transition: &Transition, model: Option<&OverlayModel>) {
    let Some(path) = env::var_os("KEYMAP_OVERLAY_E2E_STATE_FILE") else {
        return;
    };
    let state = match (transition, model) {
        (
            Transition::Show {
                keyboard_id,
                layers,
            },
            Some(model),
        ) => format!(
            "show keyboard={keyboard_id} layers={layers:?} size={}x{} keys={} encoders={} held={}",
            model.width,
            model.height,
            model.keys.len(),
            model.encoders.len(),
            model.keys.iter().filter(|key| key.held).count()
                + model.encoders.iter().filter(|encoder| encoder.held).count(),
        ),
        (Transition::Hide, _) => "hide size=1x1".to_owned(),
        _ => return,
    };
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{state}"));
    if let Err(error) = result {
        log::error!("Failed to record Windows E2E state: {error}");
    }
}

#[derive(Clone)]
struct WinUiSink {
    pending: Arc<Mutex<PendingTransition>>,
    window: Arc<AtomicIsize>,
}

impl LayerEventSink for WinUiSink {
    fn send(&self, event: LayerEvent) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
        native::wake_window(self.window.load(Ordering::Acquire));
        true
    }
}

/// Runs the Windows frontend.
pub(crate) fn run() -> Result<()> {
    let arguments = Arguments::parse();
    if let Some(notice) = arguments.notice() {
        return write_notice(notice);
    }
    // A GUI process has no console, so an unnamed log goes to the default file
    // rather than to stderr.
    let simulated = arguments.simulate;
    let destination = match arguments.log_out {
        Some(path) => LogDestination::File(path),
        None => LogDestination::File(default_log_file()?),
    };
    initialize_logging(destination)?;
    let startup = startup_models(simulated)?;
    App::run_component::<OverlayComponent>(OverlayInput {
        models: Arc::new(startup.models),
        raw_hid_devices: Arc::new(Mutex::new(startup.raw_hid_devices)),
        simulated,
    })
    .context("The WinUI event loop failed")?;
    Ok(())
}

fn hidden_view() -> View {
    Border::new()
        .width(1.0)
        .height(1.0)
        .background(TRANSPARENT)
        .into()
}

fn log_composition_failure(models: &ModelCache, keyboard_id: u8, layers: &[u8]) {
    let Some(base) = models.get(&(keyboard_id, 0)) else {
        log::error!(
            "Failed to compose keyboard {keyboard_id} layers {layers:?}: base layer is missing"
        );
        return;
    };
    for layer in layers {
        let Some(overlay) = models.get(&(keyboard_id, *layer)) else {
            log::error!(
                "Failed to compose keyboard {keyboard_id} layers {layers:?}: layer {layer} is missing"
            );
            return;
        };
        if overlay.keys.len() != base.keys.len() || overlay.encoders.len() != base.encoders.len() {
            log::error!(
                "Failed to compose keyboard {keyboard_id} layers {layers:?}: layer {layer} shape differs from the base layer (keys {} vs {}, encoders {} vs {})",
                overlay.keys.len(),
                base.keys.len(),
                overlay.encoders.len(),
                base.encoders.len()
            );
            return;
        }
    }
    log::error!(
        "Failed to compose keyboard {keyboard_id} layers {layers:?}: unknown model mismatch"
    );
}

fn model_view(model: OverlayModel) -> View {
    let mut children = Vec::new();
    children.push(KeyedView::new(
        "header",
        label(format!("L{}", model.layer), model.header_font_size)
            .width(f64::from(model.width.saturating_sub(40)))
            .height(30.0)
            .canvas_left(20.0)
            .canvas_top(14.0),
    ));
    for (index, key) in model.keys.iter().enumerate() {
        children.push(KeyedView::new(
            format!("key-{index}"),
            Border::new()
                .width(f64::from(key.width))
                .height(f64::from(key.height))
                .background(if key.held { HELD_FILL } else { KEY_FILL })
                .border_brush(ThemeBrush::CardStroke)
                .border_thickness(Thickness::uniform(1.0))
                .corner_radius(CornerRadius::uniform(11.0))
                .canvas_left(f64::from(key.x))
                .canvas_top(f64::from(key.y))
                .content(label(key.label.join("\n"), model.key_font_size)),
        ));
    }
    for (index, encoder) in model.encoders.iter().enumerate() {
        let size = f64::from(encoder.size);
        let x = f64::from(encoder.x);
        let y = f64::from(encoder.y);
        let center_x = x + size / 2.0;
        let label_width = size * 0.7;
        let label_gap = 3.0;
        let label_height = 26.0;
        let max_label_left = (f64::from(model.width) - label_width).max(0.0);
        let counter_clockwise_left =
            (center_x - label_width - label_gap / 2.0).clamp(0.0, max_label_left);
        let clockwise_left = (center_x + label_gap / 2.0).clamp(0.0, max_label_left);
        let max_label_top = (f64::from(model.height) - label_height).max(0.0);
        let below_encoder = y + size + 4.0;
        let label_top = if y >= 30.0 {
            y - 30.0
        } else if below_encoder + label_height <= f64::from(model.height) {
            below_encoder
        } else {
            (y - 30.0).clamp(0.0, max_label_top)
        };
        children.push(KeyedView::new(
            format!("encoder-{index}"),
            Ellipse::new()
                .width(size)
                .height(size)
                .fill(if encoder.held { HELD_FILL } else { KEY_FILL })
                .stroke(ENCODER_STROKE)
                .stroke_thickness(1.0)
                .canvas_left(x)
                .canvas_top(y),
        ));
        children.push(KeyedView::new(
            format!("encoder-{index}-ccw"),
            label(encoder.counter_clockwise.join(" "), model.encoder_font_size)
                .width(label_width)
                .height(label_height)
                .canvas_left(counter_clockwise_left)
                .canvas_top(label_top),
        ));
        children.push(KeyedView::new(
            format!("encoder-{index}-cw"),
            label(encoder.clockwise.join(" "), model.encoder_font_size)
                .width(label_width)
                .height(label_height)
                .canvas_left(clockwise_left)
                .canvas_top(label_top),
        ));
        if !encoder.press.is_empty() {
            children.push(KeyedView::new(
                format!("encoder-{index}-press"),
                Border::new()
                    .width(size)
                    .height(size)
                    .canvas_left(x)
                    .canvas_top(y)
                    .content(label(
                        format!("P {}", encoder.press),
                        model.encoder_font_size,
                    )),
            ));
        }
    }
    Border::new()
        .width(f64::from(model.width))
        .height(f64::from(model.height))
        .background(OVERLAY_FILL)
        .border_brush(OVERLAY_STROKE)
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(16.0))
        .content(
            Canvas::new()
                .width(f64::from(model.width))
                .height(f64::from(model.height))
                .keyed_children(children),
        )
}

fn label(text: impl Into<String>, font_size: f64) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_size(font_size)
        .foreground(ThemeBrush::PrimaryText)
        .text_wrapping(TextWrapping::Wrap)
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
}
