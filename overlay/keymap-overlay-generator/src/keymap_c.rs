use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

const CUSTOM_KEYCODE_BASE_NAMES: [&str; 3] = ["SAFE_RANGE", "QK_USER_0", "QK_KB_0"];
const ENCODER_PAIR_NAME: &str = "ENCODER_CCW_CW";

pub fn strip_c_comments(text: &str) -> String {
    static BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").unwrap());
    static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"//[^\n]*").unwrap());
    let text = BLOCK.replace_all(text, "");
    LINE.replace_all(&text, "").into_owned()
}

/// Return `enum custom_keycodes` member names from keymap.c, in declared order.
pub fn parse_custom_keycode_names(keymap_c: &str) -> Result<Vec<String>> {
    static ENUM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)enum\s+custom_keycodes\s*\{([^}]*)\};").unwrap());
    let captures = ENUM
        .captures(keymap_c)
        .context("enum custom_keycodes not found in keymap.c")?;
    let body = strip_c_comments(&captures[1]);

    let mut names = Vec::new();
    for entry in body.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((name, value)) = entry.split_once('=') {
            let value = value.trim();
            if !CUSTOM_KEYCODE_BASE_NAMES.contains(&value) {
                bail!("Explicit keycode assignment is not supported: {entry}");
            }
            names.push(name.trim().to_string());
        } else {
            names.push(entry.to_string());
        }
    }
    Ok(names)
}

/// Parse QMK encoder bindings from keymap.c. Returns one pair list per layer,
/// or an empty list if there is no `encoder_map` at all.
///
/// Only numeric `[N] = {...}` layer designators are supported — neither of
/// this project's keyboards uses symbolic enum designators. `keymap.c` files
/// that do need `VIAL=false` Python rendering, which still handles them.
pub fn parse_encoder_map(keymap_c: &str) -> Result<Vec<Vec<[String; 2]>>> {
    let content = strip_c_comments(keymap_c);
    static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bencoder_map\b").unwrap());
    let Some(name_match) = NAME.find(&content) else {
        return Ok(Vec::new());
    };
    let equals = content[name_match.end()..]
        .find('=')
        .map(|i| i + name_match.end());
    let opening = equals.and_then(|eq| content[eq..].find('{').map(|i| i + eq));
    let opening = match (equals, opening) {
        (Some(_), Some(opening)) => opening,
        _ => bail!("Malformed encoder_map in keymap.c"),
    };
    let (body, _) = extract_delimited(&content, opening, '{', '}')?;
    parse_encoder_layers(&body)
}

fn parse_encoder_layers(content: &str) -> Result<Vec<Vec<[String; 2]>>> {
    static LAYER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\[([A-Za-z_]\w*|\d+)\]\s*=").unwrap());
    let mut indexed: HashMap<usize, Vec<[String; 2]>> = HashMap::new();
    let mut position = 0usize;
    while let Some(caps) = LAYER.captures_at(content, position) {
        let full = caps.get(0).unwrap();
        let designator = &caps[1];
        let opening = content[full.end()..]
            .find('{')
            .map(|i| i + full.end())
            .context("Malformed encoder_map layer in keymap.c")?;
        let (body, next_position) = extract_delimited(content, opening, '{', '}')?;
        let layer_index: usize = designator.parse().map_err(|_| {
            anyhow!(
                "encoder_map layer designator '{designator}' is not numeric; \
                 symbolic/enum designators are not supported here — use VIAL=false rendering instead"
            )
        })?;
        if indexed.contains_key(&layer_index) {
            bail!("Duplicate encoder_map layer {layer_index} in keymap.c");
        }
        indexed.insert(layer_index, parse_encoder_pairs(&body)?);
        position = next_position;
    }
    if indexed.is_empty() {
        bail!("encoder_map has no layer designators in keymap.c");
    }
    let max_index = *indexed.keys().max().unwrap();
    let mut layers = vec![Vec::new(); max_index + 1];
    for (index, pairs) in indexed {
        layers[index] = pairs;
    }
    Ok(layers)
}

fn parse_encoder_pairs(content: &str) -> Result<Vec<[String; 2]>> {
    let mut pairs = Vec::new();
    let mut position = 0usize;
    while let Some(offset) = content[position..].find(ENCODER_PAIR_NAME) {
        let start = position + offset;
        let after_name = start + ENCODER_PAIR_NAME.len();
        let opening = content[after_name..]
            .find('(')
            .map(|i| i + after_name)
            .with_context(|| format!("Malformed {ENCODER_PAIR_NAME} in keymap.c"))?;
        let (arguments, next_position) = extract_delimited(content, opening, '(', ')')?;
        pairs.push(split_pair(&arguments)?);
        position = next_position;
    }
    Ok(pairs)
}

fn split_pair(arguments: &str) -> Result<[String; 2]> {
    let mut depth = 0i32;
    let mut separators = Vec::new();
    for (index, ch) in arguments.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => separators.push(index),
            _ => {}
        }
    }
    if separators.len() != 1 || depth != 0 {
        bail!("Malformed {ENCODER_PAIR_NAME} arguments in keymap.c");
    }
    let separator = separators[0];
    let first = arguments[..separator].trim().to_string();
    let second = arguments[separator + 1..].trim().to_string();
    if first.is_empty() || second.is_empty() {
        bail!("Empty {ENCODER_PAIR_NAME} argument in keymap.c");
    }
    Ok([first, second])
}

/// Return text within one balanced delimiter pair (exclusive) and the byte
/// offset just past its closing delimiter.
fn extract_delimited(
    content: &str,
    opening_index: usize,
    opening: char,
    closing: char,
) -> Result<(String, usize)> {
    let mut depth = 0i32;
    for (index, ch) in content[opening_index..].char_indices() {
        let absolute = opening_index + index;
        if ch == opening {
            depth += 1;
        } else if ch == closing {
            depth -= 1;
            if depth == 0 {
                return Ok((
                    content[opening_index + opening.len_utf8()..absolute].to_string(),
                    absolute + closing.len_utf8(),
                ));
            }
        }
    }
    bail!("Unclosed {opening} in encoder_map")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_custom_keycode_names_in_order() {
        let keymap_c = r#"
        enum custom_keycodes {
          KC_ALPHA = SAFE_RANGE, // α
          KC_BETA,               // β
        };
        "#;
        assert_eq!(
            parse_custom_keycode_names(keymap_c).unwrap(),
            vec!["KC_ALPHA".to_string(), "KC_BETA".to_string()]
        );
    }

    #[test]
    fn rejects_an_explicit_non_base_assignment() {
        let keymap_c = "enum custom_keycodes { KC_ALPHA = 5 };";
        assert!(parse_custom_keycode_names(keymap_c).is_err());
    }

    #[test]
    fn no_encoder_map_returns_empty() {
        assert_eq!(
            parse_encoder_map("enum custom_keycodes { KC_A };").unwrap(),
            Vec::<Vec<[String; 2]>>::new()
        );
    }

    #[test]
    fn parses_numeric_layer_designators() {
        let keymap_c = r#"
        const uint16_t PROGMEM encoder_map[2][1][2] = {
            [0] = {ENCODER_CCW_CW(KC_VOLD, KC_VOLU)},
            [1] = {ENCODER_CCW_CW(KC_TRNS, KC_TRNS)},
        };
        "#;
        assert_eq!(
            parse_encoder_map(keymap_c).unwrap(),
            vec![
                vec![["KC_VOLD".to_string(), "KC_VOLU".to_string()]],
                vec![["KC_TRNS".to_string(), "KC_TRNS".to_string()]],
            ]
        );
    }

    #[test]
    fn preserves_nested_keycode_arguments() {
        let keymap_c = r#"
        const uint16_t PROGMEM encoder_map[1][1][2] = {
            [0] = {ENCODER_CCW_CW(LCTL(KC_Z), LT(1, KC_X))},
        };
        "#;
        assert_eq!(
            parse_encoder_map(keymap_c).unwrap(),
            vec![vec![["LCTL(KC_Z)".to_string(), "LT(1, KC_X)".to_string()]]]
        );
    }

    #[test]
    fn rejects_a_symbolic_layer_designator() {
        let keymap_c = r#"
        const uint16_t PROGMEM encoder_map[1][1][2] = {
            [BASE] = {ENCODER_CCW_CW(KC_VOLD, KC_VOLU)},
        };
        "#;
        let error = parse_encoder_map(keymap_c).unwrap_err();
        assert!(error.to_string().contains("not numeric"));
    }

    #[test]
    fn rejects_empty_encoder_actions() {
        let keymap_c = r#"
        const uint16_t PROGMEM encoder_map[1][1][2] = {
            [0] = {ENCODER_CCW_CW(, KC_VOLU)},
        };
        "#;
        assert!(parse_encoder_map(keymap_c).is_err());
    }
}
