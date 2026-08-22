use anyhow::{Context, Result};
use log::{info, warn};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const LAYOUT_NAME: &str = "LAYOUT";

#[cfg(target_os = "macos")]
const PLATFORM: &str = "macos";
#[cfg(target_os = "linux")]
const PLATFORM: &str = "linux";
#[cfg(target_os = "windows")]
const PLATFORM: &str = "windows";

/// Generates any `<keyboard_id>.json` missing from `asset_dir`, for every
/// keyboard configured under `keyboard_config_dir`, by shelling out to
/// `keymap-overlay-generator` (a standalone binary installed alongside this
/// one — see its own crate for why it isn't linked in directly). Runs once
/// at startup, ahead of the listener, never on the keypress hot path. Each
/// keyboard is independent and best-effort: one that isn't currently
/// connected is skipped with a warning, not a failure.
pub fn fill_missing_models(asset_dir: &Path, keyboard_config_dir: &Path) -> Result<()> {
    let generator = generator_binary_path()?;
    if !generator.is_file() {
        warn!(
            "Skipping self-heal: {} not found next to this executable",
            generator.display()
        );
        return Ok(());
    }

    let entries = fs::read_dir(keyboard_config_dir).with_context(|| {
        format!(
            "Failed to read keyboard config directory {}",
            keyboard_config_dir.display()
        )
    })?;

    for entry in entries {
        let entry = entry.with_context(|| {
            format!(
                "Failed to read an entry in {}",
                keyboard_config_dir.display()
            )
        })?;
        let path = entry.path();
        let Some(keyboard_id) = keyboard_id_from_dir(&path) else {
            continue;
        };
        let output_path = asset_dir.join(format!("{keyboard_id}.json"));
        if output_path.exists() {
            continue;
        }
        generate_one(&generator, &path, keyboard_id, &output_path);
    }
    Ok(())
}

fn keyboard_id_from_dir(path: &Path) -> Option<u8> {
    if !path.is_dir() || !path.join("config.json").is_file() {
        return None;
    }
    path.file_name()?.to_str()?.parse().ok()
}

fn generate_one(generator: &Path, keyboard_dir: &Path, keyboard_id: u8, output_path: &Path) {
    info!("Generating a missing model for keyboard {keyboard_id}...");
    let result = Command::new(generator)
        .arg("--keyboard-json")
        .arg(keyboard_dir.join("keyboard.json"))
        .arg("--keyboard-config")
        .arg(keyboard_dir.join("config.json"))
        .arg("--layout-name")
        .arg(LAYOUT_NAME)
        .arg("--keyboard-id")
        .arg(keyboard_id.to_string())
        .arg("--platform")
        .arg(PLATFORM)
        .output();

    let output = match result {
        Ok(output) => output,
        Err(error) => {
            warn!("Could not run the model generator for keyboard {keyboard_id}: {error:#}");
            return;
        }
    };
    if !output.status.success() {
        warn!(
            "Model generator failed for keyboard {keyboard_id}, probably not connected: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return;
    }

    let tmp_path = output_path.with_extension("json.tmp");
    let write_result =
        fs::write(&tmp_path, &output.stdout).and_then(|()| fs::rename(&tmp_path, output_path));
    if let Err(error) = write_result {
        warn!("Failed to write the generated model for keyboard {keyboard_id}: {error:#}");
        let _ = fs::remove_file(&tmp_path);
    } else {
        info!("Generated a model for keyboard {keyboard_id}");
    }
}

fn generator_binary_path() -> Result<PathBuf> {
    let mut path = env::current_exe().context("Failed to determine this executable's path")?;
    path.pop();
    #[cfg(target_os = "windows")]
    path.push("keymap-overlay-generator.exe");
    #[cfg(not(target_os = "windows"))]
    path.push("keymap-overlay-generator");
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn keyboard_id_from_dir_requires_a_config_json_and_a_numeric_name() {
        let dir = TempDir::new().expect("temp dir");
        let keyboard = dir.path().join("1");
        fs::create_dir(&keyboard).expect("mkdir");
        fs::write(keyboard.join("config.json"), "{}").expect("write");
        assert_eq!(keyboard_id_from_dir(&keyboard), Some(1));

        let no_config = dir.path().join("2");
        fs::create_dir(&no_config).expect("mkdir");
        assert_eq!(keyboard_id_from_dir(&no_config), None);

        let non_numeric = dir.path().join("not-a-number");
        fs::create_dir(&non_numeric).expect("mkdir");
        fs::write(non_numeric.join("config.json"), "{}").expect("write");
        assert_eq!(keyboard_id_from_dir(&non_numeric), None);
    }

    #[test]
    fn fill_missing_models_skips_a_keyboard_whose_output_already_exists() {
        let config_dir = TempDir::new().expect("temp dir");
        let keyboard = config_dir.path().join("1");
        fs::create_dir(&keyboard).expect("mkdir");
        fs::write(keyboard.join("config.json"), "{}").expect("write");

        let asset_dir = TempDir::new().expect("temp dir");
        fs::write(asset_dir.path().join("1.json"), "{}").expect("write");

        // No generator binary is set up in this test environment, so a
        // keyboard actually needing generation would warn-and-skip; this
        // only asserts the already-satisfied keyboard is never touched.
        fill_missing_models(asset_dir.path(), config_dir.path()).expect("self-heal");
        assert_eq!(
            fs::read_to_string(asset_dir.path().join("1.json")).unwrap(),
            "{}"
        );
    }
}
