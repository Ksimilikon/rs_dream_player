use std::sync::Arc;

use crate::{music::playlist::Playlist, storage::config::Config};
pub mod config;

pub struct Storage {
    playlists: Vec<Playlist>,
    cfg: Config,
}

impl Storage {
    /// music from OS to index for this program
    pub fn indexing_music(&mut self) {}
    /// search this playlist in Storage
    /// -> replace/add
    /// and save in db
    pub fn save_playlist(&mut self, playlist: Arc<Playlist>) {}
}
