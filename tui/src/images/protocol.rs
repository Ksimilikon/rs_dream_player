//! детекция graphics-протокола терминала.
//!
//! ratatui-image умеет запросить терминал (через управляющие последовательности
//! к stdio) и определить доступный протокол: Sixel / Kitty / iTerm2, либо
//! откатиться на «halfblocks» (юникодные полублоки — работает везде, но это не
//! настоящая графика). По требованию halfblocks-фолбэк считаем «не поддерживается»:
//! в этом случае место под изображение на вкладке SONG схлопывается.

use ratatui_image::picker::{Picker, ProtocolType};

/// пытается детектировать НАСТОЯЩИЙ graphics-протокол (sixel/kitty/iterm2).
/// Возвращает `Some(Picker)` только если протокол реальный. Halfblocks-фолбэк и
/// любые ошибки запроса (например, stdout не является терминалом) → `None` —
/// терминал считается не поддерживающим показ изображений.
pub fn detect_graphics_picker() -> Option<Picker> {
    let picker = Picker::from_query_stdio().ok()?;
    match picker.protocol_type() {
        ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2 => Some(picker),
        ProtocolType::Halfblocks => None,
    }
}
