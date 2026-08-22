use crate::custom_keycodes::{CustomKeycode, custom_keycode_labels, parse_custom_keycodes};
use crate::labels::Platform;
use crate::model::{LayerSource, build_layer_model};
use crate::types::{KeyboardConfig, KeyboardJson, KeyboardModels};
use anyhow::{Context, Result, bail};
use hidapi::{HidApi, HidDevice};
use std::collections::HashMap;

pub fn open_device(api: &HidApi, keyboard: &KeyboardJson) -> Result<HidDevice> {
    let vendor_id = parse_hex_u16(&keyboard.usb.vid)
        .with_context(|| format!("Invalid vendor id: {}", keyboard.usb.vid))?;
    let product_id = parse_hex_u16(&keyboard.usb.pid)
        .with_context(|| format!("Invalid product id: {}", keyboard.usb.pid))?;

    let device_info = api
        .device_list()
        .find(|device| {
            device.usage_page() == vitaly::protocol::USAGE_PAGE
                && device.usage() == vitaly::protocol::USAGE_ID
                && device.vendor_id() == vendor_id
                && device.product_id() == product_id
        })
        .with_context(|| {
            format!("No Raw HID interface found for device {vendor_id:04x}:{product_id:04x}")
        })?;

    Ok(api.open_path(device_info.path())?)
}

fn parse_hex_u16(value: &str) -> Result<u16> {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    Ok(u16::from_str_radix(digits, 16)?)
}

pub fn read_keyboard_models(
    dev: &HidDevice,
    keyboard: &KeyboardJson,
    config: &KeyboardConfig,
    layout_name: &str,
    keyboard_id: u8,
    platform: Platform,
    pixels_per_unit: i64,
) -> Result<KeyboardModels> {
    let capabilities = vitaly::protocol::scan_capabilities(dev)?;
    if capabilities.layer_count == 0 {
        bail!("Device reports zero layers");
    }
    let vial_meta = vitaly::protocol::load_vial_meta(dev)?;
    let rows = vial_meta["matrix"]["rows"]
        .as_u64()
        .context("matrix/rows not found in the device's Vial meta")? as u8;
    let cols = vial_meta["matrix"]["cols"]
        .as_u64()
        .context("matrix/cols not found in the device's Vial meta")? as u8;
    let custom_keycodes = parse_custom_keycodes(&vial_meta)?;
    let display_labels = custom_keycode_labels(&custom_keycodes);

    let keymap = vitaly::protocol::load_layers_keys(dev, capabilities.layer_count, rows, cols)?;
    let encoder_count = keyboard.encoder_count();

    let layout = keyboard.layout_keys(layout_name)?;
    for key in layout {
        let (row, col) = key.matrix;
        if row >= rows || col >= cols {
            bail!(
                "Layout matrix position {row},{col} is outside the device's {rows}x{cols} matrix"
            );
        }
    }

    let mut layer_sources = Vec::with_capacity(capabilities.layer_count as usize);
    for layer_index in 0..capabilities.layer_count {
        let keys = layout
            .iter()
            .map(|key| {
                let (row, col) = key.matrix;
                resolve_keycode(
                    keymap.get(layer_index, row, col),
                    capabilities.vial_version,
                    &custom_keycodes,
                )
            })
            .collect();
        let mut encoders = Vec::with_capacity(encoder_count);
        for encoder_index in 0..encoder_count as u8 {
            let encoder = vitaly::protocol::load_encoder(dev, layer_index, encoder_index)?;
            encoders.push([
                resolve_keycode(encoder.ccw, capabilities.vial_version, &custom_keycodes),
                resolve_keycode(encoder.cw, capabilities.vial_version, &custom_keycodes),
            ]);
        }
        layer_sources.push(LayerSource { keys, encoders });
    }

    let base_layer = layer_sources[0].clone();
    let mut layers = HashMap::new();
    for (layer_index, layer) in layer_sources.iter().enumerate() {
        let model = build_layer_model(
            keyboard,
            config,
            layout_name,
            layer_index as u8,
            &base_layer,
            layer,
            &display_labels,
            platform,
            pixels_per_unit,
        )?;
        layers.insert(layer_index as u8, model);
    }

    Ok(KeyboardModels {
        keyboard_id,
        layers,
    })
}

fn resolve_keycode(raw: u16, vial_version: u32, custom_keycodes: &[CustomKeycode]) -> String {
    if let Some(index) = vitaly::keycodes::is_custom(raw, vial_version)
        && let Some(keycode) = custom_keycodes.get(index as usize)
    {
        return keycode.name.clone();
    }
    vitaly::keycodes::qid_to_name(raw, vial_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session's original bug: a device-sourced custom keycode must
    /// resolve to its real name (e.g. "KC_ALPHA"), not vitaly's generic
    /// QK_KB_<n> fallback, so its display glyph can be found later.
    #[test]
    fn a_custom_keycode_resolves_to_its_real_name() {
        let vial_version = 6;
        let base = vitaly::keycodes::qid_to_name(0x0000, vial_version); // sanity: standard range unaffected
        assert_ne!(base, "");

        let custom_keycodes = vec![
            CustomKeycode {
                name: "KC_ALPHA".to_string(),
                short_name: "α".to_string(),
            },
            CustomKeycode {
                name: "KC_BETA".to_string(),
                short_name: "β".to_string(),
            },
        ];
        let base_keycode = 0x7E00u16;
        assert_eq!(
            vitaly::keycodes::is_custom(base_keycode, vial_version),
            Some(0),
            "0x7E00 (QK_KB_0) must be vitaly's custom-keycode base for the resolver to work"
        );

        assert_eq!(
            resolve_keycode(base_keycode, vial_version, &custom_keycodes),
            "KC_ALPHA"
        );
        assert_eq!(
            resolve_keycode(base_keycode + 1, vial_version, &custom_keycodes),
            "KC_BETA"
        );
    }

    #[test]
    fn a_standard_keycode_resolves_via_the_generic_name_table() {
        assert_eq!(resolve_keycode(0x0001, 6, &[]), "KC_TRANSPARENT");
    }

    #[test]
    fn a_custom_keycode_beyond_the_known_list_falls_back_to_the_generic_name() {
        let generic = vitaly::keycodes::qid_to_name(0x7E00, 6);
        assert_eq!(resolve_keycode(0x7E00, 6, &[]), generic);
    }
}
