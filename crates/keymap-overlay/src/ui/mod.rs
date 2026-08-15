//! The overlay window, which is the one part that cannot be shared.
//!
//! Each backend owns the main thread and its own event loop, and starts the
//! shared Raw HID listener once it has a way to be woken from another thread.
//! Everything the backends have in common — the protocol, the transitions,
//! and the log — lives in `main.rs`.
//!
//! macOS builds native AppKit views from JSON. Windows uses eframe; Linux has
//! two windows, and `linux.rs` chooses between them.

#[cfg(target_os = "windows")]
mod eframe_common;

#[cfg(target_os = "macos")]
mod appkit;
#[cfg(target_os = "macos")]
pub(crate) use appkit::run;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub(crate) use windows::run;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
pub(crate) use linux::run;

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
compile_error!("keymap-overlay has a window backend for macOS, Linux and Windows");
