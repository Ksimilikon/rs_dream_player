use std::sync::Arc;

use crate::music::track_virtual::TrackVirtual;

// TODO: updated_at, created_at
pub struct Playlist {
    // NOTE: using option for anonim playlists
    name: Option<String>,
    // TODO: wrap arc raw track in TrackVirtual
    tracks: Vec<Arc<TrackVirtual>>,
}

// making playlist
impl Playlist {
    /// make Anonim playlist
    pub fn from_dir() {}
    /// make Index playlist
    pub fn from_index(name: String) {}
    /// make Anonim playlist
    pub fn make_single(track: TrackVirtual) {}
    /// make Anonim playlist
    pub fn from_tracks(tracks: Vec<TrackVirtual>) {}
}
/// controlling playlist
impl Playlist {
    pub fn save(&self, name: String) {}
    pub fn add_track(&mut self, track: TrackVirtual, pos: usize) {}
    pub fn add_track_back(&mut self, track: TrackVirtual) {}
    pub fn append_playlist(&mut self, playlist: Playlist) {}
    pub fn mix_tracks(&mut self) {}
}

impl Playlist {
    pub fn set_name(&mut self, name: String) {
        self.name = Some(name);
    }
    pub fn get_name(&self) -> Option<String> {
        self.name.clone()
    }
    pub fn get_count(&self) -> usize {
        self.tracks.len()
    }
}
