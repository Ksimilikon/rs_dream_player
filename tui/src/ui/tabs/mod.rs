//! конкретные вкладки с интерфейсом и логикой. Постоянные: PLAYLISTS, SONG.
//! Транзиентные (по команде): NEW_PLAYLIST, MOD_<name>, MOD_SONG_<...>, HELP,
//! SETTINGS, EXPORT, IMPORT.

mod editor;
mod export;
mod help;
mod import;
mod meta;
mod playlists;
mod settings;
mod song;

pub use editor::PlaylistEditor;
pub use export::ExportTab;
pub use help::HelpTab;
pub use import::ImportTab;
pub use meta::SongEditor;
pub use playlists::PlaylistsTab;
pub use settings::SettingsTab;
pub use song::SongTab;
