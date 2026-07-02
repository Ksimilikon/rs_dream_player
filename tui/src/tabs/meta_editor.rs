//! редактор метаданных трека (задача 1): модальная «вкладка», открываемая
//! клавишей `m` на вкладке SONG. Правит поля заголовка, артистов, имени файла и
//! пути к обложке; сохранение — Ctrl+S (эксклюзивная клавиша редактора), выход —
//! Esc. Пустой заголовок при сохранении оставляет прежний (поле не отправляется).

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
    Save {
        id: i64,
        title: String,
        artists: String,
        filename: String,
        cover: String,
    },
}

/// поля редактора (порядок соответствует навигации сверху вниз).
const FIELD_COUNT: usize = 4;

pub struct MetaEditorState {
    /// id редактируемого трека в бд.
    id: i64,
    title: String,
    artists: String,
    filename: String,
    cover: String,
    /// активное поле (0..FIELD_COUNT).
    field: usize,
}

impl MetaEditorState {
    /// открывает редактор, заполняя поля текущими значениями трека. `filename`
    /// — только имя файла (без каталога); `cover` — текущий путь обложки.
    pub fn new(
        id: i64,
        title: String,
        artists: String,
        filename: String,
        cover: String,
    ) -> Self {
        Self {
            id,
            title,
            artists,
            filename,
            cover,
            field: 0,
        }
    }

    /// изменяемая ссылка на буфер активного поля.
    fn active_buf(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.title,
            1 => &mut self.artists,
            2 => &mut self.filename,
            _ => &mut self.cover,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> MetaEditorOutcome {
        // сохранение — эксклюзивная клавиша редактора.
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return MetaEditorOutcome::Save {
                id: self.id,
                title: self.title.trim().to_string(),
                artists: self.artists.trim().to_string(),
                filename: self.filename.trim().to_string(),
                cover: self.cover.trim().to_string(),
            };
        }
        match key.code {
            KeyCode::Esc => return MetaEditorOutcome::Cancel,
            KeyCode::Up => self.field = self.field.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                self.field = (self.field + 1).min(FIELD_COUNT - 1);
            }
            KeyCode::Backspace => {
                self.active_buf().pop();
            }
            KeyCode::Char(c) => self.active_buf().push(c),
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

        let labels = ["TITLE", "ARTISTS (comma separated)", "FILE NAME", "COVER PATH"];
        let values = [&self.title, &self.artists, &self.filename, &self.cover];
        let items: Vec<ListItem> = (0..FIELD_COUNT)
            .map(|i| {
                let cursor = if i == self.field { "_" } else { "" };
                let line = format!("{:<26} {}{cursor}", format!("{}:", labels[i]), values[i]);
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
                "Up/Down (Tab) move field | type to edit | Ctrl+S save | Esc cancel",
            )
            .style(Style::new().fg(Color::DarkGray)),
            hints,
        );
    }
}
