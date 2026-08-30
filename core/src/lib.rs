pub mod config;
pub mod dirs;
pub mod event_loop;

use std::thread::{self, JoinHandle};

use audio::AudioEngine;
use storage::Db;

use crate::{config::Config, dirs::Dirs};

pub const FILE_NAME_DB: &str = "index.sqlite";
pub const FILE_NAME_CONFIG: &str = "config.toml";

pub struct Core {
    config: config::Config,
    db: Option<Db>,
    dirs: Dirs,
    engine: AudioEngine,
    worker: JoinHandle<()>,
}
impl Core {
    pub fn new(dirs: Dirs, need_db: bool) -> Self {
        let db = if need_db {
            // WARN: panic
            Some(storage::Db::init(dirs.data.join(FILE_NAME_DB)).unwrap())
        } else {
            None
        };
        // WARN: panic
        let config = Config::load(&dirs.data.join(FILE_NAME_CONFIG)).unwrap();
        let engine = AudioEngine::new().unwrap();
        let thread = thread::spawn(move || {});
        Self {
            config,
            db,
            dirs,
            engine,
            worker: thread,
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
