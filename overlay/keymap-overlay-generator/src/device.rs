use crate::custom_keycodes::{CustomKeycode, custom_keycode_labels, parse_custom_keycodes};
use crate::labels::Platform;
use crate::model::{LayerSource, build_layer_model};
use crate::types::{KeyboardConfig, KeyboardJson, KeyboardModels};
use crate::vial;
use anyhow::{Context, Result, bail};
use hidapi::{HidApi, HidDevice};
use std::collections::HashMap;

pub fn open_device(api: &HidApi, keyboard: &KeyboardJson) -> Result<HidDevice> {
    let vendor_id = parse_hex_u16(&keyboard.usb.vid)
        .with_context(|| format!("Invalid vendor id: {}", keyboard.usb.vid))?;
    let product_id = parse_hex_u16(&keyboard.usb.pid)
        .with_context(|| format!("Invalid product id: {}", keyboard.usb.pid))?;

    let matches = api
        .device_list()
        .filter(|device| {
            device.usage_page() == vial::USAGE_PAGE
                && device.usage() == vial::USAGE_ID
                && device.vendor_id() == vendor_id
                && device.product_id() == product_id
        })
        .collect::<Vec<_>>();
    let [device_info] = matches.as_slice() else {
        match matches.len() {
            0 => bail!("No Raw HID interface found for device {vendor_id:04x}:{product_id:04x}"),
            count => bail!(
                "Found {count} Raw HID interfaces for device {vendor_id:04x}:{product_id:04x}; \
                 disconnect identical keyboards so only the intended device remains"
            ),
        }
    };

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
    let device_model = vial::read_device_model(dev, keyboard.encoder_count())?;
    let rows = device_model.matrix_rows;
    let cols = device_model.matrix_cols;
    let custom_keycodes = parse_custom_keycodes(&device_model.custom_keycodes)?;
    let display_labels = custom_keycode_labels(&custom_keycodes);

    let layout = keyboard.layout_keys(layout_name)?;
    for key in layout {
        let (row, col) = key.matrix;
        if row >= rows || col >= cols {
            bail!(
                "Layout matrix position {row},{col} is outside the device's {rows}x{cols} matrix"
            );
        }
    }

    let mut layer_sources = Vec::with_capacity(device_model.layer_count as usize);
    for layer_index in 0..device_model.layer_count {
        let keys = layout
            .iter()
            .map(|key| {
                let (row, col) = key.matrix;
                let index = layer_index as usize * rows as usize * cols as usize
                    + row as usize * cols as usize
                    + col as usize;
                resolve_keycode(device_model.keycodes[index], &custom_keycodes)
            })
            .collect();
        let encoders = device_model.encoders[layer_index as usize]
            .iter()
            .map(|pair| {
                [
                    resolve_keycode(pair[0], &custom_keycodes),
                    resolve_keycode(pair[1], &custom_keycodes),
                ]
            })
            .collect();
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

fn resolve_keycode(raw: u16, custom_keycodes: &[CustomKeycode]) -> String {
    if let Some(index) = raw.checked_sub(0x7E00)
        && let Some(keycode) = custom_keycodes.get(index as usize)
    {
        return keycode.name.clone();
    }
    standard_keycode_name(raw)
}

fn standard_keycode_name(raw: u16) -> String {
    match raw {
        0x0000 => "KC_NO".to_string(),
        0x0001 => "KC_TRNS".to_string(),
        0x0004..=0x001D => format!("KC_{}", (b'A' + (raw - 0x0004) as u8) as char),
        0x001E..=0x0026 => format!("KC_{}", raw - 0x001D),
        0x0027 => "KC_0".to_string(),
        0x0028 => "KC_ENT".to_string(),
        0x0029 => "KC_ESC".to_string(),
        0x002A => "KC_BSPC".to_string(),
        0x002B => "KC_TAB".to_string(),
        0x002C => "KC_SPC".to_string(),
        0x002D => "KC_MINS".to_string(),
        0x002E => "KC_EQL".to_string(),
        0x002F => "KC_LBRC".to_string(),
        0x0030 => "KC_RBRC".to_string(),
        0x0031 => "KC_BSLS".to_string(),
        0x0033 => "KC_SCLN".to_string(),
        0x0034 => "KC_QUOT".to_string(),
        0x0035 => "KC_GRV".to_string(),
        0x0036 => "KC_COMM".to_string(),
        0x0037 => "KC_DOT".to_string(),
        0x0038 => "KC_SLSH".to_string(),
        0x0039 => "KC_CAPS".to_string(),
        0x003A..=0x0045 => format!("KC_F{}", raw - 0x0039),
        0x0046 => "KC_PSCR".to_string(),
        0x0047 => "KC_SCRL".to_string(),
        0x0048 => "KC_PAUS".to_string(),
        0x0049 => "KC_INS".to_string(),
        0x004A => "KC_HOME".to_string(),
        0x004B => "KC_PGUP".to_string(),
        0x004C => "KC_DEL".to_string(),
        0x004D => "KC_END".to_string(),
        0x004E => "KC_PGDN".to_string(),
        0x004F => "KC_RIGHT".to_string(),
        0x0050 => "KC_LEFT".to_string(),
        0x0051 => "KC_DOWN".to_string(),
        0x0052 => "KC_UP".to_string(),
        0x007F => "KC_MUTE".to_string(),
        0x0080 => "KC_VOLU".to_string(),
        0x0081 => "KC_VOLD".to_string(),
        0x00B5 => "KC_MNXT".to_string(),
        0x00B6 => "KC_MPRV".to_string(),
        0x00CD => "KC_MPLY".to_string(),
        0x00E0 => "KC_LCTL".to_string(),
        0x00E1 => "KC_LSFT".to_string(),
        0x00E2 => "KC_LALT".to_string(),
        0x00E3 => "KC_LGUI".to_string(),
        0x00E4 => "KC_RCTL".to_string(),
        0x00E5 => "KC_RSFT".to_string(),
        0x00E6 => "KC_RALT".to_string(),
        0x00E7 => "KC_RGUI".to_string(),
        0x5220..=0x523F => format!("MO({})", raw & 0x001F),
        0x7C53 => "QK_BOOT".to_string(),
        _ => format!("0x{raw:04X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session's original bug: a device-sourced custom keycode must
    /// resolve to its real name (e.g. "KC_ALPHA"), not vitaly's generic
    /// QK_KB_<n> fallback, so its display glyph can be found later.
    #[test]
    fn a_custom_keycode_resolves_to_its_real_name() {
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
        assert_eq!(resolve_keycode(base_keycode, &custom_keycodes), "KC_ALPHA");
        assert_eq!(
            resolve_keycode(base_keycode + 1, &custom_keycodes),
            "KC_BETA"
        );
    }

    #[test]
    fn a_standard_keycode_resolves_via_the_generic_name_table() {
        assert_eq!(resolve_keycode(0x0001, &[]), "KC_TRNS");
    }

    #[test]
    fn a_custom_keycode_beyond_the_known_list_falls_back_to_the_generic_name() {
        assert_eq!(resolve_keycode(0x7E00, &[]), "0x7E00");
    }

    #[test]
    fn standard_modifier_names_match_the_platform_label_table() {
        assert_eq!(resolve_keycode(0x00E3, &[]), "KC_LGUI");
        assert_eq!(resolve_keycode(0x00E7, &[]), "KC_RGUI");
    }
}
