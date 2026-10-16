//! command hints line (the key in green, the description plain).

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::ui::PLAYING_GREEN;

/// a hint: (key, action).
pub type Hint = (&'static str, &'static str);

/// builds the hints line from keys only (no descriptions — as short as
/// possible, so it doesn't overflow the screen). Full explanations are in
/// `:help`.
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
