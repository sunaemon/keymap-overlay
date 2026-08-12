//! The Linux overlay window, drawn as a wlr-layer-shell surface.
//!
//! Wayland gives an ordinary application window no say over stacking and no way
//! to be ignored by the pointer, so the overlay is a layer surface on the
//! overlay layer with an empty input region: above normal windows, never
//! focused, and transparent to clicks. Compositors that do not implement
//! `zwlr_layer_shell_v1` (notably GNOME) cannot run it.
//!
//! The surface stays unmapped while no layer is held. Showing a layer maps it
//! again, which per the protocol means committing without a buffer, waiting for
//! the configure that follows, and only then attaching the image.

use anyhow::{Context as _, Result};
use image::{Rgba, RgbaImage};
use log::warn;
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{self, Event as ChannelEvent, Sender};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{wl_output, wl_shm, wl_surface};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};
use std::path::PathBuf;
use std::sync::Arc;

use crate::{
    ImageCache, LayerEventSink, ListenerEvent, Transition, image_path, load_image_cache,
    premultiply, spawn_raw_hid_listener, transition_for_event,
};

/// The pool grows on demand; this only avoids a resize for the first image.
const INITIAL_POOL_BYTES: usize = 1024 * 1024;

/// Whether this compositor can host the overlay, which is exactly whether it
/// offers `zwlr_layer_shell_v1`.
///
/// Asking costs a second connection, since `run` opens its own. That is one
/// round trip at startup, and it keeps the probe from having to hand a
/// half-built session over to the caller.
pub(crate) fn is_available() -> bool {
    let Ok(connection) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, queue)) = registry_queue_init::<OverlayState>(&connection) else {
        return false;
    };
    LayerShell::bind(&globals, &queue.handle()).is_ok()
}

pub(crate) fn run(assets_dir: PathBuf) -> Result<()> {
    let connection =
        Connection::connect_to_env().context("Failed to connect to the Wayland display")?;
    let (globals, queue) =
        registry_queue_init(&connection).context("Failed to enumerate the Wayland globals")?;
    let queue_handle = queue.handle();

    let compositor = CompositorState::bind(&globals, &queue_handle)
        .context("The compositor does not offer wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &queue_handle).context(
        "The compositor does not implement zwlr_layer_shell_v1, which the overlay needs to draw above other windows",
    )?;
    let shm = Shm::bind(&globals, &queue_handle).context("The compositor does not offer wl_shm")?;

    let surface = compositor.create_surface(&queue_handle);
    let input_region =
        Region::new(&compositor).context("Failed to create the empty input region")?;
    let layer = layer_shell.create_layer_surface(
        &queue_handle,
        surface,
        Layer::Overlay,
        Some("keymap-overlay"),
        None,
    );
    let pool = SlotPool::new(INITIAL_POOL_BYTES, &shm)
        .context("Failed to create the shared memory pool")?;

    let mut event_loop: EventLoop<OverlayState> =
        EventLoop::try_new().context("Failed to create the event loop")?;
    let handle = event_loop.handle();
    WaylandSource::new(connection, queue)
        .insert(handle.clone())
        .map_err(|error| anyhow::anyhow!("Failed to watch the Wayland connection: {error}"))?;

    // The listener thread hands events to the loop through this channel, which
    // wakes it; nothing polls and an idle overlay costs nothing.
    let (sender, receiver) = channel::channel();
    handle
        .insert_source(receiver, |event, _, state: &mut OverlayState| {
            if let ChannelEvent::Msg(layer_event) = event {
                state.handle_layer_event(layer_event);
            }
        })
        .map_err(|error| anyhow::anyhow!("Failed to watch for layer events: {error}"))?;
    spawn_raw_hid_listener(ChannelSink { sender });

    let images = load_image_cache(&assets_dir)?;
    let mut state = OverlayState {
        assets_dir,
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &queue_handle),
        shm,
        pool,
        layer,
        input_region,
        held_keys: Vec::new(),
        images,
        image: None,
        mapped: false,
        closed: false,
    };

    while !state.closed {
        event_loop
            .dispatch(None, &mut state)
            .context("The Wayland event loop failed")?;
    }
    // The compositor only closes the surface when it is going away, so let the
    // service manager start us again rather than sitting here without a window.
    Err(anyhow::anyhow!("The compositor closed the overlay surface"))
}

#[derive(Clone)]
struct ChannelSink {
    sender: Sender<ListenerEvent>,
}

impl LayerEventSink for ChannelSink {
    fn send(&self, event: ListenerEvent) -> bool {
        self.sender.send(event).is_ok()
    }
}

struct OverlayState {
    assets_dir: PathBuf,
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,
    input_region: Region,
    held_keys: Vec<(u8, u8)>,
    images: ImageCache,
    image: Option<Arc<RgbaImage>>,
    /// Whether a buffer is attached. A layer surface only exists on screen
    /// while one is, and the two states accept different requests.
    mapped: bool,
    closed: bool,
}

impl OverlayState {
    fn handle_layer_event(&mut self, event: ListenerEvent) {
        let transition = transition_for_event(&mut self.held_keys, event);
        match transition {
            Transition::Show { keyboard_id, layer } => self.show_layer(keyboard_id, layer),
            Transition::Hide => self.hide(),
            Transition::Ignore => {}
        }
    }

    fn show_layer(&mut self, keyboard_id: u8, layer: u8) {
        let path = image_path(&self.assets_dir, keyboard_id, layer);
        let image = match self.images.get(&(keyboard_id, layer)) {
            Some(image) => Arc::clone(image),
            None => {
                warn!("Overlay image is unavailable: {}", path.display());
                // Stay hidden rather than leaving the previous layer on screen
                // and its key recorded as active.
                self.hide();
                return;
            }
        };

        let (width, height) = image.dimensions();
        let same_size = self
            .image
            .as_ref()
            .is_some_and(|current| current.dimensions() == image.dimensions());
        self.image = Some(image);

        if self.mapped && same_size {
            self.draw();
            return;
        }

        // A differently sized or currently hidden surface needs a fresh
        // configure. Same-sized visible layers take the direct path above so
        // the compositor replaces their buffers in one atomic commit.
        self.unmap();
        // Unmapping resets the layer surface to the state it had when it was
        // created, so every one of these has to be sent again. A layer surface
        // with no anchors must be given a size before it is committed, or the
        // compositor rejects the zero size as a protocol error.
        self.layer.set_layer(Layer::Overlay);
        self.layer
            .set_keyboard_interactivity(KeyboardInteractivity::None);
        // A negative zone lets the overlay cover panels that reserve space
        // instead of being pushed aside by them.
        self.layer.set_exclusive_zone(-1);
        // No anchor: the compositor centres a surface that asks for a size.
        self.layer.set_size(width, height);
        self.layer.commit();
    }

    fn hide(&mut self) {
        self.image = None;
        self.unmap();
    }

    /// Attaching a null buffer unmaps the surface, which is how the overlay is
    /// hidden; there is no window to leave behind.
    ///
    /// Does nothing when the surface is already unmapped: that commit would
    /// carry the size-less state a layer surface starts in, which the
    /// compositor rejects.
    fn unmap(&mut self) {
        if !self.mapped {
            return;
        }
        self.layer.attach(None, 0, 0);
        self.layer.commit();
        self.mapped = false;
    }

    fn draw(&mut self) {
        let Some(image) = self.image.clone() else {
            return;
        };
        if let Err(error) = self.present(&image) {
            warn!("Failed to present the overlay: {error:#}");
        }
    }

    fn present(&mut self, image: &RgbaImage) -> Result<()> {
        let width = image.width() as i32;
        let height = image.height() as i32;
        let (buffer, canvas) = self
            .pool
            .create_buffer(width, height, width * 4, wl_shm::Format::Argb8888)
            .context("Failed to allocate a shared memory buffer")?;
        for (pixel, chunk) in image.pixels().zip(canvas.chunks_exact_mut(4)) {
            chunk.copy_from_slice(&premultiplied_bgra(pixel));
        }

        let surface = self.layer.wl_surface();
        // Click-through: an empty input region hands every pointer event to
        // whatever sits underneath. Surface state survives an unmap, but it
        // costs nothing to state it on each map.
        surface.set_input_region(Some(self.input_region.wl_region()));
        surface.damage_buffer(0, 0, width, height);
        buffer
            .attach_to(surface)
            .context("Failed to attach the overlay buffer")?;
        self.layer.commit();
        self.mapped = true;
        Ok(())
    }
}

/// wl_shm's ARGB8888 is premultiplied and little-endian, so a pixel travels as
/// blue, green, red, alpha.
fn premultiplied_bgra(pixel: &Rgba<u8>) -> [u8; 4] {
    let [red, green, blue, alpha] = pixel.0;
    [
        premultiply(blue, alpha),
        premultiply(green, alpha),
        premultiply(red, alpha),
        alpha,
    ]
}

impl LayerShellHandler for OverlayState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        _: LayerSurfaceConfigure,
        _: u32,
    ) {
        // The configure that follows the bufferless commit in show_layer is
        // what the image is waiting for. The requested size is not read back:
        // the overlay is drawn at the image's own size, the way it is on macOS.
        self.draw();
    }
}

impl CompositorHandler for OverlayState {
    /// The image is presented at its pixel size on every display, which is what
    /// the macOS window does too; `DPI` in the Makefile is how the images are
    /// sized for a screen.
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    /// Never called: the overlay draws on layer events, not on frame callbacks,
    /// so it never asks for one.
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

/// Required by the compositor delegate, which tracks which outputs a surface is
/// on; the overlay itself does not care.
impl OutputHandler for OverlayState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for OverlayState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for OverlayState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

// One blanket `Dispatch` impl covers the compositor, output, shm and layer
// objects at once: it routes every event to the `Dispatch2` impl the toolkit
// wrote for that object's user data, which is where the per-object delegate
// macros went in 0.21. The registry keeps its own macro.
delegate_dispatch2!(OverlayState);
delegate_registry!(OverlayState);
