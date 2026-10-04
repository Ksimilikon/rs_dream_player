use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};

/// события для работы с бд (по аналогии с `DBusEvent` в dbus-слое).
/// именование повторяет методы [`crate::db::Db`]: `get_*` — чтение,
/// `save_*` — upsert. `String`-поля — это ключи: хеш песни для треков и имя
/// для плейлистов.
pub enum DbEvent {
    /// сохранить/проиндексировать трек.
    SaveTrack(TrackVirtual),
    /// загрузить трек по хешу.
    GetTrack(String),
    /// сохранить плейлист.
    SavePlaylist(Playlist),
    /// загрузить плейлист по имени.
    GetPlaylist(String),
    /// проверить наличие песни с таким хешем.
    HashExist(String),
    /// выборка плейлистов по параметрам (name, id). Без фильтров — все.
    GetPlaylists {
        name: Option<String>,
        id: Option<i64>,
    },
    /// выборка песен из общего пула по параметрам (name, artist, id, hash).
    GetTracks {
        name: Option<String>,
        artist: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    },
}
