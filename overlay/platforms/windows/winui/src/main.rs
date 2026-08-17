#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
fn main() -> anyhow::Result<()> {
    windows::run()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("keymap-overlay-winui is only available on Windows");
}
