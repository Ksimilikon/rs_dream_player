//! MOD_SONG_<...> tab: editing the metadata of the selected track. Only tags
//! can be changed (title, artists, album, genres, cover, color, label);
//! technical fields (bitrate, etc.) are shown but not editable.
//! E toggles embedding the cover into the audio file on save.
//! Confirm — W, cancel — Esc.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::ffi::TrackMeta;
use crate::ui::{
    Model, MetaEdit, Tab, TabEvent,
    layouts::{Hint, TextField},
};

const LABEL_W: usize = 11;

struct Field {
    label: &'static str,
    value: TextField,
}

pub struct SongEditor {
    id: i64,
    title: String,
    fields: Vec<Field>,
    readonly: Vec<(String, String)>,
    sel: usize,
    editing: bool,
    /// embed the COVER picture into the file tags on save.
    embed_cover: bool,
}

impl SongEditor {
    pub fn new(track: &TrackMeta) -> Self {
        let field = |label, v: &str| Field {
            label,
            value: TextField::new(v),
        };
        let fields = vec![
            field("TITLE", &track.title),
            field("ARTISTS", &track.artists_line()),
            field("ALBUM", track.album.as_deref().unwrap_or("")),
            field("GENRES", &track.genres.join(", ")),
            field("COVER", track.cover_art.as_deref().unwrap_or("")),
            field("COLOR", ""),
            field("LABEL", ""),
        ];
        let readonly = vec![
            ("PATH".into(), track.path.clone().unwrap_or_else(|| "-".into())),
            ("BITRATE".into(), format!("{} kbps", track.bitrate)),
            ("SAMPLERATE".into(), format!("{} Hz", track.sample_rate)),
            ("DURATION".into(), format!("{}s", track.duration_s)),
            ("LISTENS".into(), track.listen_count.to_string()),
        ];
        let short: String = track.title.chars().take(5).collect();
        Self {
            id: track.id.unwrap_or(-1),
            title: format!("MOD_SONG_{short}"),
            fields,
            readonly,
            sel: 0,
            editing: false,
            embed_cover: false,
        }
    }

    /// builds the edit set (an empty title/artists/cover/color/label field —
    /// "don't change"; album/genres are always applied, empty clears them).
    fn to_edit(&self) -> MetaEdit {
        let get = |i: usize| self.fields[i].value.text();
        let split = |s: String| {
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let non_empty = |s: String| (!s.is_empty()).then_some(s);
        MetaEdit {
            id: self.id,
            title: non_empty(get(0)),
            artists: non_empty(get(1)).map(split),
            album: Some(get(2)),
            genres: Some(split(get(3))),
            cover: non_empty(get(4)),
            embed_cover: self.embed_cover,
            color: non_empty(get(5)),
            label: non_empty(get(6)),
        }
    }

    fn handle_edit(&mut self, key: KeyEvent) {
        let field = &mut self.fields[self.sel].value;
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => field.backspace(),
            KeyCode::Delete => field.delete(),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => field.word_left(),
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => field.word_right(),
            KeyCode::Left => field.left(),
            KeyCode::Right => field.right(),
            KeyCode::Home => field.home(),
            KeyCode::End => field.end(),
            KeyCode::Char(c) => field.insert(c),
            _ => {}
        }
    }
}

impl Tab for SongEditor {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" EDIT METADATA (editable) ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        for (i, f) in self.fields.iter().enumerate() {
            let marker = if i == self.sel { "> " } else { "  " };
            let label_style = if i == self.sel {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            lines.push(Line::from(vec![
                Span::raw(marker),
                Span::styled(format!("{:<width$}", f.label, width = LABEL_W), label_style),
                Span::raw(f.value.text()),
            ]));
        }
        let embed = if self.embed_cover { "[x]" } else { "[ ]" };
        lines.push(Line::from(Span::styled(
            format!("  {:<width$}{embed} embed cover into file (E)", "", width = LABEL_W),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "read-only:",
            Style::default().add_modifier(Modifier::UNDERLINED),
        )));
        for (k, v) in &self.readonly {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{k:<width$}", width = LABEL_W),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(v.clone()),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), inner);

        if self.editing {
            // caret: marker indent (2) + label width + position in the value.
            let col = inner.x + 2 + LABEL_W as u16 + self.fields[self.sel].value.caret() as u16;
            frame.set_cursor_position((col, inner.y + self.sel as u16));
        }
    }

    fn on_key(&mut self, key: KeyEvent, _model: &Model) -> TabEvent {
        if self.editing {
            self.handle_edit(key);
            return TabEvent::None;
        }
        match key.code {
            KeyCode::Esc => TabEvent::Close,
            KeyCode::Char('W') => TabEvent::SaveMeta(self.to_edit()),
            KeyCode::Char('E') => {
                self.embed_cover = !self.embed_cover;
                TabEvent::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.sel = (self.sel + 1).min(self.fields.len() - 1);
                TabEvent::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.sel = self.sel.saturating_sub(1);
                TabEvent::None
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                self.editing = true;
                TabEvent::None
            }
            _ => TabEvent::None,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        vec![("jk", "fld"), ("Ent", "edit"), ("E", "embed"), ("W", "save"), ("Esc", "cxl")]
    }

    /// all input is captured only while a field is being edited.
    fn wants_input(&self) -> bool {
        self.editing
    }
}
