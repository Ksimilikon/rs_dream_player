use std::sync::{
    Arc,
    mpsc::{Receiver, Sender},
};

use audio_structs::playlist::Playlist;

use crate::{orchestrator::engine::EngineEvent, playlist_manager::PlaylistManager};

pub enum PlaylistManagerEvent {
    Next,
    Prev,
    Select(usize),
    Playlist(Playlist),
}
pub fn spawn(rx: Receiver<PlaylistManagerEvent>, tx_engine: Arc<Sender<EngineEvent>>) {
    let worker_manager = std::thread::spawn(move || {
        handler_manager(tx_engine, rx);
    });
}
fn handler_manager(tx_engine: Arc<Sender<EngineEvent>>, rx: Receiver<PlaylistManagerEvent>) {
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
                }
            }
            PlaylistManagerEvent::Playlist(p) => {
                println!("Setting Playlist:\n {:#?}", &p);
                let res = manager.set_playlist(p);
                if let Err(err) = res {
                    println!(
                        "ERROR::Orchestrator::handler_manager::set_playlist::{}",
                        err
                    );
                }
            }
        }
    }
}
