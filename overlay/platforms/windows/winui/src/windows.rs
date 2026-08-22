//! Experimental pure-Rust WinUI 3 frontend.

#[allow(unsafe_code)]
mod native;

use anyhow::{Context, Result};
use keymap_overlay_runtime::{
    Arguments, LayerEvent, LayerEventSink, LogDestination, ModelCache, OverlayModel, Parser as _,
    PendingTransition, SimulatedLayer, Transition, compose_model, default_asset_dir,
    default_log_file, initialize_logging, load_model_cache, spawn_layer_event_source, write_notice,
};
use std::sync::{Arc, Mutex};
use windows_reactor::{
    AsyncSetState, BackgroundExt, BrushBinding, Canvas, CanvasChildExt, Color, Component, Element,
    HorizontalAlignment, KeyExt, LayoutExt, RenderCx, Shape, TextStyleExt, Thickness,
    VerticalAlignment, WindowSize, border, text_block, tokens,
};

const TRANSPARENT: Color = Color {
    a: 0,
    r: 0,
    g: 0,
    b: 0,
};
const KEY_FILL: Color = Color {
    a: 0xE0,
    r: 0xF1,
    g: 0xF4,
    b: 0xF8,
};
const HELD_FILL: Color = Color {
    a: 0xFF,
    r: 0xFF,
    g: 0xDD,
    b: 0xDD,
};
const ENCODER_STROKE: Color = Color {
    a: 0x60,
    r: 0x20,
    g: 0x24,
    b: 0x2C,
};
const OVERLAY_FILL: Color = Color {
    a: 0xE8,
    r: 0xD8,
    g: 0xE0,
    b: 0xEA,
};
const OVERLAY_STROKE: Color = Color {
    a: 0x70,
    r: 0x60,
    g: 0x67,
    b: 0x73,
};

pub(super) struct OverlayComponent {
    models: Arc<ModelCache>,
    simulated: Option<SimulatedLayer>,
}

impl Component for OverlayComponent {
    fn render(&self, _props: &(), context: &mut RenderCx) -> Element {
        render(context, Arc::clone(&self.models), self.simulated)
    }
}

#[derive(Clone)]
struct WinUiSink {
    pending: Arc<Mutex<PendingTransition>>,
    set_transition: AsyncSetState<Transition>,
}

impl LayerEventSink for WinUiSink {
    fn send(&self, event: LayerEvent) -> bool {
        let transition = {
            let mut pending = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.push(event);
            pending.take()
        };
        if transition != Transition::Ignore {
            self.set_transition.call(transition);
        }
        true
    }
}

/// Runs the experimental WinUI frontend without changing the WPF release path.
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
    let directory = arguments.asset_dir.map_or_else(default_asset_dir, Ok)?;
    let models = Arc::new(load_model_cache(&directory)?);
    windows_reactor::bootstrap().context("Failed to initialize the Windows App SDK runtime")?;
    native::run(OverlayComponent { models, simulated }).context("The WinUI event loop failed")?;
    Ok(())
}

fn render(
    context: &mut windows_reactor::RenderCx,
    models: Arc<ModelCache>,
    simulated: Option<SimulatedLayer>,
) -> Element {
    let (transition, set_transition) = context.use_async_state(Transition::Hide);
    context.use_effect((), move || start_listener(set_transition, simulated));

    let model = match &transition {
        Transition::Show {
            keyboard_id,
            layers,
        } => {
            let model = compose_model(&models, *keyboard_id, layers);
            if model.is_none() {
                log_composition_failure(&models, *keyboard_id, layers);
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
    context.use_effect((transition, window_size), move || {
        native::request_window(window_size);
    });

    model.map_or_else(hidden_canvas, model_canvas)
}

fn start_listener(set_transition: AsyncSetState<Transition>, simulated: Option<SimulatedLayer>) {
    let listener = spawn_layer_event_source(
        WinUiSink {
            pending: Arc::new(Mutex::new(PendingTransition::default())),
            set_transition,
        },
        simulated,
    );
    native::install_listener(listener);
}

fn hidden_canvas() -> Element {
    Canvas::new(Vec::<Element>::new())
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

fn model_canvas(model: OverlayModel) -> Element {
    let mut children = Vec::new();
    children.push(
        label(format!("L{}", model.layer), model.header_font_size)
            .width(f64::from(model.width.saturating_sub(40)))
            .height(30.0)
            .canvas_left(20.0)
            .canvas_top(14.0)
            .with_key("header")
            .into(),
    );
    for (index, key) in model.keys.iter().enumerate() {
        children.push(
            border(label(key.label.join("\n"), model.key_font_size))
                .width(f64::from(key.width))
                .height(f64::from(key.height))
                .background(if key.held {
                    BrushBinding::Direct(HELD_FILL)
                } else {
                    BrushBinding::Direct(KEY_FILL)
                })
                .border_brush(tokens::CardStroke)
                .border_thickness(Thickness::uniform(1.0))
                .corner_radius(11.0)
                .canvas_left(f64::from(key.x))
                .canvas_top(f64::from(key.y))
                .with_key(format!("key-{index}"))
                .into(),
        );
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
        children.push(
            Shape::ellipse()
                .width(size)
                .height(size)
                .fill(if encoder.held { HELD_FILL } else { KEY_FILL })
                .stroke(ENCODER_STROKE)
                .stroke_thickness(1.0)
                .canvas_left(x)
                .canvas_top(y)
                .with_key(format!("encoder-{index}"))
                .into(),
        );
        children.push(
            label(encoder.counter_clockwise.join(" "), model.encoder_font_size)
                .width(label_width)
                .height(label_height)
                .canvas_left(counter_clockwise_left)
                .canvas_top(label_top)
                .with_key(format!("encoder-{index}-ccw"))
                .into(),
        );
        children.push(
            label(encoder.clockwise.join(" "), model.encoder_font_size)
                .width(label_width)
                .height(label_height)
                .canvas_left(clockwise_left)
                .canvas_top(label_top)
                .with_key(format!("encoder-{index}-cw"))
                .into(),
        );
        if !encoder.press.is_empty() {
            children.push(
                border(label(
                    format!("P {}", encoder.press),
                    model.encoder_font_size,
                ))
                .width(size)
                .height(size)
                .canvas_left(x)
                .canvas_top(y)
                .with_key(format!("encoder-{index}-press"))
                .into(),
            );
        }
    }
    border(
        Canvas::new(children)
            .width(f64::from(model.width))
            .height(f64::from(model.height))
            .background(TRANSPARENT),
    )
    .width(f64::from(model.width))
    .height(f64::from(model.height))
    .background(OVERLAY_FILL)
    .border_brush(OVERLAY_STROKE)
    .border_thickness(Thickness::uniform(1.0))
    .corner_radius(16.0)
    .into()
}

fn label(text: impl Into<String>, font_size: f64) -> windows_reactor::TextBlock {
    text_block(text)
        .font_family("Segoe UI")
        .font_size(font_size)
        .foreground(tokens::PrimaryText)
        .wrap()
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Center)
}
