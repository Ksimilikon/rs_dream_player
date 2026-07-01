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
            Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(center);
        let cur = model.tracks.get(model.current);
        let title = cur.map(|t| t.title.clone()).unwrap_or_default();
        let artists = cur.map(|t| t.artists.clone()).unwrap_or_default();
        frame.render_widget(
            Paragraph::new(format!("TITLE: {title}\nARTISTS: {artists}"))
                .block(Block::new().title("TRACK")),
            info_area,
        );

        // заглушка под обложку (реальный рендер картинки — на будущее).
        // Квадрат подстраивается под размер области.
        let cover = square(cover_area);
        if cover.width >= 2 && cover.height >= 2 {
            frame.render_widget(Block::bordered().title("COVER"), cover);
        }
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
            KeyCode::Enter => {
                if self.cursor < model.tracks.len() {
                    Some(Action::SelectSong(self.cursor))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
