use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

pub mod structs;
#[derive(Serialize, Deserialize, Debug)]
pub struct Indexer {
    path: PathBuf,
    playlists: Vec<structs::Playlist>,
}

impl Indexer {
    pub fn new(path: PathBuf) -> Self {
        let mut res = Self {
            path: path.join("index.json"),
            playlists: Vec::new(),
        };
        match res.load_index() {
            Ok(_) => (),
            Err(e) => println!("WARNING::load_index()::{}", e),
        }
        res
    }
    /// load index from file
    pub fn load_index(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if !self.path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&self.path)?;
        let loaded: Indexer = serde_json::from_str(&content)?;

        self.playlists = loaded.playlists;
        Ok(())
    }
    /// save index to file
    pub fn save_index(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string_pretty(&self)?;

        fs::write(&self.path, json)?;
        Ok(())
    }
}
