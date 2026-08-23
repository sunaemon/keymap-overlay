use anyhow::{Context, Result};
use clap::Parser;
use keymap_overlay_generator::{labels::Platform, read_live_keyboard_models};
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Build a display model from one connected Vial keyboard")]
struct Args {
    #[arg(long)]
    keyboard_json: PathBuf,
    #[arg(long)]
    keyboard_config: PathBuf,
    #[arg(long)]
    layout_name: String,
    #[arg(long)]
    keyboard_id: u8,
    #[arg(long, value_enum)]
    platform: Platform,
    #[arg(long)]
    pixels_per_unit: i64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let models = read_live_keyboard_models(
        &args.keyboard_json,
        &args.keyboard_config,
        &args.layout_name,
        args.keyboard_id,
        args.platform,
        args.pixels_per_unit,
    )?;
    serde_json::to_writer(std::io::stdout(), &models)
        .context("Failed to serialize the keyboard display model")?;
    Ok(())
}
