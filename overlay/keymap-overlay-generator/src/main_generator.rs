use anyhow::{Context, Result};
use clap::Parser;
use hidapi::HidApi;
use keymap_overlay_generator::labels::Platform;
use keymap_overlay_generator::types::{KeyboardConfig, KeyboardJson};
use keymap_overlay_generator::{device, read_json};
use std::io::Write;
use std::path::PathBuf;

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

    #[arg(
        long,
        default_value_t = 64,
        value_parser = clap::value_parser!(i64).range(1..)
    )]
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
