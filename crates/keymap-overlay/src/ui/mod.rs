//! The overlay window, which is the one part that cannot be shared.
//!
//! Each backend owns the main thread and its own event loop, and starts the
//! shared Raw HID listener once it has a way to be woken from another thread.
//! Everything the two have in common — the protocol, the transitions, the
//! images, the log — lives in `main.rs`.
//!
//! macOS and Windows have one window each; Linux has two, and `linux.rs`
//! chooses between them. The two eframe windows keep only what is specific to
//! their system, over the shared `eframe_common.rs`.

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod eframe_common;

#[cfg(target_os = "macos")]
mod eframe_window;
#[cfg(target_os = "macos")]
pub(crate) use eframe_window::run;

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
