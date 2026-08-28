pub mod custom_keycodes;
pub mod device;
pub mod keymap_c;
pub mod labels;
pub mod model;
pub mod qmk_keymap;
pub mod types;
pub mod vial;

use anyhow::{Context, Result};
use hidapi::HidApi;
use labels::Platform;
use log::warn;
use std::path::Path;

pub fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Builds the live display model for one connected Vial keyboard.
///
/// This is linked into the platform overlay process; it intentionally does
/// not write EEPROM or spawn a second executable.
pub fn read_live_keyboard_models(
    keyboard_json: &Path,
    keyboard_config: &Path,
    layout_name: &str,
    keyboard_id: u8,
    platform: Platform,
    pixels_per_unit: i64,
) -> Result<types::KeyboardModels> {
    let keyboard: types::KeyboardJson = read_json(keyboard_json)?;
    let config: types::KeyboardConfig = read_json(keyboard_config)?;
    let api = HidApi::new().context("Failed to initialize HID API")?;
    let device = device::open_device(&api, &keyboard)?;
    device::read_keyboard_models(
        &device,
        &keyboard,
        &config,
        layout_name,
        keyboard_id,
        platform,
        pixels_per_unit,
    )
}

/// Builds in-memory models for every connected self-describing Vial keyboard.
pub fn read_connected_keyboard_models(platform: Platform) -> Result<Vec<types::KeyboardModels>> {
    let api = HidApi::new().context("Failed to initialize HID API")?;
    Ok(collect_connected_keyboard_models(
        api.device_list()
            .filter(|info| info.usage_page() == vial::USAGE_PAGE && info.usage() == vial::USAGE_ID),
        |info| {
            let device = api
                .open_path(info.path())
                .with_context(|| format!("Failed to open Raw HID device {:?}", info.path()))?;
            device::read_self_describing_keyboard_models(&device, platform)
                .with_context(|| format!("Failed to read Vial device {:?}", info.path()))
        },
    ))
}

fn collect_connected_keyboard_models<T>(
    devices: impl IntoIterator<Item = T>,
    mut read: impl FnMut(T) -> Result<Option<types::KeyboardModels>>,
) -> Vec<types::KeyboardModels> {
    devices
        .into_iter()
        .filter_map(|device| match read(device) {
            Ok(model) => model,
            Err(error) => {
                warn!("Skipping unusable Raw HID device: {error:#}");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    #[test]
    fn one_unusable_device_does_not_discard_other_models() {
        let models = collect_connected_keyboard_models([1, 2, 3], |device| match device {
            1 => Ok(Some(types::KeyboardModels {
                keyboard_id: 1,
                layers: Default::default(),
            })),
            2 => bail!("not a self-describing Vial keyboard"),
            _ => Ok(None),
        });

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].keyboard_id, 1);
    }
}
