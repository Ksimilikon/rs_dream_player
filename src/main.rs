use std::sync::{Arc, Mutex, mpsc};

use crate::audio::{dbus::Dbus, player::Player, playlist::Playlist};

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

    let (tx, rx) = tokio::sync::mpsc::channel::<Arc<audio::song::metadata::Metadata>>(16);

    let song_path = &args[1];
    let playlist = Playlist::from_dir(song_path).unwrap();
    let player = Arc::new(Mutex::new(Player::new(Some(tx))));
    // player.set_playlist(playlist);
    // player.play();
    let _ = Dbus::start_server(player.clone(), rx);
    println!("start_server");
    println!("player lock main");
    player.lock().unwrap().set_playlist(playlist);
    player.lock().unwrap().play();
    println!("player unlock main");
    // only for test
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
}
