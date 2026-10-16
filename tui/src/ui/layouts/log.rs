//! журнал сообщений вкладки (экспорт/импорт): строки с уровнем, новые снизу,
//! при переполнении видны последние.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

#[derive(Clone, Copy)]
enum Level {
    Info,
    Ok,
    Warn,
    Error,
}

#[derive(Default)]
pub struct MessageLog {
    lines: Vec<(Level, String)>,
}

impl MessageLog {
    pub fn info(&mut self, msg: impl Into<String>) {
        self.lines.push((Level::Info, msg.into()));
    }
    pub fn ok(&mut self, msg: impl Into<String>) {
        self.lines.push((Level::Ok, msg.into()));
    }
    pub fn warn(&mut self, msg: impl Into<String>) {
        self.lines.push((Level::Warn, msg.into()));
    }
    pub fn error(&mut self, msg: impl Into<String>) {
        self.lines.push((Level::Error, msg.into()));
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default().borders(Borders::ALL).title(" messages ");
        let visible = block.inner(area).height as usize;
        let skip = self.lines.len().saturating_sub(visible);
        let lines: Vec<Line> = self.lines[skip..]
            .iter()
            .map(|(level, msg)| {
                let style = match level {
                    Level::Info => Style::default(),
                    Level::Ok => Style::default().fg(crate::ui::PLAYING_GREEN),
                    Level::Warn => Style::default().fg(Color::Yellow),
                    Level::Error => Style::default().fg(Color::Red),
                };
                Line::styled(msg.clone(), style)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}
