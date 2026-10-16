//! unified list: selector, scrolling (via `ListState`), highlighting of the
//! "current" (playing) item and optional sorting. The data (lines) is passed
//! at draw time — the widget only keeps the navigation state.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::ui::PLAYING_GREEN;

/// sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn toggled(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SortDir::Asc => "asc",
            SortDir::Desc => "desc",
        }
    }
}

/// which metadata field the pool is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
}

impl SortKey {
    pub fn next(self) -> Self {
        match self {
            SortKey::Title => SortKey::Artist,
            SortKey::Artist => SortKey::Album,
            SortKey::Album => SortKey::Title,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Title => "title",
            SortKey::Artist => "artist",
            SortKey::Album => "album",
        }
    }
}

/// one list line to draw.
pub struct ListLine {
    pub text: String,
    /// highlight in green (the playing track, etc.).
    pub playing: bool,
    /// optional text color (e.g. red for invalid ones).
    pub color: Option<Color>,
}

impl ListLine {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            playing: false,
            color: None,
        }
    }
    pub fn playing(mut self, yes: bool) -> Self {
        self.playing = yes;
        self
    }
    pub fn color(mut self, c: Option<Color>) -> Self {
        self.color = c;
        self
    }
}

/// list navigation state.
pub struct ListView {
    state: ListState,
    len: usize,
}

impl Default for ListView {
    fn default() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self { state, len: 0 }
    }
}

impl ListView {
    pub fn selected(&self) -> usize {
        self.state.selected().unwrap_or(0)
    }

    /// syncs the length and clamps the selector into `[0, len)`.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        if len == 0 {
            self.state.select(Some(0));
        } else if self.selected() >= len {
            self.state.select(Some(len - 1));
        }
    }

    pub fn select(&mut self, i: usize) {
        let i = if self.len == 0 { 0 } else { i.min(self.len - 1) };
        self.state.select(Some(i));
    }

    pub fn up(&mut self) {
        let s = self.selected().saturating_sub(1);
        self.state.select(Some(s));
    }

    pub fn down(&mut self) {
        if self.len == 0 {
            return;
        }
        let s = (self.selected() + 1).min(self.len - 1);
        self.state.select(Some(s));
    }

    /// draws the list with a titled border. `focused` — highlight the border.
    pub fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        lines: Vec<ListLine>,
        focused: bool,
    ) {
        self.set_len(lines.len());
        let items: Vec<ListItem> = lines
            .into_iter()
            .map(|l| {
                let mut style = Style::default();
                if l.playing {
                    style = style.fg(PLAYING_GREEN).add_modifier(Modifier::BOLD);
                } else if let Some(c) = l.color {
                    style = style.fg(c);
                }
                ListItem::new(Line::from(l.text).style(style))
            })
            .collect();

        let border = if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border)
                    .title(format!(" {title} ")),
            )
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, area, &mut self.state);
    }
}
