mod hotplug;
#[cfg(not(target_os = "windows"))]
mod ui;

use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};
use keymap_core::{
    ActiveLayerChange, ActiveLayerState, RawLayerEvent, carries_report_magic,
    parse_raw_layer_event, transition_for, transition_for_disconnect,
};
#[cfg(not(target_os = "windows"))]
use log::error;
use log::{info, warn};
#[cfg(any(not(target_os = "windows"), test))]
use serde::{Deserialize, Serialize};
#[cfg(any(not(target_os = "windows"), test))]
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

pub(crate) const RAW_USAGE_PAGE: u16 = 0xFF60;
pub(crate) const RAW_USAGE_ID: u16 = 0x61;
const LOG_DIRECTORY_ENV: &str = "KEYMAP_OVERLAY_LOG_DIR";
const MAX_LOG_BYTES: u64 = 1_048_576;
const MAX_LOG_FILES: u8 = 3;
/// How long a reader blocks before checking whether its session was cancelled.
/// Reads are otherwise blocking, so this is the only idle wakeup the app has.
const READ_TIMEOUT: i32 = 1_000;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

/// Starts logging and runs the native AppKit overlay or Linux renderer service.
#[cfg(not(target_os = "windows"))]
pub fn run_native_overlay() -> Result<()> {
    initialize_logging()?;

    if let Err(error) = ui::run(assets_dir()?) {
        error!("Keymap overlay stopped: {error:#}");
        return Err(error);
    }
    Ok(())
}

/// Where the Raw HID listener delivers the events it reads.
///
/// The listener runs on its own thread while the platform backend owns the main
/// one, so delivering an event also has to wake whatever loop that backend
/// runs. Each does it differently — an AppKit channel, a Linux D-Bus service,
/// or a WPF dispatcher callback — and this is the seam between them.
///
/// Cloneable because each device gets its own reader thread.
pub trait LayerEventSink: Clone + Send {
    /// Returns whether the receiving end is still there; a reader stops once
    /// it is not.
    fn send(&self, event: ListenerEvent) -> bool;
}

/// An event from the HID listener, including loss of the device itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListenerEvent {
    Layer(RawLayerEvent),
    Disconnected { keyboard_id: Option<u8> },
}

#[cfg(any(not(target_os = "windows"), test))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct OverlayModel {
    pub(crate) version: u8,
    pub(crate) layer: u8,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) header_font_size: f64,
    pub(crate) key_font_size: f64,
    pub(crate) encoder_font_size: f64,
    pub(crate) keys: Vec<DisplayKey>,
    pub(crate) encoders: Vec<DisplayEncoder>,
}

#[cfg(any(not(target_os = "windows"), test))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DisplayKey {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) label: Vec<String>,
    pub(crate) held: bool,
    #[serde(default)]
    pub(crate) transparent: bool,
    #[serde(default)]
    pub(crate) momentary_layer: Option<u8>,
}

#[cfg(any(not(target_os = "windows"), test))]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DisplayEncoder {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) size: u32,
    pub(crate) counter_clockwise: Vec<String>,
    pub(crate) clockwise: Vec<String>,
    pub(crate) press: String,
    pub(crate) held: bool,
    #[serde(default)]
    pub(crate) counter_clockwise_transparent: bool,
    #[serde(default)]
    pub(crate) clockwise_transparent: bool,
    #[serde(default)]
    pub(crate) press_transparent: bool,
    #[serde(default)]
    pub(crate) momentary_layer: Option<u8>,
}

#[cfg(any(not(target_os = "windows"), test))]
pub(crate) type ModelCache = HashMap<(u8, u8), OverlayModel>;

/// A running listener that platform device notifications can ask to re-enumerate.
#[derive(Clone)]
pub struct RawHidListenerHandle {
    session: hotplug::RunningSession,
}

impl RawHidListenerHandle {
    /// Ends the current reader session after the platform reports a device arrival.
    pub fn device_arrived(&self) {
        self.session.end();
    }
}

pub fn spawn_raw_hid_listener(sink: impl LayerEventSink + 'static) -> RawHidListenerHandle {
    let session = hotplug::RunningSession::default();
    hotplug::spawn_watcher(session.clone());
    let handle = RawHidListenerHandle {
        session: session.clone(),
    };
    thread::spawn(move || {
        loop {
            if let Err(error) = run_raw_hid_session(&sink, &session) {
                warn!("Raw HID listener stopped: {error:#}");
            }
            // Also the grace period a keyboard needs between announcing itself
            // and being openable, which is why an arrival is not raced.
            thread::sleep(RECONNECT_INTERVAL);
        }
    });
    handle
}

/// What a report should do to the overlay, given the held momentary layers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Transition {
    Show {
        keyboard_id: u8,
        layers: Vec<u8>,
    },
    Hide,
    #[default]
    Ignore,
}

/// The held layers, and the one window update the events seen so far call for.
///
/// Every backend needs the same reduction: several events can pile up before a
/// UI loop gets to act on them, and only their final state should ever reach the
/// screen. Intermediate restores and switches are not drawn on the way to a
/// newer layer or a hide.
///
/// It accumulates rather than folding an iterator because every backend can
/// receive several HID events before its UI callback runs.
#[derive(Default)]
pub struct PendingTransition {
    held_keys: Vec<(u8, u8)>,
    transition: Transition,
}

impl PendingTransition {
    /// Folds one event in, keeping the latest transition that changes anything.
    pub fn push(&mut self, event: ListenerEvent) {
        let transition = transition_for_event(&mut self.held_keys, event);
        if transition != Transition::Ignore {
            self.transition = transition;
        }
    }

    /// Takes what the window should do now, leaving nothing pending behind.
    pub fn take(&mut self) -> Transition {
        std::mem::take(&mut self.transition)
    }
}

pub fn transition_for_event(held_keys: &mut Vec<(u8, u8)>, event: ListenerEvent) -> Transition {
    let change = match event {
        ListenerEvent::Layer(event) => transition_for(held_keys, event),
        ListenerEvent::Disconnected { keyboard_id } => {
            transition_for_disconnect(held_keys, keyboard_id)
        }
    };
    match change {
        ActiveLayerChange::Unchanged => Transition::Ignore,
        ActiveLayerChange::Changed(Some(ActiveLayerState {
            keyboard_id,
            layers,
        })) => Transition::Show {
            keyboard_id,
            layers,
        },
        ActiveLayerChange::Changed(None) => Transition::Hide,
    }
}

/// Loads every installed semantic layer model before the listener can show one.
#[cfg(not(target_os = "windows"))]
pub(crate) fn load_model_cache(assets_dir: &Path) -> Result<ModelCache> {
    let mut models = HashMap::new();
    for entry in fs::read_dir(assets_dir)
        .with_context(|| format!("Failed to read asset directory {}", assets_dir.display()))?
    {
        let entry = entry
            .with_context(|| format!("Failed to read an entry in {}", assets_dir.display()))?;
        let path = entry.path();
        let Some(key) = model_key(&path) else {
            continue;
        };
        let model: OverlayModel = serde_json::from_reader(
            File::open(&path).with_context(|| format!("Failed to open {}", path.display()))?,
        )
        .with_context(|| format!("Failed to parse {}", path.display()))?;
        if !matches!(model.version, 1 | 2) {
            anyhow::bail!(
                "Unsupported overlay model version {} in {}",
                model.version,
                path.display()
            );
        }
        if model.layer != key.1 {
            anyhow::bail!("Layer in {} does not match its filename", path.display());
        }
        models.insert(key, model);
    }
    Ok(models)
}

#[cfg(not(target_os = "windows"))]
fn model_key(path: &Path) -> Option<(u8, u8)> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return None;
    }
    let (keyboard_id, layer) = path.file_stem()?.to_str()?.split_once("_L")?;
    Some((keyboard_id.parse().ok()?, layer.parse().ok()?))
}

#[cfg(any(not(target_os = "windows"), test))]
pub(crate) fn compose_model(
    models: &ModelCache,
    keyboard_id: u8,
    layers: &[u8],
) -> Option<OverlayModel> {
    let mut model = models.get(&(keyboard_id, 0))?.clone();
    for layer in layers {
        let overlay = models.get(&(keyboard_id, *layer))?;
        apply_overlay(&mut model, overlay)?;
        model.layer = *layer;
    }
    for key in &mut model.keys {
        key.held = key
            .momentary_layer
            .is_some_and(|layer| layers.contains(&layer));
    }
    for encoder in &mut model.encoders {
        encoder.held = encoder
            .momentary_layer
            .is_some_and(|layer| layers.contains(&layer));
    }
    model.version = 2;
    Some(model)
}

#[cfg(any(not(target_os = "windows"), test))]
fn apply_overlay(model: &mut OverlayModel, overlay: &OverlayModel) -> Option<()> {
    if overlay.keys.len() != model.keys.len() || overlay.encoders.len() != model.encoders.len() {
        return None;
    }
    for (key, overlay_key) in model.keys.iter_mut().zip(&overlay.keys) {
        if !overlay_key.transparent {
            *key = overlay_key.clone();
        }
    }
    for (encoder, overlay_encoder) in model.encoders.iter_mut().zip(&overlay.encoders) {
        if !overlay_encoder.counter_clockwise_transparent {
            encoder.counter_clockwise = overlay_encoder.counter_clockwise.clone();
        }
        if !overlay_encoder.clockwise_transparent {
            encoder.clockwise = overlay_encoder.clockwise.clone();
        }
        if !overlay_encoder.press_transparent {
            encoder.press = overlay_encoder.press.clone();
            encoder.momentary_layer = overlay_encoder.momentary_layer;
        }
    }
    Some(())
}

/// Initializes the rotating file logger used by every platform frontend.
pub fn initialize_logging() -> Result<()> {
    let log_directory = resolve_log_directory(env::var_os(LOG_DIRECTORY_ENV), home_directory())?;
    fs::create_dir_all(&log_directory)
        .with_context(|| format!("Failed to create log directory {}", log_directory.display()))?;
    let writer = RotatingLogWriter::new(log_directory.join("overlay.log"))?;

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Pipe(Box::new(writer)))
        .try_init()
        .map_err(|error| anyhow::anyhow!("Failed to initialize logger: {error}"))?;
    Ok(())
}

/// Takes the environment as arguments so the fallback order stays testable;
/// `env::set_var` is unsafe in this edition and the workspace forbids unsafe.
fn resolve_log_directory(configured: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    configured
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".local/var/log/keymap-overlay")))
        .context("Neither KEYMAP_OVERLAY_LOG_DIR nor a home directory is set")
}

fn home_directory() -> Option<OsString> {
    resolve_home_directory(env::var_os("HOME"), env::var_os("USERPROFILE"))
}

/// The user's home directory, under whichever name this system knows it by.
///
/// Windows sets `USERPROFILE` and not `HOME`, and the overlay runs there as a
/// native process started from the Run key, so it inherits no shell's idea of
/// `HOME`. An MSYS2 `HOME` such as `/home/user` is not an absolute Windows
/// path, so Windows ignores it and uses `USERPROFILE` when both are set.
fn resolve_home_directory(
    home: Option<OsString>,
    user_profile: Option<OsString>,
) -> Option<OsString> {
    #[cfg(target_os = "windows")]
    {
        home.filter(|path| Path::new(path).is_absolute())
            .or(user_profile)
    }

    #[cfg(not(target_os = "windows"))]
    home.or(user_profile)
}

#[cfg(not(target_os = "windows"))]
fn assets_dir() -> Result<PathBuf> {
    // args_os, not args: the latter panics on a non-UTF-8 argument, and an
    // asset path handed to us on the command line is an arbitrary byte string.
    resolve_assets_dir(env::args_os().nth(1), home_directory())
}

#[cfg(any(not(target_os = "windows"), test))]
fn resolve_assets_dir(argument: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    if let Some(path) = argument {
        return Ok(PathBuf::from(path));
    }

    let home = home.context("No home directory is set")?;
    Ok(PathBuf::from(home).join(".config/keymap-overlay"))
}

/// Reads from every connected Raw HID device until one of them disconnects, or
/// another one appears, then returns so the caller can enumerate again.
fn run_raw_hid_session(
    sink: &impl LayerEventSink,
    session: &hotplug::RunningSession,
) -> Result<()> {
    let api = HidApi::new().context("Failed to enumerate HID devices")?;
    let devices: Vec<(HidDevice, String)> = api
        .device_list()
        .filter(|device| device.usage_page() == RAW_USAGE_PAGE && device.usage() == RAW_USAGE_ID)
        .filter_map(|device_info| match device_info.open_device(&api) {
            Ok(device) => Some((device, device_info.path().to_string_lossy().into_owned())),
            Err(error) => {
                warn!(
                    "Failed to open Raw HID device {:04x}:{:04x}: {error}",
                    device_info.vendor_id(),
                    device_info.product_id()
                );
                None
            }
        })
        .collect();

    if devices.is_empty() {
        return Ok(());
    }

    info!("Listening on {} Raw HID device(s)", devices.len());
    // One reader per device: hidapi cannot wait on several devices at once, so
    // sharing a thread would mean polling and adding latency for each keyboard.
    let cancelled = Arc::new(AtomicBool::new(false));
    session.attach(&cancelled);
    let result = thread::scope(|scope| -> Result<()> {
        // HidDevice is Send but not Sync, so each reader owns its device.
        let mut readers = Vec::new();
        for (device, path) in devices {
            let sink = sink.clone();
            let cancelled = Arc::clone(&cancelled);
            readers.push(scope.spawn(move || {
                let result = receive_from_device(&device, &path, &sink, &cancelled);
                // Any disconnect ends the session so all devices are reopened.
                cancelled.store(true, Ordering::Relaxed);
                result
            }));
        }
        for reader in readers {
            match reader.join() {
                Ok(result) => result?,
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        Ok(())
    });
    session.detach();
    result
}

fn receive_from_device(
    device: &HidDevice,
    path: &str,
    sink: &impl LayerEventSink,
    cancelled: &AtomicBool,
) -> Result<()> {
    let mut report = [0_u8; 33];
    let mut keyboard_id = None;
    while !cancelled.load(Ordering::Relaxed) {
        let length = match device.read_timeout(&mut report, READ_TIMEOUT) {
            Ok(length) => length,
            Err(error) => {
                // A bootloader transition can remove the keyboard before it
                // sends the matching layer release. Clear the UI state rather
                // than leaving the last layer visible until reconnect.
                sink.send(ListenerEvent::Disconnected { keyboard_id });
                return Err(error).with_context(|| format!("Failed to read Raw HID device {path}"));
            }
        };
        let frame = &report[..length];
        let Some(event) = parse_raw_layer_event(frame) else {
            // Unrelated traffic (VIAL) shares this interface and is expected,
            // but a frame carrying our magic that still fails to parse means
            // the firmware and the overlay disagree about the wire format.
            if carries_report_magic(frame) {
                warn!("Ignoring malformed KMO report of {length} bytes");
            }
            continue;
        };
        info!(
            "Layer event: keyboard={} layer={} pressed={}",
            event.keyboard_id, event.layer, event.pressed
        );
        keyboard_id = Some(event.keyboard_id);
        if !sink.send(ListenerEvent::Layer(event)) {
            return Ok(());
        }
    }
    if keyboard_id.is_some() {
        // Another reader ended the session. Closing this device can lose its
        // matching release report, so clear any layer it may still hold.
        sink.send(ListenerEvent::Disconnected { keyboard_id });
    }
    Ok(())
}

struct RotatingLogWriter {
    path: PathBuf,
    file: File,
    written_bytes: u64,
    max_bytes: u64,
}

impl RotatingLogWriter {
    // Returns anyhow::Result because this runs before the logger exists, so the
    // path has to travel with the error to be of any use.
    fn new(path: PathBuf) -> Result<Self> {
        Self::with_limit(path, MAX_LOG_BYTES)
    }

    /// The limit is a parameter so rotation can be exercised without writing
    /// megabytes; production callers use [`RotatingLogWriter::new`].
    fn with_limit(path: PathBuf, max_bytes: u64) -> Result<Self> {
        let file = open_log_file(&path)
            .with_context(|| format!("Failed to open log file {}", path.display()))?;
        // Tracked from here on so that writing a line costs no extra syscall.
        let written_bytes = file
            .metadata()
            .with_context(|| format!("Failed to inspect log file {}", path.display()))?
            .len();
        Ok(Self {
            path,
            file,
            written_bytes,
            max_bytes,
        })
    }

    fn rotate_if_needed(&mut self, incoming_bytes: usize) -> io::Result<()> {
        if self.written_bytes.saturating_add(incoming_bytes as u64) <= self.max_bytes {
            return Ok(());
        }

        self.file.flush()?;
        remove_file_if_exists(&rotated_log_path(&self.path, MAX_LOG_FILES))?;
        for index in (1..MAX_LOG_FILES).rev() {
            rename_if_exists(
                &rotated_log_path(&self.path, index),
                &rotated_log_path(&self.path, index + 1),
            )?;
        }
        rename_if_exists(&self.path, &rotated_log_path(&self.path, 1))?;
        self.file = open_log_file(&self.path)?;
        self.written_bytes = 0;
        Ok(())
    }
}

impl Write for RotatingLogWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed(buffer.len())?;
        let written_bytes = self.file.write(buffer)?;
        self.written_bytes = self.written_bytes.saturating_add(written_bytes as u64);
        Ok(written_bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn open_log_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn rotated_log_path(path: &Path, index: u8) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.display(), index))
}

fn rename_if_exists(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn display_key(label: &str, transparent: bool, momentary_layer: Option<u8>) -> DisplayKey {
        DisplayKey {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
            label: vec![label.to_owned()],
            held: false,
            transparent,
            momentary_layer,
        }
    }

    fn overlay_model(layer: u8, keys: Vec<DisplayKey>) -> OverlayModel {
        OverlayModel {
            version: 2,
            layer,
            width: 10,
            height: 10,
            header_font_size: 14.0,
            key_font_size: 10.0,
            encoder_font_size: 10.0,
            keys,
            encoders: vec![],
        }
    }

    #[test]
    fn active_layer_changes_are_translated_for_the_ui() {
        assert_eq!(
            transition_for_event(
                &mut vec![],
                ListenerEvent::Layer(RawLayerEvent {
                    keyboard_id: 1,
                    layer: 2,
                    pressed: true,
                }),
            ),
            Transition::Show {
                keyboard_id: 1,
                layers: vec![2],
            }
        );
        assert_eq!(
            transition_for_event(
                &mut vec![(1, 2)],
                ListenerEvent::Disconnected {
                    keyboard_id: Some(1),
                },
            ),
            Transition::Hide
        );
        assert_eq!(
            transition_for_event(
                &mut vec![(1, 2)],
                ListenerEvent::Disconnected {
                    keyboard_id: Some(2),
                },
            ),
            Transition::Ignore
        );
    }

    #[test]
    fn models_follow_qmk_precedence_and_transparency() {
        let mut models = ModelCache::new();
        models.insert(
            (1, 0),
            overlay_model(
                0,
                vec![
                    display_key("BASE A", false, None),
                    display_key("L3", false, Some(3)),
                ],
            ),
        );
        models.insert(
            (1, 1),
            overlay_model(
                1,
                vec![
                    display_key("LAYER 1", false, None),
                    display_key("", true, None),
                ],
            ),
        );
        models.insert(
            (1, 3),
            overlay_model(
                3,
                vec![
                    display_key("", true, None),
                    display_key("LAYER 3", false, None),
                ],
            ),
        );

        let composed = compose_model(&models, 1, &[1, 3]).expect("composed model");

        assert_eq!(composed.layer, 3);
        assert_eq!(composed.keys[0].label, ["LAYER 1"]);
        assert_eq!(composed.keys[1].label, ["LAYER 3"]);

        let without_layer_one = compose_model(&models, 1, &[3]).expect("composed model");
        assert_eq!(without_layer_one.keys[0].label, ["BASE A"]);
    }

    #[test]
    fn queued_events_only_expose_their_final_state_to_the_ui() {
        let mut pending = PendingTransition {
            held_keys: vec![(1, 2)],
            transition: Transition::Ignore,
        };
        let events = [
            ListenerEvent::Layer(RawLayerEvent {
                keyboard_id: 1,
                layer: 3,
                pressed: true,
            }),
            ListenerEvent::Layer(RawLayerEvent {
                keyboard_id: 1,
                layer: 3,
                pressed: false,
            }),
            ListenerEvent::Layer(RawLayerEvent {
                keyboard_id: 1,
                layer: 2,
                pressed: false,
            }),
        ];
        for event in events {
            pending.push(event);
        }

        assert_eq!(pending.take(), Transition::Hide);
        assert!(pending.held_keys.is_empty());
    }

    /// Taking the transition must not hand the same update out a second time.
    #[test]
    fn nothing_is_pending_once_it_has_been_taken() {
        let mut pending = PendingTransition::default();
        pending.push(ListenerEvent::Layer(RawLayerEvent {
            keyboard_id: 1,
            layer: 2,
            pressed: true,
        }));

        assert_eq!(
            pending.take(),
            Transition::Show {
                keyboard_id: 1,
                layers: vec![2],
            }
        );
        assert_eq!(pending.take(), Transition::Ignore);
    }

    /// An event that changes nothing must not clear a transition still waiting
    /// to be drawn.
    #[test]
    fn an_ignored_event_leaves_an_earlier_transition_pending() {
        let mut pending = PendingTransition::default();
        pending.push(ListenerEvent::Layer(RawLayerEvent {
            keyboard_id: 1,
            layer: 2,
            pressed: true,
        }));
        pending.push(ListenerEvent::Disconnected {
            keyboard_id: Some(9),
        });

        assert_eq!(
            pending.take(),
            Transition::Show {
                keyboard_id: 1,
                layers: vec![2],
            }
        );
    }

    #[test]
    fn an_explicit_assets_directory_wins_over_home() {
        let directory = resolve_assets_dir(
            Some(OsString::from("/somewhere/else")),
            Some(OsString::from("/home/user")),
        )
        .expect("an explicit argument needs no environment");

        assert_eq!(directory, PathBuf::from("/somewhere/else"));
    }

    #[test]
    fn the_assets_directory_defaults_under_home() {
        let directory = resolve_assets_dir(None, Some(OsString::from("/home/user")))
            .expect("HOME is enough on its own");

        assert_eq!(
            directory,
            PathBuf::from("/home/user/.config/keymap-overlay")
        );
    }

    #[test]
    fn the_assets_directory_needs_home_when_no_argument_is_given() {
        assert!(resolve_assets_dir(None, None).is_err());
    }

    #[test]
    fn the_configured_log_directory_wins_over_home() {
        let directory = resolve_log_directory(
            Some(OsString::from("/var/log/overlay")),
            Some(OsString::from("/home/user")),
        )
        .expect("the service definition always sets the log directory");

        assert_eq!(directory, PathBuf::from("/var/log/overlay"));
    }

    #[test]
    fn the_log_directory_defaults_under_home() {
        let directory = resolve_log_directory(None, Some(OsString::from("/home/user")))
            .expect("HOME is enough on its own");

        assert_eq!(
            directory,
            PathBuf::from("/home/user/.local/var/log/keymap-overlay")
        );
    }

    #[test]
    fn the_log_directory_needs_one_of_the_two_variables() {
        assert!(resolve_log_directory(None, None).is_err());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn home_wins_over_user_profile_where_both_are_set() {
        assert_eq!(
            resolve_home_directory(
                Some(OsString::from("/home/user")),
                Some(OsString::from(r"C:\Users\user"))
            ),
            Some(OsString::from("/home/user"))
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn user_profile_replaces_a_non_native_home() {
        assert_eq!(
            resolve_home_directory(
                Some(OsString::from("/home/user")),
                Some(OsString::from(r"C:\Users\user"))
            ),
            Some(OsString::from(r"C:\Users\user"))
        );
    }

    #[test]
    fn user_profile_stands_in_for_an_unset_home() {
        // The Windows login autostart process inherits no HOME at all.
        assert_eq!(
            resolve_home_directory(None, Some(OsString::from(r"C:\Users\user"))),
            Some(OsString::from(r"C:\Users\user"))
        );
    }

    #[test]
    fn neither_variable_leaves_the_home_directory_unknown() {
        assert_eq!(resolve_home_directory(None, None), None);
    }

    fn contents(path: &Path) -> String {
        fs::read_to_string(path).expect("log file should exist")
    }

    /// Returns a writer over `<temp>/overlay.log` and the directory holding it,
    /// which must stay alive for as long as the writer is used.
    fn writer(max_bytes: u64) -> (TempDir, RotatingLogWriter) {
        let directory = TempDir::new().expect("temp dir");
        let writer = RotatingLogWriter::with_limit(directory.path().join("overlay.log"), max_bytes)
            .expect("writer");
        (directory, writer)
    }

    #[test]
    fn writes_up_to_the_limit_do_not_rotate() {
        let (directory, mut writer) = writer(10);

        writer.write_all(b"0123456789").expect("write");
        writer.flush().expect("flush");

        assert_eq!(
            contents(&directory.path().join("overlay.log")),
            "0123456789"
        );
        assert!(!directory.path().join("overlay.log.1").exists());
    }

    #[test]
    fn a_write_crossing_the_limit_rotates_first() {
        let (directory, mut writer) = writer(10);

        writer.write_all(b"0123456789").expect("write");
        writer.write_all(b"abc").expect("write");
        writer.flush().expect("flush");

        // The full line lands in the new file rather than being split.
        assert_eq!(contents(&directory.path().join("overlay.log")), "abc");
        assert_eq!(
            contents(&directory.path().join("overlay.log.1")),
            "0123456789"
        );
    }

    #[test]
    fn rotation_keeps_only_the_configured_number_of_previous_logs() {
        let (directory, mut writer) = writer(4);

        for line in [b"aaaa", b"bbbb", b"cccc", b"dddd", b"eeee"] {
            writer.write_all(line).expect("write");
        }
        writer.flush().expect("flush");

        let log = directory.path().join("overlay.log");
        assert_eq!(contents(&log), "eeee");
        assert_eq!(contents(&rotated_log_path(&log, 1)), "dddd");
        assert_eq!(contents(&rotated_log_path(&log, 2)), "cccc");
        assert_eq!(contents(&rotated_log_path(&log, 3)), "bbbb");
        // The oldest file is deleted rather than growing the retention set.
        assert!(!rotated_log_path(&log, MAX_LOG_FILES + 1).exists());
    }

    #[test]
    fn an_existing_log_is_appended_to() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join("overlay.log");
        fs::write(&path, "12345678").expect("seed log");

        let mut writer = RotatingLogWriter::with_limit(path.clone(), 100).expect("writer");
        writer.write_all(b"abc").expect("write");
        writer.flush().expect("flush");

        assert_eq!(contents(&path), "12345678abc");
    }

    /// A restart must not forget how large the log already is, or the file
    /// would grow past the limit until the process happened to write enough.
    #[test]
    fn an_existing_log_counts_toward_the_limit() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join("overlay.log");
        fs::write(&path, "12345678").expect("seed log");

        let mut writer = RotatingLogWriter::with_limit(path.clone(), 10).expect("writer");
        writer.write_all(b"abc").expect("write");
        writer.flush().expect("flush");

        assert_eq!(contents(&path), "abc");
        assert_eq!(contents(&rotated_log_path(&path, 1)), "12345678");
    }

    #[test]
    fn a_write_larger_than_the_limit_still_lands() {
        let (directory, mut writer) = writer(4);

        writer.write_all(b"0123456789").expect("write");
        writer.flush().expect("flush");

        assert_eq!(
            contents(&directory.path().join("overlay.log")),
            "0123456789"
        );
    }

    #[test]
    fn rotated_logs_are_numbered_after_the_base_name() {
        assert_eq!(
            rotated_log_path(Path::new("/var/log/overlay.log"), 2),
            PathBuf::from("/var/log/overlay.log.2")
        );
    }

    /// The first rotation happens with no previous files to move.
    #[test]
    fn the_rotation_helpers_tolerate_missing_files() {
        let directory = TempDir::new().expect("temp dir");
        let missing = directory.path().join("absent.log");

        rename_if_exists(&missing, &directory.path().join("absent.log.1")).expect("rename");
        remove_file_if_exists(&missing).expect("remove");
    }
}
