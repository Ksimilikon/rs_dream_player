use serde::{Deserialize, Serialize};

use crate::audio::song::Song;

#[derive(Debug, Serialize, Deserialize)]
pub struct Playlist {
    songs: Vec<Song>,
}
