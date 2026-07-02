use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, List, ListState, Paragraph},
};

use super::{Action, Hint, Tab, song_items, square};
use crate::images::ImageManager;
use crate::model::Model;

/// вкладка с подробностями текущего плейлиста: список песен, инфо о треке и
/// (если терминал поддерживает graphics-протокол) обложка трека.
pub struct SongTab {
    cursor: usize,
    /// показ обложек в терминале + детекция поддержки протокола.
    images: ImageManager,
}

impl SongTab {
    pub fn new(images: ImageManager) -> Self {
        Self { cursor: 0, images }
    }
}

impl Tab for SongTab {
    fn title(&self) -> &str {
        "SONG"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model) {
        let [left, center] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(area);

        // прокрутка списка песен: держим курсор в поле зрения.
        let mut list_state = ListState::default().with_selected(Some(self.cursor));
        frame.render_stateful_widget(
            List::new(song_items(&model.tracks, Some(model.current), Some(self.cursor)))
                .block(Block::new().borders(Borders::RIGHT).title("SONGS")),
            left,
            &mut list_state,
        );

        // по центру — инфо о текущем треке + квадратное место под обложку
        let [info_area, cover_area] =
            Layout::vertical([Constraint::Length(5), Constraint::Min(1)]).areas(center);
        let cur = model.tracks.get(model.current);
        let title = cur.map(|t| t.title.clone()).unwrap_or_default();
        let artists = cur.map(|t| t.artists.clone()).unwrap_or_default();
        // альбом — под исполнителями; жанры через запятую — под альбомом (прочерк
        // если пусто).
        let album = cur
            .and_then(|t| t.album.clone())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "-".to_string());
        let genres = match cur.map(|t| t.genres.join(", ")) {
            Some(g) if !g.is_empty() => g,
            _ => "-".to_string(),
        };
        frame.render_widget(
            Paragraph::new(format!(
                "TITLE: {title}\nARTISTS: {artists}\nALBUM: {album}\nGENRES: {genres}"
            ))
            .block(Block::new().title("TRACK")),
            info_area,
        );

        // обложка текущего трека. Если терминал поддерживает graphics-протокол —
        // резервируем квадрат и рисуем картинку; иначе место под изображение
        // вообще не выделяется (схлопывается) — остаются только метка и путь.
        // приоритет — отдельный файл обложки из индекса; иначе достаём встроенную
        // обложку из тегов самого аудиофайла.
        self.images.set_source(
            cur.and_then(|t| t.cover.as_deref()),
            cur.and_then(|t| t.path.as_deref()),
        );

        let (label_area, path_area) = if self.images.supported() {
            let [cover_box, label_area, path_area] = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .areas(cover_area);
            let cover = square(cover_box);
            if cover.width >= 2 && cover.height >= 2 {
                if self.images.has_image() {
                    self.images.render(frame, cover);
                } else {
                    // протокол есть, но у трека нет обложки — показываем слот.
                    frame.render_widget(
                        Paragraph::new("<no cover>").block(Block::bordered().title("COVER")),
                        cover,
                    );
                }
            }
            (label_area, path_area)
        } else {
            // терминал не умеет графику: без квадрата под обложку.
            let [label_area, path_area, _rest] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .areas(cover_area);
            (label_area, path_area)
        };

        let label = cur
            .and_then(|t| t.user_label.clone())
            .filter(|l| !l.is_empty())
            .map(|l| format!("LABEL: {l}"))
            .unwrap_or_default();
        frame.render_widget(Paragraph::new(label), label_area);

        // путь к файлу трека на диске (прочерк, если неизвестен).
        let file_path = cur
            .and_then(|t| t.path.clone())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "-".to_string());
        frame.render_widget(Paragraph::new(format!("PATH: {file_path}")), path_area);
    }

    fn on_key(&mut self, key: KeyEvent, model: &Model) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Char('J') => {
                if self.cursor + 1 < model.tracks.len() {
                    self.cursor += 1;
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                self.cursor = self.cursor.saturating_sub(1);
                None
            }
            KeyCode::Enter => match model.tracks.get(self.cursor) {
                // недействительный трек — меню действий (путь/удаление), иначе играем.
                Some(t) if t.invalid => Some(Action::InvalidAction(t.id)),
                Some(_) => Some(Action::SelectSong(self.cursor)),
                None => None,
            },
            // редактор метаданных трека под курсором (эксклюзив вкладки SONG).
            KeyCode::Char('m') | KeyCode::Char('M') => {
                if self.cursor < model.tracks.len() {
                    Some(Action::EditMeta(self.cursor))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn hints(&self) -> &'static [Hint] {
        &[("j/k", "move"), ("Enter", "play"), ("m", "meta")]
    }

    /// новый плейлист — курсор списка песен возвращаем в начало.
    fn on_tracks_changed(&mut self) {
        self.cursor = 0;
    }
}
