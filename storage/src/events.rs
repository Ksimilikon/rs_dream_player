use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};

/// события для работы с бд (по аналогии с `DBusEvent` в dbus-слое).
/// `String`-поля — это ключи: хеш песни для треков и имя для плейлистов.
pub enum DbEvent {
    /// сохранить/проиндексировать трек.
    SaveTrack(TrackVirtual),
    /// загрузить трек по хешу.
    LoadTrack(String),
    /// сохранить плейлист.
    SavePlaylist(Playlist),
    /// загрузить плейлист по имени.
    LoadPlaylist(String),
    /// проверить наличие песни с таким хешем.
    HashExist(String),
    /// получить все плейлисты.
    ListPlaylists,
    /// выборка плейлистов по параметрам (name, id).
    FindPlaylist {
        name: Option<String>,
        id: Option<i64>,
    },
    /// выборка песен из общего пула по параметрам (name, artist, id, hash).
    FindTrack {
        name: Option<String>,
        artist: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    },
}
