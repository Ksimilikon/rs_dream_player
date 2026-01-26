use std::time::Duration;

use super::{playlist::Playlist, types::Volume};

pub struct Player {
    cur_playlist: Option<Playlist>,
    is_playing: bool,
    volume: Volume,
    position: Duration,
    preset: Vec<String>,
}

// impl PlayerAgentOS for PlayerState {
//     fn send_meta_data(&self) -> Result<(), Box<dyn std::error::Error>> {
//
//     }
// }
