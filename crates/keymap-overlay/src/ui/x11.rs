//! The X11 overlay window, used where a Wayland compositor has no layer shell.
//!
//! The window is override-redirect, which means the window manager does not
//! manage it: it is never given focus, never restacked below anything it
//! manages, and never decorated or repositioned. That matters more than it
//! sounds. Asking a window manager for always-on-top instead — the usual
//! `_NET_WM_STATE_ABOVE` route — is a request it may ignore, and a managed
//! window takes focus when it is mapped, which for this overlay would mean
//! swallowing the keystrokes the layer key was held for.
//!
//! Being unmanaged also means nothing places the window, so it is centred on
//! the monitor by hand, and hiding is unmapping, as on Wayland.
//!
//! winit is used directly rather than through eframe because eframe offers no
//! way to ask for an override-redirect window. Pixels are uploaded with X11's
//! defined 32-bit ARGB format; drawing one decoded image per key hold does not
//! need a GPU context.

use anyhow::{Context as _, Result, bail};
use image::{Rgba, RgbaImage};
use keymap_core::RawLayerEvent;
use log::warn;
use std::borrow::Cow;
use std::path::PathBuf;
use std::rc::Rc;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::monitor::MonitorHandle;
use winit::platform::x11::WindowAttributesExtX11;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowAttributes, WindowId};
use x11rb::connection::Connection as _;
use x11rb::image::{BitsPerPixel, Image, ImageOrder, ScanlinePad};
use x11rb::protocol::xproto::{ConnectionExt as _, CreateGCAux};
use x11rb::rust_connection::RustConnection;

use crate::{
    LayerEventSink, Transition, image_path, load_image, premultiply, spawn_raw_hid_listener,
    transition_for,
};

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let event_loop = EventLoop::<RawLayerEvent>::with_user_event()
        .build()
        .context("Failed to create the X11 event loop")?;
    // The proxy is the wake-up: sending on it both queues the event and returns
    // the loop from its wait, so nothing polls.
    spawn_raw_hid_listener(ProxySink {
        proxy: event_loop.create_proxy(),
    });

    let mut app = OverlayApp {
        assets_dir,
        window: None,
        held_keys: Vec::new(),
        image: None,
        error: None,
    };
    event_loop
        .run_app(&mut app)
        .context("The X11 event loop failed")?;
    // Handlers cannot return, so a fatal error travels out here instead.
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[derive(Clone)]
struct ProxySink {
    proxy: EventLoopProxy<RawLayerEvent>,
}

impl LayerEventSink for ProxySink {
    fn send(&self, event: RawLayerEvent) -> bool {
        self.proxy.send_event(event).is_ok()
    }
}

struct OverlayApp {
    assets_dir: PathBuf,
    window: Option<OverlayWindow>,
    held_keys: Vec<(u8, u8)>,
    image: Option<RgbaImage>,
    error: Option<anyhow::Error>,
}

struct OverlayWindow {
    window: Rc<Window>,
    renderer: X11Renderer,
}

struct X11Renderer {
    connection: RustConnection,
    window: u32,
    gc: u32,
    image_order: ImageOrder,
}

impl OverlayApp {
    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let attributes = WindowAttributes::default()
            .with_title("Keymap Overlay")
            .with_decorations(false)
            .with_transparent(true)
            .with_visible(false)
            .with_override_redirect(true);
        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .context("Failed to create the X11 overlay window")?,
        );
        // Click-through. Unlike the window level, this is a property of the
        // window itself rather than a request to the window manager.
        window
            .set_cursor_hittest(false)
            .context("Failed to make the overlay click-through")?;

        let renderer = X11Renderer::new(&window)?;

        self.window = Some(OverlayWindow { window, renderer });
        Ok(())
    }

    fn show_layer(&mut self, keyboard_id: u8, layer: u8) {
        let path = image_path(&self.assets_dir, keyboard_id, layer);
        let image = match load_image(&path) {
            Ok(image) => image,
            Err(error) => {
                warn!("Failed to load overlay image {}: {error:#}", path.display());
                // Stay hidden rather than leaving the previous layer on screen
                // and its key recorded as active.
                self.hide();
                return;
            }
        };
        let Some(state) = &self.window else {
            return;
        };

        let size = PhysicalSize::new(image.width(), image.height());
        // An unmanaged window is placed by nobody else, and the size has to be
        // set before the window is mapped so it is never shown at the old one.
        let _ = state.window.request_inner_size(size);
        if let Some(monitor) = state.window.current_monitor().or_else(|| {
            state
                .window
                .available_monitors()
                .next()
                .or_else(|| state.window.primary_monitor())
        }) {
            state
                .window
                .set_outer_position(centered_position(&monitor, size));
        }
        state.window.set_visible(true);
        state.window.request_redraw();

        self.image = Some(image);
    }

    fn hide(&mut self) {
        self.image = None;
        if let Some(state) = &self.window {
            state.window.set_visible(false);
        }
    }

    fn draw(&mut self) {
        let (Some(state), Some(image)) = (&mut self.window, &self.image) else {
            return;
        };
        if let Err(error) = present(state, image) {
            warn!("Failed to present the overlay: {error:#}");
        }
    }

    /// Records a fatal error and ends the loop, since a handler cannot return
    /// one and carrying on without a window would leave a silent process.
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

fn present(state: &OverlayWindow, image: &RgbaImage) -> Result<()> {
    state.renderer.present(image)
}

impl X11Renderer {
    fn new(window: &Window) -> Result<Self> {
        let window = match window
            .window_handle()
            .context("Failed to get the X11 window handle")?
            .as_raw()
        {
            RawWindowHandle::Xlib(handle) => handle.window as u32,
            RawWindowHandle::Xcb(handle) => handle.window.get(),
            handle => bail!("The X11 backend returned an unexpected window handle: {handle:?}"),
        };
        let (connection, _) =
            x11rb::connect(None).context("Failed to open the X11 drawing connection")?;
        let depth = connection
            .get_geometry(window)
            .context("Failed to query the X11 overlay window")?
            .reply()
            .context("Failed to read the X11 overlay window geometry")?
            .depth;
        if depth != 32 {
            bail!("The X11 server did not provide the 32-bit visual required for transparency");
        }

        let gc = connection
            .generate_id()
            .context("Failed to allocate an X11 graphics context ID")?;
        connection
            .create_gc(gc, window, &CreateGCAux::new())
            .context("Failed to create the X11 graphics context")?
            .check()
            .context("The X11 server rejected the graphics context")?;
        let image_order = connection
            .setup()
            .image_byte_order
            .try_into()
            .context("The X11 server reported an unsupported image byte order")?;

        Ok(Self {
            connection,
            window,
            gc,
            image_order,
        })
    }

    fn present(&self, image: &RgbaImage) -> Result<()> {
        let width =
            u16::try_from(image.width()).context("The overlay image is too wide for X11")?;
        let height =
            u16::try_from(image.height()).context("The overlay image is too tall for X11")?;
        if width == 0 || height == 0 {
            bail!("The overlay image has a zero width or height");
        }

        let mut pixels = Vec::with_capacity(usize::from(width) * usize::from(height) * 4);
        for pixel in image.pixels() {
            pixels.extend_from_slice(&argb_bytes(pixel, self.image_order));
        }
        let image = Image::new(
            width,
            height,
            ScanlinePad::Pad32,
            32,
            BitsPerPixel::B32,
            self.image_order,
            Cow::Owned(pixels),
        )
        .context("Failed to encode the X11 overlay image")?;
        for cookie in image
            .put(&self.connection, self.window, self.gc, 0, 0)
            .context("Failed to upload the X11 overlay image")?
        {
            cookie
                .check()
                .context("The X11 server rejected the overlay image")?;
        }
        self.connection
            .flush()
            .context("Failed to flush the X11 overlay image")
    }
}

/// X11 composites a 32-bit visual as premultiplied ARGB, one pixel per word.
fn premultiplied_argb(pixel: &Rgba<u8>) -> u32 {
    let [red, green, blue, alpha] = pixel.0;
    u32::from_be_bytes([
        alpha,
        premultiply(red, alpha),
        premultiply(green, alpha),
        premultiply(blue, alpha),
    ])
}

fn argb_bytes(pixel: &Rgba<u8>, image_order: ImageOrder) -> [u8; 4] {
    let pixel = premultiplied_argb(pixel);
    match image_order {
        ImageOrder::LsbFirst => pixel.to_le_bytes(),
        ImageOrder::MsbFirst => pixel.to_be_bytes(),
    }
}

fn centered_position(monitor: &MonitorHandle, size: PhysicalSize<u32>) -> PhysicalPosition<i32> {
    let origin = monitor.position();
    let available = monitor.size();
    PhysicalPosition::new(
        origin.x + (available.width.saturating_sub(size.width) / 2) as i32,
        origin.y + (available.height.saturating_sub(size.height) / 2) as i32,
    )
}

impl ApplicationHandler<RawLayerEvent> for OverlayApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(error) = self.create_window(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: RawLayerEvent) {
        match transition_for(&mut self.held_keys, event) {
            Transition::Show { keyboard_id, layer } => self.show_layer(keyboard_id, layer),
            Transition::Hide => self.hide(),
            Transition::Ignore => {}
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::RedrawRequested => self.draw(),
            // Unmanaged windows get no close button, so this only arrives if
            // the X server itself is going away.
            WindowEvent::CloseRequested | WindowEvent::Destroyed => event_loop.exit(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transparent_pixel_stays_transparent() {
        assert_eq!(premultiplied_argb(&Rgba([255, 255, 255, 0])), 0x0000_0000);
    }

    #[test]
    fn an_opaque_pixel_keeps_its_channels() {
        assert_eq!(
            premultiplied_argb(&Rgba([0x12, 0x34, 0x56, 255])),
            0xFF12_3456
        );
        assert_eq!(
            argb_bytes(&Rgba([0x12, 0x34, 0x56, 255]), ImageOrder::LsbFirst),
            [0x56, 0x34, 0x12, 0xFF]
        );
        assert_eq!(
            argb_bytes(&Rgba([0x12, 0x34, 0x56, 255]), ImageOrder::MsbFirst),
            [0xFF, 0x12, 0x34, 0x56]
        );
    }

    /// Half-transparent white is half-bright in a premultiplied buffer; leaving
    /// the channels alone would draw it too bright over a dark background.
    #[test]
    fn a_partly_transparent_pixel_is_premultiplied() {
        assert_eq!(premultiplied_argb(&Rgba([255, 255, 255, 128])), 0x8080_8080);
    }
}
