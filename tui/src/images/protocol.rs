//! detection of the terminal graphics protocol.
//!
//! ratatui-image can query the terminal (via escape sequences on stdio) and
//! detect the available protocol: Sixel / Kitty / iTerm2, or fall back to
//! "halfblocks" (unicode half blocks — works everywhere, but it isn't real
//! graphics). As required, the halfblocks fallback counts as "not supported":
//! in that case the image area on the SONG tab collapses.

use ratatui_image::picker::{Picker, ProtocolType};

/// tries to detect a REAL graphics protocol (sixel/kitty/iterm2). Returns
/// `Some(Picker)` only if the protocol is real. The halfblocks fallback and any
/// query error (e.g. stdout is not a terminal) → `None` — the terminal is
/// considered unable to show images.
pub fn detect_graphics_picker() -> Option<Picker> {
    let picker = Picker::from_query_stdio().ok()?;
    match picker.protocol_type() {
        ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2 => Some(picker),
        ProtocolType::Halfblocks => None,
    }
}
