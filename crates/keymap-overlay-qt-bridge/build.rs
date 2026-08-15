use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }

    let qt_quick = pkg_config::Config::new()
        .probe("Qt6Quick")
        .expect("Qt 6 Quick development files are required on Linux");
    let qt_dbus = pkg_config::Config::new()
        .probe("Qt6DBus")
        .expect("Qt 6 D-Bus development files are required on Linux");
    let output_directory = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    generate_moc(&output_directory);

    let mut build = cxx_build::bridge("src/lib.rs");
    build
        .file("src/qt_backend.cpp")
        .include(".")
        .include(&output_directory)
        .includes(qt_quick.include_paths)
        .includes(qt_dbus.include_paths)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-Wsfinae-incomplete=0")
        .compile("keymap-overlay-qt");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/qt_backend.cpp");
    println!("cargo:rerun-if-changed=src/qt_backend.h");
}

fn generate_moc(output_directory: &Path) {
    let output = output_directory.join("qt_backend.moc");
    let status = Command::new(moc_executable())
        .args(["src/qt_backend.cpp", "-o"])
        .arg(&output)
        .status()
        .expect("Failed to run Qt's moc executable");
    assert!(status.success(), "Qt's moc executable failed");
}

fn moc_executable() -> PathBuf {
    if let Some(configured) = env::var_os("QT_MOC_EXECUTABLE") {
        return configured.into();
    }
    for variable in ["host_bins", "libexecdir"] {
        if let Ok(directory) = pkg_config::get_variable("Qt6Core", variable) {
            let candidate = PathBuf::from(directory).join("moc");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from("moc")
}
