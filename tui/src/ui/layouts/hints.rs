//! строка подсказок по командам (клавиша — зелёным, описание — обычным).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::ui::PLAYING_GREEN;

/// подсказка: (клавиша, действие).
pub type Hint = (&'static str, &'static str);

/// собирает строку подсказок только из клавиш (без описаний — максимально
/// кратко, чтобы не уходило за экран). Полные пояснения — в `:help`.
pub fn hints_line(hints: &[Hint]) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    for (i, (key, _desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            (*key).to_string(),
            Style::default()
                .fg(PLAYING_GREEN)
                .add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans)
}
