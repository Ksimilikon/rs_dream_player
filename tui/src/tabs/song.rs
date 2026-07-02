use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    widgets::{Block, Borders, List, Paragraph},
};

use super::{Action, Tab, song_items, square};
use crate::model::Model;

/// вкладка с подробностями текущего плейлиста: список песен, инфо о треке и
/// квадратное место под обложку.
#[derive(Default)]
pub struct SongTab {
    cursor: usize,
}

impl Tab for SongTab {
    fn title(&self) -> &str {
        "SONG"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model) {
        let [left, center] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(area);

        frame.render_widget(
            List::new(song_items(&model.tracks, Some(model.current), Some(self.cursor)))
                .block(Block::new().borders(Borders::RIGHT).title("SONGS")),
            left,
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

        // заглушка под обложку (реальный рендер картинки — на будущее): в рамке
        // показываем путь до обложки; текстовая метка пользователя — под обложкой.
        let [cover_box, label_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(cover_area);
        let cover = square(cover_box);
        if cover.width >= 2 && cover.height >= 2 {
            let path = cur
                .and_then(|t| t.cover.clone())
                .unwrap_or_else(|| "<no cover>".to_string());
            frame.render_widget(
                Paragraph::new(path).block(Block::bordered().title("COVER")),
                cover,
            );
        }
        let label = cur
            .and_then(|t| t.user_label.clone())
            .filter(|l| !l.is_empty())
            .map(|l| format!("LABEL: {l}"))
            .unwrap_or_default();
        frame.render_widget(Paragraph::new(label), label_area);
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
}
