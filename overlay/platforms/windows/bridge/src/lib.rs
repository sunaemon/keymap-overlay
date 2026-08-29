//! Narrow C ABI between the WPF frontend and the shared Rust HID runtime.

use keymap_overlay_runtime::{
    Arguments, LayerEvent, LayerEventSink, LayerEventSourceHandle, LogDestination, OverlayModel,
    Parser, PendingTransition, SimulatedLayer, StartupModels, StartupRawHidDevice, Transition,
    default_log_file, initialize_logging, spawn_layer_event_source, startup_models,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, OnceLock};

// Tag: bits 24-31; layer count: bits 16-23; keyboard ID: bits 8-15;
// highest active layer: bits 0-7.
const TRANSITION_IGNORE: u32 = 0;
const TRANSITION_HIDE: u32 = 1;
const TRANSITION_SHOW: u32 = 2 << 24;

static STATE: OnceLock<Arc<SharedState>> = OnceLock::new();
static LOGGING: OnceLock<Result<(), String>> = OnceLock::new();
static PREPARED_MODELS: OnceLock<PreparedModels> = OnceLock::new();

struct PreparedModels {
    json: Vec<u8>,
    raw_hid_devices: Mutex<Vec<StartupRawHidDevice>>,
    modeled_keyboard_ids: Vec<u8>,
}

#[derive(Serialize)]
struct KeyboardModels {
    keyboard_id: u8,
    layers: BTreeMap<u8, OverlayModel>,
}

#[derive(Clone)]
struct BridgeSink {
    state: Arc<SharedState>,
}

struct SharedState {
    listener: OnceLock<LayerEventSourceHandle>,
    pending: Mutex<PendingTransition>,
    published_layers: Mutex<Vec<u8>>,
    wake: extern "system" fn(),
}

impl LayerEventSink for BridgeSink {
    fn send(&self, event: LayerEvent) -> bool {
        self.state
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
        (self.state.wake)();
        true
    }
}

/// Refreshes connected keyboard models before WPF loads the asset directory.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_prepare() -> i32 {
    catch_unwind(prepare).unwrap_or(-1)
}

/// Returns the byte length of the prepared in-memory model JSON.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_models_json_length() -> usize {
    PREPARED_MODELS
        .get()
        .map_or(0, |prepared| prepared.json.len())
}

/// Returns a stable pointer to the prepared in-memory model JSON.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_models_json() -> *const u8 {
    PREPARED_MODELS
        .get()
        .map_or(std::ptr::null(), |prepared| prepared.json.as_ptr())
}

/// Starts the HID listener. Returns zero, or a negative value on failure.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_start(wake: extern "system" fn()) -> i32 {
    catch_unwind(AssertUnwindSafe(|| start(wake, None))).unwrap_or(-1)
}

/// Starts a synthetic layer source instead of HID. Returns zero on success.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_start_simulated(
    wake: extern "system" fn(),
    keyboard_id: u8,
    layer: u8,
) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        start(wake, Some(SimulatedLayer { keyboard_id, layer }))
    }))
    .unwrap_or(-1)
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

fn start(wake: extern "system" fn(), simulated: Option<SimulatedLayer>) -> i32 {
    if STATE.get().is_some() {
        return -2;
    }
    if let Err(error) = initialize_bridge_logging() {
        eprintln!("{error}");
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
    let startup_devices = PREPARED_MODELS.get().map_or_else(Vec::new, |prepared| {
        std::mem::take(
            &mut *prepared
                .raw_hid_devices
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    });
    let listener = spawn_layer_event_source(
        BridgeSink {
            state: Arc::clone(&shared),
        },
        simulated,
        startup_devices,
        PREPARED_MODELS
            .get()
            .into_iter()
            .flat_map(|prepared| prepared.modeled_keyboard_ids.iter().copied()),
    );
    if shared.listener.set(listener).is_err() {
        return -2;
    }
    0
}

fn prepare() -> i32 {
    if let Err(error) = initialize_bridge_logging() {
        eprintln!("{error}");
        return -1;
    }
    let arguments = match Arguments::try_parse() {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("Failed to parse arguments: {error}");
            return -1;
        }
    };
    let models = match startup_models(arguments.simulate).and_then(prepare_models) {
        Ok(models) => models,
        Err(error) => {
            eprintln!("Failed to read connected keyboard models: {error:#}");
            return -1;
        }
    };
    PREPARED_MODELS.set(models).map_or(-2, |()| 0)
}

fn prepare_models(startup: StartupModels) -> anyhow::Result<PreparedModels> {
    let modeled_keyboard_ids = startup
        .models
        .keys()
        .map(|(keyboard_id, _)| *keyboard_id)
        .collect();
    Ok(PreparedModels {
        json: serialize_models(startup.models)?,
        raw_hid_devices: Mutex::new(startup.raw_hid_devices),
        modeled_keyboard_ids,
    })
}

fn serialize_models(models: HashMap<(u8, u8), OverlayModel>) -> anyhow::Result<Vec<u8>> {
    let mut keyboards: BTreeMap<u8, BTreeMap<u8, OverlayModel>> = BTreeMap::new();
    for ((keyboard_id, layer), model) in models {
        keyboards
            .entry(keyboard_id)
            .or_default()
            .insert(layer, model);
    }
    serde_json::to_vec(
        &keyboards
            .into_iter()
            .map(|(keyboard_id, layers)| KeyboardModels {
                keyboard_id,
                layers,
            })
            .collect::<Vec<_>>(),
    )
    .map_err(Into::into)
}

fn initialize_bridge_logging() -> Result<(), String> {
    LOGGING
        .get_or_init(|| {
            let log_file = default_log_file()
                .map_err(|error| format!("Failed to resolve the log file: {error:#}"))?;
            initialize_logging(LogDestination::File(log_file))
                .map_err(|error| format!("Failed to initialize logging: {error:#}"))
        })
        .clone()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_grouped_keyboard_models() {
        assert_eq!(serialize_models(HashMap::new()).unwrap(), b"[]");
    }
}
