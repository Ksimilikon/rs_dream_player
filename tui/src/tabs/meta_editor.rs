//! редактор метаданных трека (задача 1): модальная «вкладка», открываемая
//! клавишей `m` на вкладке SONG. Правит теги файла (заголовок, артисты, альбом,
//! жанры), имя файла, обложку, а также прилагаемые к бд метки (цвет, текст).
//! Сохранение — Ctrl+S (эксклюзивная клавиша), выход — Esc. Пустое поле при
//! сохранении оставляет прежнее значение (не отправляется).

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

/// результат обработки клавиши редактором метаданных.
pub enum MetaEditorOutcome {
    /// остаёмся в редакторе.
    None,
    /// выйти без изменений.
    Cancel,
    /// сохранить: применяются только непустые изменённые поля.
    Save(MetaEdit),
}

/// собранные значения полей редактора.
pub struct MetaEdit {
    pub id: i64,
    pub title: String,
    pub artists: String,
    pub album: String,
    pub genres: String,
    pub filename: String,
    pub cover: String,
    pub color: String,
    pub label: String,
}

/// поля редактора (порядок соответствует навигации сверху вниз).
const FIELD_COUNT: usize = 8;
const LABELS: [&str; FIELD_COUNT] = [
    "TITLE",
    "ARTISTS (comma separated)",
    "ALBUM",
    "GENRES (comma separated)",
    "FILE NAME",
    "COVER PATH",
    "COLOR (red/pink/orange/green/blue/cyan/purple)",
    "TEXT LABEL",
];

pub struct MetaEditorState {
    id: i64,
    fields: [String; FIELD_COUNT],
    /// активное поле (0..FIELD_COUNT).
    field: usize,
}

impl MetaEditorState {
    /// открывает редактор, заполняя поля текущими значениями трека. Имя файла в
    /// TUI не хранится, поэтому передаётся пустым.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: i64,
        title: String,
        artists: String,
        album: String,
        genres: String,
        cover: String,
        color: String,
        label: String,
    ) -> Self {
        Self {
            id,
            fields: [
                title,
                artists,
                album,
                genres,
                String::new(),
                cover,
                color,
                label,
            ],
            field: 0,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> MetaEditorOutcome {
        // сохранение — эксклюзивная клавиша редактора.
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            let f = &self.fields;
            return MetaEditorOutcome::Save(MetaEdit {
                id: self.id,
                title: f[0].trim().to_string(),
                artists: f[1].trim().to_string(),
                album: f[2].trim().to_string(),
                genres: f[3].trim().to_string(),
                filename: f[4].trim().to_string(),
                cover: f[5].trim().to_string(),
                color: f[6].trim().to_string(),
                label: f[7].trim().to_string(),
            });
        }
        match key.code {
            KeyCode::Esc => return MetaEditorOutcome::Cancel,
            KeyCode::Up => self.field = self.field.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                self.field = (self.field + 1).min(FIELD_COUNT - 1);
            }
            KeyCode::Backspace => {
                self.fields[self.field].pop();
            }
            KeyCode::Char(c) => self.fields[self.field].push(c),
            _ => {}
        }
        MetaEditorOutcome::None
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let [title_area, fields_area, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);

        frame.render_widget(
            Paragraph::new("EDIT METADATA").style(Style::new().fg(Color::Cyan)),
            title_area,
        );

        let items: Vec<ListItem> = (0..FIELD_COUNT)
            .map(|i| {
                let cursor = if i == self.field { "_" } else { "" };
                let line = format!(
                    "{:<46} {}{cursor}",
                    format!("{}:", LABELS[i]),
                    self.fields[i]
                );
                let item = ListItem::new(line);
                if i == self.field {
                    item.style(Style::new().add_modifier(Modifier::REVERSED))
                } else {
                    item
                }
            })
            .collect();
        frame.render_widget(
            List::new(items).block(Block::new().borders(Borders::ALL).title("FIELDS")),
            fields_area,
        );

        frame.render_widget(
            Paragraph::new(
                "Up/Down (Tab) move field | type to edit | empty = keep | Ctrl+S save | Esc cancel",
            )
            .style(Style::new().fg(Color::DarkGray)),
            hints,
        );
    }
}
