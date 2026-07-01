//! вкладки интерфейса: общий трейт и реализации.

use ratatui::{
    Frame,
    crossterm::event::KeyEvent,
    layout::Rect,
    style::{Modifier, Style},
    widgets::ListItem,
};

use crate::model::{Model, TrackInfo};

mod playlists;
mod settings;
mod song;

pub use playlists::PlaylistsTab;
pub use settings::SettingsTab;
pub use song::SongTab;

/// действие, которое вкладка просит выполнить в ответ на ввод.
pub enum Action {
    /// загрузить плейлист по имени.
    LoadPlaylist(String),
    /// загрузить виртуальный плейлист со всем пулом песен.
    LoadPool,
    /// выбрать трек по индексу.
    SelectSong(usize),
}

/// элементы списка песен: всегда с номерами. `playing` — индекс играющего
/// трека (маркер `▶`), `cursor` — индекс выделения курсором.
fn song_items(
    tracks: &[TrackInfo],
    playing: Option<usize>,
    cursor: Option<usize>,
) -> Vec<ListItem<'static>> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let marker = if Some(i) == playing { "▶" } else { " " };
            let line = format!("{marker}{:>2}. {} — {}", i + 1, t.title, t.artists);
            let item = ListItem::new(line);
            if Some(i) == cursor {
                item.style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                item
            }
        })
        .collect()
}

/// наибольший «визуальный квадрат» по центру `area`. Учитывает, что ячейка
/// терминала примерно вдвое выше своей ширины, поэтому ширина = 2 × высота.
fn square(area: Rect) -> Rect {
    let h = area.height.min(area.width / 2);
    let w = h * 2;
    Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// вкладка интерфейса. Хранит свой курсор, рисует из общего [`Model`] и
/// по клавишам возвращает [`Action`] для приложения.
pub trait Tab {
    fn title(&self) -> &str;
    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model);
    fn on_key(&mut self, key: KeyEvent, model: &Model) -> Option<Action>;
}
