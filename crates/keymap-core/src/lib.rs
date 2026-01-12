use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyboardConfig {
    pub qmk_keyboard: String,
}

pub struct ProjectContext {
    root_dir: PathBuf,
    keyboards_dir: PathBuf,
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

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn keyboards_dir(&self) -> &Path {
        &self.keyboards_dir
    }

    fn validate_keyboard_id(id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if id.trim().is_empty() {
            return Err("Keyboard ID cannot be empty".into());
        }
        if id.contains("..") || id.contains('/') || id.contains('\\') {
            return Err(format!(
                "Invalid Keyboard ID '{}': Path traversal characters are not allowed",
                id
            )
            .into());
        }
        Ok(())
    }

    pub fn get_keyboard_config(
        &self,
        keyboard_id: &str,
    ) -> Result<KeyboardConfig, Box<dyn std::error::Error + Send + Sync>> {
        Self::validate_keyboard_id(keyboard_id)?;
        let config_path = self.keyboards_dir.join(keyboard_id).join("config.json");
        let content = fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config at {:?}: {}", config_path, e))?;
        let config: KeyboardConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config at {:?}: {}", config_path, e))?;
        Ok(config)
    }

    pub fn get_build_dir(
        &self,
        keyboard_id: &str,
    ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        Self::validate_keyboard_id(keyboard_id)?;
        Ok(self.root_dir.join("build").join(keyboard_id))
    }
}
