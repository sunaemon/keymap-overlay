#![allow(unsafe_code)]

//! Production Windows frontend using the stable Win32 API through windows-rs.

use anyhow::Result;
use keymap_overlay_runtime::{
    Arguments, LayerEvent, LayerEventSink, LogDestination, ModelCache, OverlayModel, Parser as _,
    PendingTransition, Transition, compose_model, default_log_file, initialize_logging,
    spawn_layer_event_source, startup_models, write_notice,
};
use std::env;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DRAW_TEXT_FORMAT, DeleteObject, DrawTextW, Ellipse, EndPaint,
    FillRect, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, HMENU, HWND_TOPMOST,
    IDC_ARROW, LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, WM_APP, WM_CREATE, WM_DESTROY, WM_DEVICECHANGE, WM_PAINT,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

const WINDOW_CLASS: PCWSTR = w!("KeymapOverlayWindow");
const WINDOW_TITLE: PCWSTR = w!("Keymap Overlay");
const WM_OVERLAY_TRANSITION: u32 = WM_APP + 1;
const DBT_DEVNODES_CHANGED: usize = 0x0007;
const COLOR_KEY: COLORREF = COLORREF(0x00FF00FF);
const WINDOW_EDGE: i32 = 1;
/// Vertical room an encoder's counter-clockwise and clockwise labels need
/// above and below its circle.
const ENCODER_LABEL_MARGIN: i32 = 30;
const DT_CENTER: u32 = 0x0001;
const DT_CALCRECT: u32 = 0x0400;

static LISTENER: OnceLock<keymap_overlay_runtime::LayerEventSourceHandle> = OnceLock::new();

struct State {
    models: Arc<ModelCache>,
    pending: Arc<Mutex<PendingTransition>>,
    current: Mutex<Option<OverlayModel>>,
    window: AtomicIsize,
}

#[derive(Clone)]
struct Sink {
    pending: Arc<Mutex<PendingTransition>>,
    window: Arc<AtomicIsize>,
}

impl LayerEventSink for Sink {
    fn send(&self, event: LayerEvent) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
        let raw_window = self.window.load(Ordering::Acquire);
        if raw_window == 0 {
            log::error!("Cannot wake the Windows overlay before its window is ready");
            return false;
        }
        let window = HWND(raw_window as *mut _);
        if let Err(error) =
            unsafe { PostMessageW(Some(window), WM_OVERLAY_TRANSITION, WPARAM(0), LPARAM(0)) }
        {
            log::error!("Failed to wake the Windows overlay: {error}");
            return false;
        }
        true
    }
}

/// Runs the production Windows frontend without a framework or managed host.
pub(crate) fn run() -> Result<()> {
    let arguments = Arguments::parse();
    if let Some(notice) = arguments.notice() {
        return write_notice(notice);
    }
    let simulated = arguments.simulate;
    let destination = arguments
        .log_out
        .map(LogDestination::File)
        .unwrap_or(LogDestination::File(default_log_file()?));
    initialize_logging(destination)?;
    let startup = startup_models(simulated)?;
    let models = Arc::new(startup.models);
    let pending = Arc::new(Mutex::new(PendingTransition::default()));
    let state = Box::new(State {
        models: Arc::clone(&models),
        pending: Arc::clone(&pending),
        current: Mutex::new(None),
        window: AtomicIsize::new(0),
    });
    let window = create_window(Box::into_raw(state))?;
    let state = unsafe { state_from_window(window) };
    state.window.store(window.0 as isize, Ordering::Release);
    let event_window = Arc::new(AtomicIsize::new(window.0 as isize));
    let listener = spawn_layer_event_source(
        Sink {
            pending,
            window: event_window,
        },
        simulated,
        startup.raw_hid_devices,
        models.keys().map(|(keyboard_id, _)| *keyboard_id),
    );
    let _ = LISTENER.set(listener);
    message_loop()
}

fn create_window(state: *mut State) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: WINDOW_CLASS,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        RegisterClassW(&class);
        let window = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
            WINDOW_CLASS,
            WINDOW_TITLE,
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1,
            1,
            None,
            Some(HMENU::default()),
            Some(instance.into()),
            Some(state.cast()),
        )?;
        SetLayeredWindowAttributes(
            window,
            COLOR_KEY,
            255,
            windows::Win32::UI::WindowsAndMessaging::LWA_COLORKEY,
        )?;
        let _ = ShowWindow(window, SW_HIDE);
        Ok(window)
    }
}

fn message_loop() -> Result<()> {
    unsafe {
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).into() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    parameter: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if message == WM_CREATE {
        let create = unsafe { &*(data.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize) };
        return LRESULT(0);
    }
    if message == WM_DESTROY {
        let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut State;
        if !pointer.is_null() {
            drop(unsafe { Box::from_raw(pointer) });
        }
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    if message == WM_OVERLAY_TRANSITION {
        unsafe { apply_transition(window) };
        return LRESULT(0);
    }
    if message == WM_PAINT {
        unsafe { paint(window) };
        return LRESULT(0);
    }
    if message == WM_DEVICECHANGE
        && parameter.0 == DBT_DEVNODES_CHANGED
        && let Some(listener) = LISTENER.get()
    {
        listener.device_arrived();
    }
    unsafe { DefWindowProcW(window, message, parameter, data) }
}

unsafe fn apply_transition(window: HWND) {
    let state = unsafe { state_from_window(window) };
    let transition = state
        .pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if matches!(transition, Transition::Ignore) {
        return;
    }
    let model = match &transition {
        Transition::Show {
            keyboard_id,
            layers,
        } => compose_model(&state.models, *keyboard_id, layers),
        Transition::Hide => None,
        Transition::Ignore => unreachable!("handled before changing the window"),
    };
    write_e2e_state(&transition, model.as_ref());
    let (width, height) = model.as_ref().map(window_size).unwrap_or((1, 1));
    *state
        .current
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = model;
    unsafe {
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            0,
            0,
            width,
            height,
            SWP_NOACTIVATE,
        );
    };
    unsafe {
        let _ = InvalidateRect(Some(window), None, true);
    };
    unsafe {
        let _ = ShowWindow(
            window,
            if width == 1 {
                SW_HIDE
            } else {
                SW_SHOWNOACTIVATE
            },
        );
    };
}

/// Records native presentation transitions for the Windows E2E harness.
fn write_e2e_state(transition: &Transition, model: Option<&OverlayModel>) {
    write_e2e_state_to(
        env::var_os("KEYMAP_OVERLAY_E2E_STATE_FILE"),
        transition,
        model,
    );
}

/// Formats one transition and appends it to `path`, if set.
///
/// Takes the destination as a parameter, rather than reading the
/// environment directly, so the formatting is testable without
/// `env::set_var`, which is unsafe in this edition and the crate forbids
/// unsafe outside the reviewed Win32 window boundary.
fn write_e2e_state_to(
    path: Option<OsString>,
    transition: &Transition,
    model: Option<&OverlayModel>,
) {
    let Some(path) = path else {
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
            window_size(model).0,
            window_size(model).1,
            model.keys.len(),
            model.encoders.len(),
            model.keys.iter().filter(|key| key.held).count()
                + model.encoders.iter().filter(|encoder| encoder.held).count(),
        ),
        (Transition::Hide, _) => "hide size=1x1".to_owned(),
        _ => return,
    };
    if let Err(error) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| writeln!(file, "{state}"))
    {
        log::error!("Failed to record Windows E2E state: {error}");
    }
}

unsafe fn paint(window: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let context = unsafe { BeginPaint(window, &mut paint) };
    let background = unsafe { CreateSolidBrush(COLOR_KEY) };
    unsafe { FillRect(context, &paint.rcPaint, background) };
    let state = unsafe { state_from_window(window) };
    if let Some(model) = state
        .current
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
    {
        let vertical_offset = if model.encoders.is_empty() {
            0
        } else {
            ENCODER_LABEL_MARGIN
        };
        for key in &model.keys {
            let colour = if key.held {
                COLORREF(0x00DDDDFF)
            } else {
                COLORREF(0x00F4F1E0)
            };
            let brush = unsafe { CreateSolidBrush(colour) };
            let rect = RECT {
                left: key.x as i32 + WINDOW_EDGE,
                top: key.y as i32 + WINDOW_EDGE + vertical_offset,
                right: (key.x + key.width) as i32 + WINDOW_EDGE,
                bottom: (key.y + key.height) as i32 + WINDOW_EDGE + vertical_offset,
            };
            unsafe { FillRect(context, &rect, brush) };
            unsafe {
                let _ = DeleteObject(brush.into());
            };
            unsafe {
                draw_text(
                    context,
                    &key.label.join("\n"),
                    rect.left,
                    rect.top,
                    rect.right,
                    rect.bottom,
                );
            }
        }
        for encoder in &model.encoders {
            let colour = if encoder.held {
                COLORREF(0x00DDDDFF)
            } else {
                COLORREF(0x00F4F1E0)
            };
            let brush = unsafe { CreateSolidBrush(colour) };
            let rect = RECT {
                left: encoder.x as i32 + WINDOW_EDGE,
                top: encoder.y as i32 + WINDOW_EDGE + vertical_offset,
                right: (encoder.x + encoder.size) as i32 + WINDOW_EDGE,
                bottom: (encoder.y + encoder.size) as i32 + WINDOW_EDGE + vertical_offset,
            };
            let previous_brush = unsafe { SelectObject(context, brush.into()) };
            unsafe {
                let _ = Ellipse(context, rect.left, rect.top, rect.right, rect.bottom);
            };
            unsafe { SelectObject(context, previous_brush) };
            unsafe {
                let _ = DeleteObject(brush.into());
            };
            unsafe {
                draw_text(
                    context,
                    &encoder.counter_clockwise.join("\n"),
                    rect.left,
                    rect.top - ENCODER_LABEL_MARGIN,
                    rect.right,
                    rect.top,
                );
                draw_text(
                    context,
                    &encoder.clockwise.join("\n"),
                    rect.left,
                    rect.bottom,
                    rect.right,
                    rect.bottom + ENCODER_LABEL_MARGIN,
                );
                draw_text(
                    context,
                    &encoder.press,
                    rect.left,
                    rect.top,
                    rect.right,
                    rect.bottom,
                );
            }
        }
    }
    unsafe {
        let _ = DeleteObject(background.into());
    };
    unsafe {
        let _ = EndPaint(window, &paint);
    };
}

/// Returns the popup dimensions, including the transparent one-pixel edge
/// and, when the model has encoders, the vertical margin their
/// counter-clockwise and clockwise labels need above and below the canvas.
fn window_size(model: &OverlayModel) -> (i32, i32) {
    let vertical_margin = if model.encoders.is_empty() {
        0
    } else {
        ENCODER_LABEL_MARGIN * 2
    };
    (
        model.width as i32 + WINDOW_EDGE * 2,
        model.height as i32 + WINDOW_EDGE * 2 + vertical_margin,
    )
}

/// Draws a centered, multiline label in a GDI rectangle.
///
/// `DT_VCENTER` only centers single-line text, so multiline labels are
/// centered manually: a `DT_CALCRECT` pass measures the wrapped text, then
/// the real draw starts at the vertically centered offset.
unsafe fn draw_text(
    context: windows::Win32::Graphics::Gdi::HDC,
    text: &str,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) {
    let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut measured = RECT {
        left,
        top,
        right,
        bottom: top,
    };
    unsafe {
        DrawTextW(
            context,
            &mut wide,
            &mut measured,
            DRAW_TEXT_FORMAT(DT_CENTER | DT_CALCRECT),
        )
    };
    let mut rect = RECT {
        left,
        top: centered_top(top, bottom, measured.bottom - measured.top),
        right,
        bottom,
    };
    unsafe { SetBkMode(context, TRANSPARENT) };
    unsafe { SetTextColor(context, COLORREF(0x00202020)) };
    unsafe { DrawTextW(context, &mut wide, &mut rect, DRAW_TEXT_FORMAT(DT_CENTER)) };
}

/// Vertical offset from `top` that centers `text_height` pixels of text
/// within `[top, bottom)`, without moving text taller than the space.
fn centered_top(top: i32, bottom: i32, text_height: i32) -> i32 {
    let available = (bottom - top).max(0);
    top + (available - text_height).max(0) / 2
}

unsafe fn state_from_window(window: HWND) -> &'static State {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const State;
    unsafe { &*pointer }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keymap_overlay_runtime::{DisplayEncoder, DisplayKey, RawLayerEvent};
    use std::fs;
    use tempfile::TempDir;

    fn key(x: u32, y: u32, width: u32, height: u32) -> DisplayKey {
        DisplayKey {
            x,
            y,
            width,
            height,
            label: vec!["A".to_owned()],
            held: false,
            transparent: false,
            momentary_layer: None,
        }
    }

    fn encoder(x: u32, y: u32, size: u32) -> DisplayEncoder {
        DisplayEncoder {
            x,
            y,
            size,
            counter_clockwise: vec!["VOL".to_owned(), "DOWN".to_owned()],
            clockwise: vec!["VOL".to_owned(), "UP".to_owned()],
            press: "MUTE".to_owned(),
            held: false,
            counter_clockwise_transparent: false,
            clockwise_transparent: false,
            press_transparent: false,
            momentary_layer: None,
        }
    }

    fn model(
        width: u32,
        height: u32,
        keys: Vec<DisplayKey>,
        encoders: Vec<DisplayEncoder>,
    ) -> OverlayModel {
        OverlayModel {
            version: 2,
            layer: 0,
            width,
            height,
            header_font_size: 14.0,
            key_font_size: 10.0,
            encoder_font_size: 9.0,
            keys,
            encoders,
        }
    }

    #[test]
    fn window_size_adds_only_the_transparent_edge_without_encoders() {
        let model = model(180, 140, vec![key(0, 0, 40, 40)], vec![]);
        assert_eq!(window_size(&model), (182, 142));
    }

    #[test]
    fn window_size_reserves_margin_for_encoder_direction_labels() {
        let model = model(180, 140, vec![], vec![encoder(50, 60, 50)]);
        assert_eq!(
            window_size(&model),
            (
                180 + WINDOW_EDGE * 2,
                140 + WINDOW_EDGE * 2 + ENCODER_LABEL_MARGIN * 2,
            )
        );
    }

    #[test]
    fn centered_top_centers_shorter_text_in_the_available_space() {
        assert_eq!(centered_top(0, 40, 20), 10);
    }

    #[test]
    fn centered_top_does_not_move_text_that_exactly_fills_the_space() {
        assert_eq!(centered_top(0, 40, 40), 0);
    }

    #[test]
    fn centered_top_never_returns_a_negative_offset_for_oversized_text() {
        assert_eq!(centered_top(0, 10, 40), 0);
    }

    #[test]
    fn write_e2e_state_to_records_a_show_transition_with_computed_window_size() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join("state.log");
        let model = model(180, 140, vec![], vec![encoder(50, 60, 50)]);
        write_e2e_state_to(
            Some(path.clone().into_os_string()),
            &Transition::Show {
                keyboard_id: 3,
                layers: vec![1],
            },
            Some(&model),
        );
        let contents = fs::read_to_string(&path).expect("state file");
        let (width, height) = window_size(&model);
        assert_eq!(
            contents.trim(),
            format!("show keyboard=3 layers=[1] size={width}x{height} keys=0 encoders=1 held=0")
        );
    }

    #[test]
    fn write_e2e_state_to_records_a_hide_transition() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join("state.log");
        write_e2e_state_to(Some(path.clone().into_os_string()), &Transition::Hide, None);
        let contents = fs::read_to_string(&path).expect("state file");
        assert_eq!(contents.trim(), "hide size=1x1");
    }

    #[test]
    fn write_e2e_state_to_does_nothing_without_a_configured_path() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join("state.log");
        write_e2e_state_to(None, &Transition::Hide, None);
        assert!(!path.exists());
    }

    #[test]
    fn sink_send_queues_the_event_and_reports_failure_before_the_window_exists() {
        let pending = Arc::new(Mutex::new(PendingTransition::default()));
        let sink = Sink {
            pending: Arc::clone(&pending),
            window: Arc::new(AtomicIsize::new(0)),
        };
        let sent = sink.send(LayerEvent::Report(RawLayerEvent {
            keyboard_id: 3,
            layer: 1,
            pressed: true,
        }));
        assert!(!sent);
        assert_eq!(
            pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take(),
            Transition::Show {
                keyboard_id: 3,
                layers: vec![1],
            }
        );
    }
}
