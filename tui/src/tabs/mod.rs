//! вкладки интерфейса: общий трейт и реализации.

use ratatui::{
    Frame,
    crossterm::event::KeyEvent,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};

use crate::model::{Model, TrackInfo};

mod editor;
mod help;
mod invalid_prompt;
mod meta_editor;
mod playlists;
mod settings;
mod song;

pub use editor::{EditorOutcome, EditorState};
pub use help::{help_lines, render_help};
pub use invalid_prompt::{InvalidOutcome, InvalidPromptState};
pub use meta_editor::{MetaEdit, MetaEditorOutcome, MetaEditorState};
pub use playlists::PlaylistsTab;
pub use settings::SettingsTab;
pub use song::SongTab;

/// действие, которое вкладка просит выполнить в ответ на ввод.
pub enum Action {
    /// загрузить плейлист по имени и начать с трека `start`.
    LoadPlaylist { name: String, start: usize },
    /// загрузить виртуальный плейлист со всем пулом песен, начать с `start`.
    LoadPool { start: usize },
    /// проиграть временный плейлист по id треков (не из бд), начать с `start`.
    PlayTemp { ids: Vec<i64>, start: usize },
    /// выбрать трек по индексу.
    SelectSong(usize),
    /// открыть редактор для создания нового плейлиста.
    NewPlaylist,
    /// открыть редактор для правки плейлиста под курсором.
    EditPlaylist(usize),
    /// открыть редактор метаданных трека под курсором (вкладка SONG).
    EditMeta(usize),
    /// открыть меню действий для недействительного трека (задать путь/удалить).
    InvalidAction(i64),
}

/// сопоставляет имя цветовой метки цвету терминала. Неизвестное имя → без цвета.
pub fn color_of(name: &str) -> Option<Color> {
    match name.to_ascii_lowercase().as_str() {
        "red" => Some(Color::Rgb(230, 70, 70)),
        "pink" => Some(Color::Rgb(255, 105, 180)),
        "orange" => Some(Color::Rgb(255, 165, 0)),
        "green" => Some(Color::Rgb(70, 200, 90)),
        "blue" => Some(Color::Rgb(70, 120, 230)),
        "cyan" => Some(Color::Rgb(60, 200, 220)),
        "purple" => Some(Color::Rgb(170, 90, 220)),
        _ => None,
    }
}

/// список допустимых имён цветовых меток (для валидации команды `:color`).
pub const COLOR_NAMES: &[&str] = &[
    "red", "pink", "orange", "green", "blue", "cyan", "purple",
];

/// элементы списка песен: всегда с номерами. `playing` — индекс играющего
/// трека (маркер `>`), `cursor` — индекс выделения курсором. Цветовая метка —
/// квадратик перед названием; альбом — `[альбом]` в конце; недействительные
/// треки целиком красные.
fn song_items(
    tracks: &[TrackInfo],
    playing: Option<usize>,
    cursor: Option<usize>,
) -> Vec<ListItem<'static>> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let marker = if Some(i) == playing { ">" } else { " " };
            let mut spans: Vec<Span> = Vec::new();

            // цветовой квадратик (если задан) — перед номером.
            match t.color.as_deref().and_then(color_of) {
                Some(c) => spans.push(Span::styled("■ ", Style::new().fg(c))),
                None => spans.push(Span::raw("  ")),
            }

            let album = match &t.album {
                Some(a) if !a.is_empty() => format!(" [{a}]"),
                _ => String::new(),
            };
            spans.push(Span::raw(format!(
                "{marker}{:>2}. {} - {}{album}",
                i + 1,
                t.title,
                t.artists
            )));

            let mut line = Line::from(spans);
            if t.invalid {
                // недействительные треки — красным.
                line = line.style(Style::new().fg(Color::Red));
            }
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

/// центрированный прямоугольник шириной `w` и высотой `h` (в клетках),
/// обрезанный по границам `area`. Для всплывающих окон (help, промпты).
fn centered_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let [_, mid, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(h),
        Constraint::Min(0),
    ])
    .areas(area);
    let [_, cell, _] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(w),
        Constraint::Min(0),
    ])
    .areas(mid);
    cell
}

/// вкладка интерфейса. Хранит свой курсор, рисует из общего [`Model`] и
/// по клавишам возвращает [`Action`] для приложения.
pub trait Tab {
    fn title(&self) -> &str;
    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model);
    fn on_key(&mut self, key: KeyEvent, model: &Model) -> Option<Action>;
}
