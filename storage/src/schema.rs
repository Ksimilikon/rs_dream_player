//! авторазворачивание sqlite-бд: создаёт файл вместе со всей структурой, если
//! его нет, и проверяет структуру существующего файла. Реакция на неверную
//! структуру решается вызывающим кодом (см. [`crate::db::Db::init`]).

use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use rusqlite::Connection;

/// стандартное имя файла бд.
pub const DB_FILE_NAME: &str = "music_db.sqlite";

/// таблицы, наличие которых обязательно для рабочей бд.
const REQUIRED_TABLES: [&str; 5] = [
    "tracks",
    "artists",
    "track_artists",
    "playlists",
    "playlist_tracks",
];

/// вся структура бд. Каждое выражение идемпотентно (`IF NOT EXISTS`), поэтому
/// повторный прогон только дозаполняет недостающее.
const SCHEMA_SQL: &str = "\
CREATE TABLE IF NOT EXISTS tracks (
    id          INTEGER PRIMARY KEY,
    hash        TEXT    NOT NULL UNIQUE,
    title       TEXT    NOT NULL DEFAULT 'Unknown',
    duration    INTEGER NOT NULL DEFAULT 0,
    cover_art   TEXT,
    path        TEXT    NOT NULL UNIQUE,
    source_type TEXT    NOT NULL DEFAULT 'file'
);
CREATE INDEX IF NOT EXISTS idx_tracks_hash  ON tracks(hash);
CREATE INDEX IF NOT EXISTS idx_tracks_title ON tracks(title);

CREATE TABLE IF NOT EXISTS artists (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS track_artists (
    track_id  INTEGER NOT NULL REFERENCES tracks(id)  ON DELETE CASCADE,
    artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    PRIMARY KEY (track_id, artist_id)
);

CREATE TABLE IF NOT EXISTS playlists (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE,
    cover_art  TEXT,
    created_at INTEGER,
    updated_at INTEGER
);

CREATE TABLE IF NOT EXISTS playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    song_hash   TEXT    NOT NULL,
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
);
";

/// бд существует, но её структура не проходит проверку (битый файл или
/// отсутствующие таблицы).
#[derive(Debug)]
pub struct InvalidStructure(pub String);

impl fmt::Display for InvalidStructure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid database structure: {}", self.0)
    }
}

impl Error for InvalidStructure {}

/// разворачивает всю структуру на свежем (или починяемом) соединении.
pub fn deploy(conn: &Connection) -> Result<(), Box<dyn Error>> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

/// проверяет целостность файла и наличие всех обязательных таблиц.
pub fn verify_structure(conn: &Connection) -> Result<(), InvalidStructure> {
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| InvalidStructure(format!("integrity check failed: {e}")))?;
    if integrity != "ok" {
        return Err(InvalidStructure(format!("integrity check: {integrity}")));
    }

    for table in REQUIRED_TABLES {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .map_err(|e| InvalidStructure(format!("{e}")))?;
        if count != 1 {
            return Err(InvalidStructure(format!("missing table `{table}`")));
        }
    }
    Ok(())
}

/// путь резервной копии — к имени файла добавляется суффикс `_bak`
/// (`music_db.sqlite` -> `music_db.sqlite_bak`).
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push("_bak");
    path.with_file_name(name)
}
