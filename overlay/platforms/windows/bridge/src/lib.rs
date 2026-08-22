//! Narrow C ABI between the WPF frontend and the shared Rust HID runtime.

use keymap_overlay_runtime::{
    Arguments, LayerEvent, LayerEventSink, LayerEventSourceHandle, LogDestination, Parser,
    PendingTransition, SimulatedLayer, Transition, default_asset_dir, default_log_file,
    fill_missing_models, initialize_logging, spawn_layer_event_source,
};
use std::env;
use std::ffi::OsString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

// Tag: bits 24-31; layer count: bits 16-23; keyboard ID: bits 8-15;
// highest active layer: bits 0-7.
const TRANSITION_IGNORE: u32 = 0;
const TRANSITION_HIDE: u32 = 1;
const TRANSITION_SHOW: u32 = 2 << 24;

static STATE: OnceLock<Arc<SharedState>> = OnceLock::new();
static LOGGING: OnceLock<Result<(), String>> = OnceLock::new();

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

/// Generates missing models before WPF loads the asset directory.
#[unsafe(no_mangle)]
pub extern "system" fn keymap_overlay_prepare() -> i32 {
    catch_unwind(prepare).unwrap_or(-1)
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
    let listener = spawn_layer_event_source(
        BridgeSink {
            state: Arc::clone(&shared),
        },
        simulated,
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
    let (asset_dir, keyboard_config_dir) = match startup_paths_from(env::args_os()) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("Failed to parse the Windows startup paths: {error}");
            return -1;
        }
    };
    if let Some(keyboard_config_dir) = keyboard_config_dir
        && let Err(error) = fill_missing_models(&asset_dir, &keyboard_config_dir)
    {
        eprintln!("Self-heal skipped: {error:#}");
    }
    0
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

fn startup_paths_from<I, T>(arguments: I) -> Result<(PathBuf, Option<PathBuf>), String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let arguments = Arguments::try_parse_from(arguments).map_err(|error| error.to_string())?;
    let asset_dir = arguments
        .asset_dir
        .map_or_else(default_asset_dir, Ok)
        .map_err(|error| format!("Failed to resolve the asset directory: {error:#}"))?;
    Ok((asset_dir, arguments.keyboard_config_dir))
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
    fn startup_paths_follow_the_wpf_command_line() {
        let (asset_dir, keyboard_config_dir) = startup_paths_from([
            "keymap-overlay",
            "--asset-dir",
            r"C:\assets",
            "--keyboard-config-dir",
            r"C:\keyboards",
            "--simulate",
            "1:2",
        ])
        .expect("valid WPF arguments");

        assert_eq!(asset_dir, PathBuf::from(r"C:\assets"));
        assert_eq!(keyboard_config_dir, Some(PathBuf::from(r"C:\keyboards")));
    }
}
