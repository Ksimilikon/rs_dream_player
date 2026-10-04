//! вкладка SETTINGS (`:settings`): настройки приложения. Пока единственный пункт
//! — устройство вывода звука (список, текущее зелёным, Enter переключает).
//! Открывается командой (как HELP), закрывается Shift+X. Задел на будущее:
//! сюда же добавятся громкость/прочие настройки отдельными секциями.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::Paragraph,
};

use crate::ffi;
use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Hint, ListLine, ListView},
};

#[derive(Default)]
pub struct SettingsTab {
    list: ListView,
    devices: Vec<String>,
    loaded: bool,
}

impl SettingsTab {
    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.devices = ffi::engine::output_devices();
            self.loaded = true;
        }
    }
}

impl Tab for SettingsTab {
    fn title(&self) -> String {
        "SETTINGS".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        self.ensure_loaded();
        let current = ffi::engine::current_output_device();

        let [header, body] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(area);

        let cur = current.as_deref().unwrap_or("-");
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Line::styled(
                    "OUTPUT DEVICE",
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )),
                Line::from(format!("current: {cur}   (Enter: switch · r: refresh)")),
            ]),
            header,
        );

        let lines: Vec<ListLine> = self
            .devices
            .iter()
            .map(|d| ListLine::new(d.clone()).playing(Some(d.as_str()) == current.as_deref()))
            .collect();
        self.list.render(frame, body, "devices", lines, true);
    }

    fn on_key(&mut self, key: KeyEvent, _model: &Model) -> TabEvent {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.list.down();
                TabEvent::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.list.up();
                TabEvent::None
            }
            KeyCode::Char('r') => {
                // перечитать список устройств (plug/unplug).
                self.devices = ffi::engine::output_devices();
                TabEvent::None
            }
            KeyCode::Enter => match self.devices.get(self.list.selected()) {
                Some(name) => TabEvent::SetOutputDevice(name.clone()),
                None => TabEvent::None,
            },
            _ => TabEvent::None,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        vec![("jk", "nav"), ("Ent", "set"), ("r", "refresh")]
    }
}
