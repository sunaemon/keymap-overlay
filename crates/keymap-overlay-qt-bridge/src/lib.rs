//! Safe public boundary around the native Qt/C++ Linux overlay.

#[cfg(target_os = "linux")]
#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("src/qt_backend.h");

        fn run_qt_overlay(event_fd: i32) -> Result<()>;
    }
}

/// Runs Qt's main loop and owns `event_fd` until the window exits.
#[cfg(target_os = "linux")]
pub fn run_qt_overlay(event_fd: i32) -> Result<(), cxx::Exception> {
    ffi::run_qt_overlay(event_fd)
}
