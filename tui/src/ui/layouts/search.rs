//! поиск по списку треков (`/`): запрос вводится в нижней строке, список
//! фильтруется на лету. Совпадение — без учёта регистра по всем текстовым
//! метаданным (title, artists, album, genres, cover) и пути файла; запрос из
//! нескольких слов требует совпадения каждого слова (в любом из полей).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ffi::TrackMeta;
use crate::ui::layouts::TextField;

/// результат обработки клавиши во время ввода запроса.
pub enum SearchInput {
    /// запрос изменился — фильтр пересчитан, селектор стоит сбросить в начало.
    Edited,
    /// стрелки: навигация по отфильтрованному списку, не выходя из ввода.
    Up,
    Down,
    /// Enter: ввод завершён, фильтр остаётся (пустой запрос — поиск снят).
    Done,
    /// Esc: поиск отменён и снят.
    Cancel,
    /// клавиша не изменила ничего.
    Ignored,
}

/// состояние поиска одного списка: запрос + признак активного ввода.
#[derive(Default)]
pub struct Search {
    field: TextField,
    typing: bool,
}

impl Search {
    /// `/`: начать ввод. Уже применённый запрос сохраняется для правки.
    pub fn start(&mut self) {
        self.typing = true;
    }

    /// идёт ввод запроса (вкладка должна забирать весь ввод).
    pub fn typing(&self) -> bool {
        self.typing
    }

    /// фильтр действует (идёт ввод или применён непустой запрос).
    pub fn active(&self) -> bool {
        self.typing || !self.field.is_empty()
    }

    pub fn clear(&mut self) {
        self.field.clear();
        self.typing = false;
    }

    /// строка ввода для нижней панели: текст с ведущим `/` и позиция курсора.
    pub fn prompt(&self) -> Option<(String, usize)> {
        self.typing
            .then(|| (format!("/{}", self.field.text()), self.field.caret() + 1))
    }

    /// пометка для заголовка списка (пусто, если поиск не действует).
    pub fn label(&self) -> String {
        if self.active() {
            format!("  /{}", self.field.text())
        } else {
            String::new()
        }
    }

    /// предикат совпадения трека с запросом (без запроса — совпадает всё).
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

    /// индексы `tracks`, прошедшие фильтр (в исходном порядке).
    pub fn filter(&self, tracks: &[TrackMeta]) -> Vec<usize> {
        let m = self.matcher();
        (0..tracks.len()).filter(|&i| m(&tracks[i])).collect()
    }

    /// клавиша во время ввода запроса.
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

/// все текстовые поля трека одной строкой в нижнем регистре.
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
        // числовые поля (битрейт) не участвуют.
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
