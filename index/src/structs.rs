use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

/// 1 Playlist - 1 file.toml;
#[derive(Serialize, Deserialize, Debug)]
pub struct Playlist {
    #[serde(skip)]
    pub name: String,
    pub songs: Option<Vec<Song>>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Song {
    pub name: String,
    pub album: Vec<String>,
    pub artists: Vec<String>,
    pub path: String,
    pub volume: f32,
}

pub struct LastSession {}
impl Playlist {
    // TODO: need make safety saving
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)?;

        let mut playlist: Playlist = toml::from_str(&content)?;

        playlist.name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        Ok(playlist)
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let toml_string = toml::to_string_pretty(&self)?;
        fs::write(path, toml_string)?;
        Ok(())
    }
}
