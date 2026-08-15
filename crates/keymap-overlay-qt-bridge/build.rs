fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }

    let qt = pkg_config::Config::new()
        .probe("Qt6Quick")
        .expect("Qt 6 Quick development files are required on Linux");
    let mut build = cxx_build::bridge("src/lib.rs");
    build
        .file("src/qt_backend.cpp")
        .include(".")
        .includes(qt.include_paths)
        .flag_if_supported("-std=c++17")
        .compile("keymap-overlay-qt");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/qt_backend.cpp");
    println!("cargo:rerun-if-changed=src/qt_backend.h");
}
