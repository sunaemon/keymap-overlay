use anyhow::{Context, Result};
use clap::Parser;
use hidapi::HidApi;
use keymap_overlay_generator::qmk_keymap::QmkKeymapJson;
use keymap_overlay_generator::types::KeyboardJson;
use keymap_overlay_generator::{device, flash, read_json};
use std::path::PathBuf;

/// Parses keymap.c (by way of a pre-compiled qmk-keymap.json) and writes it
/// to a connected VIAL device's dynamic-keymap EEPROM directly — no
/// read-current-state-and-merge round trip, since only the keymap and
/// encoder bindings are ever touched.
#[derive(Parser)]
#[command(name = "keymap-overlay-flash-keymap")]
struct Args {
    #[arg(long, value_name = "PATH")]
    qmk_keymap_json: PathBuf,

    #[arg(long, value_name = "PATH")]
    keyboard_json: PathBuf,

    #[arg(long, value_name = "PATH")]
    keymap_c: PathBuf,

    #[arg(long, value_name = "NAME")]
    layout_name: String,

    /// Resolve and print what would be written, without touching the device
    #[arg(long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let qmk_keymap: QmkKeymapJson = read_json(&args.qmk_keymap_json)?;
    let keyboard: KeyboardJson = read_json(&args.keyboard_json)?;
    let keymap_c_source = std::fs::read_to_string(&args.keymap_c)
        .with_context(|| format!("Failed to read {}", args.keymap_c.display()))?;

    let api = HidApi::new().context("Failed to initialize HID API")?;
    let dev = device::open_device(&api, &keyboard)?;

    let resolved = flash::resolve_for_device(
        &dev,
        &keyboard,
        &keymap_c_source,
        &qmk_keymap,
        &args.layout_name,
    )?;

    if args.dry_run {
        let json = serde_json::json!({
            "rows": resolved.rows,
            "cols": resolved.cols,
            "vial_version": resolved.vial_version,
            "layer_count": resolved.layer_count,
            "layout": resolved.layout,
            "encoder_layout": resolved.encoder_layout,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        eprintln!("(dry run: device not written)");
        return Ok(());
    }

    flash::write_to_device(&dev, &resolved)?;
    println!("✔ Wrote keymap.c to the device");
    Ok(())
}
