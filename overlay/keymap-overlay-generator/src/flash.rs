use crate::keymap_c;
use crate::qmk_keymap::QmkKeymapJson;
use crate::types::KeyboardJson;
use anyhow::{Context, Result, bail};
use hidapi::HidDevice;
use std::collections::HashMap;

/// Vial requires custom keycodes to be assigned starting at QK_KB_0 — same
/// convention the read-side generator assumes (device.rs, custom_keycodes.rs).
pub const QK_KB_0: u32 = 0x7E00;

/// One keymap layer as a row/col grid of keycode strings.
type LayerGrid = Vec<Vec<String>>;
/// One layer's encoder actions, padded to the keyboard's encoder count.
type EncoderLayer = Vec<[String; 2]>;

/// Maps each custom_keycodes enum member to its hex string, in declaration
/// order starting at QK_KB_0 — the reverse of what the read side resolves.
pub fn build_custom_keycode_map(names: &[String]) -> HashMap<String, String> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), format!("0x{:04X}", QK_KB_0 + index as u32)))
        .collect()
}

fn resolve(keycode: &str, custom_map: &HashMap<String, String>) -> String {
    custom_map
        .get(keycode)
        .cloned()
        .unwrap_or_else(|| keycode.to_string())
}

/// Places one flat QMK layer into its matrix-shaped grid, KC_NO-filled.
/// Extra keys beyond the layout's mapping are silently dropped, matching
/// generate_vitaly_layout.py's own tolerance for a mismatched layout.
fn build_layer_grid(
    flat_layer: &[String],
    mapping: &[(u8, u8)],
    rows: u8,
    cols: u8,
    custom_map: &HashMap<String, String>,
) -> LayerGrid {
    let mut grid = vec![vec!["KC_NO".to_string(); cols as usize]; rows as usize];
    for (key_index, keycode) in flat_layer.iter().enumerate() {
        let Some(&(row, col)) = mapping.get(key_index) else {
            continue;
        };
        grid[row as usize][col as usize] = resolve(keycode, custom_map);
    }
    grid
}

/// Builds one padded encoder-action list per keymap layer, KC_NO-filled, so
/// an absent or shorter encoder_map never leaves a stale device action.
fn build_encoder_layout(
    encoder_layers: &[EncoderLayer],
    encoder_count: usize,
    layer_count: usize,
    custom_map: &HashMap<String, String>,
) -> Result<Vec<EncoderLayer>> {
    let mut output = Vec::with_capacity(layer_count);
    for layer_index in 0..layer_count {
        let pairs = encoder_layers.get(layer_index).cloned().unwrap_or_default();
        if pairs.len() > encoder_count {
            bail!(
                "Layer {layer_index} defines {} encoders, expected at most {encoder_count}",
                pairs.len()
            );
        }
        let mut converted: EncoderLayer = pairs
            .iter()
            .map(|pair| [resolve(&pair[0], custom_map), resolve(&pair[1], custom_map)])
            .collect();
        converted.extend(
            (converted.len()..encoder_count).map(|_| ["KC_NO".to_string(), "KC_NO".to_string()]),
        );
        output.push(converted);
    }
    Ok(output)
}

/// Pure conversion from parsed keymap.c + qmk-keymap.json sources to the
/// grid/encoder shapes vitaly's write functions expect. No device I/O, so
/// this is exactly what the ported tests exercise.
pub fn resolve_flash_layout(
    qmk_keymap: &QmkKeymapJson,
    mapping: &[(u8, u8)],
    rows: u8,
    cols: u8,
    encoder_layers: &[EncoderLayer],
    encoder_count: usize,
    custom_map: &HashMap<String, String>,
) -> Result<(Vec<LayerGrid>, Vec<EncoderLayer>)> {
    let layout = qmk_keymap
        .layers
        .iter()
        .map(|flat_layer| build_layer_grid(flat_layer, mapping, rows, cols, custom_map))
        .collect();
    let encoder_layout = build_encoder_layout(
        encoder_layers,
        encoder_count,
        qmk_keymap.layers.len(),
        custom_map,
    )?;
    Ok((layout, encoder_layout))
}

fn keycode_value(keycode: &str, vial_version: u32) -> Result<u16> {
    if let Some(hex) = keycode
        .strip_prefix("0x")
        .or_else(|| keycode.strip_prefix("0X"))
    {
        u16::from_str_radix(hex, 16).with_context(|| format!("Invalid hex keycode: {keycode}"))
    } else {
        vitaly::keycodes::name_to_qid(keycode, vial_version)
            .with_context(|| format!("Unknown keycode: {keycode}"))
    }
}

/// What would be written to the device: the resolved layout/encoders, plus
/// enough of the device's own reported state (read-only: scan_capabilities,
/// load_vial_meta) to render it meaningfully or hand it to the writer.
pub struct ResolvedFlash {
    pub layout: Vec<LayerGrid>,
    pub encoder_layout: Vec<EncoderLayer>,
    pub rows: u8,
    pub cols: u8,
    pub vial_version: u32,
    pub layer_count: u8,
}

/// Reads the device (capabilities + Vial meta — both read-only) and resolves
/// keymap.c against it. Opens no new session — the caller already holds
/// `dev` open (from `device::open_device`).
pub fn resolve_for_device(
    dev: &HidDevice,
    keyboard: &KeyboardJson,
    keymap_c_source: &str,
    qmk_keymap: &QmkKeymapJson,
    layout_name: &str,
) -> Result<ResolvedFlash> {
    let custom_names = keymap_c::parse_custom_keycode_names(keymap_c_source)?;
    let custom_map = build_custom_keycode_map(&custom_names);
    let encoder_layers = keymap_c::parse_encoder_map(keymap_c_source)?;
    let encoder_count = keyboard.encoder_count();

    let layout_keys = keyboard.layout_keys(layout_name)?;
    let mapping: Vec<(u8, u8)> = layout_keys.iter().map(|key| key.matrix).collect();

    let capabilities = vitaly::protocol::scan_capabilities(dev)?;
    let vial_meta = vitaly::protocol::load_vial_meta(dev)?;
    let rows = vial_meta["matrix"]["rows"]
        .as_u64()
        .context("matrix/rows not found in the device's Vial meta")? as u8;
    let cols = vial_meta["matrix"]["cols"]
        .as_u64()
        .context("matrix/cols not found in the device's Vial meta")? as u8;

    if qmk_keymap.layers.len() != capabilities.layer_count as usize {
        bail!(
            "keymap.c defines {} layers, but the device was compiled with {} \
             (DYNAMIC_KEYMAP_LAYER_COUNT) — reflash the firmware, not just the keymap",
            qmk_keymap.layers.len(),
            capabilities.layer_count
        );
    }

    let (layout, encoder_layout) = resolve_flash_layout(
        qmk_keymap,
        &mapping,
        rows,
        cols,
        &encoder_layers,
        encoder_count,
        &custom_map,
    )?;

    Ok(ResolvedFlash {
        layout,
        encoder_layout,
        rows,
        cols,
        vial_version: capabilities.vial_version,
        layer_count: capabilities.layer_count,
    })
}

/// Writes a resolved keymap to the device: encoders, then the keymap itself.
pub fn write_to_device(dev: &HidDevice, resolved: &ResolvedFlash) -> Result<()> {
    for (layer_index, pairs) in resolved.encoder_layout.iter().enumerate() {
        for (encoder_index, pair) in pairs.iter().enumerate() {
            let ccw = keycode_value(&pair[0], resolved.vial_version)?;
            let cw = keycode_value(&pair[1], resolved.vial_version)?;
            vitaly::protocol::set_encoder(dev, layer_index as u8, encoder_index as u8, 0, ccw)?;
            vitaly::protocol::set_encoder(dev, layer_index as u8, encoder_index as u8, 1, cw)?;
        }
    }

    let layers_value: Vec<serde_json::Value> = resolved
        .layout
        .iter()
        .map(|grid| {
            serde_json::Value::Array(
                grid.iter()
                    .map(|row| {
                        serde_json::Value::Array(
                            row.iter().cloned().map(serde_json::Value::String).collect(),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let keymap = vitaly::protocol::Keymap::from_json(
        resolved.rows,
        resolved.cols,
        resolved.layer_count,
        &layers_value,
        resolved.vial_version,
    )?;
    vitaly::protocol::set_keymap(dev, &keymap)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn keymap(layers: &[&[&str]]) -> QmkKeymapJson {
        QmkKeymapJson {
            layers: layers
                .iter()
                .map(|layer| layer.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    /// Mirrors test_transparency_is_preserved_when_flashing: KC_TRNS must
    /// reach the grid unresolved, so EEPROM keeps inheriting from layer 0.
    #[test]
    fn transparency_is_preserved() {
        let qmk_keymap = keymap(&[&["KC_A", "KC_B"], &["KC_TRNS", "KC_TRNS"]]);
        let mapping = [(0, 0), (0, 1)];

        let (layout, _) =
            resolve_flash_layout(&qmk_keymap, &mapping, 2, 2, &[], 0, &HashMap::new()).unwrap();

        assert_eq!(
            layout,
            vec![
                vec![
                    vec!["KC_A".to_string(), "KC_B".to_string()],
                    vec!["KC_NO".to_string(), "KC_NO".to_string()]
                ],
                vec![
                    vec!["KC_TRNS".to_string(), "KC_TRNS".to_string()],
                    vec!["KC_NO".to_string(), "KC_NO".to_string()]
                ],
            ]
        );
    }

    /// Mirrors test_custom_keycode_names_are_mapped_back_to_codes.
    #[test]
    fn custom_keycode_names_are_mapped_back_to_hex() {
        let qmk_keymap = keymap(&[&["KC_ALPHA", "KC_B"]]);
        let mapping = [(0, 0), (0, 1)];
        let custom_map = map(&[("KC_ALPHA", "0x7E40")]);

        let (layout, _) =
            resolve_flash_layout(&qmk_keymap, &mapping, 2, 2, &[], 0, &custom_map).unwrap();

        assert_eq!(layout[0][0], vec!["0x7E40".to_string(), "KC_B".to_string()]);
    }

    /// Mirrors test_encoder_bindings_are_updated_when_flashing.
    #[test]
    fn encoder_bindings_replace_old_actions() {
        let qmk_keymap = keymap(&[&["KC_A"], &["KC_A"]]);
        let mapping = [(0, 0)];
        let custom_map = map(&[("CUSTOM", "0x7E40")]);
        let encoder_layers = vec![
            vec![
                ["KC_VOLD".to_string(), "KC_VOLU".to_string()],
                ["CUSTOM".to_string(), "KC_MUTE".to_string()],
            ],
            vec![["KC_TRNS".to_string(), "KC_TRNS".to_string()]],
        ];

        let (_, encoder_layout) =
            resolve_flash_layout(&qmk_keymap, &mapping, 1, 1, &encoder_layers, 2, &custom_map)
                .unwrap();

        assert_eq!(
            encoder_layout,
            vec![
                vec![
                    ["KC_VOLD".to_string(), "KC_VOLU".to_string()],
                    ["0x7E40".to_string(), "KC_MUTE".to_string()]
                ],
                vec![
                    ["KC_TRNS".to_string(), "KC_TRNS".to_string()],
                    ["KC_NO".to_string(), "KC_NO".to_string()]
                ],
            ]
        );
    }

    /// Mirrors test_missing_encoder_bindings_clear_old_device_actions: an
    /// absent encoder_map must still explicitly clear every layer to KC_NO,
    /// not leave whatever the device currently has.
    #[test]
    fn missing_encoder_bindings_clear_to_kc_no() {
        let qmk_keymap = keymap(&[&["KC_A"]]);
        let mapping = [(0, 0)];

        let (_, encoder_layout) =
            resolve_flash_layout(&qmk_keymap, &mapping, 1, 1, &[], 1, &HashMap::new()).unwrap();

        assert_eq!(
            encoder_layout,
            vec![vec![["KC_NO".to_string(), "KC_NO".to_string()]]]
        );
    }

    #[test]
    fn too_many_encoders_on_one_layer_is_rejected() {
        let qmk_keymap = keymap(&[&["KC_A"]]);
        let mapping = [(0, 0)];
        let encoder_layers = vec![vec![
            ["KC_VOLD".to_string(), "KC_VOLU".to_string()],
            ["KC_A".to_string(), "KC_B".to_string()],
        ]];

        let error = resolve_flash_layout(
            &qmk_keymap,
            &mapping,
            1,
            1,
            &encoder_layers,
            1,
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("expected at most 1"));
    }

    #[test]
    fn build_custom_keycode_map_starts_at_qk_kb_0() {
        let names = vec!["KC_ALPHA".to_string(), "KC_BETA".to_string()];
        let map = build_custom_keycode_map(&names);
        assert_eq!(map.get("KC_ALPHA"), Some(&"0x7E00".to_string()));
        assert_eq!(map.get("KC_BETA"), Some(&"0x7E01".to_string()));
    }

    /// `qmk c2json` has no idea what our keymap.c calls a custom keycode
    /// (that alias lives only in our own enum) — it renders every custom
    /// position using QMK's own generic QK_KB_<n> name instead, confirmed
    /// against real `qmk c2json` output on keyboard 1's Greek layer. Those
    /// strings pass through `resolve()` unchanged (not in `custom_map`,
    /// which is keyed by *our* enum names) — this pins down that
    /// `vitaly::keycodes::name_to_qid` still resolves them correctly on its
    /// own downstream, in `Keymap::from_json`, so that pass-through is
    /// correct and not a bug. `custom_map` substitution is only load-bearing
    /// for `encoder_map`, whose enum-name identifiers are regex-parsed
    /// straight from keymap.c source text, never compiled through qmk.
    #[test]
    fn qk_kb_names_resolve_via_vitalys_own_standard_table_unaided() {
        for n in [0u16, 5, 17, 23] {
            let name = format!("QK_KB_{n}");
            let resolved = vitaly::keycodes::name_to_qid(&name, 6).unwrap();
            assert_eq!(resolved, QK_KB_0 as u16 + n, "{name} did not round-trip");
        }
    }
}
