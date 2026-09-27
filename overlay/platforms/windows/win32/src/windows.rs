#![allow(unsafe_code)]

//! Production Windows frontend using the stable Win32 API through windows-rs.

use anyhow::{Context as _, Result, anyhow};
use keymap_overlay_runtime::{
    Arguments, LayerEvent, LayerEventSink, LogDestination, ModelStore, OverlayModel,
    OverlayPosition, OverlayPreferences, Parser as _, PendingTransition, Transition,
    default_log_file,
    desktop_tray::{DesktopTray, TrayCommand},
    initialize_logging, spawn_layer_event_source, startup_models, write_notice,
};
use std::env;
use std::ffi::OsString;
#[cfg(test)]
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::windows::process::CommandExt as _;
use std::process::Command;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
    GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, MONITOR_DEFAULTTONEAREST, MONITORINFO,
    MonitorFromPoint, ReleaseDC, SelectObject,
};
use windows::Win32::Graphics::GdiPlus::{
    CompositingModeSourceCopy, CompositingModeSourceOver, FillModeAlternate, FontStyleRegular,
    GdipAddPathArc, GdipAddPathLine, GdipClosePathFigure, GdipCreateFont,
    GdipCreateFontFamilyFromName, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
    GdipCreateSolidFill, GdipCreateStringFormat, GdipDeleteBrush, GdipDeleteFont,
    GdipDeleteFontFamily, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen,
    GdipDeleteStringFormat, GdipDrawEllipse, GdipDrawPath, GdipDrawString, GdipFillEllipse,
    GdipFillPath, GdipGraphicsClear, GdipScaleWorldTransform, GdipSetCompositingMode,
    GdipSetSmoothingMode, GdipSetStringFormatAlign, GdipSetStringFormatLineAlign,
    GdipSetTextRenderingHint, GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GpFont,
    GpFontFamily, GpGraphics, GpPath, GpSolidFill, GpStringFormat, MatrixOrderPrepend,
    Ok as GDI_PLUS_OK, RectF, SmoothingModeAntiAlias8x8, StringAlignmentCenter,
    StringAlignmentNear, TextRenderingHintAntiAliasGridFit, UnitPixel,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_USERDATA,
    GetCursorPos, GetMessageW, GetWindowLongPtrW, HMENU, HWND_TOPMOST, IDC_ARROW, LoadCursorW, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow,
    WM_APP, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_DEVICECHANGE, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

const WINDOW_CLASS: PCWSTR = w!("KeymapOverlayWindow");
const WINDOW_TITLE: PCWSTR = w!("Keymap Overlay");
const WM_OVERLAY_TRANSITION: u32 = WM_APP + 1;
const WM_TRAY_COMMAND: u32 = WM_APP + 2;
const CREATE_NO_WINDOW: u32 = 0x08000000;
const DBT_DEVNODES_CHANGED: usize = 0x0007;
const WINDOW_EDGE: i32 = 1;
const OUTER_CORNER_RADIUS: f32 = 16.0;
const KEY_CORNER_RADIUS: f32 = 11.0;
const HEADER_HORIZONTAL_INSET: f32 = 20.0;
const HEADER_TOP: f32 = 14.0;
const HEADER_HEIGHT: f32 = 30.0;
const ENCODER_LABEL_WIDTH_RATIO: f32 = 0.7;
const ENCODER_LABEL_GAP: f32 = 3.0;
const ENCODER_LABEL_VERTICAL_OFFSET: f32 = 30.0;
const ENCODER_LABEL_HEIGHT: f32 = 26.0;
const OVERLAY_FILL: u32 = 0xE8D8E0EA;
const OVERLAY_BORDER: u32 = 0x70606773;
const KEY_FILL: u32 = 0xE0F1F4F8;
const HELD_FILL: u32 = 0xFFFFDDDD;
const KEY_BORDER: u32 = 0x6020242C;
const TEXT_FILL: u32 = 0xFF20242C;

static LISTENER: OnceLock<keymap_overlay_runtime::LayerEventSourceHandle> = OnceLock::new();

struct State {
    models: ModelStore,
    pending: Arc<Mutex<PendingTransition>>,
    window: AtomicIsize,
    preferences: Mutex<OverlayPreferences>,
    launch_at_login: Mutex<bool>,
    tray: Mutex<Option<DesktopTray>>,
    tray_commands: Mutex<Receiver<TrayCommand>>,
    visible_model: Mutex<Option<OverlayModel>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f32,
}

struct EncoderLayout {
    shape: RectF,
    counter_clockwise: TextLayout,
    clockwise: TextLayout,
    press: TextLayout,
}

struct TextLayout {
    rectangle: RectF,
    text: String,
}

struct GdiPlusToken(usize);

impl Drop for GdiPlusToken {
    fn drop(&mut self) {
        unsafe { GdiplusShutdown(self.0) };
    }
}

struct RenderSurface {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    graphics: *mut GpGraphics,
}

impl Drop for RenderSurface {
    fn drop(&mut self) {
        unsafe {
            let _ = GdipDeleteGraphics(self.graphics);
            let _ = SelectObject(self.memory, self.previous_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.memory);
            let _ = ReleaseDC(None, self.screen);
        }
    }
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
    if let Err(error) =
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
    {
        log::debug!("Windows DPI awareness was already configured: {error}");
    }
    let _gdi_plus = start_gdi_plus()?;
    let startup = startup_models(simulated)?;
    let models = ModelStore::new(startup.models);
    let preferences = OverlayPreferences::load()?;
    let launch_at_login = launch_at_login_enabled();
    let pending = Arc::new(Mutex::new(PendingTransition::default()));
    let (tray_sender, tray_commands) = mpsc::channel();
    let e2e_tray_sender = tray_sender.clone();
    let state = Box::new(State {
        models: models.clone(),
        pending: Arc::clone(&pending),
        window: AtomicIsize::new(0),
        preferences: Mutex::new(preferences),
        launch_at_login: Mutex::new(launch_at_login),
        tray: Mutex::new(None),
        tray_commands: Mutex::new(tray_commands),
        visible_model: Mutex::new(None),
    });
    let window = create_window(Box::into_raw(state))?;
    let state = unsafe { state_from_window(window) };
    state.window.store(window.0 as isize, Ordering::Release);
    let tray_window = window.0 as isize;
    let tray = DesktopTray::new(preferences, launch_at_login, move |command| {
        if tray_sender.send(command).is_ok() {
            let window = HWND(tray_window as *mut _);
            if let Err(error) =
                unsafe { PostMessageW(Some(window), WM_TRAY_COMMAND, WPARAM(0), LPARAM(0)) }
            {
                log::error!("Failed to wake the Windows tray menu: {error}");
            }
        }
    })?;
    *state
        .tray
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(tray);
    if env::var_os("KEYMAP_OVERLAY_E2E_EXERCISE_TRAY").is_some_and(|value| value == "1") {
        for command in [
            TrayCommand::SetPosition(OverlayPosition::Top),
            TrayCommand::SetOpacity(75),
            TrayCommand::SetScale(125),
            TrayCommand::Reload,
        ] {
            e2e_tray_sender
                .send(command)
                .context("Failed to queue a Windows E2E tray command")?;
        }
    }
    let event_window = Arc::new(AtomicIsize::new(window.0 as isize));
    let listener = spawn_layer_event_source(
        Sink {
            pending,
            window: event_window,
        },
        simulated,
        startup.raw_hid_devices,
        models,
    );
    let _ = LISTENER.set(listener);
    if env::var_os("KEYMAP_OVERLAY_E2E_EXERCISE_TRAY").is_some_and(|value| value == "1") {
        unsafe { apply_tray_commands(window) };
    }
    message_loop()
}

fn start_gdi_plus() -> Result<GdiPlusToken> {
    let input = GdiplusStartupInput {
        GdiplusVersion: 1,
        ..Default::default()
    };
    let mut token = 0;
    let status = unsafe { GdiplusStartup(&mut token, &input, std::ptr::null_mut()) };
    check_gdi_plus(status, "start GDI+")?;
    Ok(GdiPlusToken(token))
}

fn create_window(state: *mut State) -> Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: WINDOW_CLASS,
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        RegisterClassW(&class);
        let window = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
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
    if message == WM_TRAY_COMMAND {
        unsafe { apply_tray_commands(window) };
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
    let model = model_for_transition(&state.models, &transition);
    write_e2e_state(&transition, model.as_ref());
    *state
        .visible_model
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = model.clone();
    let preferences = *state
        .preferences
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(model) = model {
        if let Err(error) = unsafe { present_model(window, &model, preferences) } {
            log::error!("Failed to render the Windows overlay: {error:#}");
            unsafe { hide_window(window) };
        }
    } else {
        unsafe { hide_window(window) };
    }
}

unsafe fn apply_tray_commands(window: HWND) {
    let state = unsafe { state_from_window(window) };
    let commands = state
        .tray_commands
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .try_iter()
        .collect::<Vec<_>>();
    for command in commands {
        if matches!(command, TrayCommand::Reload) {
            if let Some(listener) = LISTENER.get()
                && listener.reload_keyboards()
            {
                log::info!("Reloading connected keyboard models");
            }
            continue;
        }
        if matches!(command, TrayCommand::Quit) {
            let _ = unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            return;
        }
        if matches!(command, TrayCommand::ToggleLaunchAtLogin) {
            let mut launch_at_login = state
                .launch_at_login
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let enabled = !*launch_at_login;
            if let Err(error) = set_launch_at_login(enabled) {
                log::error!("Failed to change the launch-at-login setting: {error:#}");
            } else {
                *launch_at_login = enabled;
            }
            if let Some(tray) = state
                .tray
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .as_mut()
            {
                let preferences = *state
                    .preferences
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                tray.sync(preferences, *launch_at_login);
            }
            continue;
        }
        let preferences = {
            let mut preferences = state
                .preferences
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let next = command
                .updated_preferences(*preferences)
                .expect("action commands are handled before preference updates");
            if let Err(error) = next.save() {
                log::error!("Failed to save overlay preferences: {error:#}");
                if let Some(tray) = state
                    .tray
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_mut()
                {
                    let launch_at_login = *state
                        .launch_at_login
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    tray.sync(*preferences, launch_at_login);
                }
                continue;
            }
            *preferences = next;
            next
        };
        if let Some(tray) = state
            .tray
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
        {
            let launch_at_login = *state
                .launch_at_login
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            tray.sync(preferences, launch_at_login);
        }
        let model = state
            .visible_model
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(model) = model {
            if let Err(error) = unsafe { present_model(window, &model, preferences) } {
                log::error!("Failed to apply Windows overlay preferences: {error:#}");
                unsafe { hide_window(window) };
            }
        } else {
            unsafe { hide_window(window) };
        }
    }
}

fn launch_at_login_enabled() -> bool {
    let mut command = Command::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW).args([
        "query",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
        "/v",
        "KeymapOverlay",
    ]);
    command.status().is_ok_and(|status| status.success())
}

fn set_launch_at_login(enabled: bool) -> Result<()> {
    let mut command = Command::new("reg.exe");
    command.creation_flags(CREATE_NO_WINDOW).args([
        if enabled { "add" } else { "delete" },
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
        "/v",
        "KeymapOverlay",
    ]);
    if enabled {
        let executable = env::current_exe().context("Failed to find the overlay executable")?;
        command
            .args(["/t", "REG_SZ", "/d"])
            .arg(format!("\"{}\"", executable.display()))
            .arg("/f");
    } else {
        command.arg("/f");
    }
    let status = command
        .status()
        .context("Failed to update the Windows startup registry value")?;
    anyhow::ensure!(status.success(), "reg.exe failed to update launch at login");
    Ok(())
}

fn model_for_transition(models: &ModelStore, transition: &Transition) -> Option<OverlayModel> {
    match transition {
        Transition::Show {
            keyboard_id,
            layers,
        } => models.compose(*keyboard_id, layers),
        Transition::Hide => None,
        Transition::Ignore => unreachable!("handled before changing the window"),
    }
}

unsafe fn hide_window(window: HWND) {
    unsafe {
        let _ = SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 1, 1, SWP_NOACTIVATE);
        let _ = ShowWindow(window, SW_HIDE);
    }
}

unsafe fn present_model(
    window: HWND,
    model: &OverlayModel,
    preferences: OverlayPreferences,
) -> Result<()> {
    let bounds = visible_window_bounds(model, preferences);
    let surface = unsafe { RenderSurface::new(bounds.width, bounds.height)? };
    unsafe { draw_model(surface.graphics, model, bounds.scale)? };

    let destination = POINT {
        x: bounds.x,
        y: bounds.y,
    };
    let size = SIZE {
        cx: bounds.width,
        cy: bounds.height,
    };
    let source = POINT::default();
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: ((u16::from(preferences.opacity_percent) * 255) / 100) as u8,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        UpdateLayeredWindow(
            window,
            Some(surface.screen),
            Some(&destination),
            Some(&size),
            Some(surface.memory),
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )?;
        SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOACTIVATE,
        )?;
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

impl RenderSurface {
    unsafe fn new(width: i32, height: i32) -> Result<Self> {
        let screen = unsafe { GetDC(None) };
        if screen.0.is_null() {
            return Err(anyhow!("GetDC returned a null display context"));
        }
        let memory = unsafe { CreateCompatibleDC(Some(screen)) };
        if memory.0.is_null() {
            unsafe {
                let _ = ReleaseDC(None, screen);
            }
            return Err(anyhow!(
                "CreateCompatibleDC returned a null display context"
            ));
        }
        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = std::ptr::null_mut();
        let bitmap = match unsafe {
            CreateDIBSection(
                Some(screen),
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut pixels,
                None,
                0,
            )
        } {
            Ok(bitmap) => bitmap,
            Err(error) => {
                unsafe {
                    let _ = DeleteDC(memory);
                    let _ = ReleaseDC(None, screen);
                }
                return Err(error.into());
            }
        };
        let previous_bitmap = unsafe { SelectObject(memory, bitmap.into()) };
        let mut graphics = std::ptr::null_mut();
        let status = unsafe { GdipCreateFromHDC(memory, &mut graphics) };
        if status != GDI_PLUS_OK {
            unsafe {
                let _ = SelectObject(memory, previous_bitmap);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory);
                let _ = ReleaseDC(None, screen);
            }
            return Err(gdi_plus_error(status, "create a GDI+ graphics context"));
        }
        Ok(Self {
            screen,
            memory,
            bitmap,
            previous_bitmap,
            graphics,
        })
    }
}

unsafe fn draw_model(graphics: *mut GpGraphics, model: &OverlayModel, scale: f32) -> Result<()> {
    check_gdi_plus(
        unsafe { GdipSetCompositingMode(graphics, CompositingModeSourceCopy) },
        "configure transparent compositing",
    )?;
    check_gdi_plus(
        unsafe { GdipGraphicsClear(graphics, 0) },
        "clear the overlay surface",
    )?;
    check_gdi_plus(
        unsafe { GdipSetCompositingMode(graphics, CompositingModeSourceOver) },
        "configure source-over compositing",
    )?;
    check_gdi_plus(
        unsafe { GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8) },
        "enable antialiasing",
    )?;
    check_gdi_plus(
        unsafe { GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit) },
        "enable text antialiasing",
    )?;
    check_gdi_plus(
        unsafe { GdipScaleWorldTransform(graphics, scale, scale, MatrixOrderPrepend) },
        "apply monitor scaling",
    )?;

    unsafe {
        draw_rounded_rectangle(
            graphics,
            overlay_rectangle(model),
            OUTER_CORNER_RADIUS,
            OVERLAY_FILL,
            OVERLAY_BORDER,
        )?;
    }
    let fonts = unsafe { FontResources::new(model)? };
    unsafe {
        draw_text(
            graphics,
            &format!("L{}", model.layer),
            header_rectangle(model),
            fonts.header,
            StringAlignmentNear,
            fonts.text_brush,
        )?;
    }
    for key in &model.keys {
        let shape = key_shape_rectangle(key);
        let text = key_text_rectangle(key);
        unsafe {
            draw_rounded_rectangle(
                graphics,
                shape,
                KEY_CORNER_RADIUS,
                if key.held { HELD_FILL } else { KEY_FILL },
                KEY_BORDER,
            )?;
            draw_text(
                graphics,
                &key.label.join("\n"),
                text,
                fonts.key,
                StringAlignmentCenter,
                fonts.text_brush,
            )?;
        }
    }
    for encoder in &model.encoders {
        unsafe { draw_encoder(graphics, encoder, fonts.encoder, fonts.text_brush)? };
    }
    Ok(())
}

struct FontResources {
    family: *mut GpFontFamily,
    header: *mut GpFont,
    key: *mut GpFont,
    encoder: *mut GpFont,
    text_brush: *mut GpSolidFill,
}

impl FontResources {
    unsafe fn new(model: &OverlayModel) -> Result<Self> {
        let family_name: Vec<u16> = "Segoe UI".encode_utf16().chain(Some(0)).collect();
        let mut family = std::ptr::null_mut();
        check_gdi_plus(
            unsafe {
                GdipCreateFontFamilyFromName(
                    PCWSTR(family_name.as_ptr()),
                    std::ptr::null_mut(),
                    &mut family,
                )
            },
            "load Segoe UI",
        )?;
        let mut fonts = Self {
            family,
            header: std::ptr::null_mut(),
            key: std::ptr::null_mut(),
            encoder: std::ptr::null_mut(),
            text_brush: std::ptr::null_mut(),
        };
        unsafe { fonts.initialize(model) }?;
        Ok(fonts)
    }

    unsafe fn initialize(&mut self, model: &OverlayModel) -> Result<()> {
        check_gdi_plus(
            unsafe {
                GdipCreateFont(
                    self.family,
                    model.header_font_size as f32,
                    FontStyleRegular.0,
                    UnitPixel,
                    &mut self.header,
                )
            },
            "create the header font",
        )?;
        check_gdi_plus(
            unsafe {
                GdipCreateFont(
                    self.family,
                    model.key_font_size as f32,
                    FontStyleRegular.0,
                    UnitPixel,
                    &mut self.key,
                )
            },
            "create the key font",
        )?;
        check_gdi_plus(
            unsafe {
                GdipCreateFont(
                    self.family,
                    model.encoder_font_size as f32,
                    FontStyleRegular.0,
                    UnitPixel,
                    &mut self.encoder,
                )
            },
            "create the encoder font",
        )?;
        check_gdi_plus(
            unsafe { GdipCreateSolidFill(TEXT_FILL, &mut self.text_brush) },
            "create the text brush",
        )
    }
}

impl Drop for FontResources {
    fn drop(&mut self) {
        unsafe {
            if !self.text_brush.is_null() {
                let _ = GdipDeleteBrush(self.text_brush.cast());
            }
            if !self.encoder.is_null() {
                let _ = GdipDeleteFont(self.encoder);
            }
            if !self.key.is_null() {
                let _ = GdipDeleteFont(self.key);
            }
            if !self.header.is_null() {
                let _ = GdipDeleteFont(self.header);
            }
            if !self.family.is_null() {
                let _ = GdipDeleteFontFamily(self.family);
            }
        }
    }
}

unsafe fn draw_rounded_rectangle(
    graphics: *mut GpGraphics,
    rectangle: RectF,
    radius: f32,
    fill: u32,
    border: u32,
) -> Result<()> {
    let path = unsafe {
        create_rounded_path(
            rectangle.X,
            rectangle.Y,
            rectangle.Width,
            rectangle.Height,
            radius,
        )?
    };
    let mut brush = std::ptr::null_mut();
    let mut pen = std::ptr::null_mut();
    let result = (|| {
        check_gdi_plus(
            unsafe { GdipCreateSolidFill(fill, &mut brush) },
            "create a shape brush",
        )?;
        check_gdi_plus(
            unsafe { GdipCreatePen1(border, 1.0, UnitPixel, &mut pen) },
            "create a shape pen",
        )?;
        check_gdi_plus(
            unsafe { GdipFillPath(graphics, brush.cast(), path) },
            "fill a rounded rectangle",
        )?;
        check_gdi_plus(
            unsafe { GdipDrawPath(graphics, pen, path) },
            "outline a rounded rectangle",
        )
    })();
    unsafe {
        if !pen.is_null() {
            let _ = GdipDeletePen(pen);
        }
        if !brush.is_null() {
            let _ = GdipDeleteBrush(brush.cast());
        }
        let _ = GdipDeletePath(path);
    }
    result
}

unsafe fn create_rounded_path(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
) -> Result<*mut GpPath> {
    let radius = radius.min(width / 2.0).min(height / 2.0).max(0.0);
    let diameter = radius * 2.0;
    let right = x + width;
    let bottom = y + height;
    let mut path = std::ptr::null_mut();
    check_gdi_plus(
        unsafe { GdipCreatePath(FillModeAlternate, &mut path) },
        "create a rounded rectangle path",
    )?;
    let result = (|| {
        check_gdi_plus(
            unsafe { GdipAddPathLine(path, x + radius, y, right - radius, y) },
            "draw a rounded rectangle edge",
        )?;
        check_gdi_plus(
            unsafe { GdipAddPathArc(path, right - diameter, y, diameter, diameter, 270.0, 90.0) },
            "draw a rounded rectangle corner",
        )?;
        check_gdi_plus(
            unsafe { GdipAddPathLine(path, right, y + radius, right, bottom - radius) },
            "draw a rounded rectangle edge",
        )?;
        check_gdi_plus(
            unsafe {
                GdipAddPathArc(
                    path,
                    right - diameter,
                    bottom - diameter,
                    diameter,
                    diameter,
                    0.0,
                    90.0,
                )
            },
            "draw a rounded rectangle corner",
        )?;
        check_gdi_plus(
            unsafe { GdipAddPathLine(path, right - radius, bottom, x + radius, bottom) },
            "draw a rounded rectangle edge",
        )?;
        check_gdi_plus(
            unsafe { GdipAddPathArc(path, x, bottom - diameter, diameter, diameter, 90.0, 90.0) },
            "draw a rounded rectangle corner",
        )?;
        check_gdi_plus(
            unsafe { GdipAddPathLine(path, x, bottom - radius, x, y + radius) },
            "draw a rounded rectangle edge",
        )?;
        check_gdi_plus(
            unsafe { GdipAddPathArc(path, x, y, diameter, diameter, 180.0, 90.0) },
            "draw a rounded rectangle corner",
        )?;
        check_gdi_plus(
            unsafe { GdipClosePathFigure(path) },
            "close a rounded rectangle path",
        )
    })();
    if let Err(error) = result {
        unsafe {
            let _ = GdipDeletePath(path);
        }
        return Err(error);
    }
    Ok(path)
}

fn overlay_rectangle(model: &OverlayModel) -> RectF {
    let (width, height) = window_size(model);
    RectF {
        X: 0.5,
        Y: 0.5,
        Width: width as f32 - 1.0,
        Height: height as f32 - 1.0,
    }
}

fn header_rectangle(model: &OverlayModel) -> RectF {
    RectF {
        X: WINDOW_EDGE as f32 + HEADER_HORIZONTAL_INSET,
        Y: WINDOW_EDGE as f32 + HEADER_TOP,
        Width: model.width as f32 - HEADER_HORIZONTAL_INSET * 2.0,
        Height: HEADER_HEIGHT,
    }
}

fn key_shape_rectangle(key: &keymap_overlay_runtime::DisplayKey) -> RectF {
    let text = key_text_rectangle(key);
    RectF {
        X: text.X + 0.5,
        Y: text.Y + 0.5,
        Width: text.Width - 1.0,
        Height: text.Height - 1.0,
    }
}

fn key_text_rectangle(key: &keymap_overlay_runtime::DisplayKey) -> RectF {
    RectF {
        X: WINDOW_EDGE as f32 + key.x as f32,
        Y: WINDOW_EDGE as f32 + key.y as f32,
        Width: key.width as f32,
        Height: key.height as f32,
    }
}

fn encoder_layout(encoder: &keymap_overlay_runtime::DisplayEncoder) -> EncoderLayout {
    let x = WINDOW_EDGE as f32 + encoder.x as f32;
    let y = WINDOW_EDGE as f32 + encoder.y as f32;
    let size = encoder.size as f32;
    let center_x = x + size / 2.0;
    let label_width = size * ENCODER_LABEL_WIDTH_RATIO;
    let label_y = y - ENCODER_LABEL_VERTICAL_OFFSET;
    EncoderLayout {
        shape: RectF {
            X: x + 0.5,
            Y: y + 0.5,
            Width: size - 1.0,
            Height: size - 1.0,
        },
        counter_clockwise: TextLayout {
            rectangle: RectF {
                X: center_x - label_width - ENCODER_LABEL_GAP / 2.0,
                Y: label_y,
                Width: label_width,
                Height: ENCODER_LABEL_HEIGHT,
            },
            text: if encoder.counter_clockwise.is_empty() {
                String::new()
            } else {
                format!("← {}", encoder.counter_clockwise.join(" "))
            },
        },
        clockwise: TextLayout {
            rectangle: RectF {
                X: center_x + ENCODER_LABEL_GAP / 2.0,
                Y: label_y,
                Width: label_width,
                Height: ENCODER_LABEL_HEIGHT,
            },
            text: if encoder.clockwise.is_empty() {
                String::new()
            } else {
                format!("{} →", encoder.clockwise.join(" "))
            },
        },
        press: TextLayout {
            rectangle: RectF {
                X: x,
                Y: y,
                Width: size,
                Height: size,
            },
            text: if encoder.press.is_empty() {
                String::new()
            } else {
                format!("P {}", encoder.press)
            },
        },
    }
}

unsafe fn draw_encoder(
    graphics: *mut GpGraphics,
    encoder: &keymap_overlay_runtime::DisplayEncoder,
    font: *mut GpFont,
    text_brush: *mut GpSolidFill,
) -> Result<()> {
    let layout = encoder_layout(encoder);
    let mut fill = std::ptr::null_mut();
    let mut pen = std::ptr::null_mut();
    let shape_result = (|| {
        check_gdi_plus(
            unsafe {
                GdipCreateSolidFill(if encoder.held { HELD_FILL } else { KEY_FILL }, &mut fill)
            },
            "create an encoder brush",
        )?;
        check_gdi_plus(
            unsafe { GdipCreatePen1(KEY_BORDER, 1.0, UnitPixel, &mut pen) },
            "create an encoder pen",
        )?;
        check_gdi_plus(
            unsafe {
                GdipFillEllipse(
                    graphics,
                    fill.cast(),
                    layout.shape.X,
                    layout.shape.Y,
                    layout.shape.Width,
                    layout.shape.Height,
                )
            },
            "fill an encoder",
        )?;
        check_gdi_plus(
            unsafe {
                GdipDrawEllipse(
                    graphics,
                    pen,
                    layout.shape.X,
                    layout.shape.Y,
                    layout.shape.Width,
                    layout.shape.Height,
                )
            },
            "outline an encoder",
        )
    })();
    unsafe {
        if !pen.is_null() {
            let _ = GdipDeletePen(pen);
        }
        if !fill.is_null() {
            let _ = GdipDeleteBrush(fill.cast());
        }
    }
    shape_result?;

    unsafe {
        draw_text(
            graphics,
            &layout.counter_clockwise.text,
            layout.counter_clockwise.rectangle,
            font,
            StringAlignmentCenter,
            text_brush,
        )?;
        draw_text(
            graphics,
            &layout.clockwise.text,
            layout.clockwise.rectangle,
            font,
            StringAlignmentCenter,
            text_brush,
        )?;
        draw_text(
            graphics,
            &layout.press.text,
            layout.press.rectangle,
            font,
            StringAlignmentCenter,
            text_brush,
        )
    }
}

unsafe fn draw_text(
    graphics: *mut GpGraphics,
    text: &str,
    rectangle: RectF,
    font: *mut GpFont,
    alignment: windows::Win32::Graphics::GdiPlus::StringAlignment,
    brush: *mut GpSolidFill,
) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let mut format: *mut GpStringFormat = std::ptr::null_mut();
    check_gdi_plus(
        unsafe { GdipCreateStringFormat(0, 0, &mut format) },
        "create a text format",
    )?;
    let result = (|| {
        check_gdi_plus(
            unsafe { GdipSetStringFormatAlign(format, alignment) },
            "align text horizontally",
        )?;
        check_gdi_plus(
            unsafe { GdipSetStringFormatLineAlign(format, StringAlignmentCenter) },
            "align text vertically",
        )?;
        check_gdi_plus(
            unsafe {
                GdipDrawString(
                    graphics,
                    PCWSTR(wide.as_ptr()),
                    -1,
                    font,
                    &rectangle,
                    format,
                    brush.cast(),
                )
            },
            "draw text",
        )
    })();
    unsafe {
        let _ = GdipDeleteStringFormat(format);
    }
    result
}

#[cfg(test)]
fn render_scene_snapshot(model: &OverlayModel) -> String {
    let mut snapshot = String::new();
    let (width, height) = window_size(model);
    writeln!(snapshot, "canvas width={width} height={height}").expect("write to string");
    write_shape_snapshot(
        &mut snapshot,
        "round-rect",
        overlay_rectangle(model),
        Some(OUTER_CORNER_RADIUS),
        OVERLAY_FILL,
        OVERLAY_BORDER,
    );
    write_text_snapshot(
        &mut snapshot,
        header_rectangle(model),
        model.header_font_size,
        "near",
        &format!("L{}", model.layer),
    );
    for key in &model.keys {
        write_shape_snapshot(
            &mut snapshot,
            "round-rect",
            key_shape_rectangle(key),
            Some(KEY_CORNER_RADIUS),
            if key.held { HELD_FILL } else { KEY_FILL },
            KEY_BORDER,
        );
        write_text_snapshot(
            &mut snapshot,
            key_text_rectangle(key),
            model.key_font_size,
            "center",
            &key.label.join("\n"),
        );
    }
    for encoder in &model.encoders {
        let layout = encoder_layout(encoder);
        write_shape_snapshot(
            &mut snapshot,
            "ellipse",
            layout.shape,
            None,
            if encoder.held { HELD_FILL } else { KEY_FILL },
            KEY_BORDER,
        );
        for text in [layout.counter_clockwise, layout.clockwise, layout.press] {
            if !text.text.is_empty() {
                write_text_snapshot(
                    &mut snapshot,
                    text.rectangle,
                    model.encoder_font_size,
                    "center",
                    &text.text,
                );
            }
        }
    }
    snapshot
}

#[cfg(test)]
fn write_shape_snapshot(
    snapshot: &mut String,
    kind: &str,
    rectangle: RectF,
    radius: Option<f32>,
    fill: u32,
    border: u32,
) {
    write!(snapshot, "{kind} ").expect("write to string");
    write_rectangle_snapshot(snapshot, rectangle);
    if let Some(radius) = radius {
        write!(snapshot, " radius={radius:.2}").expect("write to string");
    }
    writeln!(
        snapshot,
        " fill=#{fill:08X} border=#{border:08X} width=1.00"
    )
    .expect("write to string");
}

#[cfg(test)]
fn write_text_snapshot(
    snapshot: &mut String,
    rectangle: RectF,
    font_size: f64,
    alignment: &str,
    text: &str,
) {
    write!(snapshot, "text ").expect("write to string");
    write_rectangle_snapshot(snapshot, rectangle);
    writeln!(
        snapshot,
        " font=\"Segoe UI\" size={font_size:.2} align={alignment}/center color=#{TEXT_FILL:08X} value={text:?}"
    )
    .expect("write to string");
}

#[cfg(test)]
fn write_rectangle_snapshot(snapshot: &mut String, rectangle: RectF) {
    write!(
        snapshot,
        "rect=({:.2},{:.2},{:.2},{:.2})",
        rectangle.X, rectangle.Y, rectangle.Width, rectangle.Height
    )
    .expect("write to string");
}

fn visible_window_bounds(model: &OverlayModel, preferences: OverlayPreferences) -> WindowBounds {
    let (logical_width, logical_height) = window_size(model);
    let preference_scale = f32::from(preferences.scale_percent) / 100.0;
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return WindowBounds {
            x: 0,
            y: 0,
            width: (logical_width as f32 * preference_scale).round() as i32,
            height: (logical_height as f32 * preference_scale).round() as i32,
            scale: preference_scale,
        };
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return WindowBounds {
            x: 0,
            y: 0,
            width: (logical_width as f32 * preference_scale).round() as i32,
            height: (logical_height as f32 * preference_scale).round() as i32,
            scale: preference_scale,
        };
    }
    let mut dpi_x = 96;
    let mut dpi_y = 96;
    let dpi_scale = if unsafe {
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y)
    }
    .is_ok()
    {
        dpi_x as f32 / 96.0
    } else {
        1.0
    };
    let scale = dpi_scale * preference_scale;
    positioned_window_bounds(
        info.rcWork,
        logical_width,
        logical_height,
        scale,
        preferences.position,
    )
}

#[cfg(test)]
fn centered_window_bounds(
    work_area: RECT,
    logical_width: i32,
    logical_height: i32,
    scale: f32,
) -> WindowBounds {
    positioned_window_bounds(
        work_area,
        logical_width,
        logical_height,
        scale,
        OverlayPosition::Center,
    )
}

fn positioned_window_bounds(
    work_area: RECT,
    logical_width: i32,
    logical_height: i32,
    scale: f32,
    position: OverlayPosition,
) -> WindowBounds {
    let width = (logical_width as f32 * scale).round() as i32;
    let height = (logical_height as f32 * scale).round() as i32;
    let y = match position {
        OverlayPosition::Top => work_area.top,
        OverlayPosition::Center => work_area.top + (work_area.bottom - work_area.top - height) / 2,
        OverlayPosition::Bottom => work_area.bottom - height,
    };
    WindowBounds {
        x: work_area.left + (work_area.right - work_area.left - width) / 2,
        y,
        width,
        height,
        scale,
    }
}

fn check_gdi_plus(
    status: windows::Win32::Graphics::GdiPlus::Status,
    action: &'static str,
) -> Result<()> {
    if status == GDI_PLUS_OK {
        Ok(())
    } else {
        Err(gdi_plus_error(status, action))
    }
}

fn gdi_plus_error(
    status: windows::Win32::Graphics::GdiPlus::Status,
    action: &'static str,
) -> anyhow::Error {
    anyhow!("GDI+ could not {action} (status {})", status.0)
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
            "show keyboard={keyboard_id} layers={layers:?} size={}x{} keys={} encoders={} held={} first_label={:?}",
            window_size(model).0,
            window_size(model).1,
            model.keys.len(),
            model.encoders.len(),
            model.keys.iter().filter(|key| key.held).count()
                + model.encoders.iter().filter(|encoder| encoder.held).count(),
            model.keys.first().map(|key| &key.label),
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

/// Returns the popup dimensions, including the one-pixel overlay border.
fn window_size(model: &OverlayModel) -> (i32, i32) {
    (
        model.width as i32 + WINDOW_EDGE * 2,
        model.height as i32 + WINDOW_EDGE * 2,
    )
}

unsafe fn state_from_window(window: HWND) -> &'static State {
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const State;
    unsafe { &*pointer }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keymap_overlay_runtime::{DisplayEncoder, DisplayKey, RawLayerEvent};
    use std::collections::HashMap;
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
    fn render_scene_matches_win32_reference_golden() {
        let model = OverlayModel {
            version: 2,
            layer: 2,
            width: 260,
            height: 180,
            header_font_size: 14.0,
            key_font_size: 10.0,
            encoder_font_size: 9.0,
            keys: vec![
                DisplayKey {
                    x: 20,
                    y: 60,
                    width: 60,
                    height: 50,
                    label: vec!["E2E".to_owned()],
                    held: true,
                    transparent: false,
                    momentary_layer: Some(2),
                },
                DisplayKey {
                    x: 90,
                    y: 60,
                    width: 60,
                    height: 50,
                    label: vec!["ENTER".to_owned()],
                    held: false,
                    transparent: false,
                    momentary_layer: None,
                },
            ],
            encoders: vec![encoder(180, 90, 50)],
        };
        let golden = include_str!("../tests/golden/win32-reference.scene").replace("\r\n", "\n");
        assert_eq!(render_scene_snapshot(&model), golden);
    }

    #[test]
    fn window_size_adds_only_the_transparent_edge_without_encoders() {
        let model = model(180, 140, vec![key(0, 0, 40, 40)], vec![]);
        assert_eq!(window_size(&model), (182, 142));
    }

    #[test]
    fn window_size_keeps_encoder_labels_inside_model_canvas() {
        let model = model(180, 140, vec![], vec![encoder(50, 60, 50)]);
        assert_eq!(window_size(&model), (182, 142));
    }

    #[test]
    fn centered_window_bounds_scale_and_center_on_monitor_work_area() {
        let bounds = centered_window_bounds(
            RECT {
                left: 1920,
                top: 0,
                right: 4480,
                bottom: 1400,
            },
            182,
            142,
            1.5,
        );
        assert_eq!(
            bounds,
            WindowBounds {
                x: 3063,
                y: 593,
                width: 273,
                height: 213,
                scale: 1.5,
            }
        );
    }

    #[test]
    fn window_bounds_honor_top_and_bottom_placement() {
        let work_area = RECT {
            left: 100,
            top: 50,
            right: 1_300,
            bottom: 850,
        };

        assert_eq!(
            positioned_window_bounds(work_area, 400, 200, 1.0, OverlayPosition::Top).y,
            50
        );
        assert_eq!(
            positioned_window_bounds(work_area, 400, 200, 1.0, OverlayPosition::Bottom).y,
            650
        );
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
            format!(
                "show keyboard=3 layers=[1] size={width}x{height} keys=0 encoders=1 held=0 first_label=None"
            )
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

    #[test]
    fn show_transitions_compose_the_shared_model_store() {
        let models = ModelStore::new(HashMap::from([((3, 0), model(180, 140, vec![], vec![]))]));

        let composed = model_for_transition(
            &models,
            &Transition::Show {
                keyboard_id: 3,
                layers: vec![0],
            },
        )
        .expect("show model");

        assert_eq!(composed.width, 180);
        assert!(model_for_transition(&models, &Transition::Hide).is_none());
    }

    #[test]
    #[should_panic(expected = "handled before changing the window")]
    fn ignored_transitions_never_reach_model_selection() {
        let models = ModelStore::new(HashMap::new());

        model_for_transition(&models, &Transition::Ignore);
    }
}
