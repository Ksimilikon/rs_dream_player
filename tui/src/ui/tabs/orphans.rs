//! ORPHANS tab (`:orphans`): the library's garbage collector — songs that are
//! in no playlist (the core finds them), each with its file path. `f` cycles
//! what is shown (not marked / kept / all), `i` marks a song "keep" (the
//! collector stops listing it) or clears the mark, `D` deletes the selected
//! song from the device, `A` — every shown one (both confirmed with y/N).

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::Paragraph,
};

use crate::ffi::{
    self, TrackMeta,
    db::OrphanFilter,
};
use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Hint, ListLine, ListView, fmt_time},
};

/// what the list shows, in `f` order.
const FILTERS: [(OrphanFilter, &str); 3] = [
    (OrphanFilter::NotIgnored, "not marked"),
    (OrphanFilter::Ignored, "kept"),
    (OrphanFilter::All, "all"),
];

pub struct OrphansTab {
    filter: usize,
    /// `None` — the core failed to answer.
    tracks: Option<Vec<TrackMeta>>,
    list: ListView,
}

impl OrphansTab {
    pub fn new() -> Self {
        let mut tab = Self {
            filter: 0,
            tracks: None,
            list: ListView::default(),
        };
        tab.reload();
        tab
    }

    fn reload(&mut self) {
        self.tracks = ffi::db::get_orphan_tracks(FILTERS[self.filter].0);
        self.list.set_len(self.shown().len());
    }

    fn shown(&self) -> &[TrackMeta] {
        self.tracks.as_deref().unwrap_or_default()
    }

    /// marks the selected song "keep" / clears the mark.
    fn toggle_keep(&mut self) -> TabEvent {
        let Some(t) = self.shown().get(self.list.selected()).cloned() else {
            return TabEvent::None;
        };
        let Some(id) = t.id else {
            return TabEvent::Status("the song is not from the library".into());
        };
        let keep = !t.ignore_orphan;
        if !ffi::db::set_track_ignore_orphan(id, keep) {
            return TabEvent::Status("failed to change the mark".into());
        }
        self.reload();
        TabEvent::Status(match keep {
            true => format!("'{}' kept: the collector won't list it", t.title),
            false => format!("'{}' is listed by the collector again", t.title),
        })
    }
}

impl Tab for OrphansTab {
    fn title(&self) -> String {
        "ORPHANS".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        let [header, list] = Layout::vertical([Constraint::Length(2), Constraint::Min(3)]).areas(area);
        let count = match &self.tracks {
            Some(t) => format!("{} songs", t.len()),
            None => "the core failed to answer".to_string(),
        };
        let filters: Vec<String> = FILTERS
            .iter()
            .enumerate()
            .map(|(i, (_, label))| match i == self.filter {
                true => format!("[{label}]"),
                false => label.to_string(),
            })
            .collect();
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    "SONGS IN NO PLAYLIST",
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ),
                Line::from(format!("{count}   show (f): {}", filters.join(" | "))),
            ]),
            header,
        );

        let lines: Vec<ListLine> = self
            .shown()
            .iter()
            .map(|t| {
                let kept = if t.ignore_orphan { "  [kept]" } else { "" };
                ListLine::new(format!(
                    "{:<32} {:>6}  {}{kept}",
                    t.title,
                    fmt_time(t.duration_s),
                    t.path.as_deref().unwrap_or("(no path)"),
                ))
                .color(t.invalid.then_some(Color::Red))
            })
            .collect();
        let empty = match FILTERS[self.filter].0 {
            OrphanFilter::Ignored => "no kept songs",
            _ => "every song is in some playlist",
        };
        let title = if lines.is_empty() { empty } else { "orphans" };
        self.list.render(frame, list, title, lines, true);
    }

    fn on_key(&mut self, key: KeyEvent, _model: &Model) -> TabEvent {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.list.down(),
            KeyCode::Char('k') | KeyCode::Up => self.list.up(),
            KeyCode::Char('f') => {
                self.filter = (self.filter + 1) % FILTERS.len();
                self.reload();
            }
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('i') => return self.toggle_keep(),
            KeyCode::Char('D') => {
                if let Some(t) = self.shown().get(self.list.selected()) {
                    return TabEvent::DeleteFromDevice(vec![t.clone()]);
                }
            }
            KeyCode::Char('A') if !self.shown().is_empty() => {
                return TabEvent::DeleteFromDevice(self.shown().to_vec());
            }
            _ => {}
        }
        TabEvent::None
    }

    fn hints(&self) -> Vec<Hint> {
        vec![
            ("jk", "nav"),
            ("f", "show"),
            ("i", "keep"),
            ("D", "del dev"),
            ("A", "del all"),
            ("r", "reload"),
        ]
    }

    /// the library changed (a deletion, a playlist edit): list again.
    fn on_tracks_changed(&mut self) {
        self.reload();
    }
}
