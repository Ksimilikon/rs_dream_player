use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, channel},
    },
    thread,
    time::Duration,
};

use api::bridge::{ApiCommand, NowPlaying};
use audio::AudioEngine;
use audio_structs::playlist::Playlist;
use clap::Parser;

use crate::playlist_manager::PlaylistManager;

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

enum PlaybackEvents {
    Next,
    Prev,
}
enum EngineEvents {
    Add(Vec<u8>),
    PlayStop,
}
fn main() {
    let args = Args::parse();

    if let (Some(path)) = args.path {
        let mut playlist: Playlist = Playlist::from_dir(path).unwrap();

        let (tx_manager, rx_manager) = channel::<PlaybackEvents>();
        let (tx_engine, rx_engine) = channel::<EngineEvents>();

        let worker_manager = thread::spawn(move || {
            let mut manager = PlaylistManager::new();
            manager.set_playlist(playlist);
            manager.load(manager.get_cur_number());
            tx_engine.send(EngineEvents::Add(
                manager.get_track_mut().unwrap().take_track().unwrap(),
            ));
            loop {
                if let Ok(e) = rx_manager.recv() {
                    match e {
                        PlaybackEvents::Next => {
                            let _ = manager.next();
                            manager.load(manager.get_cur_number());
                            let raw = manager.get_track_mut().unwrap().take_track().unwrap();
                            tx_engine.send(EngineEvents::Add(raw));
                            manager.load_next();
                        }
                        PlaybackEvents::Prev => {
                            let _ = manager.prev();
                            manager.load(manager.get_cur_number());
                            let raw = manager.get_track_mut().unwrap().take_track().unwrap();
                            tx_engine.send(EngineEvents::Add(raw));
                        }
                    }
                }
            }
        });
        let worker_engine = thread::spawn(move || {
            let mut engine = AudioEngine::new().unwrap();
            let tx = Arc::new(tx_manager);
            loop {
                if let Ok(e) = rx_engine.recv() {
                    match e {
                        EngineEvents::Add(raw) => {
                            let tx_clone = tx.clone();
                            engine.load(
                                raw,
                                1.,
                                Some(move || {
                                    tx_clone.send(PlaybackEvents::Next);
                                }),
                            );
                        }
                        EngineEvents::PlayStop => engine.play_pause(),
                    }
                }
            }
        });

        let _ = std::io::stdin().read_line(&mut String::new());
    }
}
