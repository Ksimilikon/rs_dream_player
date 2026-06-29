use std::{
    error::Error,
    path::{Path, PathBuf},
};

use crate::track_virtual::TrackVirtual;

#[derive(Debug)]
pub struct Playlist {
    /// NOTE: using option for anonim playlists
    name: Option<String>,
    tracks: Vec<TrackVirtual>,
    /// path to the playlist cover art on disk; may be empty.
    cover_art: Option<PathBuf>,
    updated_at: Option<u64>,
    created_at: Option<u64>,
}

// making playlist
impl Playlist {
    /// recursively scans `path`, building an anonymous (unnamed) playlist
    /// out of every music file found. Non-music/unreadable files are skipped.
    pub fn from_dir<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        let mut tracks = Vec::new();
        Self::collect_from_dir(path.as_ref(), &mut tracks)?;
        Ok(Self {
            name: None,
            tracks,
            cover_art: None,
            updated_at: None,
            created_at: None,
        })
    }

    fn collect_from_dir(dir: &Path, tracks: &mut Vec<TrackVirtual>) -> Result<(), Box<dyn Error>> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                Self::collect_from_dir(&path, tracks)?;
            } else if let Ok(track) = TrackVirtual::from_file(path, false) {
                tracks.push(track);
            }
        }
        Ok(())
    }

    /// make Anonim playlist
    pub fn from_tracks(tracks: Vec<TrackVirtual>) -> Self {
        Self {
            name: None,
            tracks,
            cover_art: None,
            updated_at: None,
            created_at: None,
        }
    }
}
/// controlling playlist
impl Playlist {
    pub fn add_track(&mut self, track: TrackVirtual, pos: usize) {
        let pos = pos.min(self.tracks.len());
        self.tracks.insert(pos, track);
    }
    pub fn add_track_back(&mut self, track: TrackVirtual) {
        self.tracks.push(track);
    }
    pub fn append_playlist(&mut self, mut playlist: Playlist) {
        self.tracks.append(&mut playlist.tracks);
    }
    /// shuffles the track order in-place (Fisher-Yates).
    pub fn mix_tracks(&mut self) {
        for i in (1..self.tracks.len()).rev() {
            let j = fastrand::usize(0..=i);
            self.tracks.swap(i, j);
        }
    }
}

impl Playlist {
    pub fn set_name(&mut self, name: String) {
        self.name = Some(name);
    }
    pub fn get_name(&self) -> Option<String> {
        self.name.clone()
    }
    /// sets (or clears, with `None`) the playlist cover art path.
    pub fn set_cover_art(&mut self, cover: Option<PathBuf>) {
        self.cover_art = cover;
    }
    /// path to the playlist cover art, if any.
    pub fn get_cover_art(&self) -> Option<&Path> {
        self.cover_art.as_deref()
    }
    pub fn get_count(&self) -> usize {
        self.tracks.len()
    }
    pub fn tracks(&self) -> &[TrackVirtual] {
        &self.tracks
    }
    pub fn get_track(&self, i: usize) -> Option<&TrackVirtual> {
        self.tracks.get(i)
    }
    pub fn get_track_mut(&mut self, i: usize) -> Option<&mut TrackVirtual> {
        self.tracks.get_mut(i)
    }
    pub fn remove_track(&mut self, i: usize) -> Option<TrackVirtual> {
        if i < self.tracks.len() {
            Some(self.tracks.remove(i))
        } else {
            None
        }
    }
    pub fn set_created_at(&mut self, time: u64) {
        self.created_at = Some(time);
    }
    pub fn get_created_at(&self) -> Option<u64> {
        self.created_at
    }
    pub fn set_updated_at(&mut self, time: u64) {
        self.updated_at = Some(time);
    }
    pub fn get_updated_at(&self) -> Option<u64> {
        self.updated_at
    }
}
