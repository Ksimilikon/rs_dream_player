pub mod config;
pub mod dirs;
pub mod event_loop;

use audio::AudioEngine;
use storage::Db;

use crate::{config::Config, dirs::Dirs};

/// db file name — the canonical one from the storage crate (matches what the
/// previous app version wrote; otherwise existing playlists won't be picked
/// up).
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
        // the data directory may not exist yet (first launch) — the db and
        // config go there; without it Db::init / config saving would fail.
        let _ = std::fs::create_dir_all(&dirs.data);

        let db = if need_db {
            // WARN: panic
            Some(storage::Db::init(dirs.data.join(FILE_NAME_DB)).unwrap())
        } else {
            None
        };
        // a missing / broken config must not break startup — use the default.
        let config = Config::load(&dirs.data.join(FILE_NAME_CONFIG)).unwrap_or_default();
        // the only core threads are audio ones, started by AudioEngine (cpal).
        let mut engine = AudioEngine::new().unwrap();
        // apply the saved master volume to the engine.
        engine.set_volume_master(config.master_volume);
        Self {
            config,
            db,
            dirs,
            engine,
        }
    }

    /// sets the master volume: applies it to the engine and saves it to the
    /// config on disk, so the value survives a restart.
    pub fn set_master_volume(&mut self, v: f32) {
        self.config.master_volume = v;
        self.engine.set_volume_master(v);
        let _ = self.config.save(&self.dirs.data.join(FILE_NAME_CONFIG));
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
