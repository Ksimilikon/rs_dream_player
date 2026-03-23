use player::types::Volume;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug)]
pub struct Playlist {
    pub songs: Vec<Song>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Song {
    pub id: Uuid,
    /// number in playlist
    pub number: u32,
    /// name file in file system
    pub name_file: String,
    pub volume: Volume,
    pub name: String,
    pub album: String,
    pub artists: Vec<String>,
}
