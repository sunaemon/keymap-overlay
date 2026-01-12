use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyboardConfig {
    pub qmk_keyboard: String,
}

pub struct ProjectContext {
    pub root_dir: PathBuf,
    pub keyboards_dir: PathBuf,
}

impl ProjectContext {
    pub fn new<P: AsRef<Path>, Q: AsRef<Path>>(root: P, keyboards_dir: Q) -> Self {
        let root_dir = root.as_ref().to_path_buf();
        let keyboards_dir = keyboards_dir.as_ref().to_path_buf();
        Self {
            root_dir,
            keyboards_dir,
        }
    }

    pub fn get_keyboard_config(
        &self,
        keyboard_id: &str,
    ) -> Result<KeyboardConfig, Box<dyn std::error::Error>> {
        let config_path = self.keyboards_dir.join(keyboard_id).join("config.json");
        let content = fs::read_to_string(&config_path)?;
        let config: KeyboardConfig = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn get_build_dir(&self, keyboard_id: &str) -> PathBuf {
        self.root_dir.join("build").join(keyboard_id)
    }
}
