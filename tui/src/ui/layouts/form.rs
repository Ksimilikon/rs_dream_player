//! a simple "label — value" form for the export / import tabs: text fields,
//! toggles and choices; a row selector and field editing.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::TextField;

const LABEL_W: usize = 12;

/// value of a form row.
pub enum FieldValue {
    Text(TextField),
    Toggle(bool),
    /// options and the index of the selected one.
    Choice(&'static [&'static str], usize),
}

pub struct FormField {
    pub label: &'static str,
    pub value: FieldValue,
    /// the row is visible but can't be changed (e.g. covers without playlists).
    pub disabled: bool,
}

impl FormField {
    pub fn text(label: &'static str, v: &str) -> Self {
        Self {
            label,
            value: FieldValue::Text(TextField::new(v)),
            disabled: false,
        }
    }
    pub fn toggle(label: &'static str, v: bool) -> Self {
        Self {
            label,
            value: FieldValue::Toggle(v),
            disabled: false,
        }
    }
    pub fn choice(label: &'static str, options: &'static [&'static str]) -> Self {
        Self {
            label,
            value: FieldValue::Choice(options, 0),
            disabled: false,
        }
    }
}

#[derive(Default)]
pub struct Form {
    pub fields: Vec<FormField>,
    pub sel: usize,
    /// the text field under the selector is being edited.
    pub editing: bool,
}

impl Form {
    pub fn new(fields: Vec<FormField>) -> Self {
        Self {
            fields,
            sel: 0,
            editing: false,
        }
    }

    pub fn text(&self, i: usize) -> String {
        match &self.fields[i].value {
            FieldValue::Text(t) => t.text(),
            _ => String::new(),
        }
    }

    pub fn toggle(&self, i: usize) -> bool {
        matches!(self.fields[i].value, FieldValue::Toggle(true)) && !self.fields[i].disabled
    }

    pub fn choice(&self, i: usize) -> usize {
        match self.fields[i].value {
            FieldValue::Choice(_, c) => c,
            _ => 0,
        }
    }

    pub fn up(&mut self) {
        self.sel = self.sel.saturating_sub(1);
    }

    pub fn down(&mut self) {
        self.sel = (self.sel + 1).min(self.fields.len().saturating_sub(1));
    }

    /// Enter on a row: text — start editing, toggle — flip it, choice — next
    /// option. `true` if the value changed.
    pub fn activate(&mut self) -> bool {
        let f = &mut self.fields[self.sel];
        if f.disabled {
            return false;
        }
        match &mut f.value {
            FieldValue::Text(_) => {
                self.editing = true;
                false
            }
            FieldValue::Toggle(v) => {
                *v = !*v;
                true
            }
            FieldValue::Choice(opts, c) => {
                *c = (*c + 1) % opts.len();
                true
            }
        }
    }

    /// input while editing a text field.
    pub fn edit_key(&mut self, key: KeyEvent) {
        let FieldValue::Text(field) = &mut self.fields[self.sel].value else {
            self.editing = false;
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => field.backspace(),
            KeyCode::Delete => field.delete(),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => field.word_left(),
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => field.word_right(),
            KeyCode::Left => field.left(),
            KeyCode::Right => field.right(),
            KeyCode::Home => field.home(),
            KeyCode::End => field.end(),
            KeyCode::Char(c) => field.insert(c),
            _ => {}
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, title: &str, focused: bool) {
        let border = match focused {
            true => Style::default().fg(Color::Cyan),
            false => Style::default().fg(Color::DarkGray),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(format!(" {title} "));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines: Vec<Line> = self
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let selected = focused && i == self.sel;
                let marker = if selected { "> " } else { "  " };
                let label_style = match (selected, f.disabled) {
                    (_, true) => Style::default().fg(Color::DarkGray),
                    (true, _) => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    (false, _) => Style::default(),
                };
                let value = match &f.value {
                    _ if f.disabled => "(n/a)".to_string(),
                    FieldValue::Text(t) => t.text(),
                    FieldValue::Toggle(v) => if *v { "[x]" } else { "[ ]" }.to_string(),
                    FieldValue::Choice(opts, c) => format!("< {} >", opts[*c]),
                };
                Line::from(vec![
                    Span::raw(marker),
                    Span::styled(format!("{:<width$}", f.label, width = LABEL_W), label_style),
                    Span::raw(value),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);

        if self.editing
            && let FieldValue::Text(t) = &self.fields[self.sel].value
        {
            // caret: marker indent (2) + label width + position in the value.
            let col = inner.x + 2 + LABEL_W as u16 + t.caret() as u16;
            frame.set_cursor_position((col, inner.y + self.sel as u16));
        }
    }
}
