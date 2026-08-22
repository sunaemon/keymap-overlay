use anyhow::{Context, Result};
use log::{info, warn};
use std::fs;
use std::path::Path;

const LAYOUT_NAME: &str = "LAYOUT";

#[cfg(target_os = "macos")]
const PLATFORM: keymap_overlay_generator::labels::Platform =
    keymap_overlay_generator::labels::Platform::Macos;
#[cfg(target_os = "linux")]
const PLATFORM: keymap_overlay_generator::labels::Platform =
    keymap_overlay_generator::labels::Platform::Linux;
#[cfg(target_os = "windows")]
const PLATFORM: keymap_overlay_generator::labels::Platform =
    keymap_overlay_generator::labels::Platform::Windows;

/// Refreshes each connected keyboard's `<keyboard_id>.json` in `asset_dir`,
/// for every keyboard configured under `keyboard_config_dir`. Runs once at
/// startup, ahead of the listener, never on the keypress hot path. Each
/// keyboard is independent and best-effort: one that isn't currently
/// connected keeps its existing cached model and is skipped with a warning.
pub fn refresh_models(asset_dir: &Path, keyboard_config_dir: &Path) -> Result<()> {
    fs::create_dir_all(asset_dir)
        .with_context(|| format!("Failed to create asset directory {}", asset_dir.display()))?;
    recover_model_backups(asset_dir)?;
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
        recover_model_backup(&output_path)?;
        if output_path.exists() {
            info!("Refreshing model for keyboard {keyboard_id}...");
        } else {
            info!("Generating model for keyboard {keyboard_id}...");
        }
        generate_one(&path, keyboard_id, &output_path);
    }
    Ok(())
}

/// Restores all valid models left as backups by an interrupted replacement.
fn recover_model_backups(asset_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(asset_dir)
        .with_context(|| format!("Failed to read asset directory {}", asset_dir.display()))?
    {
        let backup_path = entry
            .with_context(|| format!("Failed to read an entry in {}", asset_dir.display()))?
            .path();
        if backup_path
            .extension()
            .is_none_or(|extension| extension != "bak")
        {
            continue;
        }
        let output_path = backup_path.with_extension("");
        recover_model_backup(&output_path)?;
    }
    Ok(())
}

/// Restores the last valid model if an interrupted replacement left its backup.
fn recover_model_backup(output_path: &Path) -> Result<()> {
    if output_path.exists() {
        return Ok(());
    }
    let backup_path = output_path.with_extension("json.bak");
    if !backup_path.exists() {
        return Ok(());
    }
    fs::rename(&backup_path, output_path).with_context(|| {
        format!(
            "Failed to restore interrupted model replacement {}",
            output_path.display()
        )
    })
}

fn keyboard_id_from_dir(path: &Path) -> Option<u8> {
    if !path.is_dir() || !path.join("config.json").is_file() {
        return None;
    }
    path.file_name()?.to_str()?.parse().ok()
}

fn generate_one(keyboard_dir: &Path, keyboard_id: u8, output_path: &Path) {
    let model = keymap_overlay_generator::read_live_keyboard_models(
        &keyboard_dir.join("keyboard.json"),
        &keyboard_dir.join("config.json"),
        LAYOUT_NAME,
        keyboard_id,
        PLATFORM,
        64,
    );
    let output = match model.and_then(|model| serde_json::to_vec(&model).map_err(Into::into)) {
        Ok(output) => output,
        Err(error) => {
            warn!(
                "Model refresh failed for keyboard {keyboard_id}, probably not connected: {error:#}"
            );
            return;
        }
    };

    if let Err(error) = install_generated_model(output_path, keyboard_id, &output) {
        warn!("Failed to install the generated model for keyboard {keyboard_id}: {error:#}");
    } else {
        info!("Updated model for keyboard {keyboard_id}");
    }
}

fn install_generated_model(output_path: &Path, keyboard_id: u8, output: &[u8]) -> Result<()> {
    let tmp_path = output_path.with_extension("json.tmp");
    fs::write(&tmp_path, output)
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
    if let Err(error) = super::load_keyboard_model_file(&tmp_path, keyboard_id) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error).context("Generator produced an invalid model");
    }
    replace_model_file(&tmp_path, output_path)
}

fn replace_model_file(tmp_path: &Path, output_path: &Path) -> Result<()> {
    if !output_path.exists() {
        return fs::rename(tmp_path, output_path)
            .with_context(|| format!("Failed to install {}", output_path.display()));
    }

    let backup_path = output_path.with_extension("json.bak");
    let _ = fs::remove_file(&backup_path);
    fs::rename(output_path, &backup_path).with_context(|| {
        format!(
            "Failed to preserve existing model {}",
            output_path.display()
        )
    })?;
    if let Err(error) = fs::rename(tmp_path, output_path) {
        let restore_error = fs::rename(&backup_path, output_path).err();
        return match restore_error {
            Some(restore_error) => Err(error).with_context(|| {
                format!(
                    "Failed to install {} and restore its backup: {restore_error}",
                    output_path.display()
                )
            }),
            None => {
                Err(error).with_context(|| format!("Failed to replace {}", output_path.display()))
            }
        };
    }
    let _ = fs::remove_file(backup_path);
    Ok(())
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
    fn refresh_models_preserves_an_existing_model_when_the_keyboard_is_unavailable() {
        let config_dir = TempDir::new().expect("temp dir");
        let keyboard = config_dir.path().join("1");
        fs::create_dir(&keyboard).expect("mkdir");
        fs::write(keyboard.join("config.json"), "{}").expect("write");

        let asset_dir = TempDir::new().expect("temp dir");
        let existing = r#"{"keyboard_id":1,"layers":{"0":{"version":2,"layer":0,"width":1,"height":1,"header_font_size":14.0,"key_font_size":10.0,"encoder_font_size":10.0,"keys":[],"encoders":[]}}}"#;
        fs::write(asset_dir.path().join("1.json"), existing).expect("write");

        refresh_models(asset_dir.path(), config_dir.path()).expect("refresh");
        assert_eq!(
            fs::read_to_string(asset_dir.path().join("1.json")).unwrap(),
            existing
        );
    }

    #[test]
    fn refresh_models_creates_the_asset_directory() {
        let config_dir = TempDir::new().expect("temp dir");
        let root = TempDir::new().expect("temp dir");
        let asset_dir = root.path().join("missing/cache");

        refresh_models(&asset_dir, config_dir.path()).expect("refresh");

        assert!(asset_dir.is_dir());
    }

    #[test]
    fn refresh_models_restores_an_interrupted_replacement() {
        let config_dir = TempDir::new().expect("temp dir");
        let keyboard = config_dir.path().join("1");
        fs::create_dir(&keyboard).expect("mkdir");
        fs::write(keyboard.join("config.json"), "{}").expect("write");

        let asset_dir = TempDir::new().expect("temp dir");
        let backup_path = asset_dir.path().join("1.json.bak");
        let existing = r#"{"keyboard_id":1,"layers":{"0":{"version":2,"layer":0,"width":1,"height":1,"header_font_size":14.0,"key_font_size":10.0,"encoder_font_size":10.0,"keys":[],"encoders":[]}}}"#;
        fs::write(&backup_path, existing).expect("write backup");

        refresh_models(asset_dir.path(), config_dir.path()).expect("refresh");

        assert_eq!(
            fs::read_to_string(asset_dir.path().join("1.json")).unwrap(),
            existing
        );
        assert!(!backup_path.exists());
    }

    #[test]
    fn a_valid_generated_model_replaces_an_existing_model() {
        let asset_dir = TempDir::new().expect("temp dir");
        let output_path = asset_dir.path().join("1.json");
        fs::write(&output_path, "old model").expect("write old model");
        let generated = br#"{"keyboard_id":1,"layers":{"0":{"version":2,"layer":0,"width":1,"height":1,"header_font_size":14.0,"key_font_size":10.0,"encoder_font_size":10.0,"keys":[],"encoders":[]}}}"#;

        install_generated_model(&output_path, 1, generated).expect("install model");

        assert_eq!(fs::read(&output_path).unwrap(), generated);
        assert!(!output_path.with_extension("json.tmp").exists());
        assert!(!output_path.with_extension("json.bak").exists());
    }

    #[test]
    fn invalid_generated_output_preserves_an_existing_model() {
        let asset_dir = TempDir::new().expect("temp dir");
        let output_path = asset_dir.path().join("1.json");
        fs::write(&output_path, "old model").expect("write old model");

        install_generated_model(&output_path, 1, b"not json").unwrap_err();

        assert_eq!(fs::read_to_string(&output_path).unwrap(), "old model");
        assert!(!output_path.with_extension("json.tmp").exists());
    }
}
