#[cfg(not(target_os = "windows"))]
fn main() -> anyhow::Result<()> {
    keymap_overlay::run_native_overlay()
}

#[cfg(target_os = "windows")]
fn main() {
    eprintln!("The Windows frontend is built from windows/KeymapOverlay.Wpf");
}
