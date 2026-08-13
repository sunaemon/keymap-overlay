// Without this, Windows gives the login autostart process a console window that flashes on
// screen at every start and lingers behind the overlay. The subsystem also
// discards stdout and stderr, which costs nothing here: everything this binary
// reports goes through the rotating log below.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod hotplug;
mod ui;

use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};
use image::RgbaImage;
use keymap_core::{
    ActiveLayerChange, RawLayerEvent, carries_report_magic, parse_raw_layer_event, transition_for,
    transition_for_disconnect,
};
use log::{error, info, warn};
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

fn main() -> Result<()> {
    initialize_logging()?;

    if let Err(error) = run() {
        error!("Keymap overlay stopped: {error:#}");
        return Err(error);
    }
    Ok(())
}

fn run() -> Result<()> {
    let assets_dir = assets_dir()?;
    ui::run(assets_dir)
}

/// Where the Raw HID listener delivers the events it reads.
///
/// The listener runs on its own thread while the platform backend owns the main
/// one, so delivering an event also has to wake whatever loop that backend
/// runs. Each does it differently — `request_repaint` on macOS, a calloop
/// channel for the Wayland window, an event-loop proxy for the X11 one — and
/// this is the seam between them.
///
/// Cloneable because each device gets its own reader thread, the same way the
/// channel sender used to be cloned.
pub(crate) trait LayerEventSink: Clone + Send {
    /// Returns whether the receiving end is still there; a reader stops once
    /// it is not.
    fn send(&self, event: ListenerEvent) -> bool;
}

/// An event from the HID listener, including loss of the device itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListenerEvent {
    Layer(RawLayerEvent),
    Disconnected { keyboard_id: Option<u8> },
}

pub(crate) fn spawn_raw_hid_listener(sink: impl LayerEventSink + 'static) {
    let session = hotplug::RunningSession::default();
    hotplug::spawn_watcher(session.clone());
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
}

/// What a report should do to the overlay, given the held momentary layers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Transition {
    Show {
        keyboard_id: u8,
        layer: u8,
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
/// It accumulates rather than folding an iterator because that is the shape both
/// kinds of backend can use. The eframe windows drain an mpsc channel when their
/// callback runs; the Wayland and X11 windows are handed one event at a time by
/// calloop and by the winit event-loop proxy, with nothing to drain.
#[derive(Default)]
pub(crate) struct PendingTransition {
    held_keys: Vec<(u8, u8)>,
    transition: Transition,
}

impl PendingTransition {
    /// Folds one event in, keeping the latest transition that changes anything.
    pub(crate) fn push(&mut self, event: ListenerEvent) {
        let transition = transition_for_event(&mut self.held_keys, event);
        if transition != Transition::Ignore {
            self.transition = transition;
        }
    }

    /// Takes what the window should do now, leaving nothing pending behind.
    pub(crate) fn take(&mut self) -> Transition {
        std::mem::take(&mut self.transition)
    }
}

pub(crate) fn transition_for_event(
    held_keys: &mut Vec<(u8, u8)>,
    event: ListenerEvent,
) -> Transition {
    let change = match event {
        ListenerEvent::Layer(event) => transition_for(held_keys, event),
        ListenerEvent::Disconnected { keyboard_id } => {
            transition_for_disconnect(held_keys, keyboard_id)
        }
    };
    match change {
        ActiveLayerChange::Unchanged => Transition::Ignore,
        ActiveLayerChange::Changed(Some((keyboard_id, layer))) => {
            Transition::Show { keyboard_id, layer }
        }
        ActiveLayerChange::Changed(None) => Transition::Hide,
    }
}

pub(crate) fn image_path(assets_dir: &Path, keyboard_id: u8, layer: u8) -> PathBuf {
    assets_dir.join(format!("{keyboard_id}_L{layer}.png"))
}

/// Decodes a layer image as RGBA8 with unassociated alpha, which is what egui
/// takes directly and what the two Linux windows premultiply before blitting.
pub(crate) fn load_image(path: &Path) -> Result<RgbaImage> {
    let image = image::open(path)
        .with_context(|| format!("Failed to open {}", path.display()))?
        .into_rgba8();
    Ok(image)
}

pub(crate) type ImageCache = HashMap<(u8, u8), Arc<RgbaImage>>;

/// Loads every installed layer image before the listener can show one.
pub(crate) fn load_image_cache(assets_dir: &Path) -> Result<ImageCache> {
    let mut images = HashMap::new();
    for entry in fs::read_dir(assets_dir)
        .with_context(|| format!("Failed to read asset directory {}", assets_dir.display()))?
    {
        let entry = entry
            .with_context(|| format!("Failed to read an entry in {}", assets_dir.display()))?;
        let path = entry.path();
        let Some(key) = image_key(&path) else {
            continue;
        };
        match load_image(&path) {
            Ok(image) => {
                images.insert(key, Arc::new(image));
            }
            Err(error) => warn!(
                "Failed to preload overlay image {}: {error:#}",
                path.display()
            ),
        }
    }
    Ok(images)
}

fn image_key(path: &Path) -> Option<(u8, u8)> {
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        return None;
    }
    let (keyboard_id, layer) = path.file_stem()?.to_str()?.split_once("_L")?;
    Some((keyboard_id.parse().ok()?, layer.parse().ok()?))
}

/// Scales a colour channel by its alpha, which is how both Wayland's
/// ARGB8888 and X11's 32-bit visual expect the channels of a blended pixel.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn premultiply(value: u8, alpha: u8) -> u8 {
    ((u16::from(value) * u16::from(alpha)) / 255) as u8
}

fn initialize_logging() -> Result<()> {
    let log_directory = resolve_log_directory(env::var_os(LOG_DIRECTORY_ENV), home_directory())?;
    fs::create_dir_all(&log_directory)
        .with_context(|| format!("Failed to create log directory {}", log_directory.display()))?;
    let writer = RotatingLogWriter::new(log_directory.join("overlay.log"))?;

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .target(env_logger::Target::Pipe(Box::new(writer)))
        .init();
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

fn assets_dir() -> Result<PathBuf> {
    // args_os, not args: the latter panics on a non-UTF-8 argument, and an
    // asset path handed to us on the command line is an arbitrary byte string.
    resolve_assets_dir(env::args_os().nth(1), home_directory())
}

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
                layer: 2,
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
                layer: 2,
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
                layer: 2,
            }
        );
    }

    #[test]
    fn images_are_named_after_the_keyboard_and_layer() {
        assert_eq!(
            image_path(Path::new("/assets"), 2, 3),
            PathBuf::from("/assets/2_L3.png")
        );
    }

    #[test]
    fn installed_image_names_are_parsed_for_preloading() {
        assert_eq!(image_key(Path::new("12_L3.png")), Some((12, 3)));
        assert_eq!(image_key(Path::new("12_L3.PNG")), Some((12, 3)));
        assert_eq!(image_key(Path::new("overlay.log")), None);
        assert_eq!(image_key(Path::new("keyboard_Llayer.png")), None);
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
