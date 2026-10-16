use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};

/// events for working with the db (similar to `DBusEvent` in the dbus layer).
/// The naming follows the [`crate::db::Db`] methods: `get_*` — read,
/// `save_*` — upsert. `String` fields are keys: the song hash for tracks and
/// the name for playlists.
pub enum DbEvent {
    /// save / index a track.
    SaveTrack(TrackVirtual),
    /// load a track by hash.
    GetTrack(String),
    /// save a playlist.
    SavePlaylist(Playlist),
    /// load a playlist by name.
    GetPlaylist(String),
    /// check whether a song with this hash exists.
    HashExist(String),
    /// select playlists by parameters (name, id). Without filters — all.
    GetPlaylists {
        name: Option<String>,
        id: Option<i64>,
    },
    /// select songs from the shared pool by parameters (name, artist, id, hash).
    GetTracks {
        name: Option<String>,
        artist: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    },
}
