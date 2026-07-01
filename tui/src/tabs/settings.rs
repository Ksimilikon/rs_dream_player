use ratatui::{
    Frame,
    crossterm::event::KeyEvent,
    layout::Rect,
    widgets::{Block, Paragraph},
};

use super::{Action, Tab};
use crate::model::Model;

/// заготовка вкладки настроек. Пока скрыта (не входит в навигацию).
pub struct SettingsTab;

impl Tab for SettingsTab {
    fn title(&self) -> &str {
        "SETTINGS"
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        frame.render_widget(
            Paragraph::new("settings — coming soon").block(Block::new().title("SETTINGS")),
            area,
        );
    }

    fn on_key(&mut self, _key: KeyEvent, _model: &Model) -> Option<Action> {
        None
    }
}
