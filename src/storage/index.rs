use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

use audio_structs::{
    playlist::Playlist,
    track_metadata::{TrackMetadata, TrackMetadataParams},
    track_virtual::TrackVirtual,
};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Value};

use crate::storage::{config::Config, index_autocreate};

/// the sqlite-backed music index. Owns a single [`Connection`]; all methods
/// take `&self` and open their own short transactions where needed.
pub struct Index {
    conn: Connection,
    path: PathBuf,
}

impl Index {
    /// opens (or creates / repairs) the index at `path`. See
    /// [`index_autocreate::open_or_recreate`] for the deployment logic.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        let path = path.as_ref().to_path_buf();
        let conn = index_autocreate::open_or_recreate(&path)?;
        Ok(Self { conn, path })
    }

    /// path of the underlying database file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// inserts or updates a track row (keyed by file path) plus its artists,
    /// returning the row id. A track without a usable path (e.g. an external
    /// source) can't be indexed.
    pub fn index_track(&self, track: &TrackVirtual) -> Result<i64, Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        let id = upsert_track(&tx, track)?;
        tx.commit()?;
        Ok(id)
    }

    /// persists a named playlist: indexes every track it contains, then rewrites
    /// the ordered `playlist_tracks` mapping. Anonymous playlists can't be saved.
    pub fn save_playlist(&self, playlist: &Playlist) -> Result<i64, Box<dyn Error>> {
        let name = playlist
            .get_name()
            .ok_or("cannot save an anonymous playlist (it has no name)")?;

        let tx = self.conn.unchecked_transaction()?;
        let hash = playlist_hash(playlist);

        tx.execute(
            "INSERT INTO playlists (name, hash) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET hash = excluded.hash, updated_at = unixepoch()",
            params![name, hash],
        )?;
        let playlist_id: i64 =
            tx.query_row("SELECT id FROM playlists WHERE name = ?1", [&name], |r| {
                r.get(0)
            })?;

        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
        )?;
        for (pos, track) in playlist.tracks().iter().enumerate() {
            let track_id = upsert_track(&tx, track)?;
            tx.execute(
                "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                params![playlist_id, track_id, pos as i64],
            )?;
        }

        tx.commit()?;
        Ok(playlist_id)
    }

    /// looks up tracks matching any combination of the given filters. `name` and
    /// `artist` are matched case-insensitively as substrings; `id` and `hash`
    /// are exact. With no filters it returns the whole library.
    pub fn find_track(
        &self,
        name: Option<String>,
        artist: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    ) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        let mut sql = String::from(
            "SELECT DISTINCT t.id, t.path, t.title, t.duration_sec, t.sample_rate, \
             t.bitrate, t.cover_art, t.volume FROM tracks t",
        );
        if artist.is_some() {
            sql.push_str(
                " JOIN track_artists ta ON ta.track_id = t.id \
                 JOIN artists a ON a.id = ta.artist_id",
            );
        }

        let mut conds: Vec<&str> = Vec::new();
        let mut vals: Vec<Value> = Vec::new();
        if let Some(n) = &name {
            conds.push("t.title LIKE ?");
            vals.push(Value::Text(format!("%{n}%")));
        }
        if let Some(a) = &artist {
            conds.push("a.name LIKE ?");
            vals.push(Value::Text(format!("%{a}%")));
        }
        if let Some(i) = id {
            conds.push("t.id = ?");
            vals.push(Value::Integer(i));
        }
        if let Some(h) = &hash {
            conds.push("t.hash = ?");
            vals.push(Value::Text(h.clone()));
        }
        if !conds.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conds.join(" AND "));
        }
        sql.push_str(" ORDER BY t.title");

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(vals), |r| TrackRow::from_row(r))?
            .collect::<Result<Vec<_>, _>>()?;

        rows.into_iter().map(|row| self.build_track(row)).collect()
    }

    /// looks up playlists matching the given filters and loads each one fully
    /// (its ordered tracks included).
    pub fn find_playlist(
        &self,
        name: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    ) -> Result<Vec<Playlist>, Box<dyn Error>> {
        let mut sql = String::from("SELECT id, name FROM playlists");
        let mut conds: Vec<&str> = Vec::new();
        let mut vals: Vec<Value> = Vec::new();
        if let Some(n) = &name {
            conds.push("name LIKE ?");
            vals.push(Value::Text(format!("%{n}%")));
        }
        if let Some(i) = id {
            conds.push("id = ?");
            vals.push(Value::Integer(i));
        }
        if let Some(h) = &hash {
            conds.push("hash = ?");
            vals.push(Value::Text(h.clone()));
        }
        if !conds.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conds.join(" AND "));
        }
        sql.push_str(" ORDER BY name");

        let mut stmt = self.conn.prepare(&sql)?;
        let metas = stmt
            .query_map(params_from_iter(vals), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(metas.len());
        for (pid, pname) in metas {
            let tracks = self.load_playlist_tracks(pid)?;
            let mut playlist = Playlist::from_tracks(tracks);
            playlist.set_name(pname);
            out.push(playlist);
        }
        Ok(out)
    }

    /// names of all stored playlists, ordered alphabetically.
    pub fn list_playlists(&self) -> Result<Vec<String>, Box<dyn Error>> {
        let mut stmt = self.conn.prepare("SELECT name FROM playlists ORDER BY name")?;
        let names = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names)
    }

    /// writes every field of `config` into the `settings` key/value table.
    pub fn save_config(&self, config: &Config) -> Result<(), Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut set = tx.prepare(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )?;
            set.execute(params!["dir_music", path_str(&config.dir_music)])?;
            set.execute(params!["dir_config", path_str(&config.dir_config)])?;
            set.execute(params!["dir_arts", path_str(&config.dir_arts)])?;
            set.execute(params!["dir_db", path_str(&config.dir_db)])?;
            set.execute(params!["volume", config.volume.to_string()])?;
            match &config.last_playlist {
                Some(p) => set.execute(params!["last_playlist", p])?,
                None => tx.execute("DELETE FROM settings WHERE key = 'last_playlist'", [])?,
            };
            match config.last_track {
                Some(t) => set.execute(params!["last_track", t.to_string()])?,
                None => tx.execute("DELETE FROM settings WHERE key = 'last_track'", [])?,
            };
        }
        tx.commit()?;
        Ok(())
    }

    /// loads the config, starting from `defaults` and overriding each field with
    /// whatever is stored in the `settings` table.
    pub fn load_config(&self, mut config: Config) -> Result<Config, Box<dyn Error>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (key, value) = row?;
            match key.as_str() {
                "dir_music" => config.dir_music = PathBuf::from(value),
                "dir_config" => config.dir_config = PathBuf::from(value),
                "dir_arts" => config.dir_arts = PathBuf::from(value),
                "dir_db" => config.dir_db = PathBuf::from(value),
                "volume" => {
                    if let Ok(v) = value.parse() {
                        config.volume = v;
                    }
                }
                "last_playlist" => config.last_playlist = Some(value),
                "last_track" => config.last_track = value.parse().ok(),
                _ => {}
            }
        }
        Ok(config)
    }

    /// loads the ordered tracks of playlist `playlist_id`.
    fn load_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.path, t.title, t.duration_sec, t.sample_rate, t.bitrate, \
             t.cover_art, t.volume \
             FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id \
             WHERE pt.playlist_id = ?1 ORDER BY pt.position",
        )?;
        let rows = stmt
            .query_map([playlist_id], |r| TrackRow::from_row(r))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|row| self.build_track(row)).collect()
    }

    /// turns a raw row into a [`TrackVirtual`], pulling in its artist list.
    fn build_track(&self, row: TrackRow) -> Result<TrackVirtual, Box<dyn Error>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.name FROM artists a \
             JOIN track_artists ta ON ta.artist_id = a.id \
             WHERE ta.track_id = ?1 ORDER BY a.name",
        )?;
        let mut artists = stmt
            .query_map([row.id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if artists.is_empty() {
            artists.push("Unknown".into());
        }

        let metadata = TrackMetadata {
            title: row.title,
            artist: artists,
            params: Some(TrackMetadataParams {
                duration_sec: row.duration_sec as u64,
                sample_rate: row.sample_rate as u32,
                bitrate: row.bitrate as u32,
                cover_art: row.cover_art.map(PathBuf::from),
            }),
        };
        let mut track = TrackVirtual::new(row.id, PathBuf::from(row.path), metadata);
        track.volume = row.volume;
        Ok(track)
    }
}

/// raw `tracks` columns shared by every query that returns tracks.
struct TrackRow {
    id: i64,
    path: String,
    title: String,
    duration_sec: i64,
    sample_rate: i64,
    bitrate: i64,
    cover_art: Option<String>,
    volume: f32,
}

impl TrackRow {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            path: r.get(1)?,
            title: r.get(2)?,
            duration_sec: r.get(3)?,
            sample_rate: r.get(4)?,
            bitrate: r.get(5)?,
            cover_art: r.get(6)?,
            volume: r.get(7)?,
        })
    }
}

/// inserts/updates a single track row (and its artists) on the given connection
/// or transaction, returning the row id.
fn upsert_track(conn: &Connection, track: &TrackVirtual) -> Result<i64, Box<dyn Error>> {
    let path = track
        .get_path()
        .ok_or("track has no file path and cannot be indexed")?;
    let path_string = path_str(path);

    let metadata = resolve_metadata(track, path);
    let (duration, sample_rate, bitrate, cover) = match &metadata.params {
        Some(p) => (
            p.duration_sec as i64,
            p.sample_rate as i64,
            p.bitrate as i64,
            p.cover_art.as_ref().map(|c| path_str(c)),
        ),
        None => (0, 0, 0, None),
    };
    let hash = file_hash(path);

    conn.execute(
        "INSERT INTO tracks (path, hash, title, duration_sec, sample_rate, bitrate, cover_art, volume)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(path) DO UPDATE SET
            hash = excluded.hash, title = excluded.title,
            duration_sec = excluded.duration_sec, sample_rate = excluded.sample_rate,
            bitrate = excluded.bitrate, cover_art = excluded.cover_art,
            volume = excluded.volume, updated_at = unixepoch()",
        params![
            path_string,
            hash,
            metadata.title,
            duration,
            sample_rate,
            bitrate,
            cover,
            track.volume
        ],
    )?;

    let id: i64 = conn.query_row("SELECT id FROM tracks WHERE path = ?1", [&path_string], |r| {
        r.get(0)
    })?;

    conn.execute("DELETE FROM track_artists WHERE track_id = ?1", [id])?;
    for name in &metadata.artist {
        conn.execute(
            "INSERT INTO artists (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            [name],
        )?;
        let artist_id: i64 = conn
            .query_row("SELECT id FROM artists WHERE name = ?1", [name], |r| r.get(0))
            .optional()?
            .ok_or("artist row vanished during indexing")?;
        conn.execute(
            "INSERT OR IGNORE INTO track_artists (track_id, artist_id) VALUES (?1, ?2)",
            params![id, artist_id],
        )?;
    }

    Ok(id)
}

/// best-effort metadata for indexing: prefers what the track already holds,
/// then re-probes the file, and finally falls back to a filename-derived title
/// so a tagless file can still be indexed instead of failing the whole batch.
fn resolve_metadata(track: &TrackVirtual, path: &Path) -> Arc<TrackMetadata> {
    if let Ok(meta) = track.get_metadata() {
        return meta;
    }
    if let Ok(meta) = TrackMetadata::from_path(path) {
        return Arc::new(meta);
    }
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown".into());
    Arc::new(TrackMetadata {
        title,
        artist: vec!["Unknown".into()],
        params: None,
    })
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// stable, dependency-free FNV-1a hash of a file's contents, hex-encoded. Used
/// as a content fingerprint for dedup / cross-device sync. `None` if the file
/// can't be read.
fn file_hash(path: &Path) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    Some(format!("{:016x}", fnv1a(&data)))
}

/// FNV-1a over the concatenated track file paths — a cheap fingerprint of a
/// playlist's contents and order.
fn playlist_hash(playlist: &Playlist) -> String {
    let mut joined = Vec::new();
    for track in playlist.tracks() {
        if let Some(p) = track.get_path() {
            joined.extend_from_slice(p.to_string_lossy().as_bytes());
        }
        joined.push(0);
    }
    format!("{:016x}", fnv1a(&joined))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
