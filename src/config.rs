use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};

/// the app config / settings directory. On a PC (Linux) =
/// `~/.config/dream_player`; user settings go here as well.
pub fn config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "dream_player").map(|d| d.config_dir().to_path_buf())
}

/// the user settings directory. For now it matches [`config_dir`]
/// (everything lives in one place), but it is kept separate — later settings
/// and config may move to different directories.
pub fn settings_dir() -> Option<PathBuf> {
    config_dir()
}

/// path to the config file (`~/.config/dream_player/config.toml`).
pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

/// path to the DB file. On Linux everything lives in the config directory:
/// `~/.config/dream_player/music_db.sqlite`.
pub fn db_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join(storage::DB_FILE_NAME))
}

/// the user's system music directory. First try `XDG_MUSIC_DIR`, and if it
/// isn't configured, fall back to `~/Music`, so there is always a default.
pub fn music_dir() -> Option<PathBuf> {
    let dirs = UserDirs::new()?;
    Some(
        dirs.audio_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dirs.home_dir().join("Music")),
    )
}

/// the user app config, stored in a toml file.
///
/// meant to be used through `Mutex<Config>`. The fields are placeholders for
/// now — real ones will be added later; thanks to `#[serde(default)]` a field
/// missing from the file gets its default value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// the overall (master) player volume, 0.0..=1.0.
    pub master_volume: f32,
    /// storage directory: where the db and covers live. `None` — the default
    /// config directory (see [`db_file`]).
    pub storage_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            master_volume: 1.0,
            storage_path: None,
        }
    }
}

impl Config {
    /// db file path honouring `storage_path` (if set), otherwise the default
    /// config directory.
    pub fn resolve_db_path(&self) -> Option<PathBuf> {
        match &self.storage_path {
            Some(dir) => Some(dir.join(storage::DB_FILE_NAME)),
            None => db_file(),
        }
    }
}

impl Config {
    /// loads the config from `path`. If there is no file, returns the defaults
    /// and does NOT create the file. Fields missing from the file get default
    /// values.
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        let cfg = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// saves the config to `path`, creating the file (and missing parent
    /// directories). The config file appears only on this call.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_file_yields_defaults_without_creating_it() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.master_volume, 1.0);
        // loading must not create the file.
        assert!(!path.exists());
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // the file has no fields — they must become defaults.
        fs::write(&path, "\n").unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.master_volume, 1.0);
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let cfg = Config {
            master_volume: 0.42,
            storage_path: Some(PathBuf::from("/tmp/dream_store")),
        };
        cfg.save(&path).unwrap();
        assert!(path.exists());
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.master_volume, 0.42);
        assert_eq!(loaded.storage_path, Some(PathBuf::from("/tmp/dream_store")));
    }
}
