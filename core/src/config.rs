use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// total volume
    master_volume: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self { master_volume: 1. }
    }
}
impl Config {
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let text = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&text)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }
}
