//! reusable interface building blocks: a unified list, an input field with a
//! caret, track list search, a hints line, a progress bar, a "label — value"
//! form, a message log and common tab layouts.

mod form;
mod hints;
mod list;
mod log;
mod progress;
mod search;
mod text;

pub use form::{Form, FormField};
pub use hints::{Hint, hints_line};
pub use log::MessageLog;
pub use list::{ListLine, ListView, SortDir, SortKey};
pub use progress::progress_bar;
pub use search::{Search, SearchInput};
pub use text::TextField;

use ratatui::layout::{Constraint, Layout, Rect};

/// typed position number (1-based) -> target index (0-based), clamped to
/// `[0, len-1]`. Empty / not a number / `len==0` -> `None`. A value outside
/// the list takes the nearest end (1 -> start, >=len -> end).
pub fn target_index(buf: &str, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let n: usize = buf.trim().parse().ok()?;
    Some(n.clamp(1, len) - 1)
}

/// the common "two panes" layout: a list on the left and an area on the right
/// (details / pool). `left_pct` — the left pane width in percent.
pub fn two_pane(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(left_pct), Constraint::Percentage(100 - left_pct)])
            .areas(area);
    (left, right)
}
