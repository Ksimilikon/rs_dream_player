pub mod config;
pub mod dirs;
pub mod event_loop;

use std::fmt;

use audio::AudioEngine;
use storage::Db;

use crate::{config::Config, dirs::Dirs};

/// db file name — the canonical one from the storage crate (matches what the
/// previous app version wrote; otherwise existing playlists won't be picked
/// up).
pub const FILE_NAME_DB: &str = storage::DB_FILE_NAME;
pub const FILE_NAME_CONFIG: &str = "config.toml";

/// what can go wrong while bringing the core up ([`Core::new`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    /// the db file can't be opened / created / migrated.
    Db,
    /// no audio output (no device, backend failed to start).
    Audio,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            InitError::Db => "failed to open the database",
            InitError::Audio => "failed to start audio output",
        })
    }
}

impl std::error::Error for InitError {}

/// the master volume was applied, but the config could not be saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigSaveError;

impl fmt::Display for ConfigSaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("failed to save the config")
    }
}

impl std::error::Error for ConfigSaveError {}

pub struct Core {
    config: config::Config,
    db: Option<Db>,
    dirs: Dirs,
    engine: AudioEngine,
}
impl Core {
    pub fn new(dirs: Dirs, need_db: bool) -> Result<Self, InitError> {
        // the data directory may not exist yet (first launch) — the db and
        // config go there; without it Db::init / config saving would fail.
        let _ = std::fs::create_dir_all(&dirs.data);

        let db = match need_db {
            true => Some(Db::init(dirs.data.join(FILE_NAME_DB)).map_err(|_| InitError::Db)?),
            false => None,
        };
        // a missing / broken config must not break startup — use the default.
        let config = Config::load(&dirs.data.join(FILE_NAME_CONFIG)).unwrap_or_default();
        // the only core threads are audio ones, started by AudioEngine (cpal).
        let mut engine = AudioEngine::new().map_err(|_| InitError::Audio)?;
        // apply the saved master volume to the engine.
        engine.set_volume_master(config.master_volume);
        Ok(Self {
            config,
            db,
            dirs,
            engine,
        })
    }

    /// sets the master volume: applies it to the engine and saves it to the
    /// config on disk, so the value survives a restart. The volume is applied
    /// even if saving fails.
    pub fn set_master_volume(&mut self, v: f32) -> Result<(), ConfigSaveError> {
        self.config.master_volume = v;
        self.engine.set_volume_master(v);
        self.config
            .save(&self.dirs.data.join(FILE_NAME_CONFIG))
            .map_err(|_| ConfigSaveError)
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
