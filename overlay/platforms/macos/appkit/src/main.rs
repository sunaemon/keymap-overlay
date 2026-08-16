#[cfg(target_os = "macos")]
mod appkit;

#[cfg(target_os = "macos")]
fn main() -> anyhow::Result<()> {
    keymap_overlay_runtime::run_overlay(appkit::run)
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("keymap-overlay-macos is only available on macOS");
    std::process::exit(1);
}
