pub mod config;
pub mod dirs;
pub mod event_loop;

use audio::AudioEngine;
use storage::Db;

use crate::{config::Config, dirs::Dirs};

/// имя файла бд — канонічное из крейта storage (совпадает с тем, что писала
/// прежняя версия приложения; иначе существующие плейлисты не подхватятся).
pub const FILE_NAME_DB: &str = storage::DB_FILE_NAME;
pub const FILE_NAME_CONFIG: &str = "config.toml";

pub struct Core {
    config: config::Config,
    db: Option<Db>,
    dirs: Dirs,
    engine: AudioEngine,
}
impl Core {
    pub fn new(dirs: Dirs, need_db: bool) -> Self {
        // каталог данных может ещё не существовать (первый запуск) — сюда лягут
        // бд и конфиг; без него Db::init/сохранение конфига упадут.
        let _ = std::fs::create_dir_all(&dirs.data);

        let db = if need_db {
            // WARN: panic
            Some(storage::Db::init(dirs.data.join(FILE_NAME_DB)).unwrap())
        } else {
            None
        };
        // отсутствующий/битый конфиг не должен ронять запуск — берём дефолт.
        let config = Config::load(&dirs.data.join(FILE_NAME_CONFIG)).unwrap_or_default();
        // единственные потоки ядра — звуковые, их поднимает AudioEngine (cpal).
        let engine = AudioEngine::new().unwrap();
        Self {
            config,
            db,
            dirs,
            engine,
        }
    }
}

impl Core {
    pub fn get_config(&self) -> &Config {
        &self.config
    }
    pub fn get_db(&self) -> Option<&Db> {
        self.db.as_ref()
    }
    pub fn get_dirs(&self) -> &Dirs {
        &self.dirs
    }
    pub fn get_engine(&self) -> &AudioEngine {
        &self.engine
    }
    pub fn get_engine_mut(&mut self) -> &mut AudioEngine {
        &mut self.engine
    }
}
