use std::io::Cursor;

use rodio::Decoder;

use crate::audio::{player::Player, playlist::Playlist, song::track::Track};

mod audio;
mod cmd_docmsg;
mod config;
mod traits;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        // show --help
        println!("Welcome dream player");
        return;
    }

    let song_path = &args[1];
    let mut playlist = Playlist::from_dir(song_path).unwrap();
    let mut player = Player::new();
    player.set_playlist(playlist);
    player.play();

    // only for test
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
}
