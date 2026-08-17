#[cfg(target_os = "windows")]
fn main() {
    windows_reactor_setup::as_framework_dependent();
}

#[cfg(not(target_os = "windows"))]
fn main() {}
