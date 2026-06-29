use std::{
    path::PathBuf,
    sync::{Arc, mpsc::Sender, mpsc::channel},
    thread,
};

use audio::AudioEngine;
use audio_structs::playlist::Playlist;
use clap::Parser;
use dbus::{DBus, DBusData, DBusEvent};

use crate::playlist_manager::PlaylistManager;

mod config;
mod orchestrator;
mod playlist_manager;
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
    //Select,
}
enum EngineEvents {
    Add(Vec<u8>),
    PlayStop,
}

/// читает метаданные текущего трека и публикует их в MPRIS.
fn push_meta(tx: &Sender<DBusData>, manager: &PlaylistManager) {
    if let Some(track) = manager.get_track()
        && let Ok(meta) = track.get_metadata()
    {
        let _ = tx.send(DBusData {
            title: meta.title.clone(),
            artists: meta.artist.clone(),
            art: None,
        });
    }
}

fn main() {
    let args = Args::parse();

    if let Some(path) = args.path {
        let playlist: Playlist = Playlist::from_dir(path).unwrap();

        let (tx_manager, rx_manager) = channel::<PlaybackEvents>();
        let (tx_engine, rx_engine) = channel::<EngineEvents>();

        // каналы dbus-слоя: команды наружу (cmd) и метаданные внутрь (data)
        let (cmd_tx, cmd_rx) = channel::<DBusEvent>();
        let (data_tx, data_rx) = channel::<DBusData>();

        // поток с MPRIS-соединением — как остальные воркеры
        let dbus = DBus::new(cmd_tx, data_rx);
        thread::spawn(move || {
            if let Err(e) = dbus.run() {
                eprintln!("dbus: {e}");
            }
        });

        // роутер: команды от DE -> внутренние каналы плеера
        let tx_manager_dbus = tx_manager.clone();
        let tx_engine_dbus = tx_engine.clone();
        thread::spawn(move || {
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    DBusEvent::Next => {
                        let _ = tx_manager_dbus.send(PlaybackEvents::Next);
                    }
                    DBusEvent::Prev => {
                        let _ = tx_manager_dbus.send(PlaybackEvents::Prev);
                    }
                    DBusEvent::PlayPause | DBusEvent::Play | DBusEvent::Pause | DBusEvent::Stop => {
                        let _ = tx_engine_dbus.send(EngineEvents::PlayStop);
                    }
                }
            }
        });

        let _worker_manager = thread::spawn(move || {
            let mut manager = PlaylistManager::new();
            let _ = manager.set_playlist(playlist);
            let _ = manager.load(manager.get_cur_number());
            push_meta(&data_tx, &manager);
            let _ = tx_engine.send(EngineEvents::Add(
                manager.get_track_mut().unwrap().take_track().unwrap(),
            ));
            loop {
                if let Ok(e) = rx_manager.recv() {
                    match e {
                        PlaybackEvents::Next => {
                            let _ = manager.next();
                            let _ = manager.load(manager.get_cur_number());
                            push_meta(&data_tx, &manager);
                            let raw = manager.get_track_mut().unwrap().take_track().unwrap();
                            let _ = tx_engine.send(EngineEvents::Add(raw));
                            let _ = manager.load_next();
                        }
                        PlaybackEvents::Prev => {
                            let _ = manager.prev();
                            let _ = manager.load(manager.get_cur_number());
                            push_meta(&data_tx, &manager);
                            let raw = manager.get_track_mut().unwrap().take_track().unwrap();
                            let _ = tx_engine.send(EngineEvents::Add(raw));
                        }
                    }
                }
            }
        });

        let _worker_engine = thread::spawn(move || {
            let mut engine = AudioEngine::new().unwrap();
            let tx = Arc::new(tx_manager);
            loop {
                if let Ok(e) = rx_engine.recv() {
                    match e {
                        EngineEvents::Add(raw) => {
                            let tx_clone = tx.clone();
                            let _ = engine.load(
                                raw,
                                1.,
                                Some(move || {
                                    let _ = tx_clone.send(PlaybackEvents::Next);
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
