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
pub use keymap_overlay_generator::types::{DisplayEncoder, DisplayKey, OverlayModel};
use keymap_overlay_generator::{StartupLayerEvent, contract::simulation_models};
use log::{error, info, warn};
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
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
const SIMULATED_PRESS_DURATION: Duration = Duration::from_secs(2);
const SIMULATED_RELEASE_DURATION: Duration = Duration::from_secs(1);

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
    /// Write the log to this file, rotating it, instead of to stderr
    #[arg(long, value_name = "PATH")]
    pub log_out: Option<PathBuf>,

    /// Repeatedly simulate holding KEYBOARD_ID:LAYER instead of reading HID
    #[arg(long, value_name = "KEYBOARD_ID:LAYER")]
    pub simulate: Option<SimulatedLayer>,

    /// Print this project's own licence terms
    #[arg(long, exclusive = true)]
    pub license: bool,

    /// Print the third-party notices, as HTML
    #[arg(long, exclusive = true)]
    pub third_party_licenses: bool,
}

/// One keyboard and momentary layer to exercise without HID hardware.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SimulatedLayer {
    pub keyboard_id: u8,
    pub layer: u8,
}

impl FromStr for SimulatedLayer {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (keyboard_id, layer) = value
            .split_once(':')
            .ok_or_else(|| "expected KEYBOARD_ID:LAYER".to_owned())?;
        Ok(Self {
            keyboard_id: keyboard_id
                .parse()
                .map_err(|_| "keyboard ID must be an integer from 0 to 255".to_owned())?,
            layer: layer
                .parse()
                .map_err(|_| "layer must be an integer from 0 to 255".to_owned())?,
        })
    }
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

/// Initializes the shared runtime and gives live in-memory models to a frontend.
pub fn run_overlay(
    frontend: impl FnOnce(StartupModels, Option<SimulatedLayer>) -> Result<()>,
) -> Result<()> {
    let arguments = Arguments::parse();
    if let Some(notice) = arguments.notice() {
        return write_notice(notice);
    }
    let simulated = arguments.simulate;
    initialize_logging(arguments.log_destination())?;
    let model_fixture = simulated.or_else(|| {
        env::var("KEYMAP_OVERLAY_E2E_MODEL")
            .ok()
            .and_then(|value| value.parse().ok())
    });
    let models = startup_models(model_fixture)?;

    if let Err(error) = frontend(models, simulated) {
        error!("Keymap overlay stopped: {error:#}");
        return Err(error);
    }
    Ok(())
}

/// Models and layer reports collected before a frontend starts its live listener.
pub struct StartupModels {
    pub models: ModelCache,
    pub raw_hid_devices: Vec<StartupRawHidDevice>,
}

/// An already-open device the live listener adopts after startup model reads.
pub struct StartupRawHidDevice {
    device: HidDevice,
    path: String,
    keyboard_id: u8,
    layer_events: Vec<StartupLayerEvent>,
}

/// Returns live Vial models, or an in-memory fixture for simulation mode.
pub fn startup_models(simulated: Option<SimulatedLayer>) -> Result<StartupModels> {
    Ok(match simulated {
        Some(simulated) => StartupModels {
            models: simulation_models(simulated.keyboard_id, simulated.layer)?,
            raw_hid_devices: Vec::new(),
        },
        None => load_live_models()?,
    })
}

/// Reads every connected keyboard and retains layer reports interleaved with Vial responses.
fn load_live_models() -> Result<StartupModels> {
    #[cfg(target_os = "macos")]
    let platform = keymap_overlay_generator::labels::Platform::Macos;
    #[cfg(target_os = "linux")]
    let platform = keymap_overlay_generator::labels::Platform::Linux;
    #[cfg(target_os = "windows")]
    let platform = keymap_overlay_generator::labels::Platform::Windows;

    let mut models = ModelCache::new();
    let mut raw_hid_devices = Vec::new();
    for connected in keymap_overlay_generator::read_connected_keyboard_models(platform)? {
        let keymap_overlay_generator::ConnectedKeyboard {
            models: generated,
            device,
            path,
            layer_events,
        } = connected;
        let keyboard_id = generated.keyboard_id;
        if generated
            .layers
            .keys()
            .any(|layer| models.contains_key(&(keyboard_id, *layer)))
        {
            anyhow::bail!("More than one connected keyboard uses KEYBOARD_ID {keyboard_id}");
        }
        models.extend(
            generated
                .layers
                .into_iter()
                .map(|(layer, model)| ((keyboard_id, layer), model)),
        );
        raw_hid_devices.push(StartupRawHidDevice {
            device,
            path,
            keyboard_id,
            layer_events,
        });
    }
    Ok(StartupModels {
        models,
        raw_hid_devices,
    })
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

pub type ModelCache = HashMap<(u8, u8), OverlayModel>;

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

/// A hardware listener, or a synthetic event source used for manual testing.
#[derive(Clone)]
pub enum LayerEventSourceHandle {
    RawHid(RawHidListenerHandle),
    Simulated,
}

impl LayerEventSourceHandle {
    /// Requests hardware enumeration; simulation mode has no devices to scan.
    pub fn device_arrived(&self) -> bool {
        match self {
            Self::RawHid(listener) => listener.device_arrived(),
            Self::Simulated => false,
        }
    }

    /// Returns whether this source needs platform device-arrival notifications.
    pub fn uses_raw_hid(&self) -> bool {
        matches!(self, Self::RawHid(_))
    }
}

/// Starts either the real Raw HID listener or a repeating synthetic key hold.
pub fn spawn_layer_event_source(
    sink: impl LayerEventSink + 'static,
    simulated: Option<SimulatedLayer>,
    startup_devices: Vec<StartupRawHidDevice>,
    modeled_keyboard_ids: impl IntoIterator<Item = u8>,
) -> LayerEventSourceHandle {
    let Some(simulated) = simulated else {
        return LayerEventSourceHandle::RawHid(spawn_raw_hid_listener(
            sink,
            startup_devices,
            modeled_keyboard_ids.into_iter().collect(),
        ));
    };
    thread::spawn(move || {
        info!(
            "Simulating layer events: keyboard={} layer={}",
            simulated.keyboard_id, simulated.layer
        );
        loop {
            if !sink.send(LayerEvent::Report(RawLayerEvent {
                keyboard_id: simulated.keyboard_id,
                layer: simulated.layer,
                pressed: true,
            })) {
                return;
            }
            thread::sleep(SIMULATED_PRESS_DURATION);
            if !sink.send(LayerEvent::Report(RawLayerEvent {
                keyboard_id: simulated.keyboard_id,
                layer: simulated.layer,
                pressed: false,
            })) {
                return;
            }
            thread::sleep(SIMULATED_RELEASE_DURATION);
        }
    });
    LayerEventSourceHandle::Simulated
}

fn replay_startup_layer_events(
    sink: &impl LayerEventSink,
    device_events: impl IntoIterator<Item = (u8, Vec<StartupLayerEvent>)>,
) {
    let mut events = device_events
        .into_iter()
        .flat_map(|(keyboard_id, events)| events.into_iter().map(move |event| (keyboard_id, event)))
        .collect::<Vec<_>>();
    events.sort_by_key(|(_, event)| event.sequence);

    for (keyboard_id, startup_event) in events {
        let event = startup_event.event;
        if event.keyboard_id != keyboard_id {
            warn!(
                "Ignoring startup layer event for keyboard {} from device model {}",
                event.keyboard_id, keyboard_id
            );
            continue;
        }
        if !sink.send(LayerEvent::Report(event)) {
            return;
        }
    }
}

impl RawHidListenerHandle {
    /// Requests enumeration after an arrival, returning whether one was queued.
    pub fn device_arrived(&self) -> bool {
        self.requester.request()
    }
}

pub fn spawn_raw_hid_listener(
    sink: impl LayerEventSink + 'static,
    startup_devices: Vec<StartupRawHidDevice>,
    modeled_keyboard_ids: HashSet<u8>,
) -> RawHidListenerHandle {
    let modeled_keyboard_ids = Arc::new(modeled_keyboard_ids);
    let (wake, requests) = mpsc::channel();
    let requester = EnumerationRequester::new(wake);
    let handle = RawHidListenerHandle {
        requester: requester.clone(),
    };
    thread::spawn(move || {
        let active_paths = Arc::new(Mutex::new(HashSet::new()));
        adopt_startup_raw_hid_devices(
            startup_devices,
            &sink,
            &active_paths,
            &modeled_keyboard_ids,
            &requester,
        );
        enumerate_raw_hid_devices(&sink, &active_paths, &modeled_keyboard_ids, &requester);
        loop {
            if requests.recv().is_err() {
                return;
            }
            // Give a newly announced keyboard time to become openable. Existing
            // readers remain alive and cannot lose releases during this grace.
            thread::sleep(RECONNECT_INTERVAL);
            requester.begin_enumeration();
            enumerate_raw_hid_devices(&sink, &active_paths, &modeled_keyboard_ids, &requester);
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

fn adopt_startup_raw_hid_devices<S: LayerEventSink + 'static>(
    mut startup_devices: Vec<StartupRawHidDevice>,
    sink: &S,
    active_paths: &Arc<Mutex<HashSet<String>>>,
    modeled_keyboard_ids: &Arc<HashSet<u8>>,
    requester: &EnumerationRequester,
) {
    for startup in &startup_devices {
        active_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(startup.path.clone());
    }
    replay_startup_layer_events(
        sink,
        startup_devices.iter_mut().map(|startup| {
            (
                startup.keyboard_id,
                std::mem::take(&mut startup.layer_events),
            )
        }),
    );
    let opened = startup_devices.len();
    for startup in startup_devices {
        spawn_raw_hid_reader(
            startup.device,
            startup.path,
            Some(startup.keyboard_id),
            sink.clone(),
            Arc::clone(active_paths),
            Arc::clone(modeled_keyboard_ids),
            requester.clone(),
        );
    }
    if opened > 0 {
        info!("Adopted {opened} startup Raw HID device(s)");
    }
}

/// Opens newly discovered Raw HID devices without interrupting active readers.
fn enumerate_raw_hid_devices<S: LayerEventSink + 'static>(
    sink: &S,
    active_paths: &Arc<Mutex<HashSet<String>>>,
    modeled_keyboard_ids: &Arc<HashSet<u8>>,
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
        spawn_raw_hid_reader(
            device,
            path,
            None,
            sink.clone(),
            Arc::clone(active_paths),
            Arc::clone(modeled_keyboard_ids),
            requester.clone(),
        );
    }
    if opened > 0 {
        info!("Listening on {opened} new Raw HID device(s)");
    }
    if retry_needed {
        requester.request();
    }
}

fn spawn_raw_hid_reader(
    device: HidDevice,
    path: String,
    keyboard_id: Option<u8>,
    sink: impl LayerEventSink + 'static,
    active_paths: Arc<Mutex<HashSet<String>>>,
    modeled_keyboard_ids: Arc<HashSet<u8>>,
    requester: EnumerationRequester,
) {
    // HidDevice is Send but not Sync, so each reader owns its device.
    thread::spawn(move || {
        if let Err(error) =
            receive_from_device(&device, &path, keyboard_id, &modeled_keyboard_ids, &sink)
        {
            warn!("Raw HID reader stopped: {error:#}");
        }
        active_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&path);
        requester.request();
    });
}

fn receive_from_device(
    device: &HidDevice,
    path: &str,
    mut keyboard_id: Option<u8>,
    modeled_keyboard_ids: &HashSet<u8>,
    sink: &impl LayerEventSink,
) -> Result<()> {
    let mut report = [0_u8; 33];
    let mut warned_keyboard_ids = HashSet::new();
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
        if !layer_event_matches_startup(modeled_keyboard_ids, keyboard_id, event.keyboard_id) {
            if warned_keyboard_ids.insert(event.keyboard_id) {
                warn!(
                    "Ignoring layer events for keyboard {} because this HID device has no matching startup model",
                    event.keyboard_id
                );
            }
            continue;
        }
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

fn layer_event_matches_startup(
    modeled_keyboard_ids: &HashSet<u8>,
    keyboard_id: Option<u8>,
    event_keyboard_id: u8,
) -> bool {
    modeled_keyboard_ids.contains(&event_keyboard_id)
        && keyboard_id.is_none_or(|expected| expected == event_keyboard_id)
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

    #[derive(Clone)]
    struct ChannelSink(mpsc::Sender<LayerEvent>);

    impl LayerEventSink for ChannelSink {
        fn send(&self, event: LayerEvent) -> bool {
            self.0.send(event).is_ok()
        }
    }

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
    fn simulated_startup_uses_only_in_memory_models() {
        let simulated = SimulatedLayer {
            keyboard_id: 12,
            layer: 3,
        };

        let startup = startup_models(Some(simulated)).expect("simulation startup succeeds");

        assert!(startup.raw_hid_devices.is_empty());
        assert_eq!(
            startup.models.keys().copied().collect::<HashSet<_>>(),
            HashSet::from([(12, 0), (12, 3)])
        );
        assert_eq!(startup.models[&(12, 0)].keys[0].label, ["BASE"]);
        assert_eq!(startup.models[&(12, 3)].keys[0].label, ["E2E"]);
    }

    #[test]
    fn a_simulated_source_immediately_presses_the_requested_layer() {
        let (sender, receiver) = mpsc::channel();
        let source = spawn_layer_event_source(
            ChannelSink(sender),
            Some(SimulatedLayer {
                keyboard_id: 12,
                layer: 3,
            }),
            Vec::new(),
            std::iter::empty(),
        );

        assert!(!source.uses_raw_hid());
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(LayerEvent::Report(RawLayerEvent {
                keyboard_id: 12,
                layer: 3,
                pressed: true,
            }))
        );
    }

    #[test]
    fn startup_layer_events_are_replayed_in_cross_device_order() {
        let (sender, receiver) = mpsc::channel();
        let first_device_event = RawLayerEvent {
            keyboard_id: 2,
            layer: 3,
            pressed: true,
        };
        let second_device_event = RawLayerEvent {
            keyboard_id: 9,
            layer: 2,
            pressed: true,
        };
        let device_events = vec![
            (
                2,
                vec![
                    StartupLayerEvent {
                        sequence: 2,
                        event: first_device_event,
                    },
                    StartupLayerEvent {
                        sequence: 3,
                        event: RawLayerEvent {
                            keyboard_id: 9,
                            layer: 4,
                            pressed: true,
                        },
                    },
                ],
            ),
            (
                9,
                vec![StartupLayerEvent {
                    sequence: 1,
                    event: second_device_event,
                }],
            ),
        ];

        replay_startup_layer_events(&ChannelSink(sender), device_events);

        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            vec![
                LayerEvent::Report(second_device_event),
                LayerEvent::Report(first_device_event),
            ]
        );
    }

    #[test]
    fn startup_layer_events_keep_each_devices_internal_order() {
        let (sender, receiver) = mpsc::channel();
        let events = vec![
            RawLayerEvent {
                keyboard_id: 2,
                layer: 1,
                pressed: true,
            },
            RawLayerEvent {
                keyboard_id: 2,
                layer: 3,
                pressed: true,
            },
        ];

        replay_startup_layer_events(
            &ChannelSink(sender),
            [(
                2,
                events
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(sequence, event)| StartupLayerEvent {
                        sequence: sequence as u64,
                        event,
                    })
                    .collect(),
            )],
        );

        assert_eq!(
            receiver.try_iter().collect::<Vec<_>>(),
            events
                .into_iter()
                .map(LayerEvent::Report)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn live_layer_events_require_a_matching_startup_model_and_device() {
        let modeled_keyboard_ids = HashSet::from([2, 3]);

        assert!(layer_event_matches_startup(&modeled_keyboard_ids, None, 2));
        assert!(layer_event_matches_startup(
            &modeled_keyboard_ids,
            Some(2),
            2
        ));
        assert!(!layer_event_matches_startup(&modeled_keyboard_ids, None, 9));
        assert!(!layer_event_matches_startup(
            &modeled_keyboard_ids,
            Some(2),
            3
        ));
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
    fn a_simulated_layer_identifies_the_keyboard_and_layer() {
        let arguments = parse(&["--simulate", "12:3"]).expect("a simulated layer is valid");

        assert_eq!(
            arguments.simulate,
            Some(SimulatedLayer {
                keyboard_id: 12,
                layer: 3,
            })
        );
    }

    #[test]
    fn a_simulated_layer_rejects_malformed_or_out_of_range_values() {
        assert!(parse(&["--simulate", "1"]).is_err());
        assert!(parse(&["--simulate", "256:2"]).is_err());
        assert!(parse(&["--simulate", "1:256"]).is_err());
        assert!(parse(&["--simulate", "one:two"]).is_err());
    }

    /// A bare path used to be accepted positionally, which turned a mistyped
    /// option into a directory the overlay would fail to read much later.
    #[test]
    fn a_bare_path_is_not_an_asset_directory() {
        assert!(parse(&["/somewhere/else"]).is_err());
    }

    /// Without this a mistyped flag becomes an asset path, and the overlay
    /// fails with "no such file or directory" instead of naming the option.
    #[test]
    fn an_unknown_option_is_rejected_rather_than_opened_as_a_path() {
        assert!(parse(&["--versoin"]).is_err());
        assert!(parse(&["--license-text"]).is_err());
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
        let arguments = parse(&[]).expect("no log flag is valid");

        assert!(matches!(
            arguments.log_destination(),
            LogDestination::Stderr
        ));
    }

    /// The launchd plist passes one, because launchd redirects stderr to a file
    /// it never rotates.
    #[test]
    fn log_out_names_the_file_to_rotate() {
        let arguments = parse(&["--log-out", "/var/log/overlay.log"]).expect("a log file is valid");
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
