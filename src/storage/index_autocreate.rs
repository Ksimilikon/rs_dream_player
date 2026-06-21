//! auto-deployment of the sqlite index: creates the database file together with
//! its full structure when it is missing, repairs / re-creates it when the file
//! is corrupted, and migrates the schema forward when it falls behind.

use std::{error::Error, fs, path::Path};

use rusqlite::Connection;

/// current schema version. Bump this and add a branch in [`migrate`] whenever
/// the structure below changes, so existing databases get upgraded in place.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// the whole structure of the index. Every statement is idempotent
/// (`IF NOT EXISTS`) so running it again only fills in what is missing — this is
/// what lets us heal a database whose structure was partially dropped.
const SCHEMA_SQL: &str = "\
CREATE TABLE IF NOT EXISTS tracks (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL UNIQUE,
    hash         TEXT,
    title        TEXT    NOT NULL DEFAULT 'Unknown',
    duration_sec INTEGER NOT NULL DEFAULT 0,
    sample_rate  INTEGER NOT NULL DEFAULT 0,
    bitrate      INTEGER NOT NULL DEFAULT 0,
    cover_art    TEXT,
    volume       REAL    NOT NULL DEFAULT 1.0,
    created_at   INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at   INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_tracks_title ON tracks(title);
CREATE INDEX IF NOT EXISTS idx_tracks_hash  ON tracks(hash);

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
    hash       TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX IF NOT EXISTS idx_playlists_hash ON playlists(hash);

CREATE TABLE IF NOT EXISTS playlist_tracks (
    playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id    INTEGER NOT NULL REFERENCES tracks(id)    ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, position)
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT
);
";

/// opens the database at `path`, creating the file and its parent directory if
/// needed. A file that fails the integrity check is moved aside (to
/// `<name>.corrupt`) and re-created from scratch, so a broken index never blocks
/// startup. On success the connection is fully migrated and ready to use.
pub(crate) fn open_or_recreate(path: &Path) -> Result<Connection, Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }

    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", true)?;

    if integrity_ok(&conn) {
        deploy_db(&conn)?;
        return Ok(conn);
    }

    // the file exists but is not a healthy sqlite database: preserve it for
    // forensics and start over with a fresh one.
    drop(conn);
    let backup = path.with_extension("corrupt");
    let _ = fs::remove_file(&backup);
    fs::rename(path, &backup)?;

    let conn = Connection::open(path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    deploy_db(&conn)?;
    Ok(conn)
}

/// runs `PRAGMA integrity_check`; a non-`ok` result (or any error reading it)
/// means the file is unusable as a database.
fn integrity_ok(conn: &Connection) -> bool {
    match conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0)) {
        Ok(result) => result == "ok",
        Err(_) => false,
    }
}

/// migrates `conn` up to [`SCHEMA_VERSION`] and (re)applies the idempotent
/// schema so any missing structure is filled back in.
fn deploy_db(conn: &Connection) -> Result<(), Box<dyn Error>> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    if version < SCHEMA_VERSION {
        migrate(conn, version)?;
    }

    // defensive re-apply: heals a database whose version is current but whose
    // structure was partially damaged/dropped.
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

/// applies migration steps from `from` up to [`SCHEMA_VERSION`], then stamps the
/// new version. Add a branch per version bump.
fn migrate(conn: &Connection, from: i64) -> Result<(), Box<dyn Error>> {
    let mut version = from;
    while version < SCHEMA_VERSION {
        match version {
            // 0 -> 1: initial structure.
            0 => conn.execute_batch(SCHEMA_SQL)?,
            // future migrations go here, e.g.
            // 1 => conn.execute_batch("ALTER TABLE tracks ADD COLUMN ...")?,
            _ => {}
        }
        version += 1;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}
