use serde::Deserialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize)]
pub struct CustomKeycode {
    pub name: String,
    #[serde(default, rename = "shortName")]
    pub short_name: String,
}

pub fn parse_custom_keycodes(vial_meta: &serde_json::Value) -> anyhow::Result<Vec<CustomKeycode>> {
    match vial_meta.get("customKeycodes") {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(value) => Ok(serde_json::from_value(value.clone())?),
    }
}

/// Keyed by name (e.g. "KC_ALPHA"), matching how a resolved keycode string is
/// looked up during rendering.
pub fn custom_keycode_labels(custom_keycodes: &[CustomKeycode]) -> HashMap<String, String> {
    custom_keycodes
        .iter()
        .filter(|keycode| !keycode.short_name.is_empty())
        .map(|keycode| (keycode.name.clone(), keycode.short_name.clone()))
        .collect()
}
