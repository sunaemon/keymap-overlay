use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize)]
pub struct KeyboardJson {
    pub layouts: HashMap<String, Layout>,
    pub usb: UsbConfig,
    #[serde(default)]
    pub encoder: Option<EncoderConfig>,
}

impl KeyboardJson {
    pub fn layout_keys(&self, layout_name: &str) -> anyhow::Result<&[LayoutKey]> {
        self.layouts
            .get(layout_name)
            .map(|layout| layout.layout.as_slice())
            .ok_or_else(|| anyhow::anyhow!("Layout {layout_name} not found in keyboard.json"))
    }

    pub fn encoder_count(&self) -> usize {
        self.encoder
            .as_ref()
            .map_or(0, |encoder| encoder.rotary.len())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Layout {
    pub layout: Vec<LayoutKey>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LayoutKey {
    pub x: f64,
    pub y: f64,
    pub matrix: (u8, u8),
    #[serde(default = "unit")]
    pub w: f64,
    #[serde(default = "unit")]
    pub h: f64,
    #[serde(default)]
    pub r: f64,
}

fn unit() -> f64 {
    1.0
}

#[derive(Clone, Debug, Deserialize)]
pub struct UsbConfig {
    pub vid: String,
    pub pid: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EncoderConfig {
    pub rotary: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct KeyboardConfig {
    #[serde(default)]
    pub encoders: Vec<EncoderPlacement>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EncoderPlacement {
    #[serde(default)]
    pub matrix: Option<(u8, u8)>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

impl EncoderPlacement {
    pub fn validate(&self) -> anyhow::Result<()> {
        let has_coordinates = self.x.is_some() || self.y.is_some();
        if self.matrix.is_some() && has_coordinates {
            anyhow::bail!("encoder placement cannot mix matrix and coordinates");
        }
        if self.matrix.is_none() && (self.x.is_none() || self.y.is_none()) {
            anyhow::bail!("encoder placement requires matrix or both x and y");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OverlayModel {
    pub version: u8,
    pub layer: u8,
    pub width: u32,
    pub height: u32,
    pub header_font_size: f64,
    pub key_font_size: f64,
    pub encoder_font_size: f64,
    pub keys: Vec<DisplayKey>,
    pub encoders: Vec<DisplayEncoder>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DisplayKey {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub label: Vec<String>,
    pub held: bool,
    pub transparent: bool,
    pub momentary_layer: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DisplayEncoder {
    pub x: u32,
    pub y: u32,
    pub size: u32,
    pub counter_clockwise: Vec<String>,
    pub clockwise: Vec<String>,
    pub press: String,
    pub held: bool,
    pub counter_clockwise_transparent: bool,
    pub clockwise_transparent: bool,
    pub press_transparent: bool,
    pub momentary_layer: Option<u8>,
}

#[derive(Serialize)]
pub struct KeyboardModels {
    pub keyboard_id: u8,
    pub layers: HashMap<u8, OverlayModel>,
}
