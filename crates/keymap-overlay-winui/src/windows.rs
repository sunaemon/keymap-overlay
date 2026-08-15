//! Experimental pure-Rust WinUI 3 frontend.

#[allow(unsafe_code)]
mod native;

use anyhow::{Context, Result};
use keymap_overlay::{
    LayerEventSink, ListenerEvent, ModelCache, OverlayModel, PendingTransition, Transition,
    assets_dir, compose_model, initialize_logging, load_model_cache, spawn_raw_hid_listener,
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
}

impl Component for OverlayComponent {
    fn render(&self, _props: &(), context: &mut RenderCx) -> Element {
        render(context, Arc::clone(&self.models))
    }
}

#[derive(Clone)]
struct WinUiSink {
    pending: Arc<Mutex<PendingTransition>>,
    set_transition: AsyncSetState<Transition>,
}

impl LayerEventSink for WinUiSink {
    fn send(&self, event: ListenerEvent) -> bool {
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
    initialize_logging()?;
    let models = Arc::new(load_model_cache(&assets_dir()?)?);
    windows_reactor::bootstrap().context("Failed to initialize the Windows App SDK runtime")?;
    native::run(OverlayComponent { models }).context("The WinUI event loop failed")?;
    Ok(())
}

fn render(context: &mut windows_reactor::RenderCx, models: Arc<ModelCache>) -> Element {
    let (transition, set_transition) = context.use_async_state(Transition::Hide);
    context.use_effect((), move || start_listener(set_transition));

    let model = match &transition {
        Transition::Show {
            keyboard_id,
            layers,
        } => compose_model(&models, *keyboard_id, layers),
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

fn start_listener(set_transition: AsyncSetState<Transition>) {
    let listener = spawn_raw_hid_listener(WinUiSink {
        pending: Arc::new(Mutex::new(PendingTransition::default())),
        set_transition,
    });
    native::install_listener(listener);
}

fn hidden_canvas() -> Element {
    Canvas::new(Vec::<Element>::new())
        .width(1.0)
        .height(1.0)
        .background(TRANSPARENT)
        .into()
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
        let label_top = y - 30.0;
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
                .height(26.0)
                .canvas_left(center_x - label_width - label_gap / 2.0)
                .canvas_top(label_top)
                .with_key(format!("encoder-{index}-ccw"))
                .into(),
        );
        children.push(
            label(encoder.clockwise.join(" "), model.encoder_font_size)
                .width(label_width)
                .height(26.0)
                .canvas_left(center_x + label_gap / 2.0)
                .canvas_top(label_top)
                .with_key(format!("encoder-{index}-cw"))
                .into(),
        );
        if !encoder.press.is_empty() {
            children.push(
                label(format!("P {}", encoder.press), model.encoder_font_size)
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
