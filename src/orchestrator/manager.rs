use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    mpsc::{Receiver, Sender},
};
use std::time::{SystemTime, UNIX_EPOCH};

use audio_structs::{cover_art, playlist::Playlist, track_metadata::TrackMetadata};
use dbus::DBusData;
use storage::{Db, traits::indexator::Indexator};
use tui::{PlaylistEntry, TrackInfo, Update};

use crate::{config, orchestrator::engine::EngineEvent, playlist_manager::PlaylistManager};

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
    /// задать заголовок трека (тег файла + бд).
    SetTitle { id: i64, title: String },
    /// задать список артистов трека (тег файла + бд).
    SetArtists { id: i64, artists: Vec<String> },
    /// переименовать файл трека на диске (та же папка) + обновить путь в бд.
    RenameFile { id: i64, name: String },
    /// скопировать обложку в каталог конфига и сохранить путь в бд (приоритет).
    SetCover { id: i64, path: String },
    /// встроить обложку прямо в теги файла.
    SetCoverTag { id: i64, path: String },
    /// проиндексировать заданный каталог в бд.
    Scan(String),
    /// проверить наличие файлов треков; отсутствующие удаляются из индекса.
    /// `None` — весь индекс, `Some(name)` — конкретный плейлист.
    Check { playlist: Option<String> },
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
            let (title, artists, cover) = match t.get_metadata() {
                Ok(m) => (
                    m.title.clone(),
                    m.artist.join(", "),
                    m.params
                        .as_ref()
                        .and_then(|p| p.cover_art.as_ref())
                        .map(|c| c.to_string_lossy().into_owned()),
                ),
                Err(_) => ("Unknown".to_string(), String::new(), None),
            };
            TrackInfo {
                id: t.index_id().unwrap_or(-1),
                title,
                artists,
                volume: t.volume,
                cover,
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

/// один трек библиотеки по его id (или `None`, если не найден).
fn track_by_id(db: &Db, id: i64) -> Option<audio_structs::track_virtual::TrackVirtual> {
    db.find_track(None, None, Some(id), None)
        .ok()?
        .into_iter()
        .next()
}

/// перечитывает трек из бд и шлёт в UI обновление его метаданных + обновлённый
/// список плейлистов (для предпросмотров).
fn push_track_meta(db: &Db, tx_ui: &Sender<Update>, id: i64) {
    if let Some(track) = track_by_id(db, id)
        && let Ok(meta) = track.get_metadata()
    {
        let cover = meta
            .params
            .as_ref()
            .and_then(|p| p.cover_art.as_ref())
            .map(|c| c.to_string_lossy().into_owned());
        let _ = tx_ui.send(Update::TrackMeta {
            id,
            title: meta.title.clone(),
            artists: meta.artist.join(", "),
            cover,
        });
    }
    let _ = tx_ui.send(Update::Playlists(playlist_entries(db)));
}

/// задаёт заголовок трека: пишет тег файла (сохраняя текущих артистов) и бд.
fn edit_title(db: &Db, tx_ui: &Sender<Update>, id: i64, title: String) {
    let Some(track) = track_by_id(db, id) else {
        let _ = tx_ui.send(Update::Error(format!("track #{id} not found in index")));
        return;
    };
    let Some(path) = track.get_path() else {
        let _ = tx_ui.send(Update::Error("track has no file path".into()));
        return;
    };
    let artists = track
        .get_metadata()
        .map(|m| m.artist.clone())
        .unwrap_or_default();
    if let Err(e) = TrackMetadata::write_tags(path, &title, &artists) {
        let _ = tx_ui.send(Update::Error(format!("failed to write tags: {e}")));
        return;
    }
    if let Err(e) = db.update_track_meta(id, Some(&title), None) {
        let _ = tx_ui.send(Update::Error(format!("failed to update index: {e}")));
        return;
    }
    push_track_meta(db, tx_ui, id);
}

/// задаёт список артистов трека: пишет тег файла (сохраняя заголовок) и бд.
fn edit_artists(db: &Db, tx_ui: &Sender<Update>, id: i64, artists: Vec<String>) {
    let Some(track) = track_by_id(db, id) else {
        let _ = tx_ui.send(Update::Error(format!("track #{id} not found in index")));
        return;
    };
    let Some(path) = track.get_path() else {
        let _ = tx_ui.send(Update::Error("track has no file path".into()));
        return;
    };
    let title = track
        .get_metadata()
        .map(|m| m.title.clone())
        .unwrap_or_else(|_| "Unknown".to_string());
    if let Err(e) = TrackMetadata::write_tags(path, &title, &artists) {
        let _ = tx_ui.send(Update::Error(format!("failed to write tags: {e}")));
        return;
    }
    if let Err(e) = db.update_track_meta(id, None, Some(&artists)) {
        let _ = tx_ui.send(Update::Error(format!("failed to update index: {e}")));
        return;
    }
    push_track_meta(db, tx_ui, id);
}

/// переименовывает файл трека на диске (в той же папке, сохраняя расширение) и
/// обновляет путь в бд. `name` берётся как имя без каталога и без расширения.
fn edit_rename(db: &Db, tx_ui: &Sender<Update>, id: i64, name: String) {
    let Some(track) = track_by_id(db, id) else {
        let _ = tx_ui.send(Update::Error(format!("track #{id} not found in index")));
        return;
    };
    let Some(path) = track.get_path().map(Path::to_path_buf) else {
        let _ = tx_ui.send(Update::Error("track has no file path".into()));
        return;
    };
    // защита от подстановки каталога: берём только имя файла, без расширения.
    let stem = Path::new(&name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if stem.is_empty() {
        let _ = tx_ui.send(Update::Error("invalid file name".into()));
        return;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let new_path = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => parent.join(format!("{stem}.{ext}")),
        None => parent.join(stem),
    };
    if new_path == path {
        return;
    }
    if new_path.exists() {
        let _ = tx_ui.send(Update::Error("target file already exists".into()));
        return;
    }
    if let Err(e) = std::fs::rename(&path, &new_path) {
        let _ = tx_ui.send(Update::Error(format!("failed to rename file: {e}")));
        return;
    }
    if let Err(e) = db.rename_track_path(id, &new_path) {
        let _ = tx_ui.send(Update::Error(format!("failed to update index path: {e}")));
        return;
    }
    let _ = tx_ui.send(Update::Notice(format!(
        "renamed to {}",
        new_path.file_name().unwrap_or_default().to_string_lossy()
    )));
    let _ = tx_ui.send(Update::Playlists(playlist_entries(db)));
}

/// копирует изображение в каталог конфига (covers/) и сохраняет путь в бд —
/// приоритетный источник обложки для отображения.
fn edit_cover_db(db: &Db, tx_ui: &Sender<Update>, id: i64, path: String) {
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            let _ = tx_ui.send(Update::Error(format!("cannot read image: {e}")));
            return;
        }
    };
    let Some(cfg) = config::config_dir() else {
        let _ = tx_ui.send(Update::Error("no config directory available".into()));
        return;
    };
    let covers = cfg.join("covers");
    match cover_art::save_cover_art(&bytes, &covers, &id.to_string()) {
        Ok(saved) => {
            if let Err(e) = db.set_track_cover(id, Some(&saved)) {
                let _ = tx_ui.send(Update::Error(format!("failed to update index: {e}")));
                return;
            }
            push_track_meta(db, tx_ui, id);
        }
        Err(e) => {
            let _ = tx_ui.send(Update::Error(format!("invalid cover: {e}")));
        }
    }
}

/// встраивает изображение прямо в теги файла трека (без записи в бд).
fn edit_cover_tag(db: &Db, tx_ui: &Sender<Update>, id: i64, path: String) {
    let Some(track) = track_by_id(db, id) else {
        let _ = tx_ui.send(Update::Error(format!("track #{id} not found in index")));
        return;
    };
    let Some(file_path) = track.get_path() else {
        let _ = tx_ui.send(Update::Error("track has no file path".into()));
        return;
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            let _ = tx_ui.send(Update::Error(format!("cannot read image: {e}")));
            return;
        }
    };
    match TrackMetadata::write_cover(file_path, &bytes) {
        Ok(()) => {
            let _ = tx_ui.send(Update::Notice("cover embedded into file tags".into()));
        }
        Err(e) => {
            let _ = tx_ui.send(Update::Error(format!("failed to embed cover: {e}")));
        }
    }
}

/// индексирует каталог `dir` в бд и обновляет список плейлистов в UI.
fn scan_dir(db: &Db, tx_ui: &Sender<Update>, dir: String) {
    let path = PathBuf::from(&dir);
    if !path.is_dir() {
        let _ = tx_ui.send(Update::Error(format!("not a directory: {dir}")));
        return;
    }
    match db.index_dir(&path) {
        Ok(pool) => {
            let _ = tx_ui.send(Update::Playlists(playlist_entries(db)));
            let _ = tx_ui.send(Update::Notice(format!(
                "scan done: library now has {} tracks",
                pool.get_count()
            )));
        }
        Err(e) => {
            let _ = tx_ui.send(Update::Error(format!("scan failed: {e}")));
        }
    }
}

/// проверяет наличие файлов треков; отсутствующие удаляет из индекса и
/// сообщает о них красным. `playlist` = `None` — весь индекс.
fn check_files(db: &Db, tx_ui: &Sender<Update>, playlist: Option<String>) {
    let pairs = match &playlist {
        None => db.track_paths(),
        Some(name) => db.playlist_track_paths(name),
    };
    let pairs = match pairs {
        Ok(p) => p,
        Err(e) => {
            let _ = tx_ui.send(Update::Error(format!("check failed: {e}")));
            return;
        }
    };
    let missing: Vec<(i64, String)> = pairs
        .iter()
        .filter(|(_, p)| !p.exists())
        .map(|(id, p)| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned());
            (*id, name)
        })
        .collect();

    let scope = playlist.as_deref().unwrap_or("library");
    if missing.is_empty() {
        let _ = tx_ui.send(Update::Notice(format!(
            "all {} files present ({scope})",
            pairs.len()
        )));
        return;
    }
    for (id, _) in &missing {
        let _ = db.remove_track(*id);
    }
    let names: Vec<String> = missing.iter().map(|(_, n)| n.clone()).collect();
    let _ = tx_ui.send(Update::Error(format!(
        "removed {} missing tracks from index ({scope}), not on disk: {}",
        missing.len(),
        names.join(", ")
    )));
    let _ = tx_ui.send(Update::Playlists(playlist_entries(db)));
}

/// загружает и проигрывает текущий трек; при ошибке из-за отсутствия файла
/// удаляет трек из индекса и in-memory плейлиста, шлёт красное сообщение и
/// переходит к следующему. Повторяет, пока трек не загрузится или плейлист не
/// опустеет (задача 2).
fn play_current_recover(
    manager: &mut PlaylistManager,
    db: Option<&Db>,
    tx_engine: &Sender<EngineEvent>,
    tx_data: &Sender<DBusData>,
    tx_ui: &Sender<Update>,
) {
    loop {
        if manager.is_empty() {
            return;
        }
        if manager.load_current().is_ok() {
            play_current(manager, tx_engine, tx_data, tx_ui);
            return;
        }
        // загрузка не удалась: удаляем трек только если файла действительно нет.
        let desc = manager.current_descriptor();
        let missing = desc
            .as_ref()
            .map(|(_, path, _)| path.as_ref().map(|p| !p.exists()).unwrap_or(true))
            .unwrap_or(true);
        match (desc, missing) {
            (Some((index_id, _, title)), true) => {
                if let (Some(db), Some(id)) = (db, index_id) {
                    let _ = db.remove_track(id);
                }
                let _ = tx_ui.send(Update::Error(format!(
                    "track not found on disk, removed from index: {title}"
                )));
                manager.remove_current();
                if let Some(db) = db {
                    let _ = tx_ui.send(Update::Playlists(playlist_entries(db)));
                }
                // повторяем цикл: пробуем трек, вставший на освободившееся место.
            }
            _ => {
                let _ = tx_ui.send(Update::Error("failed to load current track".into()));
                return;
            }
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
                manager.step_next();
                play_current_recover(&mut manager, db.as_ref(), &tx_engine, &tx_data, &tx_ui);
            }
            PlaylistManagerEvent::Prev => {
                manager.step_prev();
                play_current_recover(&mut manager, db.as_ref(), &tx_engine, &tx_data, &tx_ui);
            }
            PlaylistManagerEvent::Select(number) => {
                if let Err(err) = manager.goto(number) {
                    println!(
                        "ERROR::Orchestrator::handler_manager::select_track::{}",
                        err
                    );
                } else {
                    play_current_recover(&mut manager, db.as_ref(), &tx_engine, &tx_data, &tx_ui);
                }
            }
            PlaylistManagerEvent::Playlist(p) => {
                // стартовый плейлист: UI уже знает его треки, вкладку не переключаем.
                let _ = manager.set_playlist(p);
                play_current_recover(&mut manager, db.as_ref(), &tx_engine, &tx_data, &tx_ui);
            }
            PlaylistManagerEvent::LoadByName(name) => {
                let Some(db_ref) = &db else {
                    println!("WARN::Orchestrator::handler_manager::LoadByName::no db");
                    continue;
                };
                match db_ref.load_playlist(name) {
                    Err(err) => {
                        println!("ERROR::Orchestrator::handler_manager::LoadByName::{}", err)
                    }
                    Ok(p) => {
                        let name = p.get_name().unwrap_or_else(|| "---".to_string());
                        let tracks = playlist_view(&p);
                        let _ = tx_ui.send(Update::Playlist { name, tracks });
                        let _ = manager.set_playlist(p);
                        play_current_recover(
                            &mut manager,
                            db.as_ref(),
                            &tx_engine,
                            &tx_data,
                            &tx_ui,
                        );
                    }
                }
            }
            PlaylistManagerEvent::LoadPool => {
                let Some(db_ref) = &db else {
                    println!("WARN::Orchestrator::handler_manager::LoadPool::no db");
                    continue;
                };
                match db_ref.pool_playlist() {
                    Err(err) => println!("ERROR::Orchestrator::handler_manager::LoadPool::{}", err),
                    Ok(p) => {
                        let tracks = playlist_view(&p);
                        let _ = tx_ui.send(Update::Playlist {
                            name: "ALL SONGS".to_string(),
                            tracks,
                        });
                        let _ = manager.set_playlist(p);
                        play_current_recover(
                            &mut manager,
                            db.as_ref(),
                            &tx_engine,
                            &tx_data,
                            &tx_ui,
                        );
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
                let Some(db_ref) = &db else {
                    println!("WARN::Orchestrator::handler_manager::PlayTemp::no db");
                    continue;
                };
                // анонимный плейлист: в бд не пишется, только проигрывается.
                let playlist = playlist_from_ids(db_ref, &ids);
                let tracks = playlist_view(&playlist);
                let _ = tx_ui.send(Update::Playlist {
                    name: "(unnamed)".to_string(),
                    tracks,
                });
                let _ = manager.set_playlist(playlist);
                play_current_recover(&mut manager, db.as_ref(), &tx_engine, &tx_data, &tx_ui);
            }
            // правка метаданных / индексация / проверка — требуют бд.
            PlaylistManagerEvent::SetTitle { id, title } => {
                if let Some(db) = &db {
                    edit_title(db, &tx_ui, id, title);
                }
            }
            PlaylistManagerEvent::SetArtists { id, artists } => {
                if let Some(db) = &db {
                    edit_artists(db, &tx_ui, id, artists);
                }
            }
            PlaylistManagerEvent::RenameFile { id, name } => {
                if let Some(db) = &db {
                    edit_rename(db, &tx_ui, id, name);
                }
            }
            PlaylistManagerEvent::SetCover { id, path } => {
                if let Some(db) = &db {
                    edit_cover_db(db, &tx_ui, id, path);
                }
            }
            PlaylistManagerEvent::SetCoverTag { id, path } => {
                if let Some(db) = &db {
                    edit_cover_tag(db, &tx_ui, id, path);
                }
            }
            PlaylistManagerEvent::Scan(dir) => {
                if let Some(db) = &db {
                    scan_dir(db, &tx_ui, dir);
                }
            }
            PlaylistManagerEvent::Check { playlist } => {
                if let Some(db) = &db {
                    check_files(db, &tx_ui, playlist);
                }
            }
        }
    }
}
