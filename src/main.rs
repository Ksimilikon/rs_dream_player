use std::sync::{Arc, Mutex, mpsc};

use crate::audio::{dbus::Dbus, player::Player, playlist::Playlist};

mod audio;
mod cmd_docmsg;
mod config;
mod traits;

pub const NAME: &str = "org.mpris.dream_player";
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        // show --help
        println!("Welcome dream player");
        return;
    }

    let (tx, rx) = mpsc::channel::<String>();

    let song_path = &args[1];
    let playlist = Playlist::from_dir(song_path).unwrap();
    let player = Arc::new(Mutex::new(Player::new(Some(tx))));
    // player.set_playlist(playlist);
    // player.play();
    Dbus::start_server(Arc::clone(&player), rx);
    player.lock().unwrap().set_playlist(playlist);
    player.lock().unwrap().play();
    // only for test
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
}
