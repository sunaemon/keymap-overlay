use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Layout {
    pub layout: Vec<LayoutKey>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UsbConfig {
    pub vid: String,
    pub pid: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EncoderConfig {
    pub rotary: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KeyboardConfig {
    #[serde(default)]
    pub qmk_keyboard: String,
    #[serde(default)]
    pub encoders: Vec<EncoderPlacement>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EncoderPlacement {
    #[serde(default)]
    pub matrix: Option<(u8, u8)>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeymapOverlayMetadata {
    pub keyboard_id: u8,
    pub layout_name: String,
    pub pixels_per_unit: i64,
    pub keyboard: KeyboardJson,
    pub config: KeyboardConfig,
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

#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OverlayModel {
    #[cfg_attr(feature = "contract-schema", schemars(range(min = 2, max = 2)))]
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

#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DisplayKey {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub label: Vec<String>,
    pub held: bool,
    #[serde(default)]
    pub transparent: bool,
    #[serde(default)]
    pub momentary_layer: Option<u8>,
}

#[cfg_attr(feature = "contract-schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DisplayEncoder {
    pub x: u32,
    pub y: u32,
    pub size: u32,
    pub counter_clockwise: Vec<String>,
    pub clockwise: Vec<String>,
    pub press: String,
    pub held: bool,
    #[serde(default)]
    pub counter_clockwise_transparent: bool,
    #[serde(default)]
    pub clockwise_transparent: bool,
    #[serde(default)]
    pub press_transparent: bool,
    #[serde(default)]
    pub momentary_layer: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct KeyboardModels {
    pub keyboard_id: u8,
    pub layers: HashMap<u8, OverlayModel>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn older_display_models_default_composition_metadata() {
        let key: DisplayKey = serde_json::from_value(json!({
            "x": 0,
            "y": 0,
            "width": 10,
            "height": 10,
            "label": ["A"],
            "held": false
        }))
        .expect("display key without composition metadata");
        let encoder: DisplayEncoder = serde_json::from_value(json!({
            "x": 0,
            "y": 0,
            "size": 10,
            "counter_clockwise": ["LEFT"],
            "clockwise": ["RIGHT"],
            "press": "PLAY",
            "held": false
        }))
        .expect("display encoder without composition metadata");

        assert!(!key.transparent);
        assert_eq!(key.momentary_layer, None);
        assert!(!encoder.counter_clockwise_transparent);
        assert!(!encoder.clockwise_transparent);
        assert!(!encoder.press_transparent);
        assert_eq!(encoder.momentary_layer, None);
    }

    #[test]
    fn encoder_placement_rejects_mixed_position_sources() {
        let placement = EncoderPlacement {
            matrix: Some((0, 1)),
            x: Some(2.0),
            y: None,
        };

        assert_eq!(
            placement
                .validate()
                .expect_err("mixed placement")
                .to_string(),
            "encoder placement cannot mix matrix and coordinates"
        );
    }

    #[test]
    fn encoder_placement_requires_a_complete_position() {
        let placement = EncoderPlacement {
            matrix: None,
            x: Some(2.0),
            y: None,
        };

        assert_eq!(
            placement
                .validate()
                .expect_err("incomplete placement")
                .to_string(),
            "encoder placement requires matrix or both x and y"
        );
    }

    #[test]
    fn layout_sizes_and_valid_encoder_positions_use_their_defaults() {
        let key: LayoutKey = serde_json::from_value(json!({
            "x": 0.0,
            "y": 1.0,
            "matrix": [2, 3]
        }))
        .expect("layout key with default size");
        let matrix_placement = EncoderPlacement {
            matrix: Some((2, 3)),
            x: None,
            y: None,
        };
        let coordinate_placement = EncoderPlacement {
            matrix: None,
            x: Some(0.0),
            y: Some(1.0),
        };

        assert_eq!((key.w, key.h), (1.0, 1.0));
        matrix_placement.validate().expect("matrix placement");
        coordinate_placement
            .validate()
            .expect("coordinate placement");
    }

    #[test]
    fn missing_layout_names_report_the_requested_name() {
        let keyboard = KeyboardJson {
            layouts: HashMap::new(),
            usb: UsbConfig {
                vid: "0x1234".to_owned(),
                pid: "0x5678".to_owned(),
            },
            encoder: None,
        };

        assert_eq!(
            keyboard
                .layout_keys("LAYOUT_missing")
                .expect_err("missing layout")
                .to_string(),
            "Layout LAYOUT_missing not found in keyboard.json"
        );
    }
}
