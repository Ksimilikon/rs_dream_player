use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
};

use super::{Action, Tab, song_items};
use crate::model::Model;

/// активная панель вкладки: список плейлистов слева или песни справа.
#[derive(PartialEq)]
enum Focus {
    Left,
    Right,
}

/// вкладка со списком плейлистов (слева) и песнями текущего плейлиста (по центру).
/// hjkl переключают панель и двигают курсор; Enter слева грузит плейлист, справа —
/// играет плейлист с выбранной песни (для недействительных — меню действий).
pub struct PlaylistsTab {
    cursor: usize,
    song_cursor: usize,
    focus: Focus,
}

impl Default for PlaylistsTab {
    fn default() -> Self {
        Self {
            cursor: 0,
            song_cursor: 0,
            focus: Focus::Left,
        }
    }
}

impl PlaylistsTab {
    /// песни плейлиста под курсором (для правой панели).
    fn preview<'a>(&self, model: &'a Model) -> &'a [crate::model::TrackInfo] {
        model
            .playlists
            .get(self.cursor)
            .map(|e| e.tracks.as_slice())
            .unwrap_or(&[])
    }
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
                // курсор плейлиста подсвечиваем только при фокусе на левой панели.
                if i == self.cursor && self.focus == Focus::Left {
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

        // по центру — песни выбранного плейлиста; курсор песни при фокусе справа.
        let preview = self.preview(model);
        let song_cursor = (self.focus == Focus::Right).then_some(self.song_cursor);
        frame.render_widget(
            List::new(song_items(preview, None, song_cursor)).block(Block::new().title("SONGS")),
            center,
        );
    }

    fn on_key(&mut self, key: KeyEvent, model: &Model) -> Option<Action> {
        match key.code {
            // переключение панелей.
            KeyCode::Char('h') | KeyCode::Char('H') => {
                self.focus = Focus::Left;
                None
            }
            KeyCode::Char('l') | KeyCode::Char('L') => {
                if !self.preview(model).is_empty() {
                    self.focus = Focus::Right;
                }
                None
            }
            KeyCode::Char('j') | KeyCode::Char('J') => {
                match self.focus {
                    Focus::Left => {
                        if self.cursor + 1 < model.playlists.len() {
                            self.cursor += 1;
                            self.song_cursor = 0;
                        }
                    }
                    Focus::Right => {
                        if self.song_cursor + 1 < self.preview(model).len() {
                            self.song_cursor += 1;
                        }
                    }
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                match self.focus {
                    Focus::Left => {
                        self.cursor = self.cursor.saturating_sub(1);
                        self.song_cursor = 0;
                    }
                    Focus::Right => self.song_cursor = self.song_cursor.saturating_sub(1),
                }
                None
            }
            KeyCode::Enter => {
                let entry = model.playlists.get(self.cursor)?;
                // слева — грузим плейлист с начала; справа — с выбранной песни
                // (недействительная песня открывает меню действий).
                let start = match self.focus {
                    Focus::Left => 0,
                    Focus::Right => {
                        if let Some(song) = self.preview(model).get(self.song_cursor)
                            && song.invalid
                        {
                            return Some(Action::InvalidAction(song.id));
                        }
                        self.song_cursor
                    }
                };
                Some(if entry.pool {
                    Action::LoadPool { start }
                } else if entry.temp {
                    Action::PlayTemp {
                        ids: entry.tracks.iter().map(|t| t.id).collect(),
                        start,
                    }
                } else {
                    Action::LoadPlaylist {
                        name: entry.name.clone(),
                        start,
                    }
                })
            }
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
