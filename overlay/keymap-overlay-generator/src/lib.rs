pub mod custom_keycodes;
pub mod device;
pub mod flash;
pub mod keymap_c;
pub mod labels;
pub mod model;
pub mod qmk_keymap;
pub mod types;

use anyhow::{Context, Result};

pub fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("Failed to parse {}", path.display()))
}
