use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use keymap_core::ProjectContext;
use log::{debug, info};
use std::env;
use std::path::PathBuf;
use std::process::Command;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Directory containing keyboard configurations
    #[arg(short, long, default_value = "example")]
    keyboards_dir: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show information about a keyboard
    Info {
        #[arg(short, long)]
        keyboard_id: String,
    },
    /// Compile the firmware for a keyboard
    Compile {
        #[arg(short, long)]
        keyboard_id: String,
    },
}

fn handle_info(ctx: &ProjectContext, keyboard_id: &str) -> Result<()> {
    let config = ctx.get_keyboard_config(keyboard_id)?;
    println!("Keyboard ID: {}", keyboard_id);
    println!("QMK Keyboard: {}", config.qmk_keyboard);
    println!(
        "Build Directory: {}",
        ctx.get_build_dir(keyboard_id)?.display()
    );
    Ok(())
}

fn handle_compile(ctx: &ProjectContext, keyboard_id: &str) -> Result<()> {
    let config = ctx.get_keyboard_config(keyboard_id)?;

    println!("Compiling {} ({})...", keyboard_id, config.qmk_keyboard);

    let qmk_build_dir = ctx.get_build_dir(keyboard_id)?.join("qmk_build");
    let qmk_build_dir_str = qmk_build_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Build directory path contains invalid UTF-8 characters"))?;

    debug!("Executing qmk compile via mise...");
    // Thin wrapper calling 'qmk compile' via mise
    let status = Command::new("mise")
        .args([
            "exec",
            "--",
            "qmk",
            "compile",
            "-kb",
            &config.qmk_keyboard,
            "-km",
            "keymap", // Uses the default "keymap" directory name for custom keymaps
            "-e",
            &format!("KEYBOARD_ID={}", keyboard_id),
            "-e",
            &format!("BUILD_DIR={}", qmk_build_dir_str),
        ])
        .status()
        .context("Failed to execute qmk compile")?;

    if !status.success() {
        anyhow::bail!("qmk compile failed with exit code: {:?}", status.code());
    }
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    let current_dir = env::current_dir()?;
    debug!("Current directory: {:?}", current_dir);

    // Resolve keyboards_dir relative to current_dir if it's relative
    let keyboards_dir = if cli.keyboards_dir.is_relative() {
        current_dir.join(cli.keyboards_dir)
    } else {
        cli.keyboards_dir
    };
    debug!("Keyboards directory: {:?}", keyboards_dir);

    let ctx = ProjectContext::new(current_dir, keyboards_dir);

    match cli.command {
        Commands::Info { keyboard_id } => {
            info!("showing information for keyboard: {}", keyboard_id);
            handle_info(&ctx, &keyboard_id)
        }
        Commands::Compile { keyboard_id } => {
            info!("Starting compilation for keyboard: {}", keyboard_id);
            handle_compile(&ctx, &keyboard_id)
        }
    }
}
