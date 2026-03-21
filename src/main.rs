use std::{path::PathBuf, sync::Arc};

use clap::Parser;
use player::song::metadata::Metadata;

use crate::audio::dbus::Dbus;
use player::player::Player;
use player::playlist::Playlist;

mod audio;
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
    let config = config::Config::default();

    let mut mod_manager = api::ModManager::new();
    let _ = mod_manager.load_mods("mods");
    println!("{:#?}", mod_manager);

    let (tx, rx) = tokio::sync::mpsc::channel::<Arc<Metadata>>(16);
    let player = Player::new(Some(tx));
    api::player::init(player.clone());

    if let Some(path) = args.path {
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
