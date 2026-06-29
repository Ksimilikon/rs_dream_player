use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};

/// каталог конфигов/настроек приложения. На ПК (Linux) = `~/.config/dream_player`;
/// сюда же кладутся пользовательские настройки.
pub fn config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "dream_player").map(|d| d.config_dir().to_path_buf())
}

/// путь к файлу конфига (`~/.config/dream_player/config.toml`).
pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

/// путь к файлу БД. На Linux всё лежит в каталоге конфига:
/// `~/.config/dream_player/music_db.sqlite`.
pub fn db_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join(storage::DB_FILE_NAME))
}

/// системный каталог музыки пользователя (`XDG_MUSIC_DIR`, обычно `~/Music`).
pub fn music_dir() -> Option<PathBuf> {
    UserDirs::new().and_then(|d| d.audio_dir().map(Path::to_path_buf))
}

/// пользовательский конфиг приложения, хранящийся в toml-файле.
///
/// Предполагается работа через `Mutex<Config>`. Поля пока заглушечные —
/// настоящие будут добавлены позже; благодаря `#[serde(default)]` отсутствие
/// поля в файле подставляет значение по умолчанию.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// заглушка №1.
    pub stub_one: String,
    /// заглушка №2.
    pub stub_two: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            stub_one: String::new(),
            stub_two: 0,
        }
    }
}

impl Config {
    /// загружает конфиг из `path`. Если файла нет — возвращает значения по
    /// умолчанию и НЕ создаёт файл. Отсутствующие в файле поля заполняются
    /// значениями по умолчанию.
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)?;
        let cfg = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// сохраняет конфиг в `path`, создавая файл (и недостающие родительские
    /// каталоги). Файл конфига появляется только при этом вызове.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
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
        assert_eq!(cfg.stub_one, "");
        assert_eq!(cfg.stub_two, 0);
        // загрузка не должна создавать файл.
        assert!(!path.exists());
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // в файле только одно поле — второе должно стать дефолтным.
        fs::write(&path, "stub_two = 7\n").unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.stub_two, 7);
        assert_eq!(cfg.stub_one, "");
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let cfg = Config {
            stub_one: "hi".into(),
            stub_two: 42,
        };
        cfg.save(&path).unwrap();
        assert!(path.exists());
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.stub_one, "hi");
        assert_eq!(loaded.stub_two, 42);
    }
}
