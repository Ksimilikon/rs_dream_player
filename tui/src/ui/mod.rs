//! interface layer: [`layouts`] — reusable building blocks, [`tabs`] —
//! concrete tabs with logic. Also the shared view model the tabs read, and the
//! [`Tab`] trait.

pub mod layouts;
pub mod tabs;

use ratatui::{Frame, crossterm::event::KeyEvent, layout::Rect, style::Color};

use crate::ffi::TrackMeta;
use layouts::Hint;

/// color of the playing track / accents.
pub const PLAYING_GREEN: Color = Color::Green;
/// color of the help header (shift+x closes).
pub const HELP_RED: Color = Color::Red;

/// a playlist as the interface sees it (name + tracks + flags).
pub struct PlaylistView {
    pub name: String,
    pub tracks: Vec<TrackMeta>,
    /// the shared "ALL SONGS" pool (first in the list, can't be deleted).
    pub pool: bool,
    /// playlist id in the db (for operations); `None` for the pool.
    pub id: Option<i64>,
}

/// state of the playing playlist.
pub struct Playing {
    pub name: String,
    pub tracks: Vec<TrackMeta>,
    /// index of the playing track in `tracks`.
    pub current: usize,
}

/// view model available to tabs (read-only while drawing / handling keys).
pub struct Model {
    /// library: `[0]` — the "ALL SONGS" pool, then playlists from the db.
    pub playlists: Vec<PlaylistView>,
    /// the active playing playlist (if any).
    pub playing: Option<Playing>,
    pub master_vol: f32,
    /// playback position in seconds (for the progress bar).
    pub pos: u64,
}

impl Model {
    /// the "ALL SONGS" pool.
    pub fn pool(&self) -> &[TrackMeta] {
        self.playlists
            .iter()
            .find(|p| p.pool)
            .map(|p| p.tracks.as_slice())
            .unwrap_or(&[])
    }
}

/// an action a tab asks the application to perform.
pub enum TabEvent {
    None,
    /// close the current (transient) tab (editor cancel).
    Close,
    /// play library playlist `index` starting from track `start`.
    PlayPlaylist { index: usize, start: usize },
    /// select the playing track by index (SONG tab).
    SelectPlaying(usize),
    /// open the playlist creation tab.
    NewPlaylist,
    /// open the editing tab for library playlist `index`.
    EditPlaylist(usize),
    /// open editing of the playing playlist (SONG tab).
    EditPlayingPlaylist,
    /// delete library playlist `index`.
    DeletePlaylist(usize),
    /// open the metadata editor for track `index` in the playing list.
    EditMeta(usize),
    /// move a track within the playing list (temporarily, not written to the db).
    MovePlaying { from: usize, to: usize },
    /// remove track `index` from the playing list; `from_db` — also from the
    /// playlist in the db.
    RemovePlaying { index: usize, from_db: bool },
    /// save a playlist: name + track ids (new or edited).
    SavePlaylist { name: String, ids: Vec<i64> },
    /// apply a track metadata edit.
    SaveMeta(MetaEdit),
    /// switch the output device by name.
    SetOutputDevice(String),
    /// the library changed from outside (import) — re-read it.
    LibraryChanged,
    /// a one-off message for the status line.
    Status(String),
}

/// a set of track metadata edits (an empty `None` field — don't change).
#[derive(Default)]
pub struct MetaEdit {
    pub id: i64,
    pub title: Option<String>,
    pub artists: Option<Vec<String>>,
    pub album: Option<String>,
    pub genres: Option<Vec<String>>,
    pub cover: Option<String>,
    pub color: Option<String>,
    pub label: Option<String>,
}

/// an interface tab.
pub trait Tab {
    /// title for the tab bar.
    fn title(&self) -> String;
    /// draw into the given area.
    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model);
    /// handle a key (except global ones) -> the requested action.
    fn on_key(&mut self, key: KeyEvent, model: &Model) -> TabEvent;
    /// command hints of the tab (for the bottom line).
    fn hints(&self) -> Vec<Hint>;
    /// whether the tab can be closed with Shift+X. All can, except PLAYLISTS.
    fn closable(&self) -> bool {
        true
    }
    /// the tab is typing text into a field right now: meanwhile the app gives
    /// it all input (so letters don't fire as global commands). Outside active
    /// input the tab behaves like any other (H/L, global commands).
    fn wants_input(&self) -> bool {
        false
    }
    /// the tab's input line for the bottom panel (search `/`): text and the
    /// cursor position in characters. `None` — the tab isn't typing anything.
    fn prompt(&self) -> Option<(String, usize)> {
        None
    }
    /// the track list changed — reset cursors.
    fn on_tracks_changed(&mut self) {}
    /// a tab-specific command (`:<tab> <args>`, e.g. `:sync listen`); the
    /// error goes to the status line.
    fn command(&mut self, _args: &str) -> Result<(), String> {
        Ok(())
    }
    /// called every loop tick: background work of the tab (a running sync)
    /// may ask the application for something.
    fn poll(&mut self) -> TabEvent {
        TabEvent::None
    }
}
