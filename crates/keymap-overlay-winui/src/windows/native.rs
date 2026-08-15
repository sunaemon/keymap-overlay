//! Audited Win32 boundary for WinUI window interop.

use keymap_overlay::RawHidListenerHandle;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWL_EXSTYLE, GWL_STYLE, GWLP_WNDPROC, GetCursorPos, GetWindowLongPtrW,
    HWND_TOPMOST, LWA_COLORKEY, SET_WINDOW_POS_FLAGS, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_DEVICECHANGE, WNDPROC, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};
use windows_core::{HRESULT, IUnknown, IUnknown_Vtbl, Interface};
use windows_reactor::{WindowSize, with_active_host};

const DBT_DEVNODES_CHANGED: usize = 0x0007;

static LISTENER: OnceLock<RawHidListenerHandle> = OnceLock::new();
static ORIGINAL_WINDOW_PROC: AtomicIsize = AtomicIsize::new(0);

pub(super) fn install_listener(listener: RawHidListenerHandle) {
    let _ = LISTENER.set(listener);
}

pub(super) fn update_window(size: WindowSize) {
    if let Some(result) = with_active_host(|host| native_window_handle(host.window())) {
        match result {
            Ok(window) => unsafe {
                configure_window(window);
                position_window(window, size);
            },
            Err(error) => log::error!("Failed to access the WinUI window: {error}"),
        }
    }
}

unsafe fn configure_window(window: HWND) {
    let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
    let overlay_ex_style = WINDOW_EX_STYLE(
        (ex_style as u32)
            | WS_EX_LAYERED.0
            | WS_EX_TRANSPARENT.0
            | WS_EX_TOOLWINDOW.0
            | WS_EX_NOACTIVATE.0,
    );
    unsafe { SetWindowLongPtrW(window, GWL_EXSTYLE, overlay_ex_style.0 as isize) };

    let style = unsafe { GetWindowLongPtrW(window, GWL_STYLE) } as u32;
    let removed =
        WS_CAPTION.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_SYSMENU.0;
    let overlay_style = WINDOW_STYLE(style & !removed);
    unsafe { SetWindowLongPtrW(window, GWL_STYLE, overlay_style.0 as isize) };
    let _ = unsafe { SetLayeredWindowAttributes(window, COLORREF(0), 0, LWA_COLORKEY) };

    if ORIGINAL_WINDOW_PROC.load(Ordering::Relaxed) == 0 {
        let previous = unsafe {
            SetWindowLongPtrW(
                window,
                GWLP_WNDPROC,
                window_proc as *const () as usize as isize,
            )
        };
        let _ = ORIGINAL_WINDOW_PROC.compare_exchange(
            0,
            previous,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }
}

unsafe fn position_window(window: HWND, size: WindowSize) {
    let (x, y, width, height) = if size.width <= 1.0 || size.height <= 1.0 {
        (0, 0, 1, 1)
    } else {
        visible_window_bounds(size)
    };
    let flags = SET_WINDOW_POS_FLAGS(SWP_NOACTIVATE.0 | SWP_SHOWWINDOW.0 | SWP_FRAMECHANGED.0);
    let _ = unsafe { SetWindowPos(window, Some(HWND_TOPMOST), x, y, width, height, flags) };
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
    if message == WM_DEVICECHANGE
        && parameter.0 == DBT_DEVNODES_CHANGED
        && let Some(listener) = LISTENER.get()
    {
        listener.device_arrived();
    }
    let previous = ORIGINAL_WINDOW_PROC.load(Ordering::Relaxed);
    let previous: WNDPROC = unsafe { std::mem::transmute(previous) };
    unsafe { CallWindowProcW(previous, window, message, parameter, data) }
}

windows_core::imp::define_interface!(
    IWindowNative,
    IWindowNative_Vtbl,
    0xeecdbf0e_bae9_4cb6_a68e_9598e1cb57bb
);
windows_core::imp::interface_hierarchy!(IWindowNative, IUnknown);

impl IWindowNative {
    unsafe fn window_handle(&self, window: *mut *mut c_void) -> HRESULT {
        unsafe { (Interface::vtable(self).window_handle)(Interface::as_raw(self), window) }
    }
}

#[repr(C)]
pub struct IWindowNative_Vtbl {
    base__: IUnknown_Vtbl,
    window_handle: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

fn native_window_handle(window: &impl Interface) -> windows_core::Result<HWND> {
    let native = window.cast::<IWindowNative>()?;
    let mut handle = std::ptr::null_mut();
    unsafe { native.window_handle(&mut handle).ok()? };
    Ok(HWND(handle))
}
