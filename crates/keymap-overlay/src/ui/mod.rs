//! The overlay window, which is the one part that cannot be shared.
//!
//! Each backend owns the main thread and its own event loop, and starts the
//! shared Raw HID listener once it has a way to be woken from another thread.
//! Everything the backends have in common — the protocol, the transitions,
//! and the log — lives in `main.rs`.
//!
//! macOS builds native AppKit views from JSON and Linux builds native Qt Quick
//! items from the same model. Windows owns its process in the WPF frontend and
//! reaches the shared listener through the sibling bridge crate.

#[cfg(target_os = "macos")]
mod appkit;
#[cfg(target_os = "macos")]
pub(crate) use appkit::run;

#[cfg(target_os = "linux")]
mod qt;
#[cfg(target_os = "linux")]
pub(crate) use qt::run;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("the Rust executable has a window backend for macOS and Linux");
