//! Versioned display-model contract shared by every native renderer.

use crate::types::OverlayModel;
use anyhow::{Context, Result};
use std::collections::HashMap;

pub const DISPLAY_MODEL_VERSION: u8 = 2;

const FIXTURE_JSON: [&str; 3] = [
    include_str!("../../display-model-contract/fixtures/base-v2.json"),
    include_str!("../../display-model-contract/fixtures/layer-v2.json"),
    include_str!("../../display-model-contract/fixtures/encoder-v2.json"),
];

/// Loads every checked-in renderer contract fixture through the canonical type.
pub fn fixtures() -> Result<Vec<OverlayModel>> {
    FIXTURE_JSON
        .iter()
        .map(|fixture| {
            serde_json::from_str(fixture).context("Failed to parse a display-model fixture")
        })
        .collect()
}

/// Builds the base and held-layer models used by every native renderer's E2E path.
pub fn simulation_models(keyboard_id: u8, layer: u8) -> Result<HashMap<(u8, u8), OverlayModel>> {
    let models = fixtures()?;
    let mut base = models[0].clone();
    let mut held = models[1].clone();
    retarget_layer(&mut base, 0, layer);
    retarget_layer(&mut held, layer, layer);

    Ok(HashMap::from([
        ((keyboard_id, 0), base),
        ((keyboard_id, layer), held),
    ]))
}

/// Generates the checked-in JSON Schema from the canonical Rust model.
#[cfg(feature = "contract-schema")]
pub fn schema_json() -> Result<String> {
    let schema = schemars::schema_for!(OverlayModel);
    serde_json::to_string_pretty(&schema).context("Failed to serialize the display-model schema")
}

fn retarget_layer(model: &mut OverlayModel, model_layer: u8, momentary_layer: u8) {
    model.layer = model_layer;
    for key in &mut model.keys {
        if key.momentary_layer.is_some() {
            key.momentary_layer = Some(momentary_layer);
        }
    }
    for encoder in &mut model.encoders {
        if encoder.momentary_layer.is_some() {
            encoder.momentary_layer = Some(momentary_layer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fixture_is_current_and_round_trips_losslessly() {
        for (source, model) in FIXTURE_JSON.iter().zip(fixtures().expect("valid fixtures")) {
            assert_eq!(model.version, DISPLAY_MODEL_VERSION);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(source).expect("fixture JSON"),
                serde_json::to_value(model).expect("serialized fixture")
            );
        }
    }

    #[test]
    fn simulation_models_retarget_the_shared_renderer_fixtures() {
        let models = simulation_models(12, 3).expect("simulation models");

        assert_eq!(models.keys().copied().collect::<Vec<_>>().len(), 2);
        assert_eq!(models[&(12, 0)].layer, 0);
        assert_eq!(models[&(12, 0)].keys[0].momentary_layer, Some(3));
        assert_eq!(models[&(12, 3)].layer, 3);
        assert_eq!(models[&(12, 3)].keys[0].momentary_layer, Some(3));
    }

    #[test]
    fn retarget_layer_updates_encoder_momentary_actions() {
        let mut encoder = fixtures().expect("fixtures")[2].clone();

        retarget_layer(&mut encoder, 4, 5);

        assert_eq!(encoder.layer, 4);
        assert_eq!(encoder.encoders[0].momentary_layer, Some(5));
    }

    #[cfg(feature = "contract-schema")]
    #[test]
    fn schema_names_the_model_and_pins_the_current_version() {
        let schema: serde_json::Value =
            serde_json::from_str(&schema_json().expect("generated schema")).expect("schema JSON");

        assert_eq!(schema["title"], "OverlayModel");
        assert_eq!(schema["properties"]["version"]["minimum"], 2);
        assert_eq!(schema["properties"]["version"]["maximum"], 2);
    }
}
