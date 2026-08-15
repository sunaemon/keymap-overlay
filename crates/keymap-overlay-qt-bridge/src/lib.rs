//! Safe public boundary around the native Qt/C++ Linux overlay.

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("src/qt_backend.h");

        fn run_qt_overlay(assets_dir: &str, event_fd: i32) -> Result<()>;
    }
}

/// Runs Qt's main loop and owns `event_fd` until the window exits.
pub fn run_qt_overlay(assets_dir: &str, event_fd: i32) -> Result<(), cxx::Exception> {
    ffi::run_qt_overlay(assets_dir, event_fd)
}
