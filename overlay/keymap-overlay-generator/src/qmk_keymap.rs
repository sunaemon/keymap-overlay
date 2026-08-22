use serde::Deserialize;

/// Mirrors model/src/types.py's QmkKeymapJson — only `layers` is needed here.
#[derive(Clone, Debug, Deserialize)]
pub struct QmkKeymapJson {
    pub layers: Vec<Vec<String>>,
}
