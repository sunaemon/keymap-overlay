use crate::custom_keycodes::{CustomKeycode, custom_keycode_labels, parse_custom_keycodes};
use crate::labels::Platform;
use crate::model::{LayerSource, build_layer_model};
use crate::types::{KeyboardConfig, KeyboardJson, KeyboardModels, KeymapOverlayMetadata};
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
    build_keyboard_models(
        device_model,
        keyboard,
        config,
        layout_name,
        keyboard_id,
        platform,
        pixels_per_unit,
    )
}

/// Builds models using only metadata and dynamic state embedded in one device.
pub fn read_self_describing_keyboard_models(
    dev: &HidDevice,
    platform: Platform,
) -> Result<Option<KeyboardModels>> {
    let definition = vial::read_device_definition(dev)?;
    let Some(metadata) = definition.get("keymapOverlay").cloned() else {
        return Ok(None);
    };
    let metadata: KeymapOverlayMetadata = serde_json::from_value(metadata)
        .context("Invalid keymapOverlay metadata in the device's Vial definition")?;
    let device_model = vial::read_device_model_with_definition(
        dev,
        definition,
        metadata.keyboard.encoder_count(),
    )?;
    build_keyboard_models(
        device_model,
        &metadata.keyboard,
        &metadata.config,
        &metadata.layout_name,
        metadata.keyboard_id,
        platform,
        metadata.pixels_per_unit,
    )
    .map(Some)
}

fn build_keyboard_models(
    device_model: vial::DeviceModel,
    keyboard: &KeyboardJson,
    config: &KeyboardConfig,
    layout_name: &str,
    keyboard_id: u8,
    platform: Platform,
    pixels_per_unit: i64,
) -> Result<KeyboardModels> {
    let rows = device_model.matrix_rows;
    let cols = device_model.matrix_cols;
    let custom_keycodes = parse_custom_keycodes(&device_model.vial_definition)?;
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
        0x0053 => "KC_NUM".to_string(),
        0x0054 => "KC_PSLS".to_string(),
        0x0055 => "KC_PAST".to_string(),
        0x0056 => "KC_PMNS".to_string(),
        0x0057 => "KC_PPLS".to_string(),
        0x0058 => "KC_PENT".to_string(),
        0x0059..=0x0061 => format!("KC_P{}", raw - 0x0058),
        0x0062 => "KC_P0".to_string(),
        0x0063 => "KC_PDOT".to_string(),
        0x0064 => "KC_NUBS".to_string(),
        0x0065 => "KC_APP".to_string(),
        0x0066 => "KC_PWR".to_string(),
        0x0067 => "KC_PEQL".to_string(),
        0x0068..=0x0073 => format!("KC_F{}", raw - 0x0055),
        0x0074 => "KC_EXEC".to_string(),
        0x0075 => "KC_HELP".to_string(),
        0x0076 => "KC_MENU".to_string(),
        0x0077 => "KC_SLCT".to_string(),
        0x0078 => "KC_STOP".to_string(),
        0x0079 => "KC_AGIN".to_string(),
        0x007A => "KC_UNDO".to_string(),
        0x007B => "KC_CUT".to_string(),
        0x007C => "KC_COPY".to_string(),
        0x007D => "KC_PSTE".to_string(),
        0x007E => "KC_FIND".to_string(),
        0x007F => "KC_MUTE".to_string(),
        0x0080 => "KC_VOLU".to_string(),
        0x0081 => "KC_VOLD".to_string(),
        0x0082 => "KC_LCAP".to_string(),
        0x0083 => "KC_LNUM".to_string(),
        0x0084 => "KC_LSCR".to_string(),
        0x0085 => "KC_PCMM".to_string(),
        0x0086 => "KC_KP_EQUAL_AS400".to_string(),
        0x0087..=0x008F => format!("KC_INT{}", raw - 0x0086),
        0x0090..=0x0098 => format!("KC_LNG{}", raw - 0x008F),
        0x0099 => "KC_ERAS".to_string(),
        0x009A => "KC_SYRQ".to_string(),
        0x009B => "KC_CNCL".to_string(),
        0x009C => "KC_CLR".to_string(),
        0x009D => "KC_PRIR".to_string(),
        0x009E => "KC_RETN".to_string(),
        0x009F => "KC_SEPR".to_string(),
        0x00A0 => "KC_OUT".to_string(),
        0x00A1 => "KC_OPER".to_string(),
        0x00A2 => "KC_CLAG".to_string(),
        0x00A3 => "KC_CRSL".to_string(),
        0x00A4 => "KC_EXSL".to_string(),
        0x00A5 => "KC_PWR".to_string(),
        0x00A6 => "KC_SLEP".to_string(),
        0x00A7 => "KC_WAKE".to_string(),
        0x00A8 => "KC_MUTE".to_string(),
        0x00A9 => "KC_VOLU".to_string(),
        0x00AA => "KC_VOLD".to_string(),
        0x00AB => "KC_MNXT".to_string(),
        0x00AC => "KC_MPRV".to_string(),
        0x00AD => "KC_MSTP".to_string(),
        0x00AE => "KC_MPLY".to_string(),
        0x00AF => "KC_MSEL".to_string(),
        0x00B0 => "KC_EJCT".to_string(),
        0x00B1 => "KC_MAIL".to_string(),
        0x00B2 => "KC_CALC".to_string(),
        0x00B3 => "KC_MYCM".to_string(),
        0x00B4 => "KC_WSCH".to_string(),
        0x00B5 => "KC_WHOM".to_string(),
        0x00B6 => "KC_WBAK".to_string(),
        0x00B7 => "KC_WFWD".to_string(),
        0x00B8 => "KC_WSTP".to_string(),
        0x00B9 => "KC_WREF".to_string(),
        0x00BA => "KC_WFAV".to_string(),
        0x00BB => "KC_MFFD".to_string(),
        0x00BC => "KC_MRWD".to_string(),
        0x00BD => "KC_BRIU".to_string(),
        0x00BE => "KC_BRID".to_string(),
        0x00BF => "KC_CPNL".to_string(),
        0x00C0 => "KC_ASSISTANT".to_string(),
        0x00C1 => "KC_MISSION_CONTROL".to_string(),
        0x00C2 => "KC_LAUNCHPAD".to_string(),
        0x00E0 => "KC_LCTL".to_string(),
        0x00E1 => "KC_LSFT".to_string(),
        0x00E2 => "KC_LALT".to_string(),
        0x00E3 => "KC_LGUI".to_string(),
        0x00E4 => "KC_RCTL".to_string(),
        0x00E5 => "KC_RSFT".to_string(),
        0x00E6 => "KC_RALT".to_string(),
        0x00E7 => "KC_RGUI".to_string(),
        0x0100..=0x1FFF => modified_keycode_name(raw),
        0x2000..=0x3FFF => format!(
            "MT({},{})",
            modifier_name(((raw >> 8) & 0x1F) as u8),
            standard_keycode_name(raw & 0x00FF)
        ),
        0x4000..=0x4FFF => format!(
            "LT({},{})",
            (raw >> 8) & 0x000F,
            standard_keycode_name(raw & 0x00FF)
        ),
        0x5000..=0x51FF => format!(
            "LM({},{})",
            (raw >> 5) & 0x000F,
            modifier_name((raw & 0x001F) as u8)
        ),
        0x5200..=0x521F => format!("TO({})", raw & 0x001F),
        0x5220..=0x523F => format!("MO({})", raw & 0x001F),
        0x5240..=0x525F => format!("DF({})", raw & 0x001F),
        0x5260..=0x527F => format!("TG({})", raw & 0x001F),
        0x5280..=0x529F => format!("OSL({})", raw & 0x001F),
        0x52A0..=0x52BF => format!("OSM({})", modifier_name((raw & 0x001F) as u8)),
        0x52C0..=0x52DF => format!("TT({})", raw & 0x001F),
        0x52E0..=0x52FF => format!("PDF({})", raw & 0x001F),
        0x5700..=0x57FF => format!("TD({})", raw & 0x00FF),
        0x7C00 => "QK_BOOT".to_string(),
        _ => format!("0x{raw:04X}"),
    }
}

fn modified_keycode_name(raw: u16) -> String {
    let keycode = standard_keycode_name(raw & 0x00FF);
    match ((raw >> 8) & 0x1F) as u8 {
        0x01 => format!("LCTL({keycode})"),
        0x02 => format!("LSFT({keycode})"),
        0x04 => format!("LALT({keycode})"),
        0x08 => format!("LGUI({keycode})"),
        0x11 => format!("RCTL({keycode})"),
        0x12 => format!("RSFT({keycode})"),
        0x14 => format!("RALT({keycode})"),
        0x18 => format!("RGUI({keycode})"),
        modifiers => format!("QK_MODS({},{keycode})", modifier_name(modifiers)),
    }
}

fn modifier_name(modifiers: u8) -> String {
    let right = modifiers & 0x10 != 0;
    let mut names = Vec::new();
    for (bit, left_name, right_name) in [
        (0x01, "MOD_LCTL", "MOD_RCTL"),
        (0x02, "MOD_LSFT", "MOD_RSFT"),
        (0x04, "MOD_LALT", "MOD_RALT"),
        (0x08, "MOD_LGUI", "MOD_RGUI"),
    ] {
        if modifiers & bit != 0 {
            names.push(if right { right_name } else { left_name });
        }
    }
    if names.is_empty() {
        "KC_NO".to_string()
    } else {
        names.join("|")
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
    fn bootloader_keycode_matches_qmk() {
        assert_eq!(resolve_keycode(0x7C00, &[]), "QK_BOOT");
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

    #[test]
    fn encoded_qmk_keycode_families_keep_their_semantics() {
        assert_eq!(resolve_keycode(0x5202, &[]), "TO(2)");
        assert_eq!(resolve_keycode(0x5243, &[]), "DF(3)");
        assert_eq!(resolve_keycode(0x5264, &[]), "TG(4)");
        assert_eq!(resolve_keycode(0x5285, &[]), "OSL(5)");
        assert_eq!(resolve_keycode(0x52C6, &[]), "TT(6)");
        assert_eq!(resolve_keycode(0x52E7, &[]), "PDF(7)");
        assert_eq!(resolve_keycode(0x5709, &[]), "TD(9)");
    }

    #[test]
    fn tap_and_modifier_keycodes_include_their_arguments() {
        assert_eq!(resolve_keycode(0x4104, &[]), "LT(1,KC_A)");
        assert_eq!(resolve_keycode(0x2204, &[]), "MT(MOD_LSFT,KC_A)");
        assert_eq!(resolve_keycode(0x0204, &[]), "LSFT(KC_A)");
        assert_eq!(resolve_keycode(0x5062, &[]), "LM(3,MOD_LSFT)");
        assert_eq!(resolve_keycode(0x52A2, &[]), "OSM(MOD_LSFT)");
    }

    #[test]
    fn standard_keypad_and_consumer_keycodes_keep_their_qmk_names() {
        assert_eq!(resolve_keycode(0x0059, &[]), "KC_P1");
        assert_eq!(resolve_keycode(0x0063, &[]), "KC_PDOT");
        assert_eq!(resolve_keycode(0x00AB, &[]), "KC_MNXT");
        assert_eq!(resolve_keycode(0x00AE, &[]), "KC_MPLY");
    }

    #[test]
    fn full_vial_definition_preserves_custom_keycode_names() {
        let definition = serde_json::json!({
            "customKeycodes": [{ "name": "KC_ALPHA", "shortName": "α" }]
        });
        let custom_keycodes = parse_custom_keycodes(&definition).expect("valid Vial metadata");
        assert_eq!(resolve_keycode(0x7E00, &custom_keycodes), "KC_ALPHA");
    }
}
