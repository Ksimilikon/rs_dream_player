use std::{error::Error, fmt, path::PathBuf};

use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual, types::Volume};

#[derive(Debug)]
pub struct ErrorNoPlaylist;
impl fmt::Display for ErrorNoPlaylist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no playlist is loaded")
    }
}
impl Error for ErrorNoPlaylist {}

#[derive(Debug)]
pub struct ErrorTrackOutOfRange(pub usize);
impl fmt::Display for ErrorTrackOutOfRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "track index {} is out of range", self.0)
    }
}
impl Error for ErrorTrackOutOfRange {}

#[derive(Clone)]
pub enum PlaybackMode {
    InfinityPlaylist,
    /// infinity repeating one track
    RepeatSingle,
}
/// keeps track of the active [`Playlist`] and which of its tracks is
/// currently selected, loading/unloading raw audio as the selection moves.
pub struct PlaylistManager {
    playlist: Option<Playlist>,
    cur_track: usize,
    mode: PlaybackMode,
}

impl PlaylistManager {
    pub fn new() -> Self {
        Self {
            playlist: None,
            cur_track: 0,
            mode: PlaybackMode::InfinityPlaylist,
        }
    }

    /// replaces the active playlist, resets to its first track and loads it.
    pub fn set_playlist(&mut self, playlist: Playlist) -> Result<(), Box<dyn Error>> {
        self.playlist = Some(playlist);
        self.cur_track = 0;
        if self.playlist.as_ref().unwrap().get_count() > 0 {
            self.load(0)?;
        }
        Ok(())
    }

    /// moves the cursor to the next track (wrapping) without loading it.
    /// Used by the error-recovery play loop, which loads separately.
    pub fn step_next(&mut self) {
        let count = match &self.playlist {
            Some(p) if p.get_count() > 0 => p.get_count(),
            _ => return,
        };
        self.unload(self.cur_track);
        self.cur_track = (self.cur_track + 1) % count;
    }

    /// moves the cursor to the previous track (wrapping) without loading it.
    pub fn step_prev(&mut self) {
        let count = match &self.playlist {
            Some(p) if p.get_count() > 0 => p.get_count(),
            _ => return,
        };
        self.unload(self.cur_track);
        self.cur_track = (self.cur_track + count - 1) % count;
    }

    /// moves the cursor to track `number` without loading it.
    pub fn goto(&mut self, number: usize) -> Result<(), Box<dyn Error>> {
        let count = self.playlist.as_ref().ok_or(ErrorNoPlaylist)?.get_count();
        if number >= count {
            return Err(Box::new(ErrorTrackOutOfRange(number)));
        }
        self.unload(self.cur_track);
        self.cur_track = number;
        Ok(())
    }

    /// loads metadata and raw audio for the currently selected track.
    pub fn load_current(&mut self) -> Result<(), Box<dyn Error>> {
        self.load(self.cur_track)
    }

    /// `true` when there's no playlist or it has no tracks.
    pub fn is_empty(&self) -> bool {
        self.playlist
            .as_ref()
            .map(|p| p.get_count() == 0)
            .unwrap_or(true)
    }

    /// describes the current track for error recovery: its index id (if any),
    /// file path (if any) and title.
    pub fn current_descriptor(&self) -> Option<(Option<i64>, Option<PathBuf>, String)> {
        let track = self.get_track()?;
        let title = track
            .get_metadata()
            .map(|m| m.title.clone())
            .unwrap_or_else(|_| "Unknown".to_string());
        Some((
            track.index_id(),
            track.get_path().map(|p| p.to_path_buf()),
            title,
        ))
    }

    /// removes the current track from the active playlist, clamping the cursor
    /// so it points at the track that shifted into its place (or wraps to the
    /// start). Returns the removed track.
    pub fn remove_current(&mut self) -> Option<TrackVirtual> {
        let cur = self.cur_track;
        let removed = self.playlist.as_mut()?.remove_track(cur);
        if let Some(p) = &self.playlist {
            let count = p.get_count();
            if count == 0 || self.cur_track >= count {
                self.cur_track = 0;
            }
        }
        removed
    }

    /// the currently selected track, if any.
    pub fn get_track(&self) -> Option<&TrackVirtual> {
        self.playlist.as_ref()?.get_track(self.cur_track)
    }
    pub fn get_track_mut(&mut self) -> Option<&mut TrackVirtual> {
        self.playlist.as_mut()?.get_track_mut(self.cur_track)
    }
    /// loads metadata and raw audio for track `number` into RAM.
    pub fn load(&mut self, number: usize) -> Result<(), Box<dyn Error>> {
        let playlist = self.playlist.as_mut().ok_or(ErrorNoPlaylist)?;
        let track = playlist
            .get_track_mut(number)
            .ok_or(ErrorTrackOutOfRange(number))?;
        track.load_metadata()?;
        track.load_track()?;
        Ok(())
    }

    /// drops the raw audio for track `number` from RAM (metadata stays loaded).
    pub fn unload(&mut self, number: usize) {
        if let Some(playlist) = self.playlist.as_mut()
            && let Some(track) = playlist.get_track_mut(number)
        {
            track.unload_track();
        }
    }

    pub fn get_cur_number(&self) -> usize {
        self.cur_track
    }

    /// volume of the currently selected track, or `1.0` if nothing is selected.
    pub fn current_volume(&self) -> Volume {
        self.get_track().map(|t| t.volume).unwrap_or(1.0)
    }

    /// sets the volume of the currently selected track (no-op if none).
    pub fn set_current_volume(&mut self, volume: Volume) {
        let cur = self.cur_track;
        if let Some(track) = self.playlist.as_mut().and_then(|p| p.get_track_mut(cur)) {
            track.volume = volume;
        }
    }
    pub fn set_mode(&mut self, mode: PlaybackMode) {
        self.mode = mode;
    }
    pub fn get_mode(&self) -> PlaybackMode {
        self.mode.clone()
    }
}
