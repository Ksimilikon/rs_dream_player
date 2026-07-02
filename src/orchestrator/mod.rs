use std::sync::{
    Arc,
    mpsc::{Receiver, Sender, channel},
};

use ::dbus::{DBusData, DBusEvent};
use audio_structs::playlist::Playlist;
use storage::Db;
use tui::Update;

use crate::orchestrator::{engine::EngineEvent, manager::PlaylistManagerEvent};

/// ручка управления воспроизведением для внешних слоёв (например, TUI).
#[derive(Clone)]
pub struct Controls {
    tx_manager: Arc<Sender<PlaylistManagerEvent>>,
    tx_engine: Arc<Sender<EngineEvent>>,
}

impl Controls {
    pub fn next(&self) {
        let _ = self.tx_manager.send(PlaylistManagerEvent::Next);
    }
    pub fn prev(&self) {
        let _ = self.tx_manager.send(PlaylistManagerEvent::Prev);
    }
    pub fn play_pause(&self) {
        let _ = self.tx_engine.send(EngineEvent::PlayPause);
    }
    /// выбрать и проиграть трек по индексу.
    pub fn select(&self, index: usize) {
        let _ = self.tx_manager.send(PlaylistManagerEvent::Select(index));
    }
    /// загрузить плейлист по имени из бд и начать с трека `start`.
    pub fn load_playlist(&self, name: String, start: usize) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::LoadByName { name, start });
    }
    /// загрузить виртуальный плейлист со всем пулом песен, начать с трека `start`.
    pub fn load_pool(&self, start: usize) {
        let _ = self.tx_manager.send(PlaylistManagerEvent::LoadPool { start });
    }
    /// громкость текущей песни.
    pub fn set_song_volume(&self, volume: f32) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetVolume(volume));
    }
    /// общая (мастер-) громкость.
    pub fn set_master_volume(&self, volume: f32) {
        let _ = self.tx_engine.send(EngineEvent::SetMaster(volume));
    }
    /// сохранить плейлист в бд: имя + упорядоченные id треков.
    pub fn save_playlist(&self, name: String, ids: Vec<i64>) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SavePlaylist { name, ids });
    }
    /// собрать временный (несохраняемый) плейлист из id треков и проиграть с `start`.
    pub fn play_temp(&self, ids: Vec<i64>, start: usize) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::PlayTemp { ids, start });
    }
    /// задать заголовок трека (тег файла + бд).
    pub fn set_title(&self, id: i64, title: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetTitle { id, title });
    }
    /// задать список артистов трека (тег файла + бд).
    pub fn set_artists(&self, id: i64, artists: Vec<String>) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetArtists { id, artists });
    }
    /// задать альбом трека (тег файла + бд).
    pub fn set_album(&self, id: i64, album: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetAlbum { id, album });
    }
    /// задать список жанров трека (тег файла + бд).
    pub fn set_genres(&self, id: i64, genres: Vec<String>) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetGenres { id, genres });
    }
    /// задать цветовую метку трека (бд).
    pub fn set_color(&self, id: i64, color: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetColor { id, color });
    }
    /// задать текстовую метку трека (бд).
    pub fn set_label(&self, id: i64, label: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetLabel { id, label });
    }
    /// переименовать файл трека на диске + обновить путь в бд.
    pub fn rename_file(&self, id: i64, name: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::RenameFile { id, name });
    }
    /// присвоить недействительному треку новый путь к файлу.
    pub fn set_path(&self, id: i64, path: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetPath { id, path });
    }
    /// удалить трек из индекса (каскадом из плейлистов).
    pub fn remove_track(&self, id: i64) {
        let _ = self.tx_manager.send(PlaylistManagerEvent::RemoveTrack(id));
    }
    /// удалить из индекса все недействительные треки.
    pub fn purge_invalid(&self) {
        let _ = self.tx_manager.send(PlaylistManagerEvent::PurgeInvalid);
    }
    /// скопировать обложку в каталог конфига и сохранить путь в бд.
    pub fn set_cover(&self, id: i64, path: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetCover { id, path });
    }
    /// встроить обложку прямо в теги файла.
    pub fn set_cover_tag(&self, id: i64, path: String) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::SetCoverTag { id, path });
    }
    /// проиндексировать заданный каталог в бд.
    pub fn scan(&self, dir: String) {
        let _ = self.tx_manager.send(PlaylistManagerEvent::Scan(dir));
    }
    /// проверить наличие файлов: `None` — весь индекс, `Some(name)` — плейлист.
    pub fn check(&self, playlist: Option<String>) {
        let _ = self
            .tx_manager
            .send(PlaylistManagerEvent::Check { playlist });
    }
}

pub mod dbus;
pub mod engine;
pub mod errors;
pub mod manager;

pub struct Orchestrator {}
impl Orchestrator {
    /// поднимает воркеры. `db` уходит во владение менеджеру (загрузка плейлистов
    /// и сохранение громкости в рантайме), `master` — стартовая общая громкость.
    /// `initial` проигрывается сразу, если задан (режим `--playlist`); иначе плеер
    /// ждёт выбора плейлиста из UI. Возвращает канал обновлений и ручку управления.
    pub fn run(
        db: Option<Db>,
        initial: Option<Playlist>,
        master: f32,
    ) -> (Receiver<Update>, Controls) {
        let (tx_manager, rx_manager) = channel::<PlaylistManagerEvent>();
        let (tx_engine, rx_engine) = channel::<engine::EngineEvent>();

        let (tx_cmd, rx_cmd) = channel::<DBusEvent>();
        let (tx_data, rx_data) = channel::<DBusData>();
        let (tx_ui, rx_ui) = channel::<Update>();

        let arc_tx_manager = Arc::new(tx_manager);
        let arc_tx_engine = Arc::new(tx_engine);

        dbus::spawn(
            tx_cmd,
            rx_cmd,
            rx_data,
            arc_tx_manager.clone(),
            arc_tx_engine.clone(),
        );
        manager::spawn(rx_manager, arc_tx_engine.clone(), tx_data, tx_ui, db);
        engine::spawn(rx_engine, arc_tx_manager.clone(), master);

        // явный --playlist стартует сразу; в обычном режиме плеер ждёт выбора.
        if let Some(playlist) = initial {
            let _ = arc_tx_manager.send(PlaylistManagerEvent::Playlist(playlist));
        }

        let controls = Controls {
            tx_manager: arc_tx_manager,
            tx_engine: arc_tx_engine,
        };
        (rx_ui, controls)
    }
}
