#[cfg(target_os = "linux")]
mod daemon;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    keymap_overlay_runtime::run_overlay(daemon::run)
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("keymap-overlay-linux-daemon is only available on Linux");
    std::process::exit(1);
}
