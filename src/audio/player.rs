use std::time::Duration;

use super::{playlist::Playlist, song::Song, types::Volume};
use crate::traits::player_agent_os::PlayerAgentOS;

#[derive(Debug)]
pub struct PlayerState {
    cur_song: Option<Song>,
    cur_playlist: Option<Playlist>,
    is_playing: bool,
    volume: Volume,
    position: Duration,
}

// impl PlayerAgentOS for PlayerState {
//     fn send_meta_data(&self) -> Result<(), Box<dyn std::error::Error>> {
//
//     }
// }
