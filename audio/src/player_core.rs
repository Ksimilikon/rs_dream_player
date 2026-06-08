use std::sync::{Arc, Mutex};

use crate::{player::Player, playlist::Playlist};

pub struct PlayerCore {
    playlists: Vec<Playlist>,
    cur_playlist: Arc<Mutex<Playlist>>,
    player: Arc<Mutex<Playlist>>,
}

impl PlayerCore {
    pub fn new() {}
}
