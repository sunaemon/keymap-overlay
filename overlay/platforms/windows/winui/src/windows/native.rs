//! Audited Win32 boundary for the transparent XAML Island host.

#[allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    clippy::missing_safety_doc,
    clippy::upper_case_acronyms
)]
#[path = "island_bindings.rs"]
mod island_bindings;

use super::OverlayComponent;
use island_bindings::{DesktopWindowXamlSource, RectInt32, WindowId};
use keymap_overlay_runtime::LayerEventSourceHandle;
use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::sync::OnceLock;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, GetMonitorInfoW, GetStockObject, HBRUSH, HMONITOR, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromPoint,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWL_EXSTYLE, GetCursorPos, GetWindowLongPtrW, HWND_TOPMOST,
    LWA_ALPHA, LWA_COLORKEY, PostQuitMessage, RegisterClassExW, SET_WINDOW_POS_FLAGS, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SetLayeredWindowAttributes,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WM_DESTROY, WM_DEVICECHANGE,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows_core::HRESULT;
use windows_reactor::{App, Component, RenderHost, WinUIBackend, WinUIDispatcher, WindowSize};

const DBT_DEVNODES_CHANGED: usize = 0x0007;
const WINDOW_CLASS: &[u16] = &[
    b'K' as u16,
    b'e' as u16,
    b'y' as u16,
    b'm' as u16,
    b'a' as u16,
    b'p' as u16,
    b'O' as u16,
    b'v' as u16,
    b'e' as u16,
    b'r' as u16,
    b'l' as u16,
    b'a' as u16,
    b'y' as u16,
    b'I' as u16,
    b's' as u16,
    b'l' as u16,
    b'a' as u16,
    b'n' as u16,
    b'd' as u16,
    0,
];

static LISTENER: OnceLock<LayerEventSourceHandle> = OnceLock::new();

thread_local! {
    static ISLAND_HOST: RefCell<Option<IslandHost>> = const { RefCell::new(None) };
    static REQUESTED_SIZE: Cell<WindowSize> = const {
        Cell::new(WindowSize { width: 1.0, height: 1.0 })
    };
    static WINDOW_VISIBLE: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn run(component: OverlayComponent) -> windows_core::Result<()> {
    App::new().run_custom(move |_app| {
        let host = IslandHost::new(Box::new(component))?;
        ISLAND_HOST.with(|slot| *slot.borrow_mut() = Some(host));
        Ok(())
    })
}

pub(super) fn install_listener(listener: LayerEventSourceHandle) {
    if LISTENER.set(listener).is_err() {
        log::error!("The WinUI layer event source was already initialized");
    }
}

pub(super) fn request_window(size: WindowSize) {
    REQUESTED_SIZE.set(size);
}

struct IslandHost {
    _source: DesktopWindowXamlSource,
    _render_host: RenderHost<WinUIBackend, WinUIDispatcher>,
    _window: HWND,
}

impl IslandHost {
    fn new(root: Box<dyn Component>) -> windows_core::Result<Self> {
        let window = create_overlay_window()?;
        let source = DesktopWindowXamlSource::new()
            .map_err(|error| island_error("Failed to create DesktopWindowXamlSource", error))?;
        source
            .initialize(WindowId {
                value: window.0 as usize as u64,
            })
            .map_err(|error| island_error("Failed to initialize the XAML Island host", error))?;
        let site_bridge = source
            .site_bridge()
            .map_err(|error| island_error("Failed to get the XAML Island site bridge", error))?;
        let island_id = site_bridge
            .window_id()
            .map_err(|error| island_error("Failed to get the XAML Island window ID", error))?;
        let island_window = HWND(island_id.value as usize as *mut _);
        configure_island_window(island_window);
        site_bridge
            .show()
            .map_err(|error| island_error("Failed to show the XAML Island site bridge", error))?;

        let dispatcher = WinUIDispatcher::for_current_thread()?;
        let render_host = RenderHost::new(WinUIBackend::new(), root, dispatcher);
        render_host.set_marshaller(Some(WinUIDispatcher::for_current_thread()?.marshaller()));

        let source_for_render = source.clone();
        let render_host_for_render = render_host.downgrade();
        render_host.set_post_render(move |root_id| {
            let Some(root_id) = root_id else {
                return;
            };
            let Some(render_host) = render_host_for_render.upgrade() else {
                return;
            };
            let result = render_host
                .with_backend(|backend| backend.get_ui_element(root_id))
                .ok_or_else(|| windows_core::Error::from_hresult(HRESULT(0x80004005_u32 as i32)))
                .and_then(|element| source_for_render.set_content(&element));
            if let Err(error) = result {
                log::error!("Failed to attach the WinUI root to the XAML Island: {error}");
            }
        });

        render_host.set_render_complete(move |_| present_window(window, &site_bridge));
        render_host.kick();

        Ok(Self {
            _source: source,
            _render_host: render_host,
            _window: window,
        })
    }
}

fn island_error(message: &str, error: windows_core::Error) -> windows_core::Error {
    windows_core::Error::new(error.code(), format!("{message}: {error}"))
}

fn configure_island_window(window: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        let style = style | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0;
        SetWindowLongPtrW(window, GWL_EXSTYLE, style as isize);
    }
}

fn create_overlay_window() -> windows_core::Result<HWND> {
    let module = unsafe { GetModuleHandleW(None) }.map_err(map_windows_error)?;
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: HINSTANCE(module.0),
        hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
        lpszClassName: windows::core::PCWSTR(WINDOW_CLASS.as_ptr()),
        ..WNDCLASSEXW::default()
    };
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows_core::Error::new(
            HRESULT(0x80004005_u32 as i32),
            "Failed to register the XAML Island host window class",
        ));
    }

    let ex_style = WINDOW_EX_STYLE(
        WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0,
    );
    let window = unsafe {
        CreateWindowExW(
            ex_style,
            windows::core::PCWSTR(WINDOW_CLASS.as_ptr()),
            windows::core::PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(HINSTANCE(module.0)),
            None,
        )
    }
    .map_err(map_windows_error)?;
    unsafe { SetLayeredWindowAttributes(window, COLORREF(0), 0, LWA_COLORKEY | LWA_ALPHA) }
        .map_err(map_windows_error)?;
    Ok(window)
}

fn map_windows_error(error: windows::core::Error) -> windows_core::Error {
    windows_core::Error::from_hresult(HRESULT(error.code().0))
}

fn present_window(window: HWND, site_bridge: &island_bindings::DesktopChildSiteBridge) {
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
        if let Err(error) = site_bridge.move_and_resize(RectInt32 {
            x: 0,
            y: 0,
            width,
            height,
        }) {
            log::error!("Failed to resize the XAML Island: {error}");
        }
        if let Err(error) = site_bridge.move_in_z_order_at_top() {
            log::error!("Failed to move the XAML Island to the top of its Z-order: {error}");
        }
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        if first_show {
            let _ = DwmFlush();
            let _ = SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_COLORKEY | LWA_ALPHA);
        }
    }
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
    } else if message == WM_DESTROY {
        let host = ISLAND_HOST.with(|slot| slot.borrow_mut().take());
        drop(host);
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(window, message, parameter, data) }
}
