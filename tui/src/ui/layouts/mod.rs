//! переиспользуемые заготовки интерфейса: унифицированный список, поле ввода с
//! кареткой, строка подсказок, прогресс-бар и общие схемы расположения вкладок.

mod hints;
mod list;
mod progress;
mod text;

pub use hints::{Hint, hints_line};
pub use list::{ListLine, ListView, SortDir, SortKey};
pub use progress::progress_bar;
pub use text::TextField;

use ratatui::layout::{Constraint, Layout, Rect};

/// общая схема «две панели»: левый список и правая область (детали/пул).
/// `left_pct` — ширина левой панели в процентах.
pub fn two_pane(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(left_pct), Constraint::Percentage(100 - left_pct)])
            .areas(area);
    (left, right)
}
