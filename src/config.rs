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

/// каталог пользовательских настроек. Пока совпадает с [`config_dir`]
/// (всё лежит в одном месте), но вынесен отдельно — позже настройки и конфиг
/// могут разъехаться по разным каталогам.
pub fn settings_dir() -> Option<PathBuf> {
    config_dir()
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

/// системный каталог музыки пользователя. Сначала пробуем `XDG_MUSIC_DIR`,
/// а если он не настроен — откатываемся на `~/Music`, чтобы дефолт был всегда.
pub fn music_dir() -> Option<PathBuf> {
    let dirs = UserDirs::new()?;
    Some(
        dirs.audio_dir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| dirs.home_dir().join("Music")),
    )
}

/// пользовательский конфиг приложения, хранящийся в toml-файле.
///
/// Предполагается работа через `Mutex<Config>`. Поля пока заглушечные —
/// настоящие будут добавлены позже; благодаря `#[serde(default)]` отсутствие
/// поля в файле подставляет значение по умолчанию.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// общая (мастер-) громкость плеера, 0.0..=1.0.
    pub master_volume: f32,
    /// каталог хранилища: где лежат бд и обложки. `None` — каталог конфига
    /// по умолчанию (см. [`db_file`]).
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
    /// путь к файлу бд с учётом `storage_path` (если задан), иначе — каталог
    /// конфига по умолчанию.
    pub fn resolve_db_path(&self) -> Option<PathBuf> {
        match &self.storage_path {
            Some(dir) => Some(dir.join(storage::DB_FILE_NAME)),
            None => db_file(),
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
        // загрузка не должна создавать файл.
        assert!(!path.exists());
    }

    #[test]
    fn missing_field_falls_back_to_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // в файле нет полей — они должны стать дефолтными.
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
