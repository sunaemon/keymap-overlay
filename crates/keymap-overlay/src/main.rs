mod hotplug;
mod ui;

use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};
use image::RgbaImage;
use keymap_core::{RawLayerEvent, carries_report_magic, parse_raw_layer_event};
use log::{error, info, warn};
use std::env;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

const RAW_USAGE_PAGE: u16 = 0xFF60;
const RAW_USAGE_ID: u16 = 0x61;
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
    fn send(&self, event: RawLayerEvent) -> bool;
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Transition {
    Show { keyboard_id: u8, layer: u8 },
    Hide,
    Ignore,
}

/// The overlay shows the most recently pressed held layer. Releasing that
/// layer restores the next-most-recent one still held, while releasing a
/// different layer leaves the visible overlay alone.
pub(crate) fn transition_for(held_keys: &mut Vec<(u8, u8)>, event: RawLayerEvent) -> Transition {
    let key = (event.keyboard_id, event.layer);
    if event.pressed {
        held_keys.retain(|held_key| *held_key != key);
        held_keys.push(key);
        Transition::Show {
            keyboard_id: event.keyboard_id,
            layer: event.layer,
        }
    } else {
        let Some(index) = held_keys.iter().position(|held_key| *held_key == key) else {
            return Transition::Ignore;
        };
        let was_visible = index == held_keys.len() - 1;
        held_keys.remove(index);
        if !was_visible {
            Transition::Ignore
        } else if let Some((keyboard_id, layer)) = held_keys.last() {
            Transition::Show {
                keyboard_id: *keyboard_id,
                layer: *layer,
            }
        } else {
            Transition::Hide
        }
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

/// Scales a colour channel by its alpha, which is how both Wayland's
/// ARGB8888 and X11's 32-bit visual expect the channels of a blended pixel.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn premultiply(value: u8, alpha: u8) -> u8 {
    ((u16::from(value) * u16::from(alpha)) / 255) as u8
}

fn initialize_logging() -> Result<()> {
    let log_directory = resolve_log_directory(env::var_os(LOG_DIRECTORY_ENV), env::var_os("HOME"))?;
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
        .context("Neither KEYMAP_OVERLAY_LOG_DIR nor HOME is set")
}

fn assets_dir() -> Result<PathBuf> {
    // args_os, not args: the latter panics on a non-UTF-8 argument, and an
    // asset path handed to us on the command line is an arbitrary byte string.
    resolve_assets_dir(env::args_os().nth(1), env::var_os("HOME"))
}

fn resolve_assets_dir(argument: Option<OsString>, home: Option<OsString>) -> Result<PathBuf> {
    if let Some(path) = argument {
        return Ok(PathBuf::from(path));
    }

    let home = home.context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/keymap-overlay"))
}

/// Reads from every connected Raw HID device until one of them disconnects, or
/// another one appears, then returns so the caller can enumerate again.
fn run_raw_hid_session(
    sink: &impl LayerEventSink,
    session: &hotplug::RunningSession,
) -> Result<()> {
    let api = HidApi::new().context("Failed to enumerate HID devices")?;
    let devices: Vec<HidDevice> = api
        .device_list()
        .filter(|device| device.usage_page() == RAW_USAGE_PAGE && device.usage() == RAW_USAGE_ID)
        .filter_map(|device_info| match device_info.open_device(&api) {
            Ok(device) => Some(device),
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
    thread::scope(|scope| {
        // HidDevice is Send but not Sync, so each reader owns its device.
        for device in devices {
            let sink = sink.clone();
            let cancelled = Arc::clone(&cancelled);
            scope.spawn(move || {
                receive_from_device(&device, &sink, &cancelled);
                // Any disconnect ends the session so all devices are reopened.
                cancelled.store(true, Ordering::Relaxed);
            });
        }
    });
    session.detach();
    Ok(())
}

fn receive_from_device(device: &HidDevice, sink: &impl LayerEventSink, cancelled: &AtomicBool) {
    let mut report = [0_u8; 33];
    while !cancelled.load(Ordering::Relaxed) {
        let length = match device.read_timeout(&mut report, READ_TIMEOUT) {
            Ok(length) => length,
            Err(error) => {
                warn!("Failed to read Raw HID report: {error}");
                return;
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
        if !sink.send(event) {
            return;
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

    fn event(keyboard_id: u8, layer: u8, pressed: bool) -> RawLayerEvent {
        RawLayerEvent {
            keyboard_id,
            layer,
            pressed,
        }
    }

    #[test]
    fn a_press_shows_its_layer() {
        assert_eq!(
            transition_for(&mut vec![], event(1, 2, true)),
            Transition::Show {
                keyboard_id: 1,
                layer: 2
            }
        );
    }

    #[test]
    fn a_press_replaces_the_layer_already_on_screen() {
        assert_eq!(
            transition_for(&mut vec![(1, 2)], event(1, 3, true)),
            Transition::Show {
                keyboard_id: 1,
                layer: 3
            }
        );
    }

    #[test]
    fn releasing_the_layer_on_screen_hides_it() {
        assert_eq!(
            transition_for(&mut vec![(1, 2)], event(1, 2, false)),
            Transition::Hide
        );
    }

    #[test]
    fn releasing_a_layer_that_is_not_on_screen_is_ignored() {
        // Layer 3 is visible; releasing the earlier layer 2 key must not
        // replace it, but must still remove it from the held state.
        let mut held_keys = vec![(1, 2), (1, 3)];
        assert_eq!(
            transition_for(&mut held_keys, event(1, 2, false)),
            Transition::Ignore
        );
        assert_eq!(held_keys, vec![(1, 3)]);
        // The key is (keyboard, layer), so the same layer number released on a
        // different keyboard does not match what is on screen.
        assert_eq!(
            transition_for(&mut vec![(1, 2)], event(2, 2, false)),
            Transition::Ignore
        );
        // A release with nothing on screen happens after a failed image load.
        assert_eq!(
            transition_for(&mut vec![], event(1, 2, false)),
            Transition::Ignore
        );
    }

    #[test]
    fn releasing_the_latest_layer_restores_the_previous_held_layer() {
        let mut held_keys = vec![(1, 2), (1, 3)];

        assert_eq!(
            transition_for(&mut held_keys, event(1, 3, false)),
            Transition::Show {
                keyboard_id: 1,
                layer: 2,
            }
        );
        assert_eq!(held_keys, vec![(1, 2)]);
    }

    #[test]
    fn images_are_named_after_the_keyboard_and_layer() {
        assert_eq!(
            image_path(Path::new("/assets"), 2, 3),
            PathBuf::from("/assets/2_L3.png")
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
