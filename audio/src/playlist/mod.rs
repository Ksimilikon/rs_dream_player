use crate::song::{track::Track, track_virtual::TrackVirtual};

#[derive(Debug)]
pub struct Playlist {
    tracks: Vec<TrackVirtual>,
    cur_track: usize,
    name: Option<String>,
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
    pub fn next() {}
    pub fn prev() {}
    pub fn to_at(number: i64) {}
    pub fn save_to_index(name: String) {}
    pub fn add_track(&mut self, track: TrackVirtual, pos: usize) {}
    pub fn add_track_back(&mut self, track: TrackVirtual) {}
    pub fn append_playlist(&mut self, playlist: Playlist) {}
    pub fn mix_track(&mut self) {}
}

impl Playlist {
    pub fn set_name(&mut self, name: String) {
        self.name = Some(name);
    }
    pub fn get_cur_track(&self) {}
    pub fn get_name(&self) -> Option<String> {
        self.name.clone()
    }
    pub fn get_count(&self) -> usize {
        self.tracks.len()
    }
}
