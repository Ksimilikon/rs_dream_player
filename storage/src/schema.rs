//! auto-deployment of the sqlite db: creates the file with the whole
//! structure if it is missing, and verifies the structure of an existing file.
//! The reaction to an invalid structure is up to the caller (see
//! [`crate::db::Db::init`]).

use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use rusqlite::Connection;

/// the standard db file name.
pub const DB_FILE_NAME: &str = "music_db.sqlite";

/// the required structure of a working db: for each table — its required
/// columns. If a table or column is missing, the db is considered
/// incompatible.
const REQUIRED_SCHEMA: &[(&str, &[&str])] = &[
    (
        "tracks",
        &[
            "id",
            "hash",
            "title",
            "duration",
            "cover_art",
            "path",
            "source_type",
            "volume",
            "album_id",
            "color",
            "user_label",
            "invalid",
            // added by migration 2.
            "ignore_orphan",
        ],
    ),
    ("artists", &["id", "name"]),
    ("track_artists", &["track_id", "artist_id"]),
    ("albums", &["id", "name"]),
    ("genres", &["id", "name"]),
    ("track_genres", &["track_id", "genre_id"]),
    (
        "playlists",
        // `deleted_at` — added by migration 2.
        &["id", "name", "cover_art", "created_at", "updated_at", "deleted_at"],
    ),
    ("playlist_tracks", &["playlist_id", "song_hash", "position"]),
    // added by migration 1.
    ("paired_devices", &["id", "pub_key", "name"]),
];

/// the whole db structure. Every statement is idempotent (`IF NOT EXISTS`),
/// so a repeated run only fills in what is missing.
const SCHEMA_SQL: &str = "\
CREATE TABLE IF NOT EXISTS albums (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS tracks (
    id          INTEGER PRIMARY KEY,
    hash        TEXT    NOT NULL UNIQUE,
    title       TEXT    NOT NULL DEFAULT 'Unknown',
    duration    INTEGER NOT NULL DEFAULT 0,
    cover_art   TEXT,
    path        TEXT    NOT NULL UNIQUE,
    source_type TEXT    NOT NULL DEFAULT 'file',
    volume      REAL    NOT NULL DEFAULT 1.0,
    album_id    INTEGER REFERENCES albums(id) ON DELETE SET NULL,
    color       TEXT,
    user_label  TEXT,
    invalid     INTEGER NOT NULL DEFAULT 0
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

CREATE TABLE IF NOT EXISTS genres (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS track_genres (
    track_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    genre_id INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
    PRIMARY KEY (track_id, genre_id)
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

/// schema migrations on top of [`SCHEMA_SQL`]. Migration `i` lifts the db
/// from `PRAGMA user_version = i` to `i + 1`; the db created by older versions
/// of the app has `user_version = 0`. Only append to the end: an applied
/// migration is never re-run, so editing it doesn't reach existing dbs.
const MIGRATIONS: &[&str] = &[
    // 1: public keys of paired devices (vpair). `id` is a counter that is never
    // reused after a removal; `name` is set by the user for viewing only.
    "\
CREATE TABLE IF NOT EXISTS paired_devices (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    pub_key BLOB NOT NULL UNIQUE,
    name    TEXT NOT NULL DEFAULT 'Unknown'
);
",
    // 2: soft deletion of playlists — a deleted playlist keeps its row (the
    // name is its identifier across devices) with the deletion time, so the
    // deletion syncs; and a per-track mark "kept on purpose though in no
    // playlist" for the orphan collector.
    "\
ALTER TABLE playlists ADD COLUMN deleted_at INTEGER;
ALTER TABLE tracks ADD COLUMN ignore_orphan INTEGER NOT NULL DEFAULT 0;
",
];

/// the db exists but its structure fails verification (a broken file or
/// missing tables).
#[derive(Debug)]
pub struct InvalidStructure(pub String);

impl fmt::Display for InvalidStructure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid database structure: {}", self.0)
    }
}

impl Error for InvalidStructure {}

/// deploys the whole structure on a fresh (or repaired) connection.
pub fn deploy(conn: &Connection) -> Result<(), Box<dyn Error>> {
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.execute_batch(SCHEMA_SQL)?;
    migrate(conn)
}

/// applies the not yet applied [`MIGRATIONS`], each in its own transaction
/// together with the `user_version` bump. A db from a newer app version
/// (`user_version` past the list) is left as is.
pub fn migrate(conn: &Connection) -> Result<(), Box<dyn Error>> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

/// checks the file integrity and the presence of every required table.
pub fn verify_structure(conn: &Connection) -> Result<(), InvalidStructure> {
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|e| InvalidStructure(format!("integrity check failed: {e}")))?;
    if integrity != "ok" {
        return Err(InvalidStructure(format!("integrity check: {integrity}")));
    }

    for (table, columns) in REQUIRED_SCHEMA {
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

        // actual table columns (the name is the second PRAGMA table_info column).
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| InvalidStructure(format!("{e}")))?;
        let existing = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<String>>>())
            .map_err(|e| InvalidStructure(format!("{e}")))?;

        for col in *columns {
            if !existing.iter().any(|c| c == col) {
                return Err(InvalidStructure(format!(
                    "table `{table}` is missing column `{col}`"
                )));
            }
        }
    }
    Ok(())
}

/// backup path — the `_bak` suffix is appended to the file name
/// (`music_db.sqlite` -> `music_db.sqlite_bak`).
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push("_bak");
    path.with_file_name(name)
}
