//! вкладка NEW_PLAYLIST / MOD_<name>: создание и правка плейлиста. Сверху поле
//! имени, слева собираемый плейлист (упорядочен), справа пул песен с чекбоксами.
//! Настройки пула: сортировка по полю (S), направление (R), скрывать добавленные
//! (T, по умолчанию вкл). Подтверждение — W, отмена — Esc.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Paragraph},
};

use crate::ffi::TrackMeta;
use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Hint, ListLine, ListView, SortDir, SortKey, TextField, target_index},
};

#[derive(PartialEq, Eq, Clone, Copy)]
enum Focus {
    Name,
    Left,
    Right,
}

pub struct PlaylistEditor {
    title: String,
    name: TextField,
    assembled: Vec<TrackMeta>,
    pool: Vec<TrackMeta>,
    left: ListView,
    right: ListView,
    focus: Focus,
    editing_name: bool,
    sort_key: SortKey,
    sort_dir: SortDir,
    hide_added: bool,
    /// набираемый номер позиции для перемещения трека в собираемом списке.
    move_buf: String,
}

impl PlaylistEditor {
    /// новый плейлист (пустые имя и список).
    pub fn create(pool: Vec<TrackMeta>) -> Self {
        Self::build("NEW_PLAYLIST".to_string(), TextField::new(""), Vec::new(), pool)
    }

    /// правка существующего плейлиста (имя и треки предзаполнены).
    pub fn edit(name: String, tracks: Vec<TrackMeta>, pool: Vec<TrackMeta>) -> Self {
        Self::build(format!("MOD_{name}"), TextField::new(&name), tracks, pool)
    }

    fn build(title: String, name: TextField, assembled: Vec<TrackMeta>, pool: Vec<TrackMeta>) -> Self {
        Self {
            title,
            name,
            assembled,
            pool,
            left: ListView::default(),
            right: ListView::default(),
            focus: Focus::Left,
            editing_name: false,
            sort_key: SortKey::Title,
            sort_dir: SortDir::Asc,
            hide_added: true,
            move_buf: String::new(),
        }
    }

    fn is_added(&self, id: Option<i64>) -> bool {
        id.is_some_and(|id| self.assembled.iter().any(|t| t.id == Some(id)))
    }

    /// индексы пула к показу (фильтр «скрыть добавленные» + сортировка).
    fn pool_view(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.pool.len())
            .filter(|&i| !(self.hide_added && self.is_added(self.pool[i].id)))
            .collect();
        let key = self.sort_key;
        idx.sort_by(|&a, &b| {
            let ka = sort_field(&self.pool[a], key).to_lowercase();
            let kb = sort_field(&self.pool[b], key).to_lowercase();
            ka.cmp(&kb)
        });
        if self.sort_dir == SortDir::Desc {
            idx.reverse();
        }
        idx
    }

    fn add_selected(&mut self) {
        let view = self.pool_view();
        if let Some(&pi) = view.get(self.right.selected()) {
            let track = self.pool[pi].clone();
            if !self.is_added(track.id) {
                self.assembled.push(track);
            }
        }
    }

    fn remove_selected(&mut self) {
        let i = self.left.selected();
        if i < self.assembled.len() {
            self.assembled.remove(i);
        }
    }

    /// id собранных треков (для сохранения).
    fn ids(&self) -> Vec<i64> {
        self.assembled.iter().filter_map(|t| t.id).collect()
    }

    fn handle_edit_name(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing_name = false,
            KeyCode::Backspace => self.name.backspace(),
            KeyCode::Delete => self.name.delete(),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => self.name.word_left(),
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => self.name.word_right(),
            KeyCode::Left => self.name.left(),
            KeyCode::Right => self.name.right(),
            KeyCode::Home => self.name.home(),
            KeyCode::End => self.name.end(),
            KeyCode::Char(c) => self.name.insert(c),
            _ => {}
        }
    }
}

impl Tab for PlaylistEditor {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        let [name_area, panes] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(area);

        // поле имени.
        let name_focused = self.focus == Focus::Name;
        let name_border = if name_focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        frame.render_widget(
            Paragraph::new(self.name.text()).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(name_border)
                    .title(" NAME (Enter to edit) "),
            ),
            name_area,
        );
        if self.editing_name {
            frame.set_cursor_position((name_area.x + 1 + self.name.caret() as u16, name_area.y + 1));
        }

        let (left_area, right_area) = {
            let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(panes);
            (l, r)
        };

        // слева — собираемый плейлист.
        let left_lines: Vec<ListLine> = self
            .assembled
            .iter()
            .enumerate()
            .map(|(i, t)| ListLine::new(format!("{:>3}. {} — {}", i + 1, t.title, t.artists_line())))
            .collect();
        let left_title = if self.move_buf.is_empty() {
            "PLAYLIST".to_string()
        } else {
            format!("PLAYLIST  move→{}", self.move_buf)
        };
        self.left
            .render(frame, left_area, &left_title, left_lines, self.focus == Focus::Left);

        // справа — пул с чекбоксами и строкой настроек в заголовке.
        let view = self.pool_view();
        let right_lines: Vec<ListLine> = view
            .iter()
            .map(|&pi| {
                let t = &self.pool[pi];
                let mark = if self.is_added(t.id) { "[x]" } else { "[ ]" };
                ListLine::new(format!("{mark} {} — {}", t.title, t.artists_line()))
            })
            .collect();
        let right_title = format!(
            "POOL  sort:{} {}  hide:{}",
            self.sort_key.label(),
            self.sort_dir.label(),
            if self.hide_added { "on" } else { "off" }
        );
        self.right
            .render(frame, right_area, &right_title, right_lines, self.focus == Focus::Right);
    }

    fn on_key(&mut self, key: KeyEvent, _model: &Model) -> TabEvent {
        if self.editing_name {
            self.handle_edit_name(key);
            return TabEvent::None;
        }
        // набор номера позиции (только в собираемом списке): копим цифры,
        // Enter — переместить выбранный трек на эту позицию.
        if let KeyCode::Char(c) = key.code
            && c.is_ascii_digit()
        {
            if self.focus == Focus::Left && self.move_buf.len() < 6 {
                self.move_buf.push(c);
            }
            return TabEvent::None;
        }
        if key.code == KeyCode::Enter && self.focus == Focus::Left && !self.move_buf.is_empty() {
            let buf = std::mem::take(&mut self.move_buf);
            if let Some(to) = target_index(&buf, self.assembled.len()) {
                let from = self.left.selected();
                let item = self.assembled.remove(from);
                self.assembled.insert(to, item);
                self.left.select(to);
            }
            return TabEvent::None;
        }
        // любая другая клавиша сбрасывает набранный номер.
        self.move_buf.clear();
        match key.code {
            KeyCode::Esc => TabEvent::Close,
            KeyCode::Char('W') => {
                let name = self.name.text();
                if name.trim().is_empty() {
                    TabEvent::Status("playlist name required".into())
                } else {
                    TabEvent::SavePlaylist { name, ids: self.ids() }
                }
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Name => Focus::Left,
                    Focus::Left => Focus::Right,
                    Focus::Right => Focus::Name,
                };
                TabEvent::None
            }
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
                    Focus::Name => self.focus = Focus::Left,
                }
                TabEvent::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                match self.focus {
                    Focus::Left => self.left.up(),
                    Focus::Right => self.right.up(),
                    Focus::Name => {}
                }
                TabEvent::None
            }
            KeyCode::Enter => {
                match self.focus {
                    Focus::Name => self.editing_name = true,
                    Focus::Right => self.add_selected(),
                    Focus::Left => self.remove_selected(),
                }
                TabEvent::None
            }
            KeyCode::Char('S') => {
                self.sort_key = self.sort_key.next();
                TabEvent::None
            }
            KeyCode::Char('R') => {
                self.sort_dir = self.sort_dir.toggled();
                TabEvent::None
            }
            KeyCode::Char('T') => {
                self.hide_added = !self.hide_added;
                TabEvent::None
            }
            _ => TabEvent::None,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        vec![
            ("Tab/hl", "foc"),
            ("jk", "nav"),
            ("Ent", "add/edit"),
            ("0-9+Ent", "to#"),
            ("SRT", "sort"),
            ("W", "save"),
            ("Esc", "cxl"),
        ]
    }

    /// весь ввод забираем только во время правки имени.
    fn wants_input(&self) -> bool {
        self.editing_name
    }
}

/// значение поля трека для сортировки пула.
fn sort_field(t: &TrackMeta, key: SortKey) -> String {
    match key {
        SortKey::Title => t.title.clone(),
        SortKey::Artist => t.artists_line(),
        SortKey::Album => t.album.clone().unwrap_or_default(),
    }
}
