use std::error::Error;

use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};

pub trait Indexator {
    fn save_playlist(&self, playlist: Playlist) -> Result<(), Box<dyn Error>>;
    fn load_playlist(&self) -> Result<Playlist, Box<dyn Error>>;

    /// indexing tracks
    fn save_track(&self, track: TrackVirtual) -> Result<(), Box<dyn Error>>;
    fn load_track(&self, key: String) -> Result<TrackVirtual, Box<dyn Error>>;

    fn hash(track: &TrackVirtual) -> String;
    fn hash_exist(&self, hash: String) -> bool;
}
