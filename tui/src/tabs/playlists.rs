use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
};

use super::{Action, Tab, song_items};
use crate::model::Model;

/// вкладка со списком плейлистов (слева) и песнями текущего плейлиста (по центру).
#[derive(Default)]
pub struct PlaylistsTab {
    cursor: usize,
}

impl Tab for PlaylistsTab {
    fn title(&self) -> &str {
        "PLAYLISTS"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model) {
        let [left, center] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
                .areas(area);

        let items: Vec<ListItem> = model
            .playlists
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let tag = if entry.temp { " [temp]" } else { "" };
                let line = format!("{:>2}. {}{tag}", i + 1, entry.name);
                if i == self.cursor {
                    ListItem::new(line).style(Style::new().add_modifier(Modifier::REVERSED))
                } else {
                    ListItem::new(line)
                }
            })
            .collect();
        frame.render_widget(
            List::new(items).block(Block::new().borders(Borders::RIGHT).title("PLAYLISTS")),
            left,
        );

        // по центру — песни плейлиста ПОД ВЫДЕЛЕНИЕМ (не играющего)
        let preview = model
            .playlists
            .get(self.cursor)
            .map(|e| e.tracks.as_slice())
            .unwrap_or(&[]);
        frame.render_widget(
            List::new(song_items(preview, None, None)).block(Block::new().title("SONGS")),
            center,
        );
    }

    fn on_key(&mut self, key: KeyEvent, model: &Model) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Char('J') => {
                if self.cursor + 1 < model.playlists.len() {
                    self.cursor += 1;
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                self.cursor = self.cursor.saturating_sub(1);
                None
            }
            KeyCode::Enter => model.playlists.get(self.cursor).map(|e| {
                if e.pool {
                    Action::LoadPool
                } else if e.temp {
                    Action::PlayTemp(e.tracks.iter().map(|t| t.id).collect())
                } else {
                    Action::LoadPlaylist(e.name.clone())
                }
            }),
            // создать новый плейлист (спец-клавиша вкладки).
            KeyCode::Char('n') | KeyCode::Char('N') => Some(Action::NewPlaylist),
            // редактировать плейлист под курсором (нельзя пул).
            KeyCode::Char('e') | KeyCode::Char('E') => match model.playlists.get(self.cursor) {
                Some(e) if !e.pool => Some(Action::EditPlaylist(self.cursor)),
                _ => None,
            },
            _ => None,
        }
    }
}
