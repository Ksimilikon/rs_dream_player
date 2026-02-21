use std::{path::PathBuf, sync::Arc};

use clap::Parser;

use crate::audio::{dbus::Dbus, player::Player, playlist::Playlist};

mod api;
mod audio;
mod cmd_docmsg;
mod config;
mod traits;
// NOTE: need add logger

#[derive(clap::Parser, Debug)]
#[command(version, about = "core for music")]
struct Args {
    #[arg(short, long, value_name = "Dir")]
    path: Option<PathBuf>,
}
fn main() {
    let args = Args::parse();

    let (tx, rx) = tokio::sync::mpsc::channel::<Arc<audio::song::metadata::Metadata>>(16);

    let player = Player::new(Some(tx));
    // player.set_playlist(playlist);
    // player.play();

    if let Some(path) = args.path {
        println!("path {}", path.display());
        let playlist = Playlist::from_dir(path).unwrap();
        let mut guard = player.lock().unwrap();
        guard.set_playlist(playlist);
        guard.play();
    }

    let _ = Dbus::start_server(player.clone(), rx);
    // only for test
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
}
