use anyhow::{Context, Result, anyhow};
use log::debug;
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

    fn validate_keyboard_id(id: &str) -> Result<()> {
        if id.trim().is_empty() {
            return Err(anyhow!("Keyboard ID cannot be empty"));
        }
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(anyhow!(
                "Invalid Keyboard ID '{}': Only alphanumeric characters, underscores, and hyphens are allowed",
                id
            ));
        }
        Ok(())
    }

    pub fn get_keyboard_config(&self, keyboard_id: &str) -> Result<KeyboardConfig> {
        Self::validate_keyboard_id(keyboard_id)?;
        let config_path = self.keyboards_dir.join(keyboard_id).join("config.json");
        debug!("Reading keyboard config from {:?}", config_path);
        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config at {:?}", config_path))?;
        let config: KeyboardConfig = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse config at {:?}", config_path))?;
        debug!("Successfully parsed config for keyboard: {}", keyboard_id);
        Ok(config)
    }

    pub fn get_build_dir(&self, keyboard_id: &str) -> Result<PathBuf> {
        Self::validate_keyboard_id(keyboard_id)?;
        let build_dir = self.root_dir.join("build").join(keyboard_id);
        debug!(
            "Resolved build directory for {}: {:?}",
            keyboard_id, build_dir
        );
        Ok(build_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_keyboard_id() {
        assert!(ProjectContext::validate_keyboard_id("keyboard1").is_ok());
        assert!(ProjectContext::validate_keyboard_id("123").is_ok());
        assert!(ProjectContext::validate_keyboard_id("abc").is_ok());
        assert!(ProjectContext::validate_keyboard_id("key-board").is_ok());
        assert!(ProjectContext::validate_keyboard_id("key_board").is_ok());

        assert!(ProjectContext::validate_keyboard_id("").is_err());
        assert!(ProjectContext::validate_keyboard_id(" ").is_err());
        assert!(ProjectContext::validate_keyboard_id("key.board").is_err());
        assert!(ProjectContext::validate_keyboard_id("key/board").is_err());
        assert!(ProjectContext::validate_keyboard_id("key\\board").is_err());
        assert!(ProjectContext::validate_keyboard_id("..").is_err());
    }
}
