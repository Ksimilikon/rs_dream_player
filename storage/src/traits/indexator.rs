use std::error::Error;

use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};

pub trait Indexator {
    fn save_playlist(&self, playlist: Playlist) -> Result<(), Box<dyn Error>>;
    /// loads a playlist by name.
    fn get_playlist(&self, name: String) -> Result<Playlist, Box<dyn Error>>;

    /// indexing tracks
    fn save_track(&self, track: TrackVirtual) -> Result<(), Box<dyn Error>>;
    /// loads a track by hash.
    fn get_track(&self, key: String) -> Result<TrackVirtual, Box<dyn Error>>;

    fn hash(track: &TrackVirtual) -> String;
    fn hash_exist(&self, hash: String) -> bool;
}
