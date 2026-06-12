use crate::music::playlist::Playlist;

pub struct PlaylistManager {
    playlist: Option<Playlist>,
    cur_track: usize,
}

impl PlaylistManager {
    pub fn set_playlist(&mut self) {}
    pub fn next(&mut self) {}
    pub fn prev(&mut self) {}
    pub fn select_track(&mut self, number: usize) {}
    pub fn get_track(&self) {}
    pub fn load(&mut self, number: usize) {}
    pub fn unload(&mut self, number: usize) {}
    pub fn get_cur_number(&self) -> usize {
        self.cur_track
    }
}
