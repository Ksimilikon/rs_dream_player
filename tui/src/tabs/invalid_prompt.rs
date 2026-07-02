//! модальное меню для недействительного трека (задача 3): задать новый путь к
//! файлу или удалить трек из индекса. Ввод текста = новый путь; Enter применяет
//! его, Ctrl+D удаляет трек, Esc закрывает без изменений.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::centered_rect;

/// результат обработки клавиши меню.
pub enum InvalidOutcome {
    /// остаёмся в меню.
    None,
    /// закрыть без изменений.
    Cancel,
    /// присвоить треку новый путь.
    SetPath { id: i64, path: String },
    /// удалить трек из индекса.
    Remove { id: i64 },
}

pub struct InvalidPromptState {
    id: i64,
    /// заголовок трека (для показа).
    title: String,
    /// буфер ввода нового пути.
    path: String,
}

impl InvalidPromptState {
    pub fn new(id: i64, title: String) -> Self {
        Self {
            id,
            title,
            path: String::new(),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InvalidOutcome {
        // удаление — Ctrl+D.
        if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return InvalidOutcome::Remove { id: self.id };
        }
        match key.code {
            KeyCode::Esc => InvalidOutcome::Cancel,
            KeyCode::Enter => {
                let path = self.path.trim().to_string();
                if path.is_empty() {
                    InvalidOutcome::None
                } else {
                    InvalidOutcome::SetPath { id: self.id, path }
                }
            }
            KeyCode::Backspace => {
                self.path.pop();
                InvalidOutcome::None
            }
            KeyCode::Char(c) => {
                self.path.push(c);
                InvalidOutcome::None
            }
            _ => InvalidOutcome::None,
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(area, 70, 7);
        frame.render_widget(Clear, popup);
        let body = format!(
            "invalid track: {}\n\nnew path: {}_\n\nEnter: set path | Ctrl+D: delete from index | Esc: cancel",
            self.title, self.path
        );
        frame.render_widget(
            Paragraph::new(body)
                .style(Style::new().fg(Color::Red))
                .block(
                    Block::new()
                        .borders(Borders::ALL)
                        .title(" INVALID TRACK "),
                ),
            popup,
        );
    }
}
