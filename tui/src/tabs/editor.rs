//! редактор плейлиста: создание (#1), правка (#3) и временный плейлист (#5).
//!
//! Модальная «вкладка»: пока активна, забирает весь ввод у приложения. Слева —
//! песни плейлиста, справа — общий пул с поиском. Перемещение по блокам — hjkl,
//! выбор в пуле добавляет песню в плейлист, выбор в плейлисте убирает её. Порядок
//! в плейлисте меняется вводом позиции числом + Enter. Сохранение — Ctrl+S
//! (пустое имя = временный плейлист, в бд не пишется).

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::model::TrackInfo;

/// результат обработки клавиши редактором.
pub enum EditorOutcome {
    /// ничего для приложения — остаёмся в редакторе.
    None,
    /// выйти из редактора без изменений.
    Cancel,
    /// завершить: `name` пустое ⇒ временный плейлист (не в бд), иначе — запись
    /// в бд под этим именем; `ids` — упорядоченные id треков.
    Save { name: String, ids: Vec<i64> },
}

/// фаза работы: сначала вводим имя, потом редактируем состав.
enum Phase {
    Naming,
    Editing,
}

/// активный блок.
#[derive(PartialEq)]
enum Focus {
    Left,
    Right,
}

pub struct EditorState {
    /// имя плейлиста (пустое = временный при сохранении).
    name: String,
    phase: Phase,
    /// песни в плейлисте (левый блок).
    left: Vec<TrackInfo>,
    /// песни общего пула, ещё не добавленные в плейлист (правый блок).
    pool: Vec<TrackInfo>,
    focus: Focus,
    left_cursor: usize,
    right_cursor: usize,
    /// строка-фильтр пула (пустая = без фильтра).
    query: String,
    /// активен ли ввод поискового запроса.
    searching: bool,
    /// буфер ввода позиции для перестановки (в левом блоке).
    reorder: String,
}

impl EditorState {
    /// новый пустой плейлист: слева пусто, справа весь пул.
    pub fn create(pool: Vec<TrackInfo>) -> Self {
        Self::build(String::new(), Vec::new(), pool)
    }

    /// правка существующего: слева его треки, справа пул без них.
    pub fn edit(name: String, tracks: Vec<TrackInfo>, pool: Vec<TrackInfo>) -> Self {
        let in_left: std::collections::HashSet<i64> = tracks.iter().map(|t| t.id).collect();
        let rest = pool.into_iter().filter(|t| !in_left.contains(&t.id)).collect();
        Self::build(name, tracks, rest)
    }

    fn build(name: String, left: Vec<TrackInfo>, mut pool: Vec<TrackInfo>) -> Self {
        sort_by_title(&mut pool);
        Self {
            name,
            phase: Phase::Naming,
            left,
            pool,
            focus: Focus::Right,
            left_cursor: 0,
            right_cursor: 0,
            query: String::new(),
            searching: false,
            reorder: String::new(),
        }
    }

    /// индексы пула, проходящие фильтр `query` (по названию и артистам).
    fn filtered(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..self.pool.len()).collect();
        }
        let q = self.query.to_lowercase();
        self.pool
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.title.to_lowercase().contains(&q) || t.artists.to_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> EditorOutcome {
        match self.phase {
            Phase::Naming => self.key_naming(key),
            Phase::Editing => self.key_editing(key),
        }
    }

    fn key_naming(&mut self, key: KeyEvent) -> EditorOutcome {
        match key.code {
            KeyCode::Esc => EditorOutcome::Cancel,
            KeyCode::Enter => {
                self.phase = Phase::Editing;
                EditorOutcome::None
            }
            KeyCode::Backspace => {
                self.name.pop();
                EditorOutcome::None
            }
            KeyCode::Char(c) => {
                self.name.push(c);
                EditorOutcome::None
            }
            _ => EditorOutcome::None,
        }
    }

    fn key_editing(&mut self, key: KeyEvent) -> EditorOutcome {
        // сохранение — эксклюзивная клавиша редактора (работает в любом блоке).
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return EditorOutcome::Save {
                name: self.name.trim().to_string(),
                ids: self.left.iter().map(|t| t.id).collect(),
            };
        }

        // режим поиска по пулу забирает ввод символов.
        if self.searching {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.right_cursor = 0;
                }
                KeyCode::Char(c) => {
                    self.query.push(c);
                    self.right_cursor = 0;
                }
                _ => {}
            }
            return EditorOutcome::None;
        }

        match key.code {
            KeyCode::Char('/') => {
                self.focus = Focus::Right;
                self.searching = true;
            }
            KeyCode::Char('h') => self.focus = Focus::Left,
            KeyCode::Char('l') => self.focus = Focus::Right,
            KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Char('k') => self.move_cursor(-1),
            // ввод позиции для перестановки — только в левом блоке.
            KeyCode::Char(c @ '0'..='9') if self.focus == Focus::Left => self.reorder.push(c),
            KeyCode::Backspace if self.focus == Focus::Left && !self.reorder.is_empty() => {
                self.reorder.pop();
            }
            KeyCode::Enter => self.on_enter(),
            KeyCode::Esc => {
                if !self.reorder.is_empty() {
                    self.reorder.clear();
                } else {
                    return EditorOutcome::Cancel;
                }
            }
            _ => {}
        }
        EditorOutcome::None
    }

    fn move_cursor(&mut self, delta: i32) {
        match self.focus {
            Focus::Left => {
                self.left_cursor = step(self.left_cursor, delta, self.left.len());
            }
            Focus::Right => {
                self.right_cursor = step(self.right_cursor, delta, self.filtered().len());
            }
        }
    }

    fn on_enter(&mut self) {
        match self.focus {
            // пул → плейлист: добавить выбранную песню в конец.
            Focus::Right => {
                let filtered = self.filtered();
                if let Some(&pool_idx) = filtered.get(self.right_cursor) {
                    let track = self.pool.remove(pool_idx);
                    self.left.push(track);
                    let n = self.filtered().len();
                    if self.right_cursor >= n {
                        self.right_cursor = n.saturating_sub(1);
                    }
                }
            }
            // плейлист: перестановка (если введена позиция) или удаление.
            Focus::Left => {
                if self.left.is_empty() {
                    return;
                }
                if let Ok(pos) = self.reorder.parse::<usize>() {
                    self.reorder.clear();
                    let from = self.left_cursor;
                    let to = pos.clamp(1, self.left.len()) - 1;
                    let track = self.left.remove(from);
                    self.left.insert(to, track);
                    self.left_cursor = to;
                } else if !self.reorder.is_empty() {
                    self.reorder.clear();
                } else {
                    let track = self.left.remove(self.left_cursor);
                    self.pool.push(track);
                    sort_by_title(&mut self.pool);
                    if self.left_cursor >= self.left.len() {
                        self.left_cursor = self.left.len().saturating_sub(1);
                    }
                }
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let [name_area, lists, status, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        // строка имени. В фазе ввода имени показываем курсор.
        let name_line = match self.phase {
            Phase::Naming => format!("NAME: {}_   (Enter - next, Esc - cancel)", self.name),
            Phase::Editing => {
                let shown = if self.name.trim().is_empty() {
                    "<unnamed - temporary>".to_string()
                } else {
                    self.name.clone()
                };
                format!("NAME: {shown}")
            }
        };
        frame.render_widget(
            Paragraph::new(name_line).style(Style::new().fg(Color::Cyan)),
            name_area,
        );

        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(lists);

        // левый блок — песни плейлиста.
        let left_cursor = (self.focus == Focus::Left).then_some(self.left_cursor);
        frame.render_widget(
            List::new(items(&self.left, left_cursor)).block(
                Block::new()
                    .borders(Borders::RIGHT)
                    .title(focus_title("PLAYLIST", self.focus == Focus::Left)),
            ),
            left,
        );

        // правый блок — пул (по фильтру).
        let filtered = self.filtered();
        let pool_items: Vec<ListItem> = filtered
            .iter()
            .enumerate()
            .map(|(vis, &idx)| {
                let t = &self.pool[idx];
                let item = ListItem::new(format!("{} - {}", t.title, t.artists));
                if self.focus == Focus::Right && vis == self.right_cursor {
                    item.style(Style::new().add_modifier(Modifier::REVERSED))
                } else {
                    item
                }
            })
            .collect();
        frame.render_widget(
            List::new(pool_items).block(
                Block::new().title(focus_title("POOL (/ search)", self.focus == Focus::Right)),
            ),
            right,
        );

        // строка статуса: поиск и/или ввод позиции.
        let mut parts: Vec<String> = Vec::new();
        if self.searching || !self.query.is_empty() {
            let cur = if self.searching { "_" } else { "" };
            parts.push(format!("search: {}{cur}", self.query));
        }
        if !self.reorder.is_empty() {
            parts.push(format!("move to: {}", self.reorder));
        }
        frame.render_widget(
            Paragraph::new(parts.join("    ")).style(Style::new().fg(Color::Yellow)),
            status,
        );

        frame.render_widget(
            Paragraph::new(
                "h/l panel | j/k cursor | Enter add/remove | / search | digits+Enter position | Ctrl+S save | Esc exit",
            )
            .style(Style::new().fg(Color::DarkGray)),
            hints,
        );
    }
}

/// список песен с подсветкой курсора (если `cursor` задан).
fn items(tracks: &[TrackInfo], cursor: Option<usize>) -> Vec<ListItem<'static>> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let item = ListItem::new(format!("{:>2}. {} - {}", i + 1, t.title, t.artists));
            if Some(i) == cursor {
                item.style(Style::new().add_modifier(Modifier::REVERSED))
            } else {
                item
            }
        })
        .collect()
}

/// заголовок блока с маркером активного фокуса.
fn focus_title(base: &str, active: bool) -> String {
    if active {
        format!("> {base}")
    } else {
        format!("  {base}")
    }
}

fn sort_by_title(tracks: &mut [TrackInfo]) {
    tracks.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
}

/// сдвигает индекс `cur` на `delta` в пределах `[0, len)`.
fn step(cur: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let max = len - 1;
    if delta < 0 {
        cur.saturating_sub((-delta) as usize)
    } else {
        (cur + delta as usize).min(max)
    }
}
