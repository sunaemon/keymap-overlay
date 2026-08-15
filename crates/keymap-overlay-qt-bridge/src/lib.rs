//! Safe public boundary around the native Qt/C++ Linux overlay.

/// Runs Qt's main loop and subscribes to renderer state over D-Bus.
#[cfg(target_os = "linux")]
pub fn run_qt_overlay() -> Result<(), cxx::Exception> {
    ffi::run_qt_overlay()
}

#[cfg(target_os = "linux")]
#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("src/qt_backend.h");

        fn run_qt_overlay() -> Result<()>;
    }
}
