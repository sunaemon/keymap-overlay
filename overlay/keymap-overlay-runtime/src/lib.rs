use anyhow::{Context, Result};
// Re-exported so a frontend can parse the shared command line without taking a
// clap dependency of its own.
pub use clap::Parser;
use hidapi::{DeviceInfo, HidApi, HidDevice};
use keymap_core::{
    ActiveLayerChange, ActiveLayerState, PendingLayerChange, carries_report_magic,
    parse_raw_layer_event,
};
pub use keymap_core::{LayerEvent, RawLayerEvent};
pub use keymap_overlay_generator::types::{DisplayEncoder, DisplayKey, OverlayModel};
use keymap_overlay_generator::{StartupLayerEvent, contract::simulation_models};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

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

/// User-controlled overlay presentation settings shared by every frontend.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct OverlayPreferences {
    #[serde(rename = "enabled", skip_serializing)]
    legacy_enabled: Option<bool>,
    pub position: OverlayPosition,
    pub opacity_percent: u8,
    pub scale_percent: u16,
}

impl Default for OverlayPreferences {
    fn default() -> Self {
        Self {
            legacy_enabled: None,
            position: OverlayPosition::Center,
            opacity_percent: 100,
            scale_percent: 100,
        }
    }
}

/// Vertical placement of the overlay on its active display.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayPosition {
    Top,
    #[default]
    Center,
    Bottom,
}

impl OverlayPreferences {
    pub const OPACITY_CHOICES: [u8; 4] = [50, 75, 90, 100];
    pub const SCALE_CHOICES: [u16; 4] = [75, 100, 125, 150];

    /// Reads preferences, returning defaults when they have not been created.
    pub fn load() -> Result<Self> {
        let path = preferences_file()?;
        match fs::read(&path) {
            Ok(contents) => {
                let preferences: Self = serde_json::from_slice(&contents)
                    .with_context(|| format!("Failed to parse preferences {}", path.display()))?;
                preferences.validate()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => {
                Err(error).with_context(|| format!("Failed to read preferences {}", path.display()))
            }
        }
    }

    /// Persists preferences for the next process start.
    pub fn save(self) -> Result<()> {
        let preferences = self.validate()?;
        let path = preferences_file()?;
        let directory = path
            .parent()
            .context("The preferences path has no parent")?;
        fs::create_dir_all(directory).with_context(|| {
            format!(
                "Failed to create preferences directory {}",
                directory.display()
            )
        })?;
        let contents = serde_json::to_vec_pretty(&preferences)
            .context("Failed to serialize overlay preferences")?;
        write_file_atomically(&path, &contents)
            .with_context(|| format!("Failed to write preferences {}", path.display()))
    }

    fn validate(mut self) -> Result<Self> {
        anyhow::ensure!(
            Self::OPACITY_CHOICES.contains(&self.opacity_percent),
            "Overlay opacity must be one of {:?}",
            Self::OPACITY_CHOICES
        );
        anyhow::ensure!(
            Self::SCALE_CHOICES.contains(&self.scale_percent),
            "Overlay scale must be one of {:?}",
            Self::SCALE_CHOICES
        );
        self.legacy_enabled = None;
        Ok(self)
    }
}

fn write_file_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("The preferences path has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(contents)?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Location of the shared per-user preference file.
pub fn preferences_file() -> Result<PathBuf> {
    if let Some(path) = env::var_os("KEYMAP_OVERLAY_PREFERENCES_FILE") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    {
        windows_local_app_data(env::var_os("LOCALAPPDATA"), home_directory())
            .map(|root| root.join("keymap-overlay/preferences.json"))
    }
    #[cfg(target_os = "macos")]
    {
        home_directory()
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/keymap-overlay/preferences.json"))
            .context("No home directory is set")
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
            return Ok(PathBuf::from(root).join("keymap-overlay/preferences.json"));
        }
        home_directory()
            .map(PathBuf::from)
            .map(|home| home.join(".config/keymap-overlay/preferences.json"))
            .context("No home directory is set")
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod desktop_tray {
    use super::{OverlayPosition, OverlayPreferences};
    use anyhow::{Context, Result};
    #[cfg(target_os = "windows")]
    use tray_icon::menu::{CheckMenuItem, Submenu};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    const SETTINGS: &str = "settings";
    #[cfg(target_os = "windows")]
    const LAUNCH_AT_LOGIN: &str = "launch-at-login";
    #[cfg(target_os = "windows")]
    const POSITION_TOP: &str = "position-top";
    #[cfg(target_os = "windows")]
    const POSITION_CENTER: &str = "position-center";
    #[cfg(target_os = "windows")]
    const POSITION_BOTTOM: &str = "position-bottom";
    #[cfg(target_os = "windows")]
    const OPACITY_PREFIX: &str = "opacity-";
    #[cfg(target_os = "windows")]
    const SCALE_PREFIX: &str = "scale-";
    const RELOAD: &str = "reload";
    const QUIT: &str = "quit";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum TrayCommand {
        OpenSettings,
        ToggleLaunchAtLogin,
        SetPosition(OverlayPosition),
        SetOpacity(u8),
        SetScale(u16),
        Reload,
        Quit,
    }

    pub struct DesktopTray {
        _tray: TrayIcon,
        #[cfg(target_os = "windows")]
        launch_at_login: CheckMenuItem,
        #[cfg(target_os = "windows")]
        positions: [(OverlayPosition, CheckMenuItem); 3],
        #[cfg(target_os = "windows")]
        opacities: [(u8, CheckMenuItem); 4],
        #[cfg(target_os = "windows")]
        scales: [(u16, CheckMenuItem); 4],
    }

    impl DesktopTray {
        pub fn new(
            preferences: OverlayPreferences,
            launch_at_login_enabled: bool,
            handler: impl Fn(TrayCommand) + Send + Sync + 'static,
        ) -> Result<Self> {
            let reload = MenuItem::with_id(RELOAD, "Reload Keyboards", true, None);
            let version = MenuItem::new(
                format!("Keymap Overlay {}", env!("CARGO_PKG_VERSION")),
                false,
                None,
            );
            let quit = MenuItem::with_id(QUIT, "Quit", true, None);
            let separator = PredefinedMenuItem::separator();
            #[cfg(target_os = "macos")]
            let menu = {
                let settings = MenuItem::with_id(SETTINGS, "Settings…", true, None);
                Menu::with_items(&[&settings, &separator, &reload, &version, &quit])?
            };
            #[cfg(target_os = "windows")]
            let (menu, launch_at_login, positions, opacities, scales) = {
                let launch_at_login = CheckMenuItem::with_id(
                    LAUNCH_AT_LOGIN,
                    "Launch at Login",
                    true,
                    launch_at_login_enabled,
                    None,
                );
                let positions = [
                    position_item(OverlayPosition::Top, "Top", preferences),
                    position_item(OverlayPosition::Center, "Center", preferences),
                    position_item(OverlayPosition::Bottom, "Bottom", preferences),
                ];
                let position_menu = percentage_menu("Position", &positions)?;
                let opacities = OverlayPreferences::OPACITY_CHOICES.map(|value| {
                    (
                        value,
                        CheckMenuItem::with_id(
                            format!("{OPACITY_PREFIX}{value}"),
                            format!("{value}%"),
                            true,
                            preferences.opacity_percent == value,
                            None,
                        ),
                    )
                });
                let opacity_menu = percentage_menu("Opacity", &opacities)?;
                let scales = OverlayPreferences::SCALE_CHOICES.map(|value| {
                    (
                        value,
                        CheckMenuItem::with_id(
                            format!("{SCALE_PREFIX}{value}"),
                            format!("{value}%"),
                            true,
                            preferences.scale_percent == value,
                            None,
                        ),
                    )
                });
                let scale_menu = percentage_menu("Scale", &scales)?;
                let menu = Menu::with_items(&[
                    &launch_at_login,
                    &position_menu,
                    &opacity_menu,
                    &scale_menu,
                    &separator,
                    &reload,
                    &version,
                    &quit,
                ])?;
                (menu, launch_at_login, positions, opacities, scales)
            };
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let Some(command) = command_for_id(event.id().as_ref()) else {
                    return;
                };
                handler(command);
            }));
            let tray = TrayIconBuilder::new()
                .with_tooltip("Keymap Overlay")
                .with_icon(tray_icon()?)
                .with_icon_as_template(cfg!(target_os = "macos"))
                .with_menu(Box::new(menu))
                .build()
                .context("Failed to create the system tray icon")?;
            let mut tray = Self {
                _tray: tray,
                #[cfg(target_os = "windows")]
                launch_at_login,
                #[cfg(target_os = "windows")]
                positions,
                #[cfg(target_os = "windows")]
                opacities,
                #[cfg(target_os = "windows")]
                scales,
            };
            tray.sync(preferences, launch_at_login_enabled);
            Ok(tray)
        }

        pub fn sync(&mut self, preferences: OverlayPreferences, launch_at_login: bool) {
            #[cfg(target_os = "windows")]
            {
                self.launch_at_login.set_checked(launch_at_login);
                for (value, item) in &self.positions {
                    item.set_checked(*value == preferences.position);
                }
                for (value, item) in &self.opacities {
                    item.set_checked(*value == preferences.opacity_percent);
                }
                for (value, item) in &self.scales {
                    item.set_checked(*value == preferences.scale_percent);
                }
            }
            #[cfg(target_os = "macos")]
            let _ = (preferences, launch_at_login);
        }
    }

    #[cfg(target_os = "windows")]
    fn position_item(
        position: OverlayPosition,
        label: &str,
        preferences: OverlayPreferences,
    ) -> (OverlayPosition, CheckMenuItem) {
        let id = match position {
            OverlayPosition::Top => POSITION_TOP,
            OverlayPosition::Center => POSITION_CENTER,
            OverlayPosition::Bottom => POSITION_BOTTOM,
        };
        (
            position,
            CheckMenuItem::with_id(id, label, true, preferences.position == position, None),
        )
    }

    #[cfg(target_os = "windows")]
    fn percentage_menu<T>(label: &str, items: &[(T, CheckMenuItem)]) -> Result<Submenu> {
        Ok(Submenu::with_items(
            label,
            true,
            &items
                .iter()
                .map(|(_, item)| item as &dyn tray_icon::menu::IsMenuItem)
                .collect::<Vec<_>>(),
        )?)
    }

    fn command_for_id(id: &str) -> Option<TrayCommand> {
        match id {
            SETTINGS => Some(TrayCommand::OpenSettings),
            #[cfg(target_os = "windows")]
            LAUNCH_AT_LOGIN => Some(TrayCommand::ToggleLaunchAtLogin),
            #[cfg(target_os = "windows")]
            POSITION_TOP => Some(TrayCommand::SetPosition(OverlayPosition::Top)),
            #[cfg(target_os = "windows")]
            POSITION_CENTER => Some(TrayCommand::SetPosition(OverlayPosition::Center)),
            #[cfg(target_os = "windows")]
            POSITION_BOTTOM => Some(TrayCommand::SetPosition(OverlayPosition::Bottom)),
            RELOAD => Some(TrayCommand::Reload),
            QUIT => Some(TrayCommand::Quit),
            #[cfg(target_os = "macos")]
            _ => None,
            #[cfg(target_os = "windows")]
            _ => id
                .strip_prefix(OPACITY_PREFIX)
                .and_then(|value| value.parse().ok())
                .map(TrayCommand::SetOpacity)
                .or_else(|| {
                    id.strip_prefix(SCALE_PREFIX)
                        .and_then(|value| value.parse().ok())
                        .map(TrayCommand::SetScale)
                }),
        }
    }

    fn tray_icon() -> Result<Icon> {
        const SIDE: u32 = 20;
        let mut rgba = vec![0_u8; (SIDE * SIDE * 4) as usize];
        for y in 2..18 {
            for x in 1..19 {
                let pixel = ((y * SIDE + x) * 4) as usize;
                rgba[pixel..pixel + 4].copy_from_slice(&[99, 72, 180, 255]);
            }
        }
        for row in 0..2 {
            for column in 0..3 {
                for y in (5 + row * 6)..(9 + row * 6) {
                    for x in (3 + column * 6)..(7 + column * 6) {
                        let pixel = ((y * SIDE + x) * 4) as usize;
                        rgba[pixel..pixel + 4].copy_from_slice(&[255, 255, 255, 255]);
                    }
                }
            }
        }
        Icon::from_rgba(rgba, SIDE, SIDE).context("Failed to create the tray icon")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn common_tray_commands_have_stable_ids() {
            assert_eq!(command_for_id(SETTINGS), Some(TrayCommand::OpenSettings));
            assert_eq!(command_for_id(RELOAD), Some(TrayCommand::Reload));
            assert_eq!(command_for_id(QUIT), Some(TrayCommand::Quit));
            assert_eq!(command_for_id("unknown"), None);
        }

        #[cfg(target_os = "windows")]
        #[test]
        fn windows_tray_commands_parse_their_ids() {
            assert_eq!(
                command_for_id(LAUNCH_AT_LOGIN),
                Some(TrayCommand::ToggleLaunchAtLogin)
            );
            assert_eq!(
                command_for_id(POSITION_TOP),
                Some(TrayCommand::SetPosition(OverlayPosition::Top))
            );
            assert_eq!(
                command_for_id(POSITION_CENTER),
                Some(TrayCommand::SetPosition(OverlayPosition::Center))
            );
            assert_eq!(
                command_for_id(POSITION_BOTTOM),
                Some(TrayCommand::SetPosition(OverlayPosition::Bottom))
            );
            assert_eq!(
                command_for_id("opacity-75"),
                Some(TrayCommand::SetOpacity(75))
            );
            assert_eq!(
                command_for_id("scale-125"),
                Some(TrayCommand::SetScale(125))
            );
            assert_eq!(command_for_id("opacity-invalid"), None);
        }

        #[test]
        fn tray_icon_pixels_form_a_valid_native_icon() {
            assert!(tray_icon().is_ok());
        }
    }
}

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
        let keyboard_id = keyboard_id
            .parse()
            .map_err(|_| "keyboard ID must be an integer from 0 to 255".to_owned())?;
        let layer = layer
            .parse()
            .map_err(|_| "layer must be an integer from 1 to 255".to_owned())?;
        if layer == 0 {
            return Err("layer must be an integer from 1 to 255".to_owned());
        }
        Ok(Self { keyboard_id, layer })
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
    let mut models = ModelCache::new();
    let mut raw_hid_devices = Vec::new();
    for connected in keymap_overlay_generator::read_connected_keyboard_models(host_platform())? {
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

fn host_platform() -> keymap_overlay_generator::labels::Platform {
    #[cfg(target_os = "macos")]
    return keymap_overlay_generator::labels::Platform::Macos;
    #[cfg(target_os = "linux")]
    return keymap_overlay_generator::labels::Platform::Linux;
    #[cfg(target_os = "windows")]
    return keymap_overlay_generator::labels::Platform::Windows;
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
/// or a Windows component sender — and this is the seam between them.
///
/// Cloneable because each device gets its own reader thread.
pub trait LayerEventSink: Clone + Send {
    /// Returns whether the receiving end is still there; a reader stops once
    /// it is not.
    fn send(&self, event: LayerEvent) -> bool;
}

pub type ModelCache = HashMap<(u8, u8), OverlayModel>;

/// Thread-safe in-memory models shared by arrival readers and the frontend.
#[derive(Clone)]
pub struct ModelStore {
    models: Arc<RwLock<ModelCache>>,
}

impl PartialEq for ModelStore {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.models, &other.models)
    }
}

impl Eq for ModelStore {}

impl ModelStore {
    /// Creates a shared store from models loaded during startup.
    pub fn new(models: ModelCache) -> Self {
        Self {
            models: Arc::new(RwLock::new(models)),
        }
    }

    /// Composes the visible model for one keyboard's active layer stack.
    pub fn compose(&self, keyboard_id: u8, layers: &[u8]) -> Option<OverlayModel> {
        let models = self
            .models
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        compose_model(&models, keyboard_id, layers)
    }

    /// Lists keyboard and layer pairs available for an embedded preview.
    pub fn preview_choices(&self) -> Vec<(u8, u8)> {
        let models = self
            .models
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut choices = models.keys().copied().collect::<Vec<_>>();
        choices.sort_unstable();
        choices
    }

    fn contains_keyboard(&self, keyboard_id: u8) -> bool {
        self.models
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&(keyboard_id, 0))
    }

    fn add_keyboard(&self, generated: keymap_overlay_generator::types::KeyboardModels) -> bool {
        let keyboard_id = generated.keyboard_id;
        let mut models = self
            .models
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if models.contains_key(&(keyboard_id, 0)) {
            return false;
        }
        models.extend(
            generated
                .layers
                .into_iter()
                .map(|(layer, model)| ((keyboard_id, layer), model)),
        );
        drop(models);
        info!("Loaded overlay model for newly connected keyboard {keyboard_id}");
        true
    }

    fn remove_keyboard(&self, keyboard_id: u8) {
        self.models
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|(cached_keyboard_id, _), _| *cached_keyboard_id != keyboard_id);
    }
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
    reload_generation: Arc<AtomicU64>,
    reload_requested: Arc<AtomicBool>,
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

    /// Drops active readers and rereads every connected keyboard in-process.
    pub fn reload_keyboards(&self) -> bool {
        match self {
            Self::RawHid(listener) => listener.reload_keyboards(),
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
    models: ModelStore,
) -> LayerEventSourceHandle {
    let Some(simulated) = simulated else {
        return LayerEventSourceHandle::RawHid(spawn_raw_hid_listener(
            sink,
            startup_devices,
            models,
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

    /// Requests a fresh Vial model read for every connected keyboard.
    pub fn reload_keyboards(&self) -> bool {
        self.reload_generation.fetch_add(1, Ordering::AcqRel);
        self.reload_requested.store(true, Ordering::Release);
        self.requester.request()
    }
}

pub fn spawn_raw_hid_listener(
    sink: impl LayerEventSink + 'static,
    startup_devices: Vec<StartupRawHidDevice>,
    models: ModelStore,
) -> RawHidListenerHandle {
    let (wake, requests) = mpsc::channel();
    let requester = EnumerationRequester::new(wake);
    let reload_generation = Arc::new(AtomicU64::new(0));
    let reload_requested = Arc::new(AtomicBool::new(false));
    let handle = RawHidListenerHandle {
        requester: requester.clone(),
        reload_generation: Arc::clone(&reload_generation),
        reload_requested: Arc::clone(&reload_requested),
    };
    thread::spawn(move || {
        let context = RawHidContext {
            sink,
            active_paths: Arc::new(Mutex::new(HashSet::new())),
            active_keyboard_ids: Arc::new(Mutex::new(HashSet::new())),
            models,
            requester,
            reload_generation,
            reload_requested,
        };
        adopt_startup_raw_hid_devices(startup_devices, &context);
        enumerate_raw_hid_devices(&context);
        loop {
            if requests.recv().is_err() {
                return;
            }
            // Give a newly announced keyboard time to become openable. Existing
            // readers remain alive and cannot lose releases during this grace.
            thread::sleep(RECONNECT_INTERVAL);
            context.requester.begin_enumeration();
            if context.reload_requested.swap(false, Ordering::AcqRel) {
                wait_for_raw_hid_readers(&context.active_paths);
            }
            enumerate_raw_hid_devices(&context);
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
/// `HOME`. A non-native `HOME` such as `/home/user` is not an absolute Windows
/// path, so the runtime ignores it and uses `USERPROFILE` when both are set.
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

struct RawHidContext<S> {
    sink: S,
    active_paths: Arc<Mutex<HashSet<String>>>,
    active_keyboard_ids: Arc<Mutex<HashSet<u8>>>,
    models: ModelStore,
    requester: EnumerationRequester,
    reload_generation: Arc<AtomicU64>,
    reload_requested: Arc<AtomicBool>,
}

#[derive(Debug, Eq, PartialEq)]
enum DiscoveryResult {
    Opened,
    Skipped,
    Retry,
}

impl<S: Clone> Clone for RawHidContext<S> {
    fn clone(&self) -> Self {
        Self {
            sink: self.sink.clone(),
            active_paths: Arc::clone(&self.active_paths),
            active_keyboard_ids: Arc::clone(&self.active_keyboard_ids),
            models: self.models.clone(),
            requester: self.requester.clone(),
            reload_generation: Arc::clone(&self.reload_generation),
            reload_requested: Arc::clone(&self.reload_requested),
        }
    }
}

fn adopt_startup_raw_hid_devices<S: LayerEventSink + 'static>(
    mut startup_devices: Vec<StartupRawHidDevice>,
    context: &RawHidContext<S>,
) {
    for startup in &startup_devices {
        register_active_raw_hid_device(
            &context.active_paths,
            &context.active_keyboard_ids,
            &startup.path,
            startup.keyboard_id,
        );
    }
    replay_startup_layer_events(
        &context.sink,
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
            context.clone(),
        );
    }
    if opened > 0 {
        info!("Adopted {opened} startup Raw HID device(s)");
    }
}

fn register_active_raw_hid_device(
    active_paths: &Arc<Mutex<HashSet<String>>>,
    active_keyboard_ids: &Arc<Mutex<HashSet<u8>>>,
    path: &str,
    keyboard_id: u8,
) {
    active_paths
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.to_owned());
    active_keyboard_ids
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(keyboard_id);
}

/// Opens newly discovered Raw HID devices without interrupting active readers.
fn enumerate_raw_hid_devices<S: LayerEventSink + 'static>(context: &RawHidContext<S>) {
    enumerate_raw_hid_devices_with_result(
        HidApi::new().context("Failed to enumerate HID devices"),
        context,
    );
}

fn enumerate_raw_hid_devices_with_result<S: LayerEventSink + 'static>(
    api: Result<HidApi>,
    context: &RawHidContext<S>,
) {
    let api = match api {
        Ok(api) => api,
        Err(error) => {
            warn!("Raw HID enumeration failed: {error:#}");
            context.requester.request();
            return;
        }
    };
    let mut opened = 0;
    let mut retry_needed = false;
    for device_info in api
        .device_list()
        .filter(|device| device.usage_page() == RAW_USAGE_PAGE && device.usage() == RAW_USAGE_ID)
    {
        match discover_raw_hid_device(&api, device_info, context) {
            DiscoveryResult::Opened => opened += 1,
            DiscoveryResult::Retry => retry_needed = true,
            DiscoveryResult::Skipped => {}
        }
    }
    if opened > 0 {
        info!("Listening on {opened} new Raw HID device(s)");
    }
    if retry_needed {
        context.requester.request();
    }
}

fn discover_raw_hid_device<S: LayerEventSink + 'static>(
    api: &HidApi,
    device_info: &DeviceInfo,
    context: &RawHidContext<S>,
) -> DiscoveryResult {
    let path = device_info.path().to_string_lossy().into_owned();
    if context
        .active_paths
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&path)
    {
        return DiscoveryResult::Skipped;
    }
    discover_opened_raw_hid_device(
        path,
        device_info.vendor_id(),
        device_info.product_id(),
        device_info.open_device(api),
        context,
    )
}

fn discover_opened_raw_hid_device<S: LayerEventSink + 'static>(
    path: String,
    vendor_id: u16,
    product_id: u16,
    device: std::result::Result<HidDevice, hidapi::HidError>,
    context: &RawHidContext<S>,
) -> DiscoveryResult {
    let device = match device {
        Ok(device) => device,
        Err(error) => {
            warn!(
                "Failed to open Raw HID device {:04x}:{:04x}: {error}",
                vendor_id, product_id
            );
            return DiscoveryResult::Retry;
        }
    };
    context
        .active_paths
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.clone());
    match start_arriving_raw_hid_reader(device, path.clone(), context) {
        Ok(true) => DiscoveryResult::Opened,
        Ok(false) => DiscoveryResult::Skipped,
        Err(error) => {
            warn!("Failed to read newly connected Vial device {path:?}: {error:#}");
            context
                .active_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&path);
            DiscoveryResult::Retry
        }
    }
}

fn start_arriving_raw_hid_reader<S: LayerEventSink + 'static>(
    device: HidDevice,
    path: String,
    context: &RawHidContext<S>,
) -> Result<bool> {
    let mut connected = keymap_overlay_generator::read_connected_keyboard_model(
        device,
        path.clone(),
        host_platform(),
    )?;
    let Some(generated) = connected.models.take() else {
        info!("Ignoring Raw HID device without keymap overlay metadata");
        spawn_raw_hid_reader(connected.device, connected.path, None, context.clone());
        return Ok(true);
    };
    let keyboard_id = generated.keyboard_id;
    if !context
        .active_keyboard_ids
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(keyboard_id)
    {
        warn!(
            "Ignoring newly connected Raw HID device because KEYBOARD_ID {keyboard_id} is already active"
        );
        context
            .active_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&path);
        return Ok(false);
    }
    context.models.add_keyboard(generated);
    replay_startup_layer_events(&context.sink, [(keyboard_id, connected.layer_events)]);
    spawn_raw_hid_reader(
        connected.device,
        connected.path,
        Some(keyboard_id),
        context.clone(),
    );
    Ok(true)
}

fn spawn_raw_hid_reader<S: LayerEventSink + 'static>(
    device: HidDevice,
    path: String,
    keyboard_id: Option<u8>,
    context: RawHidContext<S>,
) {
    let reload_generation = context.reload_generation.load(Ordering::Acquire);
    // HidDevice is Send but not Sync, so each reader owns its device.
    thread::spawn(move || {
        if let Err(error) = receive_from_device(
            &device,
            &path,
            keyboard_id,
            &context.models,
            &context.sink,
            reload_generation,
            &context.reload_generation,
        ) {
            warn!("Raw HID reader stopped: {error:#}");
        }
        context
            .active_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&path);
        if let Some(keyboard_id) = keyboard_id {
            context
                .active_keyboard_ids
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&keyboard_id);
            context.models.remove_keyboard(keyboard_id);
        }
        context.requester.request();
    });
}

fn receive_from_device(
    device: &HidDevice,
    path: &str,
    mut keyboard_id: Option<u8>,
    models: &ModelStore,
    sink: &impl LayerEventSink,
    reader_generation: u64,
    reload_generation: &AtomicU64,
) -> Result<()> {
    let mut report = [0_u8; 33];
    let mut warned_keyboard_ids = HashSet::new();
    loop {
        if reload_generation.load(Ordering::Acquire) != reader_generation {
            sink.send(LayerEvent::Disconnected { keyboard_id });
            return Ok(());
        }
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
        if !layer_event_matches_model(models, keyboard_id, event.keyboard_id) {
            if warned_keyboard_ids.insert(event.keyboard_id) {
                warn!(
                    "Ignoring layer events for keyboard {} because this HID device has no matching model",
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

fn wait_for_raw_hid_readers(active_paths: &Mutex<HashSet<String>>) {
    let deadline = Instant::now() + Duration::from_millis((READ_TIMEOUT as u64) * 2);
    while !active_paths
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .is_empty()
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(10));
    }
}

fn layer_event_matches_model(
    models: &ModelStore,
    keyboard_id: Option<u8>,
    event_keyboard_id: u8,
) -> bool {
    models.contains_keyboard(event_keyboard_id)
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

    fn generated_fixture(
        keyboard_id: u8,
        layer: u8,
    ) -> keymap_overlay_generator::types::KeyboardModels {
        let layers = simulation_models(keyboard_id, layer)
            .expect("simulation fixture is valid")
            .into_iter()
            .map(|((_, layer), model)| (layer, model))
            .collect();
        keymap_overlay_generator::types::KeyboardModels {
            keyboard_id,
            layers,
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
    fn a_keyboard_reload_stops_readers_and_requests_enumeration() {
        let (sender, receiver) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(7));
        let reload_requested = Arc::new(AtomicBool::new(false));
        let listener = RawHidListenerHandle {
            requester: EnumerationRequester::new(sender),
            reload_generation: Arc::clone(&generation),
            reload_requested: Arc::clone(&reload_requested),
        };

        assert!(listener.reload_keyboards());
        assert_eq!(generation.load(Ordering::Acquire), 8);
        assert!(reload_requested.load(Ordering::Acquire));
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
    fn an_arriving_keyboard_becomes_composable_without_replacing_existing_models() {
        let models = ModelStore::new(ModelCache::new());
        let frontend_models = models.clone();

        assert!(models.add_keyboard(generated_fixture(12, 3)));
        assert!(frontend_models.compose(12, &[3]).is_some());
        assert!(!models.add_keyboard(generated_fixture(12, 4)));
        assert!(frontend_models.compose(12, &[3]).is_some());
        assert!(frontend_models.compose(12, &[4]).is_none());
    }

    #[test]
    fn disconnecting_a_keyboard_removes_only_its_shared_models() {
        let mut cache = simulation_models(12, 3).expect("first fixture is valid");
        cache.extend(simulation_models(13, 2).expect("second fixture is valid"));
        let models = ModelStore::new(cache);
        let frontend_models = models.clone();

        models.remove_keyboard(12);

        assert!(frontend_models.compose(12, &[3]).is_none());
        assert!(frontend_models.compose(13, &[2]).is_some());
    }

    #[test]
    fn model_store_identity_tracks_the_shared_cache() {
        let models = ModelStore::new(ModelCache::new());

        assert!(models == models.clone());
        assert!(models != ModelStore::new(ModelCache::new()));
    }

    #[test]
    fn preview_choices_are_sorted_by_keyboard_and_layer() {
        let mut cache = simulation_models(13, 2).expect("second fixture is valid");
        cache.extend(simulation_models(12, 3).expect("first fixture is valid"));
        let models = ModelStore::new(cache);

        assert_eq!(
            models.preview_choices(),
            vec![(12, 0), (12, 3), (13, 0), (13, 2)]
        );
    }

    #[test]
    fn startup_devices_register_their_path_and_keyboard_id() {
        let active_paths = Arc::new(Mutex::new(HashSet::new()));
        let active_keyboard_ids = Arc::new(Mutex::new(HashSet::new()));

        register_active_raw_hid_device(&active_paths, &active_keyboard_ids, "fixture", 12);

        assert_eq!(
            *active_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            HashSet::from(["fixture".to_owned()])
        );
        assert_eq!(
            *active_keyboard_ids
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            HashSet::from([12])
        );
    }

    #[test]
    fn host_platform_matches_the_compilation_target() {
        #[cfg(target_os = "macos")]
        assert!(matches!(
            host_platform(),
            keymap_overlay_generator::labels::Platform::Macos
        ));
        #[cfg(target_os = "linux")]
        assert!(matches!(
            host_platform(),
            keymap_overlay_generator::labels::Platform::Linux
        ));
        #[cfg(target_os = "windows")]
        assert!(matches!(
            host_platform(),
            keymap_overlay_generator::labels::Platform::Windows
        ));
    }

    #[test]
    fn enumeration_failures_request_a_retry() {
        let (event_sender, _event_receiver) = mpsc::channel();
        let (request_sender, request_receiver) = mpsc::channel();
        let context = RawHidContext {
            sink: ChannelSink(event_sender),
            active_paths: Arc::new(Mutex::new(HashSet::new())),
            active_keyboard_ids: Arc::new(Mutex::new(HashSet::new())),
            models: ModelStore::new(ModelCache::new()),
            requester: EnumerationRequester::new(request_sender),
            reload_generation: Arc::new(AtomicU64::new(0)),
            reload_requested: Arc::new(AtomicBool::new(false)),
        };

        enumerate_raw_hid_devices_with_result(Err(anyhow::anyhow!("fixture failure")), &context);

        assert_eq!(request_receiver.try_recv(), Ok(()));
    }

    #[test]
    fn devices_that_cannot_be_opened_are_retried() {
        let (event_sender, _event_receiver) = mpsc::channel();
        let (request_sender, _request_receiver) = mpsc::channel();
        let active_paths = Arc::new(Mutex::new(HashSet::new()));
        let context = RawHidContext {
            sink: ChannelSink(event_sender),
            active_paths: Arc::clone(&active_paths),
            active_keyboard_ids: Arc::new(Mutex::new(HashSet::new())),
            models: ModelStore::new(ModelCache::new()),
            requester: EnumerationRequester::new(request_sender),
            reload_generation: Arc::new(AtomicU64::new(0)),
            reload_requested: Arc::new(AtomicBool::new(false)),
        };

        let result = discover_opened_raw_hid_device(
            "fixture".to_owned(),
            0xfeed,
            0x0001,
            Err(hidapi::HidError::HidApiError {
                message: "fixture failure".to_owned(),
            }),
            &context,
        );

        assert_eq!(result, DiscoveryResult::Retry);
        assert!(
            active_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
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
            ModelStore::new(ModelCache::new()),
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
    fn a_simulated_source_ignores_hardware_refresh_requests() {
        let source = LayerEventSourceHandle::Simulated;

        assert!(!source.device_arrived());
        assert!(!source.reload_keyboards());
        assert!(!source.uses_raw_hid());
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
    fn live_layer_events_require_a_matching_model_and_device() {
        let mut models = simulation_models(2, 1).expect("first fixture is valid");
        models.extend(simulation_models(3, 1).expect("second fixture is valid"));
        let models = ModelStore::new(models);

        assert!(layer_event_matches_model(&models, None, 2));
        assert!(layer_event_matches_model(&models, Some(2), 2));
        assert!(!layer_event_matches_model(&models, None, 9));
        assert!(!layer_event_matches_model(&models, Some(2), 3));
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
        assert!(parse(&["--simulate", "1:0"]).is_err());
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

    #[test]
    fn overlay_preferences_have_stable_defaults() {
        assert_eq!(
            OverlayPreferences::default(),
            OverlayPreferences {
                legacy_enabled: None,
                position: OverlayPosition::Center,
                opacity_percent: 100,
                scale_percent: 100,
            }
        );
    }

    #[test]
    fn overlay_preferences_ignore_the_removed_enabled_setting() {
        let preferences: OverlayPreferences = serde_json::from_str(
            r#"{"enabled":false,"position":"bottom","opacity_percent":75,"scale_percent":125}"#,
        )
        .expect("legacy preferences parse");

        assert_eq!(
            preferences.validate().expect("legacy preferences validate"),
            OverlayPreferences {
                position: OverlayPosition::Bottom,
                opacity_percent: 75,
                scale_percent: 125,
                ..OverlayPreferences::default()
            }
        );
    }

    #[test]
    fn overlay_preferences_reject_values_the_menus_cannot_represent() {
        assert!(
            OverlayPreferences {
                opacity_percent: 42,
                ..OverlayPreferences::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            OverlayPreferences {
                scale_percent: 101,
                ..OverlayPreferences::default()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn atomic_write_replaces_contents_without_leaving_a_temporary_file() {
        let directory = TempDir::new().expect("temporary directory is available");
        let path = directory.path().join("preferences.json");
        fs::write(&path, b"old").expect("fixture can be written");

        write_file_atomically(&path, b"new").expect("atomic write succeeds");

        assert_eq!(fs::read(&path).expect("preferences can be read"), b"new");
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("directory can be read")
                .count(),
            1
        );
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
