use std::sync::{
    Arc,
    mpsc::{Receiver, Sender},
};

use audio_structs::playlist::Playlist;
use dbus::DBusData;

use crate::{orchestrator::engine::EngineEvent, playlist_manager::PlaylistManager};

pub enum PlaylistManagerEvent {
    Next,
    Prev,
    Select(usize),
    Playlist(Playlist),
}
pub fn spawn(
    rx: Receiver<PlaylistManagerEvent>,
    tx_engine: Arc<Sender<EngineEvent>>,
    tx_data: Sender<DBusData>,
) {
    let worker_manager = std::thread::spawn(move || {
        handler_manager(tx_engine, tx_data, rx);
    });
}

/// читает метаданные текущего трека и публикует их в MPRIS.
fn push_meta(tx_data: &Sender<DBusData>, manager: &PlaylistManager) {
    if let Some(track) = manager.get_track()
        && let Ok(meta) = track.get_metadata()
    {
        let _ = tx_data.send(DBusData {
            title: meta.title.clone(),
            artists: meta.artist.clone(),
            art: None,
        });
    }
}

fn handler_manager(
    tx_engine: Arc<Sender<EngineEvent>>,
    tx_data: Sender<DBusData>,
    rx: Receiver<PlaylistManagerEvent>,
) {
    let mut manager = PlaylistManager::new();
    while let Ok(e) = rx.recv() {
        match e {
            PlaylistManagerEvent::Next => {
                let res = manager.next();
                match res {
                    Err(err) => println!("ERROR::Orchestrator::handler_manager::NEXT::{}", err),
                    Ok(track_opt) => match track_opt {
                        None => println!(
                            "WARN::Orchestrator::handler_manager::NEXT::track is not exist"
                        ),
                        Some(track) => {
                            let res_raw = track.take_track();
                            if let Ok(b) = res_raw {
                                tx_engine.send(EngineEvent::Add(b));
                                push_meta(&tx_data, &manager);
                                manager.load_next();
                            } else {
                                println!(
                                    "ERROR::Orchestrator::handler_manager::NEXT::track is unloaded"
                                );
                            }
                        }
                    },
                }
            }
            PlaylistManagerEvent::Prev => {
                let res = manager.prev();
                match res {
                    Err(err) => println!("ERROR::Orchestrator::handler_manager::PREV::{}", err),
                    Ok(track_opt) => match track_opt {
                        None => println!(
                            "WARN::Orchestrator::handler_manager::NEXT::track is not exist"
                        ),
                        Some(track) => {
                            let res_raw = track.take_track();
                            if let Ok(b) = res_raw {
                                tx_engine.send(EngineEvent::Add(b));
                                push_meta(&tx_data, &manager);
                                manager.load_next();
                            } else {
                                println!(
                                    "ERROR::Orchestrator::handler_manager::NEXT::track is unloaded"
                                );
                            }
                        }
                    },
                }
            }
            PlaylistManagerEvent::Select(number) => {
                let res = manager.select_track(number);
                if let Err(err) = res {
                    println!(
                        "ERROR::Orchestrator::handler_manager::select_track::{}",
                        err
                    );
                } else {
                    push_meta(&tx_data, &manager);
                }
            }
            PlaylistManagerEvent::Playlist(p) => {
                let res = manager.set_playlist(p);
                if let Err(err) = res {
                    println!(
                        "ERROR::Orchestrator::handler_manager::set_playlist::{}",
                        err
                    );
                } else {
                    // костыль: сразу отдаём первый трек в движок, чтобы
                    // воспроизведение стартовало без отдельной команды.
                    push_meta(&tx_data, &manager);
                    if let Some(track) = manager.get_track_mut()
                        && let Ok(b) = track.take_track()
                    {
                        let _ = tx_engine.send(EngineEvent::Add(b));
                    }
                }
            }
        }
    }
}
