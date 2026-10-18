//! concrete tabs with interface and logic. Permanent: PLAYLISTS, SONG.
//! Transient (by command): NEW_PLAYLIST, MOD_<name>, MOD_SONG_<...>, HELP,
//! SETTINGS, EXPORT, IMPORT, SYNC, ORPHANS.

mod editor;
mod export;
mod help;
mod import;
mod meta;
mod orphans;
mod playlists;
mod settings;
mod song;
mod sync;

pub use editor::PlaylistEditor;
pub use export::ExportTab;
pub use help::HelpTab;
pub use import::ImportTab;
pub(crate) use import::write_unique;
pub use meta::SongEditor;
pub use orphans::OrphansTab;
pub use playlists::PlaylistsTab;
pub use settings::SettingsTab;
pub use song::SongTab;
pub use sync::SyncTab;
