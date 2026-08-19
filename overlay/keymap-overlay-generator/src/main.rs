mod custom_keycodes;
mod device;
mod labels;
mod model;
mod types;

use anyhow::{Context, Result};
use clap::Parser;
use hidapi::HidApi;
use labels::Platform;
use std::io::Write;
use std::path::PathBuf;
use types::{KeyboardConfig, KeyboardJson};

/// Reads a connected VIAL device and prints its installed overlay display
/// model (one keyboard, every layer) as JSON to stdout.
#[derive(Parser)]
#[command(name = "keymap-overlay-generator")]
struct Args {
    #[arg(long, value_name = "PATH")]
    keyboard_json: PathBuf,

    #[arg(long, value_name = "PATH")]
    keyboard_config: PathBuf,

    #[arg(long, value_name = "NAME")]
    layout_name: String,

    #[arg(long, value_name = "0-255")]
    keyboard_id: u8,

    #[arg(long, value_enum, default_value_t = Platform::Macos)]
    platform: Platform,

    #[arg(long, default_value_t = 64)]
    pixels_per_unit: i64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let keyboard: KeyboardJson = read_json(&args.keyboard_json)?;
    let config: KeyboardConfig = read_json(&args.keyboard_config)?;

    let api = HidApi::new().context("Failed to initialize HID API")?;
    let dev = device::open_device(&api, &keyboard)?;

    let models = device::read_keyboard_models(
        &dev,
        &keyboard,
        &config,
        &args.layout_name,
        args.keyboard_id,
        args.platform,
        args.pixels_per_unit,
    )?;

    let json = serde_json::to_string(&models)?;
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(json.as_bytes())?;
    lock.write_all(b"\n")?;
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))
}
