use std::{error::Error, path::PathBuf};

use audio_structs::types::Volume;
use directories::{ProjectDirs, UserDirs};

const QUALIFIER: &str = "io";
const ORGANIZATION: &str = "dreamplayer";
const APPLICATION: &str = "DreamPlayer";

/// file name of the sqlite index inside [`Config::dir_db`].
const DB_FILE_NAME: &str = "index.db";

/// persisted user settings, backed by the `settings` table of the sqlite index
/// (see [`crate::storage::index::Index`]).
#[derive(Debug, Clone)]
pub struct Config {
    pub dir_music: PathBuf,
    pub dir_config: PathBuf,
    pub dir_arts: PathBuf,
    pub dir_db: PathBuf,

    /// general volume for the app
    pub volume: Volume,
    pub last_playlist: Option<String>,
    pub last_track: Option<usize>,
}

impl Config {
    /// builds a config from platform conventions (XDG on linux, Known Folders
    /// on windows). These values are the fallback used when nothing is stored
    /// yet in the `settings` table.
    pub fn defaults() -> Result<Self, Box<dyn Error>> {
        let proj = ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .ok_or("cannot determine application directories for this platform")?;

        let dir_db = proj.data_dir().to_path_buf();
        let dir_config = proj.config_dir().to_path_buf();
        let dir_arts = proj.data_dir().join("arts");
        let dir_music = UserDirs::new()
            .and_then(|u| u.audio_dir().map(PathBuf::from))
            .unwrap_or_else(|| dir_db.join("music"));

        Ok(Self {
            dir_music,
            dir_config,
            dir_arts,
            dir_db,
            volume: 1.0,
            last_playlist: None,
            last_track: None,
        })
    }

    /// full path to the sqlite index file.
    pub fn db_file(&self) -> PathBuf {
        self.dir_db.join(DB_FILE_NAME)
    }
}
