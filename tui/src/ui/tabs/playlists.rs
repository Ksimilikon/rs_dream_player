//! вкладка PLAYLISTS: слева список плейлистов (пул «ALL SONGS» первым), справа —
//! треки выбранного плейлиста. Переход вправо позволяет выбрать трек, с которого
//! начнётся воспроизведение. Команды: новый / правка / удаление плейлиста.

use ratatui::{Frame, crossterm::event::KeyEvent, crossterm::event::KeyCode, layout::Rect};

use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Hint, ListLine, ListView, two_pane},
};

#[derive(PartialEq, Eq)]
enum Focus {
    Left,
    Right,
}

pub struct PlaylistsTab {
    left: ListView,
    right: ListView,
    focus: Focus,
}

impl Default for PlaylistsTab {
    fn default() -> Self {
        Self {
            left: ListView::default(),
            right: ListView::default(),
            focus: Focus::Left,
        }
    }
}

impl PlaylistsTab {
    /// треки выбранного слева плейлиста.
    fn selected_tracks<'a>(&self, model: &'a Model) -> &'a [crate::ffi::TrackMeta] {
        model
            .playlists
            .get(self.left.selected())
            .map(|p| p.tracks.as_slice())
            .unwrap_or(&[])
    }
}

impl Tab for PlaylistsTab {
    fn title(&self) -> String {
        "PLAYLISTS".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model) {
        let (left_area, right_area) = two_pane(area, 40);

        // левая панель: имена плейлистов.
        let playing_name = model.playing.as_ref().map(|p| p.name.as_str());
        let left_lines: Vec<ListLine> = model
            .playlists
            .iter()
            .map(|p| {
                let count = p.tracks.len();
                ListLine::new(format!("{} ({count})", p.name))
                    .playing(Some(p.name.as_str()) == playing_name)
            })
            .collect();
        self.left
            .render(frame, left_area, "PLAYLISTS", left_lines, self.focus == Focus::Left);

        // правая панель: треки выбранного плейлиста.
        let tracks = self.selected_tracks(model);
        let right_lines: Vec<ListLine> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                ListLine::new(format!("{:>3}. {} — {}", i + 1, t.title, t.artists_line()))
            })
            .collect();
        let title = model
            .playlists
            .get(self.left.selected())
            .map(|p| p.name.as_str())
            .unwrap_or("TRACKS");
        self.right
            .render(frame, right_area, title, right_lines, self.focus == Focus::Right);
    }

    fn on_key(&mut self, key: KeyEvent, model: &Model) -> TabEvent {
        match key.code {
            KeyCode::Char('h') => {
                self.focus = Focus::Left;
                TabEvent::None
            }
            KeyCode::Char('l') => {
                self.focus = Focus::Right;
                TabEvent::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                match self.focus {
                    Focus::Left => self.left.down(),
                    Focus::Right => self.right.down(),
                }
                TabEvent::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                match self.focus {
                    Focus::Left => self.left.up(),
                    Focus::Right => self.right.up(),
                }
                TabEvent::None
            }
            KeyCode::Enter => {
                let index = self.left.selected();
                let start = match self.focus {
                    Focus::Right => self.right.selected(),
                    Focus::Left => 0,
                };
                TabEvent::PlayPlaylist { index, start }
            }
            KeyCode::Char('N') => TabEvent::NewPlaylist,
            KeyCode::Char('E') => {
                let idx = self.left.selected();
                if model.playlists.get(idx).is_some_and(|p| !p.pool) {
                    TabEvent::EditPlaylist(idx)
                } else {
                    TabEvent::Status("cannot edit the pool playlist".into())
                }
            }
            KeyCode::Char('D') => {
                let idx = self.left.selected();
                if model.playlists.get(idx).is_some_and(|p| !p.pool) {
                    TabEvent::DeletePlaylist(idx)
                } else {
                    TabEvent::Status("cannot delete the pool playlist".into())
                }
            }
            _ => TabEvent::None,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        vec![
            ("hjkl", "nav"),
            ("Ent", "play"),
            ("N", "new"),
            ("E", "edit"),
            ("D", "del"),
        ]
    }

    fn closable(&self) -> bool {
        false
    }

    fn on_tracks_changed(&mut self) {
        self.right.select(0);
    }
}
