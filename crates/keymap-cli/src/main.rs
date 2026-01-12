use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use keymap_core::ProjectContext;
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let current_dir = env::current_dir()?;

    // Resolve keyboards_dir relative to current_dir if it's relative
    let keyboards_dir = if cli.keyboards_dir.is_relative() {
        current_dir.join(cli.keyboards_dir)
    } else {
        cli.keyboards_dir
    };

    let ctx = ProjectContext::new(current_dir, keyboards_dir);

    match cli.command {
        Commands::Info { keyboard_id } => {
            let config = ctx
                .get_keyboard_config(&keyboard_id)
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("Keyboard ID: {}", keyboard_id);
            println!("QMK Keyboard: {}", config.qmk_keyboard);
            println!(
                "Build Directory: {}",
                ctx.get_build_dir(&keyboard_id)
                    .map_err(|e| anyhow::anyhow!(e))?
                    .display()
            );
        }
        Commands::Compile { keyboard_id } => {
            let config = ctx
                .get_keyboard_config(&keyboard_id)
                .map_err(|e| anyhow::anyhow!(e))?;

            println!("Compiling {} ({})...", keyboard_id, config.qmk_keyboard);

            let qmk_build_dir = ctx
                .get_build_dir(&keyboard_id)
                .map_err(|e| anyhow::anyhow!(e))?
                .join("qmk_build");
            let qmk_build_dir_str = qmk_build_dir.to_str().ok_or_else(|| {
                anyhow::anyhow!("Build directory path contains invalid UTF-8 characters")
            })?;

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
        }
    }

    Ok(())
}
