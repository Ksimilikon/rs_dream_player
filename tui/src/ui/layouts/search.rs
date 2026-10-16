//! track list search (`/`): the query is typed in the bottom line, the list
//! is filtered on the fly. Matching is case-insensitive over every text
//! metadata field (title, artists, album, genres, cover) and the file path; a
//! multi-word query requires every word to match (in any of the fields).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ffi::TrackMeta;
use crate::ui::layouts::TextField;

/// result of handling a key while typing a query.
pub enum SearchInput {
    /// the query changed — the filter is recomputed, the selector should be
    /// reset to the top.
    Edited,
    /// arrows: navigate the filtered list without leaving input.
    Up,
    Down,
    /// Enter: input finished, the filter stays (an empty query clears the search).
    Done,
    /// Esc: the search is cancelled and cleared.
    Cancel,
    /// the key changed nothing.
    Ignored,
}

/// search state of one list: the query + an active-input flag.
#[derive(Default)]
pub struct Search {
    field: TextField,
    typing: bool,
}

impl Search {
    /// `/`: start typing. An already applied query is kept for editing.
    pub fn start(&mut self) {
        self.typing = true;
    }

    /// a query is being typed (the tab must capture all input).
    pub fn typing(&self) -> bool {
        self.typing
    }

    /// the filter is in effect (typing, or a non-empty query is applied).
    pub fn active(&self) -> bool {
        self.typing || !self.field.is_empty()
    }

    pub fn clear(&mut self) {
        self.field.clear();
        self.typing = false;
    }

    /// input line for the bottom panel: text with a leading `/` and the cursor
    /// position.
    pub fn prompt(&self) -> Option<(String, usize)> {
        self.typing
            .then(|| (format!("/{}", self.field.text()), self.field.caret() + 1))
    }

    /// a marker for the list title (empty if the search isn't in effect).
    pub fn label(&self) -> String {
        if self.active() {
            format!("  /{}", self.field.text())
        } else {
            String::new()
        }
    }

    /// whether a track matches the query (no query — everything matches).
    pub fn matcher(&self) -> impl Fn(&TrackMeta) -> bool + use<> {
        let terms: Vec<String> = self
            .field
            .text()
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        move |t| {
            if terms.is_empty() {
                return true;
            }
            let hay = haystack(t);
            terms.iter().all(|term| hay.contains(term.as_str()))
        }
    }

    /// indices of `tracks` that pass the filter (in the original order).
    pub fn filter(&self, tracks: &[TrackMeta]) -> Vec<usize> {
        let m = self.matcher();
        (0..tracks.len()).filter(|&i| m(&tracks[i])).collect()
    }

    /// a key while typing the query.
    pub fn on_key(&mut self, key: KeyEvent) -> SearchInput {
        let f = &mut self.field;
        match key.code {
            KeyCode::Esc => {
                self.clear();
                return SearchInput::Cancel;
            }
            KeyCode::Enter => {
                self.typing = false;
                return SearchInput::Done;
            }
            KeyCode::Up => return SearchInput::Up,
            KeyCode::Down => return SearchInput::Down,
            KeyCode::Backspace => f.backspace(),
            KeyCode::Delete => f.delete(),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                f.word_left();
                return SearchInput::Ignored;
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                f.word_right();
                return SearchInput::Ignored;
            }
            KeyCode::Left => {
                f.left();
                return SearchInput::Ignored;
            }
            KeyCode::Right => {
                f.right();
                return SearchInput::Ignored;
            }
            KeyCode::Home => {
                f.home();
                return SearchInput::Ignored;
            }
            KeyCode::End => {
                f.end();
                return SearchInput::Ignored;
            }
            KeyCode::Char(c) => f.insert(c),
            _ => return SearchInput::Ignored,
        }
        SearchInput::Edited
    }
}

/// every text field of a track as one lowercase string.
fn haystack(t: &TrackMeta) -> String {
    let mut parts: Vec<&str> = vec![t.title.as_str()];
    parts.extend(t.artists.iter().map(String::as_str));
    parts.extend(t.album.as_deref());
    parts.extend(t.genres.iter().map(String::as_str));
    parts.extend(t.path.as_deref());
    parts.extend(t.cover_art.as_deref());
    parts.join("\n").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;

    fn track(title: &str, artist: &str, path: &str) -> TrackMeta {
        TrackMeta {
            title: title.into(),
            artists: vec![artist.into()],
            path: Some(path.into()),
            bitrate: 320,
            ..Default::default()
        }
    }

    fn typed(q: &str) -> Search {
        let mut s = Search::default();
        s.start();
        for c in q.chars() {
            s.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        s
    }

    #[test]
    fn matches_fields_case_insensitive_all_words() {
        let tracks = [
            track("Song A", "Queen", "/music/rock/a.flac"),
            track("Other", "ABBA", "/music/pop/b.mp3"),
        ];
        assert_eq!(typed("queen").filter(&tracks), vec![0]);
        assert_eq!(typed("POP").filter(&tracks), vec![1]);
        assert_eq!(typed("music abba").filter(&tracks), vec![1]);
        assert_eq!(typed("music").filter(&tracks), vec![0, 1]);
        // numeric fields (bitrate) don't take part.
        assert!(typed("320").filter(&tracks).is_empty());
    }

    #[test]
    fn enter_keeps_filter_esc_clears() {
        let mut s = typed("x");
        s.on_key(KeyEvent::from(KeyCode::Enter));
        assert!(!s.typing() && s.active());
        s.start();
        s.on_key(KeyEvent::from(KeyCode::Esc));
        assert!(!s.active());
    }
}
