use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use api::bridge::{ApiCommand, NowPlaying};
use audio::AudioEngine;
use audio_structs::playlist::Playlist;
use clap::Parser;

mod orchestrator;
mod playlist_manager;
mod storage;
mod traits;

#[derive(clap::Parser, Debug)]
#[command(version, about = "cli for music player core")]
struct Args {
    #[arg(short, long, value_name = "Dir")]
    path: Option<PathBuf>,
}
fn main() {
    let args = Args::parse();

    if let (Some(path)) = args.path {
        let mut playlist: Playlist = Playlist::from_dir(path).unwrap();
        let mut vi_track = playlist.get_track_mut(0).unwrap();
        vi_track.load_track();
        let raw = vi_track.take_track().unwrap();
        let mut engine = AudioEngine::new().unwrap();
        engine.load(raw, 1.);

        let _ = std::io::stdin().read_line(&mut String::new());
    }
}
