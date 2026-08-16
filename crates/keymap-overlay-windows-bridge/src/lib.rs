//! Narrow C ABI between the WPF frontend and the shared Rust HID runtime.

use keymap_overlay::{
    LayerEventSink, ListenerEvent, LogDestination, PendingTransition, RawHidListenerHandle,
    Transition, default_log_file, initialize_logging, spawn_raw_hid_listener,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, OnceLock};

// Tag: bits 24-31; layer count: bits 16-23; keyboard ID: bits 8-15;
// highest active layer: bits 0-7.
const TRANSITION_IGNORE: u32 = 0;
const TRANSITION_HIDE: u32 = 1;
const TRANSITION_SHOW: u32 = 2 << 24;

static STATE: OnceLock<Arc<SharedState>> = OnceLock::new();

#[derive(Clone)]
struct BridgeSink {
    state: Arc<SharedState>,
}

struct SharedState {
    listener: OnceLock<RawHidListenerHandle>,
    pending: Mutex<PendingTransition>,
    published_layers: Mutex<Vec<u8>>,
    wake: extern "system" fn(),
}

impl LayerEventSink for BridgeSink {
    fn send(&self, event: ListenerEvent) -> bool {
        self.state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
        (self.state.wake)();
        true
    }
}

/// Starts the HID listener. Returns zero, or a negative value on failure.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_start(wake: extern "system" fn()) -> i32 {
    catch_unwind(AssertUnwindSafe(|| start(wake))).unwrap_or(-1)
}

/// Re-enumerates Raw HID interfaces after Windows reports a device arrival.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_device_arrived() {
    let _ = catch_unwind(|| {
        if let Some(listener) = STATE.get().and_then(|state| state.listener.get()) {
            listener.device_arrived();
        }
    });
}

fn start(wake: extern "system" fn()) -> i32 {
    if STATE.get().is_some() {
        return -2;
    }
    // WPF owns the process and this ABI carries no strings, so the log file is
    // the shared default rather than something the frontend can name. Both
    // failures are reported on stderr because logging does not exist yet and
    // the caller receives only a numeric code.
    let log_file = match default_log_file() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Failed to resolve the log file: {error:#}");
            return -1;
        }
    };
    if let Err(error) = initialize_logging(LogDestination::File(log_file)) {
        eprintln!("Failed to initialize logging: {error:#}");
        return -1;
    }

    let shared = Arc::new(SharedState {
        listener: OnceLock::new(),
        pending: Mutex::new(PendingTransition::default()),
        published_layers: Mutex::new(Vec::new()),
        wake,
    });
    if STATE.set(Arc::clone(&shared)).is_err() {
        return -2;
    }
    let listener = spawn_raw_hid_listener(BridgeSink {
        state: Arc::clone(&shared),
    });
    if shared.listener.set(listener).is_err() {
        return -2;
    }
    0
}

/// Returns the final queued transition packed into one FFI-safe integer.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_take_transition() -> u32 {
    let Some(state) = STATE.get() else {
        return TRANSITION_IGNORE;
    };
    match state
        .pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        Transition::Ignore => TRANSITION_IGNORE,
        Transition::Hide => {
            state
                .published_layers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clear();
            TRANSITION_HIDE
        }
        Transition::Show {
            keyboard_id,
            layers,
        } => {
            let layer_count = u8::try_from(layers.len()).unwrap_or(u8::MAX);
            let highest_layer = layers.last().copied().unwrap_or_default();
            *state
                .published_layers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = layers;
            TRANSITION_SHOW
                | (u32::from(layer_count) << 16)
                | (u32::from(keyboard_id) << 8)
                | u32::from(highest_layer)
        }
    }
}

/// Returns one layer from the most recently published show transition.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_transition_layer(index: u8) -> u8 {
    STATE
        .get()
        .and_then(|state| {
            state
                .published_layers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(usize::from(index))
                .copied()
        })
        .unwrap_or_default()
}
