use crate::labels::{Platform, keycode_labels, platform_keycode_labels};
use crate::types::{
    DisplayEncoder, DisplayKey, EncoderPlacement, KeyboardConfig, KeyboardJson, LayoutKey,
    OverlayModel,
};
use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};

const PADDING: i64 = 20;
const HEADER_HEIGHT: i64 = 38;
const KEY_INSET: i64 = 3;

fn transparent_keys() -> [&'static str; 3] {
    ["KC_TRNS", "KC_TRANSPARENT", "_______"]
}

fn is_transparent(keycode: &str) -> bool {
    transparent_keys().contains(&keycode)
}

/// A resolved encoder position: the physical key index it replaces (if any,
/// since an encoder can also sit at explicit coordinates with no key
/// underneath) and its geometry.
type EncoderPlacementResolved = (Option<usize>, f64, f64, f64, f64);

#[derive(Clone)]
pub struct LayerSource {
    /// Every position's resolved keycode name (custom keycodes already
    /// resolved to their real name, e.g. "KC_ALPHA"), before any
    /// layer>0 transparency fallthrough — matches what a raw QMK keymap
    /// layer would test transparency/momentary-layer against.
    pub keys: Vec<String>,
    pub encoders: Vec<[String; 2]>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_layer_model(
    keyboard: &KeyboardJson,
    config: &KeyboardConfig,
    layout_name: &str,
    layer_index: u8,
    base_layer: &LayerSource,
    layer: &LayerSource,
    display_labels: &HashMap<String, String>,
    platform: Platform,
    pixels_per_unit: i64,
) -> Result<OverlayModel> {
    let layout = keyboard.layout_keys(layout_name)?;
    validate_layer(
        layout,
        layer,
        base_layer,
        keyboard.encoder_count(),
        pixels_per_unit,
    )?;

    let placements = resolve_encoder_placements(keyboard, config, layout)?;

    let mut labels: HashMap<String, String> = platform_keycode_labels(platform)
        .into_iter()
        .map(|(name, glyph)| (name.to_string(), glyph.to_string()))
        .collect();
    labels.extend(
        display_labels
            .iter()
            .map(|(name, glyph)| (name.clone(), glyph.clone())),
    );
    let generic_labels = keycode_labels();

    let display_keys: Vec<String> = (0..layer.keys.len())
        .map(|index| {
            if layer_index > 0 && is_transparent(&layer.keys[index]) {
                base_layer.keys[index].clone()
            } else {
                layer.keys[index].clone()
            }
        })
        .collect();

    let display_encoders: Vec<[String; 2]> = layer
        .encoders
        .iter()
        .enumerate()
        .map(|(index, pair)| {
            [0, 1].map(|direction| {
                if layer_index > 0 && is_transparent(&pair[direction]) {
                    base_layer.encoders[index][direction].clone()
                } else {
                    pair[direction].clone()
                }
            })
        })
        .collect();

    build_model(
        layout,
        &display_keys,
        &layer.keys,
        &placements,
        &display_encoders,
        &layer.encoders,
        &labels,
        &generic_labels,
        layer_index,
        pixels_per_unit,
    )
}

fn validate_layer(
    layout: &[LayoutKey],
    layer: &LayerSource,
    base_layer: &LayerSource,
    encoder_count: usize,
    pixels_per_unit: i64,
) -> Result<()> {
    if layout.is_empty() {
        bail!("Layout must contain at least one key");
    }
    if pixels_per_unit <= 0 {
        bail!("Pixels per unit must be positive");
    }
    if layer.keys.len() != layout.len() {
        bail!(
            "Layer has {} keys, layout has {}",
            layer.keys.len(),
            layout.len()
        );
    }
    if layer.encoders.len() != encoder_count || base_layer.encoders.len() != encoder_count {
        bail!(
            "Layer has {} encoders and the base layer has {}, keyboard.json defines {}",
            layer.encoders.len(),
            base_layer.encoders.len(),
            encoder_count
        );
    }
    if layout.iter().any(|key| key.r != 0.0) {
        bail!("Rotated QMK layouts are not supported yet");
    }
    if layout.iter().any(|key| {
        !key.x.is_finite()
            || !key.y.is_finite()
            || !key.w.is_finite()
            || !key.h.is_finite()
            || key.w <= 0.0
            || key.h <= 0.0
    }) {
        bail!("Layout coordinates must be finite and key sizes must be positive");
    }
    Ok(())
}

fn resolve_encoder_placements(
    keyboard: &KeyboardJson,
    config: &KeyboardConfig,
    layout: &[LayoutKey],
) -> Result<Vec<EncoderPlacementResolved>> {
    let encoder_count = keyboard.encoder_count();
    if config.encoders.len() != encoder_count {
        bail!(
            "config.json defines {} encoder placements, keyboard.json defines {}",
            config.encoders.len(),
            encoder_count
        );
    }

    let matrix_to_index: HashMap<(u8, u8), usize> = layout
        .iter()
        .enumerate()
        .map(|(index, key)| (key.matrix, index))
        .collect();

    let mut placements = Vec::with_capacity(config.encoders.len());
    for placement in &config.encoders {
        placements.push(resolve_encoder_placement(
            placement,
            &matrix_to_index,
            layout,
        )?);
    }

    let key_indices: Vec<usize> = placements.iter().filter_map(|(index, ..)| *index).collect();
    let unique: HashSet<usize> = key_indices.iter().copied().collect();
    if key_indices.len() != unique.len() {
        bail!("Multiple encoders use the same matrix position");
    }
    Ok(placements)
}

fn resolve_encoder_placement(
    placement: &EncoderPlacement,
    matrix_to_index: &HashMap<(u8, u8), usize>,
    layout: &[LayoutKey],
) -> Result<EncoderPlacementResolved> {
    placement.validate()?;
    if let Some(matrix) = placement.matrix {
        let key_index = *matrix_to_index
            .get(&matrix)
            .with_context(|| format!("Encoder matrix position {matrix:?} is not in the layout"))?;
        let key = &layout[key_index];
        return Ok((Some(key_index), key.x, key.y, key.w, key.h));
    }
    let x = placement.x.unwrap();
    let y = placement.y.unwrap();
    if !x.is_finite() || !y.is_finite() {
        bail!("Encoder coordinates must be finite");
    }
    Ok((None, x, y, 1.0, 1.0))
}

#[allow(clippy::too_many_arguments)]
fn build_model(
    layout: &[LayoutKey],
    display_keys: &[String],
    raw_keys: &[String],
    placements: &[EncoderPlacementResolved],
    display_encoders: &[[String; 2]],
    raw_encoders: &[[String; 2]],
    display_labels: &HashMap<String, String>,
    generic_labels: &HashMap<&str, &str>,
    layer_index: u8,
    pixels_per_unit: i64,
) -> Result<OverlayModel> {
    let mut bounds: Vec<(f64, f64, f64, f64)> = layout
        .iter()
        .map(|key| (key.x, key.y, key.w, key.h))
        .collect();
    bounds.extend(placements.iter().map(|(_, x, y, w, h)| (*x, *y, *w, *h)));
    let min_x = bounds
        .iter()
        .map(|(x, ..)| *x)
        .fold(f64::INFINITY, f64::min);
    let min_y = bounds
        .iter()
        .map(|(_, y, ..)| *y)
        .fold(f64::INFINITY, f64::min);
    let max_x = bounds
        .iter()
        .map(|(x, _, w, _)| x + w)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = bounds
        .iter()
        .map(|(_, y, _, h)| y + h)
        .fold(f64::NEG_INFINITY, f64::max);
    let (width, height) = canvas_size(min_x, min_y, max_x, max_y, pixels_per_unit);

    let encoder_key_indices: HashSet<usize> =
        placements.iter().filter_map(|(index, ..)| *index).collect();

    let mut keys = Vec::new();
    for (key_index, key) in layout.iter().enumerate() {
        if encoder_key_indices.contains(&key_index) {
            continue;
        }
        let box_ = inset_box(
            pixel_box(key.x, key.y, key.w, key.h, min_x, min_y, pixels_per_unit),
            KEY_INSET,
        );
        let (left, top, right, bottom) = box_;
        let keycode = &display_keys[key_index];
        let raw_keycode = &raw_keys[key_index];
        // Held styling follows the displayed fallthrough key, while metadata
        // describing a layer switch always follows the raw key at this layer.
        keys.push(DisplayKey {
            x: left as u32,
            y: top as u32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
            label: wrap_label(
                &format_keycode(keycode, display_labels, generic_labels),
                3,
                10,
            ),
            held: momentary_layer(keycode) == Some(layer_index),
            transparent: is_transparent(raw_keycode),
            momentary_layer: momentary_layer(raw_keycode),
        });
    }

    let mut encoders = Vec::new();
    for (encoder_index, (key_index, x, y, key_width, key_height)) in placements.iter().enumerate() {
        let box_ = pixel_box(
            *x,
            *y,
            *key_width,
            *key_height,
            min_x,
            min_y,
            pixels_per_unit,
        );
        let press = key_index.map_or("KC_NO".to_string(), |index| display_keys[index].clone());
        let raw_press = key_index.map_or("KC_NO".to_string(), |index| raw_keys[index].clone());
        let (left, top, right, bottom) = inset_box(square_box(box_), 2);
        let directions = &display_encoders[encoder_index];
        encoders.push(DisplayEncoder {
            x: left as u32,
            y: top as u32,
            size: ((right - left).min(bottom - top)) as u32,
            counter_clockwise: wrap_label(
                &format_keycode(&directions[0], display_labels, generic_labels),
                2,
                5,
            ),
            clockwise: wrap_label(
                &format_keycode(&directions[1], display_labels, generic_labels),
                2,
                5,
            ),
            press: format_keycode(&press, display_labels, generic_labels),
            held: momentary_layer(&press) == Some(layer_index),
            counter_clockwise_transparent: is_transparent(&raw_encoders[encoder_index][0]),
            clockwise_transparent: is_transparent(&raw_encoders[encoder_index][1]),
            press_transparent: is_transparent(&raw_press),
            momentary_layer: momentary_layer(&raw_press),
        });
    }

    Ok(OverlayModel {
        version: 2,
        layer: layer_index,
        width,
        height,
        header_font_size: (pixels_per_unit / 4).max(14) as f64,
        key_font_size: (pixels_per_unit / 5).max(10) as f64,
        encoder_font_size: (pixels_per_unit / 6).max(10) as f64,
        keys,
        encoders,
    })
}

fn canvas_size(min_x: f64, min_y: f64, max_x: f64, max_y: f64, pixels_per_unit: i64) -> (u32, u32) {
    let width = round((max_x - min_x) * pixels_per_unit as f64) + 2 * PADDING;
    let height = round((max_y - min_y) * pixels_per_unit as f64) + 2 * PADDING + HEADER_HEIGHT;
    (width as u32, height as u32)
}

fn pixel_box(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    min_x: f64,
    min_y: f64,
    pixels_per_unit: i64,
) -> (i64, i64, i64, i64) {
    let left = round((x - min_x) * pixels_per_unit as f64) + PADDING;
    let top = round((y - min_y) * pixels_per_unit as f64) + PADDING + HEADER_HEIGHT;
    (
        left,
        top,
        left + round(width * pixels_per_unit as f64),
        top + round(height * pixels_per_unit as f64),
    )
}

/// Matches Python's `round()` (ties to even), not Rust's default
/// round-half-away-from-zero, so pixel geometry stays identical to the
/// existing Python-rendered output.
fn round(value: f64) -> i64 {
    value.round_ties_even() as i64
}

fn wrap_label(label: &str, max_lines: usize, max_chars: usize) -> Vec<String> {
    if label.is_empty() {
        return Vec::new();
    }
    let lines = wrap_text(label, max_chars);
    if lines.len() <= max_lines {
        return lines;
    }
    let mut truncated: Vec<String> = lines[..max_lines - 1].to_vec();
    let last = &lines[max_lines - 1];
    let cut = last
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    truncated.push(format!("{cut}..."));
    truncated
}

/// A simplified greedy word-wrap that breaks on whitespace and hard-breaks
/// words longer than the full line width.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        for chunk in break_long_word(word, width) {
            if current.is_empty() {
                current = chunk;
            } else if current.chars().count() + 1 + chunk.chars().count() <= width {
                current.push(' ');
                current.push_str(&chunk);
            } else {
                lines.push(std::mem::take(&mut current));
                current = chunk;
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn break_long_word(word: &str, width: usize) -> Vec<String> {
    if width == 0 || word.chars().count() <= width {
        return vec![word.to_string()];
    }
    word.chars()
        .collect::<Vec<_>>()
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn format_keycode(
    keycode: &str,
    display_labels: &HashMap<String, String>,
    generic_labels: &HashMap<&str, &str>,
) -> String {
    let keycode = keycode.trim();
    if let Some(label) = display_labels.get(keycode) {
        return label.clone();
    }
    if is_transparent(keycode) {
        return String::new();
    }
    if let Some(label) = generic_labels.get(keycode) {
        return (*label).to_string();
    }
    if let Some(layer) = momentary_layer(keycode) {
        return format!("L{layer}");
    }
    let stripped = keycode
        .strip_prefix("KC_")
        .or_else(|| keycode.strip_prefix("QK_"))
        .unwrap_or(keycode);
    stripped.replace('_', " ")
}

fn momentary_layer(keycode: &str) -> Option<u8> {
    let compact = keycode.replace(' ', "");
    let inner = compact.strip_prefix("MO(")?.strip_suffix(")")?;
    inner.parse().ok()
}

fn inset_box(box_: (i64, i64, i64, i64), inset: i64) -> (i64, i64, i64, i64) {
    let (left, top, right, bottom) = box_;
    (left + inset, top + inset, right - inset, bottom - inset)
}

fn center(box_: (i64, i64, i64, i64)) -> (i64, i64) {
    let (left, top, right, bottom) = box_;
    ((left + right).div_euclid(2), (top + bottom).div_euclid(2))
}

fn square_box(box_: (i64, i64, i64, i64)) -> (i64, i64, i64, i64) {
    let (left, top, right, bottom) = box_;
    let size = (right - left).min(bottom - top);
    let (center_x, center_y) = center(box_);
    let half = size.div_euclid(2);
    (
        center_x - half,
        center_y - half,
        center_x + half,
        center_y + half,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyboard(json: &str) -> KeyboardJson {
        serde_json::from_str(json).expect("valid keyboard.json fixture")
    }

    fn config(json: &str) -> KeyboardConfig {
        serde_json::from_str(json).expect("valid config.json fixture")
    }

    fn two_key_keyboard_with_encoder() -> KeyboardJson {
        keyboard(
            r#"{
                "usb": {"vid": "0x0001", "pid": "0x0002"},
                "layouts": {"LAYOUT": {"layout": [
                    {"x": 0, "y": 0, "matrix": [0, 0]},
                    {"x": 1, "y": 0, "matrix": [0, 1]}
                ]}},
                "encoder": {"rotary": [{"pin_a": "B0", "pin_b": "B1"}]}
            }"#,
        )
    }

    fn labels(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn layer(keys: &[&str], encoders: &[[&str; 2]]) -> LayerSource {
        LayerSource {
            keys: keys.iter().map(|k| k.to_string()).collect(),
            encoders: encoders
                .iter()
                .map(|[ccw, cw]| [ccw.to_string(), cw.to_string()])
                .collect(),
        }
    }

    /// Mirrors model/tests/test_generate_overlay_asset.py's
    /// test_builds_keys_and_an_encoder_into_the_shared_model, including its
    /// exact expected (168, 142) canvas size, as a parity check against the
    /// existing Python-rendered output.
    #[test]
    fn builds_keys_and_an_encoder_into_the_shared_model() {
        let keyboard = two_key_keyboard_with_encoder();
        let config = config(r#"{"encoders": [{"matrix": [0, 1]}]}"#);
        let base = layer(&["KC_A", "KC_MUTE"], &[["KC_VOLD", "KC_VOLU"]]);
        let layer1 = layer(&["KC_ALPHA", "KC_MUTE"], &[["KC_TRNS", "KC_TRNS"]]);
        let display_labels = labels(&[("KC_ALPHA", "α")]);

        let model = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            1,
            &base,
            &layer1,
            &display_labels,
            Platform::Macos,
            64,
        )
        .expect("model");

        assert_eq!(model.version, 2);
        assert_eq!((model.width, model.height), (168, 142));
        assert_eq!(model.keys[0].label, vec!["α".to_string()]);
        assert!(!model.keys[0].transparent);
        assert_eq!(
            model.encoders[0].counter_clockwise,
            vec!["VOL -".to_string()]
        );
        assert_eq!(model.encoders[0].clockwise, vec!["VOL +".to_string()]);
        assert!(model.encoders[0].counter_clockwise_transparent);
        assert!(model.encoders[0].clockwise_transparent);
        assert_eq!(model.encoders[0].press, "MUTE");
    }

    #[test]
    fn platform_labels_come_from_the_built_in_tables() {
        let keyboard = keyboard(
            r#"{
                "usb": {"vid": "0x0001", "pid": "0x0002"},
                "layouts": {"LAYOUT": {"layout": [{"x": 0, "y": 0, "matrix": [0, 0]}]}}
            }"#,
        );
        let config = config("{}");
        let base = layer(&["KC_LGUI"], &[]);
        let none = HashMap::new();

        let macos = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            0,
            &base,
            &base,
            &none,
            Platform::Macos,
            64,
        )
        .expect("model");
        assert_eq!(macos.keys[0].label, vec!["⌘".to_string()]);

        let linux = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            0,
            &base,
            &base,
            &none,
            Platform::Linux,
            64,
        )
        .expect("model");
        assert_eq!(linux.keys[0].label, vec!["Super".to_string()]);

        let windows = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            0,
            &base,
            &base,
            &none,
            Platform::Windows,
            64,
        )
        .expect("model");
        assert_eq!(windows.keys[0].label, vec!["⊞".to_string()]);
    }

    #[test]
    fn a_custom_keycode_label_overrides_the_platform_table() {
        let keyboard = keyboard(
            r#"{
                "usb": {"vid": "0x0001", "pid": "0x0002"},
                "layouts": {"LAYOUT": {"layout": [{"x": 0, "y": 0, "matrix": [0, 0]}]}}
            }"#,
        );
        let config = config("{}");
        let base = layer(&["KC_LGUI"], &[]);
        let display_labels = labels(&[("KC_LGUI", "★")]);

        let model = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            0,
            &base,
            &base,
            &display_labels,
            Platform::Macos,
            64,
        )
        .expect("model");

        assert_eq!(model.keys[0].label, vec!["★".to_string()]);
    }

    /// A transparent key displays the base layer's key but still reports
    /// itself as transparent — mirrors
    /// test_resolves_display_layer_without_changing_raw_keymap.
    #[test]
    fn a_transparent_key_displays_the_base_layer_but_reports_transparent() {
        let keyboard = keyboard(
            r#"{
                "usb": {"vid": "0x0001", "pid": "0x0002"},
                "layouts": {"LAYOUT": {"layout": [{"x": 0, "y": 0, "matrix": [0, 0]}]}}
            }"#,
        );
        let config = config("{}");
        let base = layer(&["KC_ALPHA"], &[]);
        let layer1 = layer(&["KC_TRNS"], &[]);
        let display_labels = labels(&[("KC_ALPHA", "α")]);

        let model = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            1,
            &base,
            &layer1,
            &display_labels,
            Platform::Macos,
            64,
        )
        .expect("model");

        assert_eq!(model.keys[0].label, vec!["α".to_string()]);
        assert!(model.keys[0].transparent);
    }

    #[test]
    fn encoder_placement_count_must_match_keyboard() {
        let keyboard = two_key_keyboard_with_encoder();
        let config = config("{}");
        let base = layer(&["KC_A", "KC_B"], &[["KC_VOLD", "KC_VOLU"]]);

        let error = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            0,
            &base,
            &base,
            &HashMap::new(),
            Platform::Macos,
            64,
        )
        .expect_err("encoder count mismatch");

        assert!(error.to_string().contains("encoder placements"));
    }

    #[test]
    fn encoder_action_count_must_match_keyboard() {
        let keyboard = two_key_keyboard_with_encoder();
        let config = config(r#"{"encoders": [{"matrix": [0, 1]}]}"#);
        let base = layer(&["KC_A", "KC_B"], &[]);

        let error = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            0,
            &base,
            &base,
            &HashMap::new(),
            Platform::Macos,
            64,
        )
        .expect_err("encoder action mismatch");

        assert!(error.to_string().contains("keyboard.json defines 1"));
    }

    #[test]
    fn rotated_layouts_are_rejected() {
        let keyboard = keyboard(
            r#"{
                "usb": {"vid": "0x0001", "pid": "0x0002"},
                "layouts": {"LAYOUT": {"layout": [{"x": 0, "y": 0, "matrix": [0, 0], "r": 15}]}}
            }"#,
        );
        let config = config("{}");
        let base = layer(&["KC_A"], &[]);

        let error = build_layer_model(
            &keyboard,
            &config,
            "LAYOUT",
            0,
            &base,
            &base,
            &HashMap::new(),
            Platform::Macos,
            64,
        )
        .expect_err("rotated layout");

        assert!(error.to_string().contains("Rotated"));
    }

    #[test]
    fn empty_layouts_are_rejected() {
        let keyboard = keyboard(
            r#"{
                "usb": {"vid": "0x0001", "pid": "0x0002"},
                "layouts": {"LAYOUT": {"layout": []}}
            }"#,
        );
        let base = layer(&[], &[]);

        let error = build_layer_model(
            &keyboard,
            &config("{}"),
            "LAYOUT",
            0,
            &base,
            &base,
            &HashMap::new(),
            Platform::Macos,
            64,
        )
        .expect_err("empty layout");

        assert!(error.to_string().contains("at least one key"));
    }

    #[test]
    fn non_positive_key_sizes_are_rejected() {
        let keyboard = keyboard(
            r#"{
                "usb": {"vid": "0x0001", "pid": "0x0002"},
                "layouts": {"LAYOUT": {"layout": [
                    {"x": 0, "y": 0, "w": 0, "matrix": [0, 0]}
                ]}}
            }"#,
        );
        let base = layer(&["KC_A"], &[]);

        let error = build_layer_model(
            &keyboard,
            &config("{}"),
            "LAYOUT",
            0,
            &base,
            &base,
            &HashMap::new(),
            Platform::Macos,
            64,
        )
        .expect_err("zero-width key");

        assert!(error.to_string().contains("sizes must be positive"));
    }

    #[test]
    fn momentary_layer_keys_render_as_l_n() {
        assert_eq!(momentary_layer("MO(2)"), Some(2));
        assert_eq!(momentary_layer("MO( 2 )"), Some(2));
        assert_eq!(momentary_layer("KC_A"), None);
        assert_eq!(
            format_keycode("MO(2)", &HashMap::new(), &keycode_labels()),
            "L2"
        );
    }

    #[test]
    fn unlabeled_keycodes_fall_back_to_a_stripped_prefix() {
        let none = HashMap::new();
        let generic = keycode_labels();
        assert_eq!(format_keycode("KC_A", &none, &generic), "A");
        assert_eq!(format_keycode("QK_BOOT", &none, &generic), "QK BOOT");
        assert_eq!(format_keycode("KC_NO", &none, &generic), "");
        assert_eq!(format_keycode("KC_TRNS", &none, &generic), "");
    }

    #[test]
    fn wrap_label_truncates_with_an_ellipsis() {
        assert_eq!(wrap_label("", 3, 10), Vec::<String>::new());
        assert_eq!(wrap_label("BRIGHT -", 3, 10), vec!["BRIGHT -".to_string()]);
        let wrapped = wrap_label("SOMETHING VERY LONG INDEED", 2, 5);
        assert_eq!(wrapped.len(), 2);
        assert!(wrapped[1].ends_with("..."));
    }
}
