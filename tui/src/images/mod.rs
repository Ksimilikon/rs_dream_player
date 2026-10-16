//! cover display right in the terminal via graphics protocols
//! (sixel/kitty/iterm2) with support detection. The module is a directory so
//! the code can be split as it grows (protocol detection, image extraction,
//! etc.).
//!
//! If the terminal doesn't support a graphics protocol,
//! [`ImageManager::supported`] returns `false`, and the SONG tab doesn't
//! reserve space for an image.
//!
//! About the protocols: sixel is static raster graphics (a single frame), with
//! no native GIF animation; a "gif" can only be shown by redrawing the area
//! frame by frame yourself. Kitty/iTerm2 can do more, but for now the goal is
//! a sixel-compatible static display.

mod protocol;
mod source;

use ratatui::{Frame, layout::Rect};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};

/// cover display manager: holds the detected terminal protocol and the
/// currently prepared image. Rebuilds the image only when the source changes
/// (so it isn't decoded every frame).
pub struct ImageManager {
    /// `Some` — the terminal supports a graphics protocol; `None` — display is off.
    picker: Option<Picker>,
    /// key of the current source (cover path) — to avoid reloading.
    current_key: Option<String>,
    /// the current cover's protocol, prepared for drawing.
    protocol: Option<StatefulProtocol>,
}

impl ImageManager {
    /// detects the terminal protocol by querying stdio. Must be called after
    /// the terminal is initialized (in raw mode).
    pub fn detect() -> Self {
        Self {
            picker: protocol::detect_graphics_picker(),
            current_key: None,
            protocol: None,
        }
    }

    /// whether the terminal can show images (a real graphics protocol exists).
    pub fn supported(&self) -> bool {
        self.picker.is_some()
    }

    /// sets the cover source of the current track: `cover` — path to a
    /// separate cover file from the index (takes priority), `audio` — path to
    /// the audio file itself (the embedded cover is taken from its tags if there
    /// is no cover file). Rebuilds the protocol only when the source changes;
    /// read / decode errors give an empty display.
    pub fn set_source(&mut self, cover: Option<&str>, audio: Option<&str>) {
        if self.picker.is_none() {
            return;
        }
        // the cache key covers both sources, so it refreshes on a track change.
        let key = format!("{}|{}", cover.unwrap_or(""), audio.unwrap_or(""));
        if self.current_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.current_key = Some(key);
        self.protocol = self.load(cover, audio);
    }

    /// whether an image is ready to show (the protocol is built).
    pub fn has_image(&self) -> bool {
        self.protocol.is_some()
    }

    /// prepares the image: first the separate cover file, then the cover
    /// embedded in the audio file tags.
    fn load(&self, cover: Option<&str>, audio: Option<&str>) -> Option<StatefulProtocol> {
        let picker = self.picker.as_ref()?;
        let img = cover
            .and_then(source::load_image)
            .or_else(|| audio.and_then(source::load_embedded))?;
        Some(picker.new_resize_protocol(img))
    }

    /// draws the current cover into `area` (if the protocol and image exist).
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if let Some(protocol) = self.protocol.as_mut() {
            frame.render_stateful_widget(
                StatefulImage::<StatefulProtocol>::default(),
                area,
                protocol,
            );
        }
    }
}
