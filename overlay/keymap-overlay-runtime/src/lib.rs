use anyhow::{Context, Result};
// Re-exported so a frontend can parse the shared command line without taking a
// clap dependency of its own.
pub use clap::Parser;
use hidapi::{HidApi, HidDevice};
use keymap_core::{
    ActiveLayerChange, ActiveLayerState, PendingLayerChange, carries_report_magic,
    parse_raw_layer_event,
};
pub use keymap_core::{LayerEvent, RawLayerEvent};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

/// Vendor-defined usage page carrying keymap overlay reports.
pub const RAW_USAGE_PAGE: u16 = 0xFF60;
/// Usage within [`RAW_USAGE_PAGE`] carrying keymap overlay reports.
pub const RAW_USAGE_ID: u16 = 0x61;
const MAX_LOG_BYTES: u64 = 1_048_576;
const MAX_LOG_FILES: u8 = 3;
/// How long a reader blocks before checking for disconnects or UI shutdown.
const READ_TIMEOUT: i32 = 1_000;
const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

/// This project's own licence terms.
///
/// Embedded rather than installed beside the executable, which lives in a
/// different directory from the models: a copy carried anywhere can still state
/// its terms.
pub const LICENSE: &str = include_str!("../../../LICENSE.md");

/// The generated third-party notice, as shipped in the release archive.
///
/// `make licenses` has to have run before a build that embeds it. The
/// pre-commit hook and the CI `check-licenses` step already guarantee that, so
/// nothing regenerates it here.
pub const THIRD_PARTY_LICENSES: &str = include_str!("../../../THIRD-PARTY-LICENSES.html");

/// The overlay's command line.
//
// These stay out of the doc comment because clap derives `--help` from it.
//
// `exclusive` is where "a notice reads no models, so it takes no asset
// directory" gets machine-checked rather than left to a reviewer.
//
// The third-party notice is deliberately not also spelled `--licenses`: one
// letter from `--license`, it would answer a typo with 168 KiB of HTML.
#[derive(Clone, Debug, Parser)]
#[command(
    name = "keymap-overlay",
    version,
    about = "Shows the held QMK momentary layer in a native overlay",
    long_about = None
)]
pub struct Arguments {
    /// Directory holding the installed <keyboard_id>.json models
    #[cfg_attr(
        target_os = "windows",
        doc = "[default: %LOCALAPPDATA%\\keymap-overlay]"
    )]
    #[cfg_attr(
        not(target_os = "windows"),
        doc = "[default: $HOME/.cache/keymap-overlay]"
    )]
    #[arg(long, value_name = "PATH")]
    pub asset_dir: Option<PathBuf>,

    /// Write the log to this file, rotating it, instead of to stderr
    #[arg(long, value_name = "PATH")]
    pub log_out: Option<PathBuf>,

    /// Print this project's own licence terms
    #[arg(long, exclusive = true)]
    pub license: bool,

    /// Print the third-party notices, as HTML
    #[arg(long, exclusive = true)]
    pub third_party_licenses: bool,
}

impl Arguments {
    /// Returns the notice to print and exit on, if one was asked for.
    pub fn notice(&self) -> Option<&'static str> {
        if self.license {
            return Some(LICENSE);
        }
        self.third_party_licenses.then_some(THIRD_PARTY_LICENSES)
    }

    /// Returns where this invocation wants its log.
    ///
    /// Defaulting to stderr is what lets the systemd unit hand the log to
    /// journald by simply not passing `--log-out`.
    pub fn log_destination(self) -> LogDestination {
        self.log_out
            .map_or(LogDestination::Stderr, LogDestination::File)
    }
}

/// Initializes the shared runtime and gives the asset directory to a frontend.
pub fn run_overlay(frontend: impl FnOnce(PathBuf) -> Result<()>) -> Result<()> {
    let arguments = Arguments::parse();
    if let Some(notice) = arguments.notice() {
        return write_notice(notice);
    }
    let directory = arguments
        .asset_dir
        .clone()
        .map_or_else(default_asset_dir, Ok)?;
    initialize_logging(arguments.log_destination())?;

    if let Err(error) = frontend(directory) {
        error!("Keymap overlay stopped: {error:#}");
        return Err(error);
    }
    Ok(())
}

/// Writes a notice to standard output, treating a closed pipe as success.
///
/// 168 KiB is normally read through `head` or a pager, and Rust ignores
/// SIGPIPE, so otherwise quitting the pager would look like a write failure.
pub fn write_notice(text: &str) -> Result<()> {
    let mut stdout = io::stdout();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.context("Failed to write to standard output"),
    }
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
    fn send(&self, event: LayerEvent) -> bool;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OverlayModel {
    pub version: u8,
    pub layer: u8,
    pub width: u32,
    pub height: u32,
    pub header_font_size: f64,
    pub key_font_size: f64,
    pub encoder_font_size: f64,
    pub keys: Vec<DisplayKey>,
    pub encoders: Vec<DisplayEncoder>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DisplayKey {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub label: Vec<String>,
    pub held: bool,
    #[serde(default)]
    pub transparent: bool,
    #[serde(default)]
    pub momentary_layer: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DisplayEncoder {
    pub x: u32,
    pub y: u32,
    pub size: u32,
    pub counter_clockwise: Vec<String>,
    pub clockwise: Vec<String>,
    pub press: String,
    pub held: bool,
    #[serde(default)]
    pub counter_clockwise_transparent: bool,
    #[serde(default)]
    pub clockwise_transparent: bool,
    #[serde(default)]
    pub press_transparent: bool,
    #[serde(default)]
    pub momentary_layer: Option<u8>,
}

pub type ModelCache = HashMap<(u8, u8), OverlayModel>;

/// One keyboard's installed `<keyboard_id>.json`: every layer in one file, since
/// a keyboard's layers are generated, installed, and read together.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct KeyboardModels {
    keyboard_id: u8,
    layers: HashMap<u8, OverlayModel>,
}

/// Coalesces platform arrival notifications into listener enumerations.
#[derive(Clone)]
struct EnumerationRequester {
    pending: Arc<AtomicBool>,
    wake: mpsc::Sender<()>,
}

impl EnumerationRequester {
    fn new(wake: mpsc::Sender<()>) -> Self {
        Self {
            pending: Arc::new(AtomicBool::new(false)),
            wake,
        }
    }

    fn request(&self) -> bool {
        if self.pending.swap(true, Ordering::Relaxed) {
            return false;
        }
        if self.wake.send(()).is_err() {
            self.pending.store(false, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn begin_enumeration(&self) {
        self.pending.store(false, Ordering::Relaxed);
    }
}

/// A running listener that platform device notifications can ask to re-enumerate.
#[derive(Clone)]
pub struct RawHidListenerHandle {
    requester: EnumerationRequester,
}

impl RawHidListenerHandle {
    /// Requests enumeration after an arrival, returning whether one was queued.
    pub fn device_arrived(&self) -> bool {
        self.requester.request()
    }
}

pub fn spawn_raw_hid_listener(sink: impl LayerEventSink + 'static) -> RawHidListenerHandle {
    let (wake, requests) = mpsc::channel();
    let requester = EnumerationRequester::new(wake);
    let handle = RawHidListenerHandle {
        requester: requester.clone(),
    };
    thread::spawn(move || {
        let active_paths = Arc::new(Mutex::new(HashSet::new()));
        enumerate_raw_hid_devices(&sink, &active_paths, &requester);
        loop {
            if requests.recv().is_err() {
                return;
            }
            // Give a newly announced keyboard time to become openable. Existing
            // readers remain alive and cannot lose releases during this grace.
            thread::sleep(RECONNECT_INTERVAL);
            requester.begin_enumeration();
            enumerate_raw_hid_devices(&sink, &active_paths, &requester);
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

/// Adapts the core reducer's final active-layer change for an overlay window.
///
/// Core keeps only the final state change from queued events; this adapter
/// translates it to show, hide, or ignore when the frontend consumes it.
#[derive(Default)]
pub struct PendingTransition {
    pending: PendingLayerChange,
}

impl PendingTransition {
    /// Folds one event in, keeping the latest transition that changes anything.
    pub fn push(&mut self, event: LayerEvent) {
        self.pending.push(event);
    }

    /// Takes what the window should do now, leaving nothing pending behind.
    pub fn take(&mut self) -> Transition {
        transition_for_change(self.pending.take())
    }
}

fn transition_for_change(change: ActiveLayerChange) -> Transition {
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
pub fn load_model_cache(assets_dir: &Path) -> Result<ModelCache> {
    let mut models = HashMap::new();
    for entry in fs::read_dir(assets_dir)
        .with_context(|| format!("Failed to read asset directory {}", assets_dir.display()))?
    {
        let entry = entry
            .with_context(|| format!("Failed to read an entry in {}", assets_dir.display()))?;
        let path = entry.path();
        let Some(keyboard_id) = keyboard_id_from_path(&path) else {
            continue;
        };
        let keyboard_models: KeyboardModels = serde_json::from_reader(
            File::open(&path).with_context(|| format!("Failed to open {}", path.display()))?,
        )
        .with_context(|| format!("Failed to parse {}", path.display()))?;
        if keyboard_models.keyboard_id != keyboard_id {
            anyhow::bail!(
                "Keyboard ID in {} does not match its filename",
                path.display()
            );
        }
        for (layer, model) in keyboard_models.layers {
            if !matches!(model.version, 1 | 2) {
                anyhow::bail!(
                    "Unsupported overlay model version {} in {}",
                    model.version,
                    path.display()
                );
            }
            if model.layer != layer {
                anyhow::bail!(
                    "Layer in {} does not match its key in \"layers\"",
                    path.display()
                );
            }
            models.insert((keyboard_id, layer), model);
        }
    }
    Ok(models)
}

fn keyboard_id_from_path(path: &Path) -> Option<u8> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return None;
    }
    path.file_stem()?.to_str()?.parse().ok()
}

pub fn compose_model(models: &ModelCache, keyboard_id: u8, layers: &[u8]) -> Option<OverlayModel> {
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

/// Where a frontend wants its log to go.
///
/// Named by the caller rather than read from the environment, because the
/// Windows Run key carries arguments but no environment at all.
pub enum LogDestination {
    /// Leave the log on stderr for the supervisor to capture.
    ///
    /// journald already timestamps, rotates and retains it, and it is where a
    /// Linux user looks first.
    Stderr,
    /// Write to this file, rotating it in-process.
    ///
    /// launchd never rotates what it redirects, so a login-to-logout process
    /// has to bound its own log.
    File(PathBuf),
}

/// Initializes the logger every platform frontend shares.
pub fn initialize_logging(destination: LogDestination) -> Result<()> {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    match destination {
        LogDestination::Stderr => {
            // journald stamps every entry it receives.
            builder.format_timestamp(None);
        }
        LogDestination::File(path) => {
            if let Some(directory) = path.parent() {
                fs::create_dir_all(directory).with_context(|| {
                    format!("Failed to create log directory {}", directory.display())
                })?;
            }
            builder.target(env_logger::Target::Pipe(Box::new(RotatingLogWriter::new(
                path,
            )?)));
        }
    }
    builder
        .try_init()
        .map_err(|error| anyhow::anyhow!("Failed to initialize logger: {error}"))?;
    Ok(())
}

/// The log file a frontend that cannot be given one on its command line uses.
///
/// Only the Windows frontends need this: they reach the shared runtime through
/// a C ABI that deliberately carries no strings.
pub fn default_log_file() -> Result<PathBuf> {
    resolve_default_log_file(env::var_os("LOCALAPPDATA"), home_directory())
}

/// Takes the environment as arguments so the fallback order stays testable;
/// `env::set_var` is unsafe in this edition and the workspace forbids unsafe.
fn resolve_default_log_file(
    local_app_data: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows_local_app_data(local_app_data, home)
            .map(|root| root.join("keymap-overlay/logs/overlay.log"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = local_app_data;
        let home = home.context("No home directory is set")?;
        Ok(PathBuf::from(home).join(".local/var/log/keymap-overlay/overlay.log"))
    }
}

/// The root Windows keeps a program's per-user data under.
///
/// Local rather than roaming `%APPDATA%`, because generated models and a log
/// both describe one machine. The fallback covers the stripped environment a
/// Run key process can inherit.
#[cfg(target_os = "windows")]
fn windows_local_app_data(
    local_app_data: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf> {
    local_app_data
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join("AppData/Local")))
        .context("Neither LOCALAPPDATA nor a home directory is set")
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

/// Where the installed layer models live when the command line names no path.
///
/// Each system is asked where its own per-user data belongs: `$XDG_CACHE_HOME`
/// means nothing to Windows, and `%LOCALAPPDATA%` means nothing to Unix. The
/// models are a regenerable cache of what a VIAL-flashed device already knows,
/// not configuration, so they sit under `.cache` rather than `.config`.
pub fn default_asset_dir() -> Result<PathBuf> {
    resolve_default_asset_dir(env::var_os("LOCALAPPDATA"), home_directory())
}

fn resolve_default_asset_dir(
    local_app_data: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        windows_local_app_data(local_app_data, home).map(|root| root.join("keymap-overlay"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = local_app_data;
        let home = home.context("No home directory is set")?;
        Ok(PathBuf::from(home).join(".cache/keymap-overlay"))
    }
}

/// Opens newly discovered Raw HID devices without interrupting active readers.
fn enumerate_raw_hid_devices<S: LayerEventSink + 'static>(
    sink: &S,
    active_paths: &Arc<Mutex<HashSet<String>>>,
    requester: &EnumerationRequester,
) {
    let api = match HidApi::new().context("Failed to enumerate HID devices") {
        Ok(api) => api,
        Err(error) => {
            warn!("Raw HID enumeration failed: {error:#}");
            requester.request();
            return;
        }
    };
    let mut opened = 0;
    let mut retry_needed = false;
    for device_info in api
        .device_list()
        .filter(|device| device.usage_page() == RAW_USAGE_PAGE && device.usage() == RAW_USAGE_ID)
    {
        let path = device_info.path().to_string_lossy().into_owned();
        if active_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&path)
        {
            continue;
        }
        let device = match device_info.open_device(&api) {
            Ok(device) => device,
            Err(error) => {
                warn!(
                    "Failed to open Raw HID device {:04x}:{:04x}: {error}",
                    device_info.vendor_id(),
                    device_info.product_id()
                );
                retry_needed = true;
                continue;
            }
        };
        active_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(path.clone());
        opened += 1;
        let sink = sink.clone();
        let active_paths = Arc::clone(active_paths);
        let requester = requester.clone();
        // HidDevice is Send but not Sync, so each reader owns its device.
        thread::spawn(move || {
            if let Err(error) = receive_from_device(&device, &path, &sink) {
                warn!("Raw HID reader stopped: {error:#}");
            }
            active_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&path);
            requester.request();
        });
    }
    if opened > 0 {
        info!("Listening on {opened} new Raw HID device(s)");
    }
    if retry_needed {
        requester.request();
    }
}

fn receive_from_device(device: &HidDevice, path: &str, sink: &impl LayerEventSink) -> Result<()> {
    let mut report = [0_u8; 33];
    let mut keyboard_id = None;
    loop {
        let length = match device.read_timeout(&mut report, READ_TIMEOUT) {
            Ok(length) => length,
            Err(error) => {
                // A bootloader transition can remove the keyboard before it
                // sends the matching layer release. Clear the UI state rather
                // than leaving the last layer visible until reconnect.
                sink.send(LayerEvent::Disconnected { keyboard_id });
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
        if !sink.send(LayerEvent::Report(event)) {
            return Ok(());
        }
    }
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
    fn an_arrival_requests_enumeration() {
        let (sender, receiver) = mpsc::channel();
        let requester = EnumerationRequester::new(sender);

        assert!(requester.request());
        assert_eq!(receiver.try_recv(), Ok(()));
    }

    #[test]
    fn an_arrival_burst_is_coalesced() {
        let (sender, receiver) = mpsc::channel();
        let requester = EnumerationRequester::new(sender);

        assert!(requester.request());
        assert!(!requester.request());
        assert!(!requester.request());
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn a_later_arrival_can_request_another_pass() {
        let (sender, receiver) = mpsc::channel();
        let requester = EnumerationRequester::new(sender);
        requester.request();
        receiver.recv().expect("first request");

        requester.begin_enumeration();

        assert!(requester.request());
        assert_eq!(receiver.try_recv(), Ok(()));
    }

    #[test]
    fn active_layer_changes_are_translated_for_the_ui() {
        assert_eq!(
            transition_for_change(ActiveLayerChange::Changed(Some(ActiveLayerState {
                keyboard_id: 1,
                layers: vec![2],
            }))),
            Transition::Show {
                keyboard_id: 1,
                layers: vec![2],
            }
        );
        assert_eq!(
            transition_for_change(ActiveLayerChange::Changed(None)),
            Transition::Hide
        );
        assert_eq!(
            transition_for_change(ActiveLayerChange::Unchanged),
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

    fn write_keyboard_models(dir: &TempDir, filename: &str, keyboard_models: &KeyboardModels) {
        fs::write(
            dir.path().join(filename),
            serde_json::to_string(keyboard_models).expect("serialize keyboard models"),
        )
        .expect("write keyboard models file");
    }

    #[test]
    fn load_model_cache_reads_every_layer_from_one_keyboard_file() {
        let dir = TempDir::new().expect("temp dir");
        let mut layers = HashMap::new();
        layers.insert(0, overlay_model(0, vec![display_key("BASE", false, None)]));
        layers.insert(
            1,
            overlay_model(1, vec![display_key("LAYER 1", false, None)]),
        );
        write_keyboard_models(
            &dir,
            "1.json",
            &KeyboardModels {
                keyboard_id: 1,
                layers,
            },
        );

        let models = load_model_cache(dir.path()).expect("load model cache");

        assert_eq!(models.len(), 2);
        assert_eq!(models[&(1, 0)].keys[0].label, ["BASE"]);
        assert_eq!(models[&(1, 1)].keys[0].label, ["LAYER 1"]);
    }

    #[test]
    fn load_model_cache_ignores_files_that_are_not_a_keyboard_id() {
        let dir = TempDir::new().expect("temp dir");
        fs::write(dir.path().join("README.md"), "not a model").expect("write readme");
        fs::write(dir.path().join("notes.json"), "{}").expect("write stray json");

        let models = load_model_cache(dir.path()).expect("load model cache");

        assert!(models.is_empty());
    }

    #[test]
    fn load_model_cache_rejects_a_keyboard_id_that_does_not_match_its_filename() {
        let dir = TempDir::new().expect("temp dir");
        let mut layers = HashMap::new();
        layers.insert(0, overlay_model(0, vec![]));
        write_keyboard_models(
            &dir,
            "1.json",
            &KeyboardModels {
                keyboard_id: 2,
                layers,
            },
        );

        let error = load_model_cache(dir.path()).expect_err("mismatched keyboard id");

        assert!(error.to_string().contains("does not match its filename"));
    }

    #[test]
    fn load_model_cache_rejects_a_layer_that_does_not_match_its_map_key() {
        let dir = TempDir::new().expect("temp dir");
        let mut layers = HashMap::new();
        layers.insert(0, overlay_model(1, vec![]));
        write_keyboard_models(
            &dir,
            "1.json",
            &KeyboardModels {
                keyboard_id: 1,
                layers,
            },
        );

        let error = load_model_cache(dir.path()).expect_err("mismatched layer key");

        assert!(error.to_string().contains("does not match its key"));
    }

    #[test]
    fn load_model_cache_rejects_an_unsupported_model_version() {
        let dir = TempDir::new().expect("temp dir");
        let mut model = overlay_model(0, vec![]);
        model.version = 99;
        let mut layers = HashMap::new();
        layers.insert(0, model);
        write_keyboard_models(
            &dir,
            "1.json",
            &KeyboardModels {
                keyboard_id: 1,
                layers,
            },
        );

        let error = load_model_cache(dir.path()).expect_err("unsupported version");

        assert!(
            error
                .to_string()
                .contains("Unsupported overlay model version")
        );
    }

    /// Each system is asked where its own per-user data belongs.
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_default_asset_directory_sits_under_home() {
        assert_eq!(
            resolve_default_asset_dir(None, Some(OsString::from("/home/user")))
                .expect("HOME is enough on its own"),
            PathBuf::from("/home/user/.cache/keymap-overlay")
        );
        assert!(resolve_default_asset_dir(None, None).is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_default_asset_directory_follows_the_windows_convention() {
        assert_eq!(
            resolve_default_asset_dir(Some(OsString::from(r"C:\Users\user\AppData\Local")), None)
                .expect("LOCALAPPDATA is enough on its own"),
            PathBuf::from(r"C:\Users\user\AppData\Local").join("keymap-overlay")
        );
        assert_eq!(
            resolve_default_asset_dir(None, Some(OsString::from(r"C:\Users\user")))
                .expect("USERPROFILE is enough on its own"),
            PathBuf::from(r"C:\Users\user")
                .join("AppData/Local")
                .join("keymap-overlay")
        );
        assert!(resolve_default_asset_dir(None, None).is_err());
    }

    fn parse(arguments: &[&str]) -> Result<Arguments, clap::Error> {
        Arguments::try_parse_from(
            std::iter::once("keymap-overlay").chain(arguments.iter().copied()),
        )
    }

    #[test]
    fn license_flags_select_the_embedded_notices() {
        assert_eq!(parse(&["--license"]).expect("flag").notice(), Some(LICENSE));
        assert_eq!(
            parse(&["--third-party-licenses"]).expect("flag").notice(),
            Some(THIRD_PARTY_LICENSES)
        );
    }

    /// One letter from `--license`, and it would answer that typo with 168 KiB
    /// of HTML, so the short spelling must stay unrecognised.
    #[test]
    fn the_third_party_notice_has_no_one_letter_spelling() {
        assert!(parse(&["--licenses"]).is_err());
    }

    #[test]
    fn an_asset_directory_runs_the_overlay() {
        let arguments =
            parse(&["--asset-dir", "/somewhere/else"]).expect("a path is a valid command line");

        assert_eq!(arguments.notice(), None);
        assert_eq!(arguments.asset_dir, Some(PathBuf::from("/somewhere/else")));
    }

    /// A bare path used to be accepted positionally, which turned a mistyped
    /// option into a directory the overlay would fail to read much later.
    #[test]
    fn a_bare_path_is_not_an_asset_directory() {
        assert!(parse(&["/somewhere/else"]).is_err());
    }

    /// The service definitions pass the directory explicitly, but running the
    /// overlay by hand should not require repeating the default.
    #[test]
    fn the_asset_directory_may_be_omitted() {
        let arguments = parse(&[]).expect("no arguments is a valid command line");

        assert_eq!(arguments.notice(), None);
        assert_eq!(arguments.asset_dir, None);
    }

    /// A notice does not read the models, so pairing it with a directory is a
    /// mistake worth reporting rather than quietly ignoring.
    #[test]
    fn notice_flags_take_no_asset_directory() {
        assert!(parse(&["--license", "/somewhere/else"]).is_err());
        assert!(parse(&["/somewhere/else", "--third-party-licenses"]).is_err());
    }

    /// Without this a mistyped flag becomes an asset path, and the overlay
    /// fails with "no such file or directory" instead of naming the option.
    #[test]
    fn an_unknown_option_is_rejected_rather_than_opened_as_a_path() {
        assert!(parse(&["--versoin"]).is_err());
        assert!(parse(&["--license-text"]).is_err());
    }

    #[test]
    fn only_one_asset_directory_is_accepted() {
        assert!(parse(&["--asset-dir", "/first", "--asset-dir", "/second"]).is_err());
    }

    /// Guards the `include_str!` paths: a wrong one fails the build, but an
    /// empty or truncated notice would not.
    #[test]
    fn the_embedded_notices_carry_their_terms() {
        assert!(LICENSE.contains("MIT License"));
        assert!(LICENSE.contains("GPL-2.0-or-later"));
        assert!(THIRD_PARTY_LICENSES.contains("keymap-overlay third-party licenses"));
    }

    /// The systemd unit passes no `--log-out`, which is how journald ends up
    /// owning the log instead of the in-process rotator.
    #[test]
    fn without_log_out_the_log_stays_on_stderr() {
        let arguments = parse(&["--asset-dir", "/x"]).expect("no log flag is valid");

        assert!(matches!(
            arguments.log_destination(),
            LogDestination::Stderr
        ));
    }

    /// The launchd plist passes one, because launchd redirects stderr to a file
    /// it never rotates.
    #[test]
    fn log_out_names_the_file_to_rotate() {
        let arguments = parse(&[
            "--asset-dir",
            "/assets",
            "--log-out",
            "/var/log/overlay.log",
        ])
        .expect("a path and a log file are valid together");

        assert_eq!(arguments.asset_dir, Some(PathBuf::from("/assets")));
        assert!(matches!(
            arguments.log_destination(),
            LogDestination::File(path) if path == Path::new("/var/log/overlay.log")
        ));
    }

    /// A notice exits before logging starts, so pairing the two is a mistake.
    #[test]
    fn log_out_cannot_be_combined_with_a_notice() {
        assert!(parse(&["--license", "--log-out", "/var/log/overlay.log"]).is_err());
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

    #[cfg(target_os = "windows")]
    #[test]
    fn the_default_log_file_follows_the_windows_convention() {
        assert_eq!(
            resolve_default_log_file(
                Some(OsString::from(r"C:\Users\user\AppData\Local")),
                Some(OsString::from(r"C:\Users\user"))
            )
            .expect("LOCALAPPDATA is enough on its own"),
            PathBuf::from(r"C:\Users\user\AppData\Local").join("keymap-overlay/logs/overlay.log")
        );
    }

    /// A process started from the Run key can inherit almost no environment.
    #[cfg(target_os = "windows")]
    #[test]
    fn the_default_log_file_falls_back_to_the_profile() {
        assert_eq!(
            resolve_default_log_file(None, Some(OsString::from(r"C:\Users\user")))
                .expect("USERPROFILE is enough on its own"),
            PathBuf::from(r"C:\Users\user")
                .join("AppData/Local")
                .join("keymap-overlay/logs/overlay.log")
        );
        assert!(resolve_default_log_file(None, None).is_err());
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_default_log_file_sits_under_home() {
        assert_eq!(
            resolve_default_log_file(None, Some(OsString::from("/home/user")))
                .expect("HOME is enough on its own"),
            PathBuf::from("/home/user/.local/var/log/keymap-overlay/overlay.log")
        );
        assert!(resolve_default_log_file(None, None).is_err());
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
