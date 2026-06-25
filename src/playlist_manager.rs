use std::{error::Error, fmt};

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

    /// moves to the next track, wrapping around to the start. Unloads the
    /// previous track's raw audio and loads the new one.
    pub fn next(&mut self) -> Result<Option<&mut TrackVirtual>, Box<dyn Error>> {
        let count = match &self.playlist {
            Some(p) if p.get_count() > 0 => p.get_count(),
            _ => return Ok(None),
        };
        self.unload(self.cur_track);
        self.cur_track = (self.cur_track + 1) % count;
        self.load(self.cur_track)?;
        Ok(self.get_track_mut())
    }

    /// moves to the previous track, wrapping around to the end.
    pub fn prev(&mut self) -> Result<Option<&mut TrackVirtual>, Box<dyn Error>> {
        let count = match &self.playlist {
            Some(p) if p.get_count() > 0 => p.get_count(),
            _ => return Ok(None),
        };
        self.unload(self.cur_track);
        self.cur_track = (self.cur_track + count - 1) % count;
        self.load(self.cur_track)?;
        Ok(self.get_track_mut())
    }

    /// jumps directly to track `number`.
    pub fn select_track(&mut self, number: usize) -> Result<&TrackVirtual, Box<dyn Error>> {
        let count = self.playlist.as_ref().ok_or(ErrorNoPlaylist)?.get_count();
        if number >= count {
            return Err(Box::new(ErrorTrackOutOfRange(number)));
        }
        self.unload(self.cur_track);
        self.cur_track = number;
        self.load(self.cur_track)?;
        Ok(self.get_track().expect("just loaded a valid index"))
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
        if let Some(playlist) = self.playlist.as_mut() {
            if let Some(track) = playlist.get_track_mut(number) {
                track.unload_track();
            }
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
