//! переиспользуемые заготовки интерфейса: унифицированный список, поле ввода с
//! кареткой, поиск по списку треков, строка подсказок, прогресс-бар, форма
//! «метка — значение», журнал сообщений и общие схемы расположения вкладок.

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

/// набранный номер позиции (1-based) -> целевой индекс (0-based), зажатый в
/// `[0, len-1]`. Пусто/нечисло/`len==0` -> `None`. Значение вне списка берёт
/// крайнее (1 -> начало, >=len -> конец).
pub fn target_index(buf: &str, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let n: usize = buf.trim().parse().ok()?;
    Some(n.clamp(1, len) - 1)
}

/// общая схема «две панели»: левый список и правая область (детали/пул).
/// `left_pct` — ширина левой панели в процентах.
pub fn two_pane(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(left_pct), Constraint::Percentage(100 - left_pct)])
            .areas(area);
    (left, right)
}
