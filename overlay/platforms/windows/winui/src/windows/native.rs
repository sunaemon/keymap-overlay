//! Audited Win32 boundary for the experimental WinUI overlay window.

use keymap_overlay_runtime::{LayerEventSourceHandle, PendingTransition, Transition};
use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    CreateRoundRectRgn, DeleteObject, GetMonitorInfoW, HGDIOBJ, HMONITOR, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromPoint, SetWindowRgn,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, FindWindowW, GWL_EXSTYLE, GWL_STYLE, GWLP_WNDPROC, GetCursorPos,
    GetWindowLongPtrW, HWND_TOPMOST, LWA_ALPHA, LWA_COLORKEY, PostMessageW, SET_WINDOW_POS_FLAGS,
    SW_HIDE, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WM_APP, WM_DEVICECHANGE, WNDPROC, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_core::HRESULT;
use windows_reactor::{LocalSender, WindowSize};

pub(super) const WINDOW_TITLE: &str = "Keymap Overlay WinUI Prototype";

const DBT_DEVNODES_CHANGED: usize = 0x0007;
const WM_OVERLAY_TRANSITION: u32 = WM_APP + 1;
const OVERLAY_CORNER_RADIUS: f64 = 16.0;

static LISTENER: OnceLock<LayerEventSourceHandle> = OnceLock::new();

thread_local! {
    static ORIGINAL_WINDOW_PROC: Cell<isize> = const { Cell::new(0) };
    static OVERLAY_WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static PENDING: Cell<Option<Arc<Mutex<PendingTransition>>>> = const { Cell::new(None) };
    static SENDER: Cell<Option<LocalSender<Transition>>> = const { Cell::new(None) };
    static REQUESTED_SIZE: Cell<WindowSize> = const {
        Cell::new(WindowSize { width: 1.0, height: 1.0 })
    };
    static WINDOW_VISIBLE: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn install_listener(listener: LayerEventSourceHandle) {
    if LISTENER.set(listener).is_err() {
        log::error!("The WinUI layer event source was already initialized");
    }
}

pub(super) fn configure_window(
    sender: LocalSender<Transition>,
    pending: Arc<Mutex<PendingTransition>>,
    shared_window: Arc<AtomicIsize>,
) -> windows_core::Result<()> {
    let window = unsafe {
        FindWindowW(
            windows::core::PCWSTR::null(),
            windows::core::w!("Keymap Overlay WinUI Prototype"),
        )
    }
    .map_err(map_windows_error)?;
    let style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32
        | WS_EX_LAYERED.0
        | WS_EX_TRANSPARENT.0
        | WS_EX_TOOLWINDOW.0
        | WS_EX_NOACTIVATE.0;
    unsafe {
        SetWindowLongPtrW(window, GWL_STYLE, WS_POPUP.0 as isize);
        SetWindowLongPtrW(window, GWL_EXSTYLE, style as isize);
        let original = SetWindowLongPtrW(
            window,
            GWLP_WNDPROC,
            window_proc as *const () as usize as isize,
        );
        if original == 0 {
            return Err(windows_core::Error::new(
                HRESULT(0x80004005_u32 as i32),
                "Failed to subclass the WinUI window",
            ));
        }
        ORIGINAL_WINDOW_PROC.set(original);
        let _ = ShowWindow(window, SW_HIDE);
    }
    OVERLAY_WINDOW.set(window);
    PENDING.set(Some(pending));
    SENDER.set(Some(sender));
    shared_window.store(window.0 as isize, Ordering::Release);
    wake_window(window.0 as isize);
    Ok(())
}

pub(super) fn request_window(size: WindowSize) {
    REQUESTED_SIZE.set(size);
    OVERLAY_WINDOW.with(|window| {
        let window = window.get();
        if !window.0.is_null() {
            present_window(window);
        }
    });
}

pub(super) fn wake_window(raw_window: isize) {
    if raw_window == 0 {
        return;
    }
    let window = HWND(raw_window as *mut _);
    if let Err(error) =
        unsafe { PostMessageW(Some(window), WM_OVERLAY_TRANSITION, WPARAM(0), LPARAM(0)) }
    {
        log::error!("Failed to wake the WinUI window: {error}");
    }
}

fn map_windows_error(error: windows::core::Error) -> windows_core::Error {
    windows_core::Error::from_hresult(HRESULT(error.code().0))
}

fn present_window(window: HWND) {
    let size = REQUESTED_SIZE.get();
    if size.width <= 1.0 || size.height <= 1.0 {
        if WINDOW_VISIBLE.replace(false) {
            unsafe {
                let _ = ShowWindow(window, SW_HIDE);
            }
        }
        return;
    }

    let first_show = !WINDOW_VISIBLE.replace(true);
    let (x, y, width, height) = visible_window_bounds(size);
    let hidden_flags = SET_WINDOW_POS_FLAGS(SWP_NOACTIVATE.0 | SWP_FRAMECHANGED.0);
    unsafe {
        if first_show {
            let _ = SetLayeredWindowAttributes(window, COLORREF(0), 0, LWA_COLORKEY | LWA_ALPHA);
        }
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            hidden_flags,
        );
        apply_rounded_region(
            window,
            width,
            height,
            OVERLAY_CORNER_RADIUS * 2.0 * monitor_scale_at(x, y),
        );
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        if first_show {
            let _ = DwmFlush();
            let _ = SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_COLORKEY | LWA_ALPHA);
        }
    }
}

fn apply_rounded_region(window: HWND, width: i32, height: i32, diameter: f64) {
    unsafe {
        let region = CreateRoundRectRgn(
            0,
            0,
            width + 1,
            height + 1,
            diameter.round() as i32,
            diameter.round() as i32,
        );
        if region.is_invalid() {
            log::error!("Failed to create the rounded WinUI window region");
            return;
        }
        if SetWindowRgn(window, Some(region), true) == 0 {
            log::error!("Failed to apply the rounded WinUI window region");
            let _ = DeleteObject(HGDIOBJ(region.0));
        }
    }
}

fn monitor_scale_at(x: i32, y: i32) -> f64 {
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    monitor_scale(monitor)
}

fn visible_window_bounds(size: WindowSize) -> (i32, i32, i32, i32) {
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        return (0, 0, size.width.round() as i32, size.height.round() as i32);
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..MONITORINFO::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        let scale = monitor_scale(monitor);
        let width = (size.width * scale).round() as i32;
        let height = (size.height * scale).round() as i32;
        let work_width = info.rcWork.right - info.rcWork.left;
        let work_height = info.rcWork.bottom - info.rcWork.top;
        let x = info.rcWork.left + (work_width - width) / 2;
        let y = info.rcWork.top + (work_height - height) / 2;
        (x, y, width, height)
    } else {
        (0, 0, size.width.round() as i32, size.height.round() as i32)
    }
}

fn monitor_scale(monitor: HMONITOR) -> f64 {
    let mut dpi_x = 96;
    let mut dpi_y = 96;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.is_ok() {
        f64::from(dpi_x) / 96.0
    } else {
        1.0
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    parameter: WPARAM,
    data: LPARAM,
) -> LRESULT {
    if message == WM_OVERLAY_TRANSITION {
        let transition = PENDING
            .take()
            .map(|pending| {
                let transition = pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                PENDING.set(Some(pending));
                transition
            })
            .unwrap_or(Transition::Ignore);
        if transition != Transition::Ignore {
            let accepted = SENDER
                .take()
                .map(|sender| {
                    let accepted = sender.send(transition);
                    SENDER.set(Some(sender));
                    accepted
                })
                .unwrap_or(false);
            if !accepted {
                log::error!("The WinUI component rejected a layer transition");
            }
        }
        return LRESULT(0);
    }
    if message == WM_DEVICECHANGE
        && parameter.0 == DBT_DEVNODES_CHANGED
        && let Some(listener) = LISTENER.get()
    {
        listener.device_arrived();
    }
    let original = ORIGINAL_WINDOW_PROC.get();
    let procedure: WNDPROC = unsafe { std::mem::transmute(original) };
    unsafe { CallWindowProcW(procedure, window, message, parameter, data) }
}
