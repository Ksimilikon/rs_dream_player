//! слой интерфейса: [`layouts`] — переиспользуемые заготовки, [`tabs`] —
//! конкретные вкладки с логикой. Здесь же общая view-модель, которую вкладки
//! читают, и трейт [`Tab`].

pub mod layouts;
pub mod tabs;

use ratatui::{Frame, crossterm::event::KeyEvent, layout::Rect, style::Color};

use crate::ffi::TrackMeta;
use layouts::Hint;

/// цвет проигрываемого трека/акцентов.
pub const PLAYING_GREEN: Color = Color::Green;
/// цвет заголовка справки (shift+x закрывает).
pub const HELP_RED: Color = Color::Red;

/// плейлист в представлении интерфейса (имя + треки + признаки).
pub struct PlaylistView {
    pub name: String,
    pub tracks: Vec<TrackMeta>,
    /// общий пул «ALL SONGS» (первый в списке, неудаляемый).
    pub pool: bool,
    /// id плейлиста в бд (для операций); `None` у пула.
    pub id: Option<i64>,
}

/// состояние проигрываемого плейлиста.
pub struct Playing {
    pub name: String,
    pub tracks: Vec<TrackMeta>,
    /// индекс играющего трека в `tracks`.
    pub current: usize,
}

/// view-модель, доступная вкладкам (read-only при отрисовке/обработке клавиш).
pub struct Model {
    /// библиотека: `[0]` — пул «ALL SONGS», далее плейлисты из бд.
    pub playlists: Vec<PlaylistView>,
    /// активный проигрываемый плейлист (если есть).
    pub playing: Option<Playing>,
    pub master_vol: f32,
    /// позиция воспроизведения в секундах (для прогресс-бара).
    pub pos: u64,
}

impl Model {
    /// пул «ALL SONGS».
    pub fn pool(&self) -> &[TrackMeta] {
        self.playlists
            .iter()
            .find(|p| p.pool)
            .map(|p| p.tracks.as_slice())
            .unwrap_or(&[])
    }
}

/// действие, которое вкладка просит выполнить у приложения.
pub enum TabEvent {
    None,
    /// закрыть текущую (транзиентную) вкладку (отмена редактора).
    Close,
    /// проиграть плейлист `index` из библиотеки, начиная с трека `start`.
    PlayPlaylist { index: usize, start: usize },
    /// выбрать играющий трек по индексу (вкладка SONG).
    SelectPlaying(usize),
    /// открыть вкладку создания плейлиста.
    NewPlaylist,
    /// открыть вкладку правки плейлиста `index` из библиотеки.
    EditPlaylist(usize),
    /// открыть правку играющего плейлиста (вкладка SONG).
    EditPlayingPlaylist,
    /// удалить плейлист `index` из библиотеки.
    DeletePlaylist(usize),
    /// открыть редактор метаданных трека `index` в играющем списке.
    EditMeta(usize),
    /// переставить трек в играющем списке (временно, без записи в бд).
    MovePlaying { from: usize, to: usize },
    /// убрать трек `index` из играющего списка; `from_db` — ещё и из плейлиста в бд.
    RemovePlaying { index: usize, from_db: bool },
    /// сохранить плейлист: имя + id треков (новый или правка).
    SavePlaylist { name: String, ids: Vec<i64> },
    /// применить правку метаданных трека.
    SaveMeta(MetaEdit),
    /// переключить устройство вывода по имени.
    SetOutputDevice(String),
    /// разовое сообщение в статус-строку.
    Status(String),
}

/// набор правок метаданных трека (пустое поле `None` — не менять).
#[derive(Default)]
pub struct MetaEdit {
    pub id: i64,
    pub title: Option<String>,
    pub artists: Option<Vec<String>>,
    pub album: Option<String>,
    pub genres: Option<Vec<String>>,
    pub cover: Option<String>,
    pub color: Option<String>,
    pub label: Option<String>,
}

/// вкладка интерфейса.
pub trait Tab {
    /// заголовок для бара вкладок.
    fn title(&self) -> String;
    /// отрисовка в выделенную область.
    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model);
    /// обработка клавиши (кроме глобальных) -> запрошенное действие.
    fn on_key(&mut self, key: KeyEvent, model: &Model) -> TabEvent;
    /// подсказки по командам вкладки (для нижней строки).
    fn hints(&self) -> Vec<Hint>;
    /// можно ли закрыть вкладку по Shift+X. Закрываемы все, кроме PLAYLISTS.
    fn closable(&self) -> bool {
        true
    }
    /// вкладка прямо сейчас вводит текст в поле: на это время приложение отдаёт
    /// ей весь ввод (чтобы буквы не срабатывали как глобальные команды). Вне
    /// активного ввода вкладка ведёт себя как все (H/L, глобальные команды).
    fn wants_input(&self) -> bool {
        false
    }
    /// список треков сменился — сбросить курсоры.
    fn on_tracks_changed(&mut self) {}
}
