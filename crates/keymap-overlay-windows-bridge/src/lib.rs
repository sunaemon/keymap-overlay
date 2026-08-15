//! Narrow C ABI between the WPF frontend and the shared Rust HID runtime.

use keymap_overlay::{
    LayerEventSink, ListenerEvent, PendingTransition, Transition, initialize_logging,
    spawn_raw_hid_listener,
};
use std::sync::{Arc, Mutex, OnceLock};

const TRANSITION_IGNORE: u32 = 0;
const TRANSITION_HIDE: u32 = 1;
const TRANSITION_SHOW: u32 = 2 << 24;

static STATE: OnceLock<BridgeState> = OnceLock::new();

#[derive(Clone)]
struct BridgeSink {
    state: Arc<SharedState>,
}

struct BridgeState {
    shared: Arc<SharedState>,
}

struct SharedState {
    pending: Mutex<PendingTransition>,
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
    if STATE.get().is_some() {
        return -2;
    }
    if initialize_logging().is_err() {
        return -1;
    }

    let shared = Arc::new(SharedState {
        pending: Mutex::new(PendingTransition::default()),
        wake,
    });
    if STATE
        .set(BridgeState {
            shared: Arc::clone(&shared),
        })
        .is_err()
    {
        return -2;
    }
    spawn_raw_hid_listener(BridgeSink { state: shared });
    0
}

/// Returns the final queued transition packed into one FFI-safe integer.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_take_transition() -> u32 {
    let Some(state) = STATE.get() else {
        return TRANSITION_IGNORE;
    };
    match state
        .shared
        .pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
    {
        Transition::Ignore => TRANSITION_IGNORE,
        Transition::Hide => TRANSITION_HIDE,
        Transition::Show { keyboard_id, layer } => {
            TRANSITION_SHOW | (u32::from(keyboard_id) << 8) | u32::from(layer)
        }
    }
}
