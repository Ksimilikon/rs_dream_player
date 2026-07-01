use std::sync::{
    Arc,
    mpsc::{Receiver, Sender},
};
use std::time::{SystemTime, UNIX_EPOCH};

use audio_structs::playlist::Playlist;
use dbus::DBusData;
use storage::{Db, traits::indexator::Indexator};
use tui::{PlaylistEntry, TrackInfo, Update};

use crate::{orchestrator::engine::EngineEvent, playlist_manager::PlaylistManager};

pub enum PlaylistManagerEvent {
    Next,
    Prev,
    Select(usize),
    Playlist(Playlist),
    /// загрузить плейлист по имени из бд.
    LoadByName(String),
    /// загрузить виртуальный плейлист со всем пулом песен.
    LoadPool,
    /// громкость текущего трека (0.0..=1.0).
    SetVolume(f32),
    /// сохранить плейлист в бд: имя + упорядоченные id треков.
    SavePlaylist { name: String, ids: Vec<i64> },
    /// собрать временный (несохраняемый) плейлист из id треков и проиграть.
    PlayTemp { ids: Vec<i64> },
}

pub fn spawn(
    rx: Receiver<PlaylistManagerEvent>,
    tx_engine: Arc<Sender<EngineEvent>>,
    tx_data: Sender<DBusData>,
    tx_ui: Sender<Update>,
    db: Option<Db>,
) {
    let worker_manager = std::thread::spawn(move || {
        handler_manager(tx_engine, tx_data, tx_ui, db, rx);
    });
}

/// публикует текущий трек: метаданные в MPRIS и его индекс в UI.
fn push_meta(tx_data: &Sender<DBusData>, tx_ui: &Sender<Update>, manager: &PlaylistManager) {
    let _ = tx_ui.send(Update::NowPlaying(manager.get_cur_number()));
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

/// краткий список треков плейлиста для UI.
fn playlist_view(p: &Playlist) -> Vec<TrackInfo> {
    p.tracks()
        .iter()
        .map(|t| {
            let (title, artists) = match t.get_metadata() {
                Ok(m) => (m.title.clone(), m.artist.join(", ")),
                Err(_) => ("Unknown".to_string(), String::new()),
            };
            TrackInfo {
                id: t.index_id().unwrap_or(-1),
                title,
                artists,
                volume: t.volume,
            }
        })
        .collect()
}

/// текущее время в секундах эпохи (для меток плейлиста).
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// собирает плейлист из упорядоченных id треков (по общему пулу бд).
fn playlist_from_ids(db: &Db, ids: &[i64]) -> Playlist {
    let mut tracks = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(found) = db.find_track(None, None, Some(*id), None)
            && let Some(track) = found.into_iter().next()
        {
            tracks.push(track);
        }
    }
    Playlist::from_tracks(tracks)
}

/// весь набор плейлистов для UI: пул «ALL SONGS» первым, затем плейлисты бд.
fn playlist_entries(db: &Db) -> Vec<PlaylistEntry> {
    let pool = db
        .pool_playlist()
        .unwrap_or_else(|_| Playlist::from_tracks(Vec::new()));
    let mut entries = vec![PlaylistEntry {
        name: "ALL SONGS".to_string(),
        tracks: playlist_view(&pool),
        pool: true,
        temp: false,
    }];
    entries.extend(db.list_playlists().unwrap_or_default().iter().map(|p| {
        PlaylistEntry {
            name: p.get_name().unwrap_or_else(|| "---".to_string()),
            tracks: playlist_view(p),
            pool: false,
            temp: false,
        }
    }));
    entries
}

/// отправляет текущий трек в движок (с его громкостью) и публикует метаданные.
fn play_current(
    manager: &mut PlaylistManager,
    tx_engine: &Sender<EngineEvent>,
    tx_data: &Sender<DBusData>,
    tx_ui: &Sender<Update>,
) {
    push_meta(tx_data, tx_ui, manager);
    if let Some(track) = manager.get_track_mut() {
        let vol = track.volume;
        if let Ok(b) = track.take_track() {
            let _ = tx_engine.send(EngineEvent::Add(b, vol));
        }
    }
}

fn handler_manager(
    tx_engine: Arc<Sender<EngineEvent>>,
    tx_data: Sender<DBusData>,
    tx_ui: Sender<Update>,
    db: Option<Db>,
    rx: Receiver<PlaylistManagerEvent>,
) {
    let mut manager = PlaylistManager::new();
    while let Ok(e) = rx.recv() {
        match e {
            PlaylistManagerEvent::Next => {
                match manager.next() {
                    Err(err) => println!("ERROR::Orchestrator::handler_manager::NEXT::{}", err),
                    Ok(None) => {
                        println!("WARN::Orchestrator::handler_manager::NEXT::track is not exist")
                    }
                    Ok(Some(track)) => {
                        let vol = track.volume;
                        match track.take_track() {
                            Ok(b) => {
                                let _ = tx_engine.send(EngineEvent::Add(b, vol));
                                push_meta(&tx_data, &tx_ui, &manager);
                                let _ = manager.load_next();
                            }
                            Err(_) => println!(
                                "ERROR::Orchestrator::handler_manager::NEXT::track is unloaded"
                            ),
                        }
                    }
                }
            }
            PlaylistManagerEvent::Prev => {
                match manager.prev() {
                    Err(err) => println!("ERROR::Orchestrator::handler_manager::PREV::{}", err),
                    Ok(None) => {
                        println!("WARN::Orchestrator::handler_manager::PREV::track is not exist")
                    }
                    Ok(Some(track)) => {
                        let vol = track.volume;
                        match track.take_track() {
                            Ok(b) => {
                                let _ = tx_engine.send(EngineEvent::Add(b, vol));
                                push_meta(&tx_data, &tx_ui, &manager);
                            }
                            Err(_) => println!(
                                "ERROR::Orchestrator::handler_manager::PREV::track is unloaded"
                            ),
                        }
                    }
                }
            }
            PlaylistManagerEvent::Select(number) => {
                if let Err(err) = manager.select_track(number) {
                    println!(
                        "ERROR::Orchestrator::handler_manager::select_track::{}",
                        err
                    );
                } else {
                    play_current(&mut manager, &tx_engine, &tx_data, &tx_ui);
                }
            }
            PlaylistManagerEvent::Playlist(p) => {
                // стартовый плейлист: UI уже знает его треки, вкладку не переключаем.
                if let Err(err) = manager.set_playlist(p) {
                    println!(
                        "ERROR::Orchestrator::handler_manager::set_playlist::{}",
                        err
                    );
                } else {
                    play_current(&mut manager, &tx_engine, &tx_data, &tx_ui);
                }
            }
            PlaylistManagerEvent::LoadByName(name) => {
                let Some(db) = &db else {
                    println!("WARN::Orchestrator::handler_manager::LoadByName::no db");
                    continue;
                };
                match db.load_playlist(name) {
                    Err(err) => {
                        println!("ERROR::Orchestrator::handler_manager::LoadByName::{}", err)
                    }
                    Ok(p) => {
                        let name = p.get_name().unwrap_or_else(|| "---".to_string());
                        let tracks = playlist_view(&p);
                        let _ = tx_ui.send(Update::Playlist { name, tracks });
                        if let Err(err) = manager.set_playlist(p) {
                            println!(
                                "ERROR::Orchestrator::handler_manager::LoadByName::set::{}",
                                err
                            );
                        } else {
                            play_current(&mut manager, &tx_engine, &tx_data, &tx_ui);
                        }
                    }
                }
            }
            PlaylistManagerEvent::LoadPool => {
                let Some(db) = &db else {
                    println!("WARN::Orchestrator::handler_manager::LoadPool::no db");
                    continue;
                };
                match db.pool_playlist() {
                    Err(err) => println!("ERROR::Orchestrator::handler_manager::LoadPool::{}", err),
                    Ok(p) => {
                        let tracks = playlist_view(&p);
                        let _ = tx_ui.send(Update::Playlist {
                            name: "ALL SONGS".to_string(),
                            tracks,
                        });
                        if let Err(err) = manager.set_playlist(p) {
                            println!("ERROR::Orchestrator::handler_manager::LoadPool::set::{}", err);
                        } else {
                            play_current(&mut manager, &tx_engine, &tx_data, &tx_ui);
                        }
                    }
                }
            }
            PlaylistManagerEvent::SetVolume(v) => {
                manager.set_current_volume(v);
                let _ = tx_engine.send(EngineEvent::SetVolume(v));
                if let Some(db) = &db
                    && let Some(track) = manager.get_track()
                    && let Err(err) = db.set_track_volume(track, v)
                {
                    println!("ERROR::Orchestrator::handler_manager::SetVolume::{}", err);
                }
            }
            PlaylistManagerEvent::SavePlaylist { name, ids } => {
                let Some(db) = &db else {
                    println!("WARN::Orchestrator::handler_manager::SavePlaylist::no db");
                    continue;
                };
                let mut playlist = playlist_from_ids(db, &ids);
                playlist.set_name(name);
                let now = now_secs();
                playlist.set_created_at(now);
                playlist.set_updated_at(now);
                match db.save_playlist(playlist) {
                    Err(err) => {
                        println!("ERROR::Orchestrator::handler_manager::SavePlaylist::{}", err)
                    }
                    // список плейлистов изменился — присылаем обновлённый набор в UI.
                    Ok(()) => {
                        let _ = tx_ui.send(Update::Playlists(playlist_entries(db)));
                    }
                }
            }
            PlaylistManagerEvent::PlayTemp { ids } => {
                let Some(db) = &db else {
                    println!("WARN::Orchestrator::handler_manager::PlayTemp::no db");
                    continue;
                };
                // анонимный плейлист: в бд не пишется, только проигрывается.
                let playlist = playlist_from_ids(db, &ids);
                let tracks = playlist_view(&playlist);
                let _ = tx_ui.send(Update::Playlist {
                    name: "(unnamed)".to_string(),
                    tracks,
                });
                if let Err(err) = manager.set_playlist(playlist) {
                    println!("ERROR::Orchestrator::handler_manager::PlayTemp::set::{}", err);
                } else {
                    play_current(&mut manager, &tx_engine, &tx_data, &tx_ui);
                }
            }
        }
    }
}
