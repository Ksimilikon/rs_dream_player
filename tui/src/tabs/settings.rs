use ratatui::{
    Frame,
    crossterm::event::KeyEvent,
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
};

use super::{Action, Tab};
use crate::model::Model;

/// вкладка настроек: пока только показывает текущий конфиг приложения.
pub struct SettingsTab;

impl Tab for SettingsTab {
    fn title(&self) -> &str {
        "SETTINGS"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model) {
        let body = if model.config_text.trim().is_empty() {
            "config: <empty>".to_string()
        } else {
            model.config_text.clone()
        };
        frame.render_widget(
            Paragraph::new(body).block(Block::new().borders(Borders::ALL).title("CONFIG")),
            area,
        );
    }

    fn on_key(&mut self, _key: KeyEvent, _model: &Model) -> Option<Action> {
        None
    }
}
