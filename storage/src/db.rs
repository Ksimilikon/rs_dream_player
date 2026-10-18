//! NOTATION (naming convention for the storage layer)
//! get_*()    -> read: `get_tracks`/`get_playlists` query (many), trait
//!               `get_track`/`get_playlist` fetch one by key (hash/name),
//!               `get_pool`, `get_track_paths`, `get_track_hashes`,
//!               `get_track_hashes_by_hash`, `get_playlist_hashes`,
//!               `get_playlist_track_paths`.
//! save_*()   -> insert-or-update a whole entity (`save_track`, `save_playlist`,
//!               `save_import` — a whole imported packet in one transaction).
//! set_*()    -> update one field of an entity (`set_track_title`,
//!               `set_track_meta`, `set_track_cover`, `set_track_path`, ...).
//! remove_*() -> delete (`remove_track`, `remove_invalid_tracks`).
//! index_*()  -> filesystem scan into the index (`index_dir`).

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

use crate::schema::{InvalidStructure, backup_path, deploy, migrate, verify_structure};
use crate::traits::indexator::Indexator;

/// sqlite db storing music and playlists. Owns a single connection; every
/// method takes `&self`.
pub struct Db {
    conn: Connection,
    path: PathBuf,
}
impl Db {
    /// opens the db at `path`. If the file is missing, creates it together
    /// with the whole structure. An existing file is migrated first; if that
    /// fails or the structure is invalid, the current file is moved aside (the
    /// `_bak` suffix is appended to its name) and a fresh db is deployed in its
    /// place.
    pub fn init(path: PathBuf) -> Result<Self, Box<dyn Error>> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        // no file — deploy the structure from scratch.
        if !path.exists() {
            let conn = Connection::open(&path)?;
            deploy(&conn)?;
            return Ok(Self { conn, path });
        }

        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "foreign_keys", true)?;

        let checked = migrate(&conn)
            .map_err(|e| InvalidStructure(format!("migration failed: {e}")))
            .and_then(|()| verify_structure(&conn));
        match checked {
            Ok(()) => Ok(Self { conn, path }),
            // failed migration / invalid structure: rename the current db to `*_bak`
            // and bring up a fresh one in its place.
            Err(_) => {
                drop(conn);
                let bak = backup_path(&path);
                let _ = std::fs::remove_file(&bak);
                std::fs::rename(&path, &bak)?;
                let conn = Connection::open(&path)?;
                deploy(&conn)?;
                Ok(Self { conn, path })
            }
        }
    }

    /// path to the db file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// builds a [`TrackVirtual`] from a `tracks` table row, pulling in the
    /// artist list.
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

        // genres of this track (M:N via track_genres).
        let mut gstmt = self.conn.prepare(
            "SELECT g.name FROM genres g \
             JOIN track_genres tg ON tg.genre_id = g.id \
             WHERE tg.track_id = ?1 ORDER BY g.name",
        )?;
        let genres = gstmt
            .query_map([row.id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let metadata = TrackMetadata {
            id: Some(row.id),
            title: row.title,
            artist: artists,
            album: row.album,
            genres,
            params: Some(TrackMetadataParams {
                duration_sec: row.duration as u64,
                sample_rate: 0,
                bitrate: 0,
                cover_art: row.cover_art.map(PathBuf::from),
                listen_count: 0,
            }),
        };
        let mut track = TrackVirtual::new(row.id, PathBuf::from(row.path), metadata);
        track.volume = row.volume as f32;
        track.invalid = row.invalid;
        track.color = row.color;
        track.user_label = row.user_label;
        track.ignore_orphan = row.ignore_orphan;
        Ok(track)
    }

    /// loads the ordered tracks of a playlist by its id.
    fn load_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        let sql = format!(
            "SELECT {TRACK_COLUMNS} \
             FROM playlist_tracks pt JOIN tracks t ON t.hash = pt.song_hash \
             WHERE pt.playlist_id = ?1 ORDER BY pt.position"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([playlist_id], TrackRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut tracks = Vec::with_capacity(rows.len());
        for row in rows {
            tracks.push(self.build_track(row)?);
        }
        Ok(tracks)
    }

    /// builds a full [`Playlist`] (with tracks and cover) from a `playlists` row.
    fn build_playlist(
        &self,
        id: i64,
        name: String,
        cover_art: Option<String>,
        created_at: Option<u64>,
        updated_at: Option<u64>,
    ) -> Result<Playlist, Box<dyn Error>> {
        let tracks = self.load_playlist_tracks(id)?;
        let mut playlist = Playlist::from_tracks(tracks);
        playlist.set_name(name);
        playlist.set_cover_art(cover_art.map(PathBuf::from));
        if let Some(t) = created_at {
            playlist.set_created_at(t);
        }
        if let Some(t) = updated_at {
            playlist.set_updated_at(t);
        }
        Ok(playlist)
    }

    /// selects playlists by parameters: `name` — case-insensitive substring,
    /// `id` — exact match. Without filters returns every playlist.
    pub fn get_playlists(
        &self,
        name: Option<String>,
        id: Option<i64>,
    ) -> Result<Vec<Playlist>, Box<dyn Error>> {
        let mut sql =
            String::from("SELECT id, name, cover_art, created_at, updated_at FROM playlists");
        // a deleted playlist is only a mark for sync.
        let mut conds: Vec<&str> = vec!["deleted_at IS NULL"];
        let mut vals: Vec<Value> = Vec::new();
        if let Some(n) = &name {
            conds.push("name LIKE ?");
            vals.push(Value::Text(format!("%{n}%")));
        }
        if let Some(i) = id {
            conds.push("id = ?");
            vals.push(Value::Integer(i));
        }
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
        sql.push_str(" ORDER BY name");

        let mut stmt = self.conn.prepare(&sql)?;
        let metas = stmt
            .query_map(params_from_iter(vals), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<i64>>(3)?.map(|t| t as u64),
                    r.get::<_, Option<i64>>(4)?.map(|t| t as u64),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(metas.len());
        for (pid, pname, cover, created, updated) in metas {
            out.push(self.build_playlist(pid, pname, cover, created, updated)?);
        }
        Ok(out)
    }

    /// songs that are in no playlist (the library's garbage candidates), by
    /// title; [`OrphanFilter`] — with / without the ones kept on purpose.
    pub fn get_orphan_tracks(&self, filter: OrphanFilter) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        let kept = match filter {
            OrphanFilter::NotIgnored => " AND t.ignore_orphan = 0",
            OrphanFilter::Ignored => " AND t.ignore_orphan = 1",
            OrphanFilter::All => "",
        };
        let sql = format!(
            "SELECT {TRACK_COLUMNS} FROM tracks t \
             WHERE NOT EXISTS (SELECT 1 FROM playlist_tracks pt WHERE pt.song_hash = t.hash){kept} \
             ORDER BY t.title"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], TrackRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|row| self.build_track(row)).collect()
    }

    /// selects songs from the shared pool by parameters. `name` and `artist`
    /// are case-insensitive substrings, `id` and `hash` — exact match. Without
    /// filters returns the whole library.
    pub fn get_tracks(
        &self,
        name: Option<String>,
        artist: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    ) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        let mut sql = format!("SELECT DISTINCT {TRACK_COLUMNS} FROM tracks t");
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
            .query_map(params_from_iter(vals), TrackRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|row| self.build_track(row)).collect()
    }

    /// recursively indexes the music directory `dir`: adds every song found
    /// to the db (unreadable / non-music files are skipped) and returns the
    /// whole library pool as a playlist.
    pub fn index_dir(&self, dir: &Path) -> Result<Playlist, Box<dyn Error>> {
        // a missing directory is not an error — just nothing to index.
        if dir.is_dir() {
            // one commit for the whole walk, not one per statement.
            let tx = self.conn.unchecked_transaction()?;
            self.index_dir_rec(dir)?;
            tx.commit()?;
        }
        self.get_pool()
    }

    /// adds the song files `paths` to the db (tags are read as in
    /// [`Self::index_dir`]) in one transaction. Files that aren't music or
    /// can't be read are skipped; a path already in the db is updated.
    /// Returns the number of files written.
    pub fn add_tracks(&self, paths: &[PathBuf]) -> Result<usize, Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut added = 0;
        for path in paths {
            if let Ok(track) = TrackVirtual::from_file(path.clone(), false) {
                upsert_track(&tx, &track, None)?;
                added += 1;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// sets the "file is invalid" mark of tracks `ids` in one transaction.
    /// Returns the number of tracks changed (unknown ids are skipped).
    pub fn set_tracks_invalid(&self, ids: &[i64], invalid: bool) -> Result<usize, Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut changed = 0;
        {
            let mut stmt = tx.prepare("UPDATE tracks SET invalid = ?2 WHERE id = ?1")?;
            for &id in ids {
                changed += stmt.execute(params![id, invalid as i64])?;
            }
        }
        tx.commit()?;
        Ok(changed)
    }

    fn index_dir_rec(&self, dir: &Path) -> Result<(), Box<dyn Error>> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.index_dir_rec(&path)?;
            } else if let Ok(track) = TrackVirtual::from_file(path, false) {
                upsert_track(&self.conn, &track, None)?;
            }
        }
        Ok(())
    }

    /// updates the saved volume of one track (identified by `path`, which is
    /// unique). A track without a path (not from the library) is silently
    /// skipped.
    pub fn set_track_volume(
        &self,
        track: &TrackVirtual,
        volume: f32,
    ) -> Result<(), Box<dyn Error>> {
        let Some(path) = track.get_path() else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE tracks SET volume = ?2 WHERE path = ?1",
            params![path.to_string_lossy(), volume as f64],
        )?;
        Ok(())
    }

    /// updates the saved track volume by its id.
    pub fn set_track_volume_id(&self, id: i64, volume: f32) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "UPDATE tracks SET volume = ?2 WHERE id = ?1",
            params![id, volume as f64],
        )?;
        Ok(())
    }

    /// soft-deletes a playlist by name: the row stays with `deleted_at` (now),
    /// so the deletion reaches other devices on sync; its track list is
    /// dropped, the tracks themselves stay in the library. An already deleted
    /// / unknown name is a no-op.
    pub fn remove_playlist(&self, name: &str) -> Result<(), Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        soft_delete(&tx, name, unix_now())?;
        tx.commit()?;
        Ok(())
    }

    /// deleted playlists (name, deletion time). `names` == `None` -> all of
    /// them; otherwise only these names (unknown / live ones are skipped).
    pub fn get_deleted_playlists(
        &self,
        names: Option<&[String]>,
    ) -> Result<Vec<DeletedPlaylistRow>, Box<dyn Error>> {
        let row = |r: &rusqlite::Row<'_>| {
            Ok(DeletedPlaylistRow {
                name: r.get(0)?,
                deleted_at: r.get::<_, i64>(1)? as u64,
            })
        };
        let Some(names) = names else {
            let mut stmt = self.conn.prepare(
                "SELECT name, deleted_at FROM playlists WHERE deleted_at IS NOT NULL ORDER BY id",
            )?;
            return Ok(stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?);
        };
        let mut stmt = self.conn.prepare(
            "SELECT name, deleted_at FROM playlists WHERE name = ?1 AND deleted_at IS NOT NULL",
        )?;
        let mut out = Vec::new();
        for name in names {
            if let Some(found) = stmt.query_map([name], row)?.next() {
                out.push(found?);
            }
        }
        Ok(out)
    }

    /// the mark "kept on purpose though in no playlist" (orphan collector).
    pub fn set_track_ignore_orphan(&self, id: i64, ignore: bool) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "UPDATE tracks SET ignore_orphan = ?2 WHERE id = ?1",
            params![id, ignore as i64],
        )?;
        Ok(())
    }

    /// playlist of the whole library song pool (every row of `tracks`).
    pub fn get_pool(&self) -> Result<Playlist, Box<dyn Error>> {
        let tracks = self.get_tracks(None, None, None, None)?;
        Ok(Playlist::from_tracks(tracks))
    }

    /// updates track metadata by its id: `title` and/or the artist list.
    /// `None` fields are left as is. Artists are rewritten entirely (as in
    /// [`upsert_track`]). The track hash and path don't change.
    pub fn set_track_meta(
        &self,
        id: i64,
        title: Option<&str>,
        artists: Option<&[String]>,
    ) -> Result<(), Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        if let Some(t) = title {
            tx.execute("UPDATE tracks SET title = ?2 WHERE id = ?1", params![id, t])?;
        }
        if let Some(list) = artists {
            tx.execute("DELETE FROM track_artists WHERE track_id = ?1", [id])?;
            for name in list {
                tx.execute(
                    "INSERT INTO artists (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
                    [name],
                )?;
                let artist_id: i64 =
                    tx.query_row("SELECT id FROM artists WHERE name = ?1", [name], |r| {
                        r.get(0)
                    })?;
                tx.execute(
                    "INSERT OR IGNORE INTO track_artists (track_id, artist_id) VALUES (?1, ?2)",
                    params![id, artist_id],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// sets (or clears with `None`) the track cover path by its id.
    pub fn set_track_cover(&self, id: i64, cover: Option<&Path>) -> Result<(), Box<dyn Error>> {
        let cover = cover.map(|p| p.to_string_lossy().into_owned());
        self.conn.execute(
            "UPDATE tracks SET cover_art = ?2 WHERE id = ?1",
            params![id, cover],
        )?;
        Ok(())
    }

    /// changes the track file path by its id (after a rename on disk).
    /// The hash stays the same, so playlist references don't break.
    pub fn set_track_path(&self, id: i64, new_path: &Path) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "UPDATE tracks SET path = ?2 WHERE id = ?1",
            params![id, new_path.to_string_lossy()],
        )?;
        Ok(())
    }

    /// removes a track from the index by its id. `track_artists` and
    /// `track_genres` are cleaned by FK cascade, and bindings in every playlist
    /// (`playlist_tracks` by `song_hash`) are removed manually — so a removal
    /// always cascades into playlists.
    pub fn remove_track(&self, id: i64) -> Result<(), Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        // first drop the playlist bindings by the track hash.
        tx.execute(
            "DELETE FROM playlist_tracks \
             WHERE song_hash IN (SELECT hash FROM tracks WHERE id = ?1)",
            [id],
        )?;
        tx.execute("DELETE FROM tracks WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// sets (or clears with `None`) the track album by its id.
    pub fn set_track_album(&self, id: i64, album: Option<&str>) -> Result<(), Box<dyn Error>> {
        let album_id = upsert_album(&self.conn, album)?;
        self.conn.execute(
            "UPDATE tracks SET album_id = ?2 WHERE id = ?1",
            params![id, album_id],
        )?;
        Ok(())
    }

    /// rewrites the track genres (M:N).
    pub fn set_track_genres(&self, id: i64, genres: &[String]) -> Result<(), Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        write_track_genres(&tx, id, genres)?;
        tx.commit()?;
        Ok(())
    }

    /// sets (or clears with `None`) the track color label.
    pub fn set_track_color(&self, id: i64, color: Option<&str>) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "UPDATE tracks SET color = ?2 WHERE id = ?1",
            params![id, color],
        )?;
        Ok(())
    }

    /// sets (or clears with `None`) the track text label.
    pub fn set_track_label(&self, id: i64, label: Option<&str>) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "UPDATE tracks SET user_label = ?2 WHERE id = ?1",
            params![id, label],
        )?;
        Ok(())
    }

    /// marks a track as invalid (the file can't be read) or clears the mark.
    pub fn set_track_invalid(&self, id: i64, invalid: bool) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "UPDATE tracks SET invalid = ?2 WHERE id = ?1",
            params![id, invalid as i64],
        )?;
        Ok(())
    }

    /// assigns a new path to track `id`. If the path is already taken by
    /// another row (e.g. the app has already indexed the renamed file), that
    /// row is removed (cascading from playlists) and the path goes to the
    /// original track. The `invalid` mark is cleared as well.
    pub fn reassign_track_path(&self, id: i64, new_path: &Path) -> Result<(), Box<dyn Error>> {
        let path_s = new_path.to_string_lossy().into_owned();
        // if another row already owns the new path — remove it.
        let dup: Option<i64> = self
            .conn
            .query_row("SELECT id FROM tracks WHERE path = ?1", [&path_s], |r| {
                r.get(0)
            })
            .ok();
        if let Some(dup_id) = dup
            && dup_id != id
        {
            self.remove_track(dup_id)?;
        }
        self.conn.execute(
            "UPDATE tracks SET path = ?2, invalid = 0 WHERE id = ?1",
            params![id, path_s],
        )?;
        Ok(())
    }

    /// removes from the index every track marked as invalid (cascading from
    /// playlists). Returns the number removed.
    pub fn remove_invalid_tracks(&self) -> Result<usize, Box<dyn Error>> {
        let ids: Vec<i64> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id FROM tracks WHERE invalid = 1")?;
            stmt.query_map([], |r| r.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for id in &ids {
            self.remove_track(*id)?;
        }
        Ok(ids.len())
    }

    /// (id, path) pairs of every library track — to check that files exist.
    pub fn get_track_paths(&self) -> Result<Vec<(i64, PathBuf)>, Box<dyn Error>> {
        let mut stmt = self.conn.prepare("SELECT id, path FROM tracks")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, PathBuf::from(r.get::<_, String>(1)?)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// (id, hash, path) triples of tracks. `ids` == `None` -> the whole
    /// library (by ascending id); otherwise — in `ids` order, ids not found are
    /// skipped.
    pub fn get_track_hashes(
        &self,
        ids: Option<&[i64]>,
    ) -> Result<Vec<TrackHashRow>, Box<dyn Error>> {
        let row = |r: &rusqlite::Row<'_>| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                PathBuf::from(r.get::<_, String>(2)?),
            ))
        };
        let Some(ids) = ids else {
            let mut stmt = self
                .conn
                .prepare("SELECT id, hash, path FROM tracks ORDER BY id")?;
            let rows = stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
            return Ok(rows);
        };

        let mut stmt = self
            .conn
            .prepare("SELECT id, hash, path FROM tracks WHERE id = ?1")?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(found) = stmt.query_map([id], row)?.next() {
                out.push(found?);
            }
        }
        Ok(out)
    }

    /// (id, hash, path) triples of tracks by their hashes, in `hashes` order;
    /// hashes not found are skipped.
    pub fn get_track_hashes_by_hash(
        &self,
        hashes: &[String],
    ) -> Result<Vec<TrackHashRow>, Box<dyn Error>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, hash, path FROM tracks WHERE hash = ?1")?;
        let mut out = Vec::with_capacity(hashes.len());
        for hash in hashes {
            let found = stmt
                .query_map([hash], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        PathBuf::from(r.get::<_, String>(2)?),
                    ))
                })?
                .next();
            if let Some(found) = found {
                out.push(found?);
            }
        }
        Ok(out)
    }

    /// playlists with their song hashes in playlist order. `names` == `None`
    /// -> all of them (by ascending id); otherwise — in `names` order, names
    /// not found are skipped. The name is the playlist identifier (unique and
    /// the same on every device, unlike the local id).
    pub fn get_playlist_hashes(
        &self,
        names: Option<&[String]>,
    ) -> Result<Vec<PlaylistHashRow>, Box<dyn Error>> {
        const COLUMNS: &str = "SELECT id, name, cover_art, created_at, updated_at FROM playlists";
        let meta = |r: &rusqlite::Row<'_>| {
            Ok(PlaylistHashRow {
                id: r.get(0)?,
                name: r.get(1)?,
                cover_art: r.get::<_, Option<String>>(2)?.map(PathBuf::from),
                created_at: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                updated_at: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                hashes: Vec::new(),
            })
        };
        let mut metas = match names {
            None => self
                .conn
                .prepare(&format!("{COLUMNS} WHERE deleted_at IS NULL ORDER BY id"))?
                .query_map([], meta)?
                .collect::<Result<Vec<_>, _>>()?,
            Some(names) => {
                let mut stmt = self
                    .conn
                    .prepare(&format!("{COLUMNS} WHERE name = ?1 AND deleted_at IS NULL"))?;
                let mut out = Vec::with_capacity(names.len());
                for name in names {
                    if let Some(found) = stmt.query_map([name], meta)?.next() {
                        out.push(found?);
                    }
                }
                out
            }
        };

        let mut tracks = self.conn.prepare(
            "SELECT song_hash FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position",
        )?;
        for pl in &mut metas {
            pl.hashes = tracks
                .query_map([pl.id], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
        }
        Ok(metas)
    }

    /// saves a packet import in one transaction (all or nothing):
    /// - `tracks` — new tracks (the file already lies at `get_path()`) with
    ///   hashes from the packet. A file written over a known track (same
    ///   path) replaces it: the track takes the packet hash, and local
    ///   playlists referring to the old hash follow it;
    /// - `playlists` — playlists by name (`id` is ignored): created or
    ///   rewritten entirely with the `hashes` list. `None` cover / dates don't
    ///   overwrite an already saved value.
    ///
    /// a deleted local playlist comes back with an imported one; `deleted` —
    /// playlists deleted on the other side: a local playlist changed at or
    /// before the deletion is soft-deleted (a later local change wins), an
    /// unknown name is remembered as deleted so the deletion travels on.
    pub fn save_import(
        &self,
        tracks: &[(TrackVirtual, String)],
        playlists: &[PlaylistHashRow],
        deleted: &[DeletedPlaylistRow],
    ) -> Result<(), Box<dyn Error>> {
        let tx = self.conn.unchecked_transaction()?;
        for (track, hash) in tracks {
            adopt_hash(&tx, track, hash)?;
            upsert_track(&tx, track, Some(hash))?;
        }
        for pl in playlists {
            tx.execute(
                "INSERT INTO playlists (name, cover_art, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(name) DO UPDATE SET \
                    cover_art = COALESCE(excluded.cover_art, cover_art), \
                    created_at = COALESCE(excluded.created_at, created_at), \
                    updated_at = COALESCE(excluded.updated_at, updated_at), \
                    deleted_at = NULL",
                params![
                    pl.name,
                    pl.cover_art.as_ref().map(|p| p.to_string_lossy().into_owned()),
                    pl.created_at.map(|t| t as i64),
                    pl.updated_at.map(|t| t as i64),
                ],
            )?;
            let playlist_id: i64 =
                tx.query_row("SELECT id FROM playlists WHERE name = ?1", [&pl.name], |r| {
                    r.get(0)
                })?;
            tx.execute(
                "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
                [playlist_id],
            )?;
            for (pos, hash) in pl.hashes.iter().enumerate() {
                tx.execute(
                    "INSERT INTO playlist_tracks (playlist_id, song_hash, position) \
                     VALUES (?1, ?2, ?3)",
                    params![playlist_id, hash, pos as i64],
                )?;
            }
        }
        for d in deleted {
            apply_deletion(&tx, &d.name, d.deleted_at)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// (id, path) pairs of the tracks of a specific playlist by its name.
    pub fn get_playlist_track_paths(&self, name: &str) -> Result<Vec<(i64, PathBuf)>, Box<dyn Error>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.path FROM playlist_tracks pt \
             JOIN tracks t ON t.hash = pt.song_hash \
             JOIN playlists p ON p.id = pt.playlist_id \
             WHERE p.name = ?1 ORDER BY pt.position",
        )?;
        let rows = stmt
            .query_map([name], |r| {
                Ok((r.get::<_, i64>(0)?, PathBuf::from(r.get::<_, String>(1)?)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// paired devices (vpair): public keys of the devices this one has paired
/// with. The list lives only on the current device and is never shared.
impl Db {
    /// every paired device, in pairing order.
    pub fn get_paired_devices(&self) -> Result<Vec<PairedDeviceRow>, Box<dyn Error>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, pub_key, name FROM paired_devices ORDER BY id")?;
        let rows = stmt
            .query_map([], PairedDeviceRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// one paired device by its id; `None` — no such device.
    pub fn get_paired_device(&self, id: i64) -> Result<Option<PairedDeviceRow>, Box<dyn Error>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, pub_key, name FROM paired_devices WHERE id = ?1",
                [id],
                PairedDeviceRow::from_row,
            )
            .optional()?;
        Ok(row)
    }

    /// id of the paired device with this public key; `None` — the key is unknown.
    pub fn get_paired_device_id(&self, pub_key: &[u8; 32]) -> Result<Option<i64>, Box<dyn Error>> {
        let id = self
            .conn
            .query_row(
                "SELECT id FROM paired_devices WHERE pub_key = ?1",
                [&pub_key[..]],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id)
    }

    /// saves the public key of a newly paired device (named `Unknown` until the
    /// user renames it) and returns its id. A known key keeps its id and name.
    pub fn save_paired_device(&self, pub_key: &[u8; 32]) -> Result<i64, Box<dyn Error>> {
        self.conn.execute(
            "INSERT INTO paired_devices (pub_key) VALUES (?1) ON CONFLICT(pub_key) DO NOTHING",
            [&pub_key[..]],
        )?;
        self.get_paired_device_id(pub_key)?
            .ok_or_else(|| "paired device vanished right after saving".into())
    }

    /// the user-given device name; for viewing only.
    pub fn set_paired_device_name(&self, id: i64, name: &str) -> Result<(), Box<dyn Error>> {
        self.conn.execute(
            "UPDATE paired_devices SET name = ?1 WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    /// forgets a paired device; its id is never handed out again.
    pub fn remove_paired_device(&self, id: i64) -> Result<(), Box<dyn Error>> {
        self.conn
            .execute("DELETE FROM paired_devices WHERE id = ?1", [id])?;
        Ok(())
    }
}

impl Indexator for Db {
    fn save_track(&self, track: TrackVirtual) -> Result<(), Box<dyn Error>> {
        upsert_track(&self.conn, &track, None)?;
        Ok(())
    }

    /// `key` is the song hash.
    fn get_track(&self, key: String) -> Result<TrackVirtual, Box<dyn Error>> {
        let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks t WHERE t.hash = ?1");
        let row = self.conn.query_row(&sql, [&key], TrackRow::from_row)?;
        self.build_track(row)
    }

    fn save_playlist(&self, playlist: Playlist) -> Result<(), Box<dyn Error>> {
        let name = playlist
            .get_name()
            .ok_or("cannot save an anonymous playlist (it has no name)")?;

        let cover = playlist
            .get_cover_art()
            .map(|p| p.to_string_lossy().into_owned());
        // a local save is a change: dated now unless the playlist carries its
        // own dates (sync compares them); an existing creation date is kept. A
        // deleted playlist with this name comes back.
        let now = unix_now();
        let created = playlist.get_created_at();
        let updated = playlist.get_updated_at().unwrap_or(now);

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO playlists (name, cover_art, created_at, updated_at) \
             VALUES (?1, ?2, COALESCE(?3, ?5), ?4) \
             ON CONFLICT(name) DO UPDATE SET \
                cover_art = excluded.cover_art, \
                created_at = COALESCE(?3, created_at, ?5), \
                updated_at = excluded.updated_at, \
                deleted_at = NULL",
            params![
                name,
                cover,
                created.map(|t| t as i64),
                updated as i64,
                now as i64
            ],
        )?;
        let playlist_id: i64 =
            tx.query_row("SELECT id FROM playlists WHERE name = ?1", [&name], |r| {
                r.get(0)
            })?;

        // rewrite the ordered track bindings of the playlist.
        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
        )?;
        for (pos, track) in playlist.tracks().iter().enumerate() {
            let hash = upsert_track(&tx, track, None)?;
            tx.execute(
                "INSERT INTO playlist_tracks (playlist_id, song_hash, position) \
                 VALUES (?1, ?2, ?3)",
                params![playlist_id, hash, pos as i64],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// `name` is the playlist name; loads it together with its ordered tracks.
    fn get_playlist(&self, name: String) -> Result<Playlist, Box<dyn Error>> {
        let (id, cover, created, updated): (i64, Option<String>, Option<i64>, Option<i64>) =
            self.conn.query_row(
                "SELECT id, cover_art, created_at, updated_at FROM playlists \
                 WHERE name = ?1 AND deleted_at IS NULL",
                [&name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        self.build_playlist(
            id,
            name,
            cover,
            created.map(|t| t as u64),
            updated.map(|t| t as u64),
        )
    }

    /// a random hash generated once when a song is added. It doesn't depend on
    /// the file content, so editing track metadata doesn't change the hash
    /// and doesn't break playlist references.
    /// TODO: for moving between devices a content fingerprint (e.g. over audio
    /// samples) can be added later as a separate field.
    fn hash(_track: &TrackVirtual) -> String {
        random_hash()
    }

    fn hash_exist(&self, hash: String) -> bool {
        self.conn
            .query_row("SELECT 1 FROM tracks WHERE hash = ?1", [&hash], |_| Ok(()))
            .is_ok()
    }
}

/// (id, hash, path) of a track — see [`Db::get_track_hashes`].
pub type TrackHashRow = (i64, String, PathBuf);

/// which songs in no playlist [`Db::get_orphan_tracks`] lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanFilter {
    /// the ones not marked "kept on purpose" (the collector's default).
    NotIgnored,
    /// only the ones marked "kept on purpose".
    Ignored,
    /// both.
    All,
}

/// a soft-deleted playlist: its name (the identifier) and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedPlaylistRow {
    pub name: String,
    pub deleted_at: u64,
}

/// unix seconds now.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// soft-deletes `name` at `at`: the row keeps its name and dates, gets
/// `deleted_at`, loses its track list. Unknown / already deleted — no-op.
fn soft_delete(conn: &Connection, name: &str, at: u64) -> Result<(), Box<dyn Error>> {
    let changed = conn.execute(
        "UPDATE playlists SET deleted_at = ?2 WHERE name = ?1 AND deleted_at IS NULL",
        params![name, at as i64],
    )?;
    if changed > 0 {
        conn.execute(
            "DELETE FROM playlist_tracks \
             WHERE playlist_id = (SELECT id FROM playlists WHERE name = ?1)",
            [name],
        )?;
    }
    Ok(())
}

/// a deletion that came from another device (import / sync): the later of
/// the local change and the deletion wins; at the same second — the deletion.
fn apply_deletion(conn: &Connection, name: &str, at: u64) -> Result<(), Box<dyn Error>> {
    let local: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT updated_at, deleted_at FROM playlists WHERE name = ?1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match local {
        // unknown here: remember the deletion so it travels on.
        None => {
            conn.execute(
                "INSERT INTO playlists (name, deleted_at) VALUES (?1, ?2)",
                params![name, at as i64],
            )?;
        }
        // live, changed at or before the deletion.
        Some((updated, None)) if updated.unwrap_or(0) as u64 <= at => soft_delete(conn, name, at)?,
        // live and changed later: the local change wins.
        Some((_, None)) => {}
        // already deleted: keep the later deletion time.
        Some((_, Some(_))) => {
            conn.execute(
                "UPDATE playlists SET deleted_at = MAX(deleted_at, ?2) WHERE name = ?1",
                params![name, at as i64],
            )?;
        }
    }
    Ok(())
}

/// playlist with song hashes — see [`Db::get_playlist_hashes`].
pub struct PlaylistHashRow {
    /// local id in this device's db (never goes into a packet; ignored by
    /// `save_import` — the playlist is looked up by name).
    pub id: i64,
    pub name: String,
    pub cover_art: Option<PathBuf>,
    pub created_at: Option<u64>,
    pub updated_at: Option<u64>,
    /// song hashes in playlist order.
    pub hashes: Vec<String>,
}

/// a `paired_devices` row — see [`Db::get_paired_devices`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedDeviceRow {
    /// counter id, never reused.
    pub id: i64,
    /// public X25519 key of the device.
    pub pub_key: [u8; 32],
    /// user-given name, for viewing only.
    pub name: String,
}

impl PairedDeviceRow {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let key: Vec<u8> = r.get(1)?;
        let pub_key = key.try_into().map_err(|key: Vec<u8>| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Blob,
                format!("public key of {} bytes instead of 32", key.len()).into(),
            )
        })?;
        Ok(Self {
            id: r.get(0)?,
            pub_key,
            name: r.get(2)?,
        })
    }
}

/// raw `tracks` columns shared by every query that returns tracks.
/// The order matches [`TRACK_COLUMNS`].
struct TrackRow {
    id: i64,
    title: String,
    duration: i64,
    cover_art: Option<String>,
    path: String,
    volume: f64,
    album: Option<String>,
    color: Option<String>,
    user_label: Option<String>,
    invalid: bool,
    ignore_orphan: bool,
}

/// track column list (with the `t` alias) for SELECTs returning [`TrackRow`].
/// The album is pulled in by a correlated subquery on `album_id`.
const TRACK_COLUMNS: &str = "t.id, t.title, t.duration, t.cover_art, t.path, t.volume, \
     (SELECT name FROM albums WHERE id = t.album_id), t.color, t.user_label, t.invalid, \
     t.ignore_orphan";

impl TrackRow {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            title: r.get(1)?,
            duration: r.get(2)?,
            cover_art: r.get(3)?,
            path: r.get(4)?,
            volume: r.get(5)?,
            album: r.get(6)?,
            color: r.get(7)?,
            user_label: r.get(8)?,
            invalid: r.get(9)?,
            ignore_orphan: r.get(10)?,
        })
    }
}

/// inserts / updates one track row (and its artists) on the given connection
/// or transaction, returning the stored hash. An already known track (same
/// `path`) keeps its original hash, so playlist references stay valid even
/// after its metadata is edited.
///
/// `hash` sets the hash of a new track (packet import, the hash comes from
/// another device); `None` -> random.
fn upsert_track(
    conn: &Connection,
    track: &TrackVirtual,
    hash: Option<&str>,
) -> Result<String, Box<dyn Error>> {
    let path = track
        .get_path()
        .ok_or("track has no file path and cannot be stored")?;
    let path_s = path.to_string_lossy().into_owned();

    let meta = resolve_metadata(track, path);
    let (duration, cover) = match &meta.params {
        Some(p) => (
            p.duration_sec as i64,
            p.cover_art
                .as_ref()
                .map(|c| c.to_string_lossy().into_owned()),
        ),
        None => (0, None),
    };
    let source = source_label(track);
    // album from the tags (1:N) → album_id.
    let album_id = upsert_album(conn, meta.album.as_deref())?;

    // a new track gets this random hash; on a `path` conflict the existing
    // hash is left alone. color/user_label/invalid/volume are user state and
    // are NOT overwritten on a rescan (ON CONFLICT) (otherwise a re-index on
    // every start would reset the volume to the 1.0 default of the freshly
    // read file).
    let new_hash = hash.map_or_else(random_hash, str::to_owned);
    conn.execute(
        "INSERT INTO tracks \
            (hash, title, duration, cover_art, path, source_type, volume, album_id, color, user_label, invalid) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(path) DO UPDATE SET \
            title = excluded.title, duration = excluded.duration, \
            cover_art = excluded.cover_art, source_type = excluded.source_type, \
            album_id = excluded.album_id",
        params![
            new_hash,
            meta.title,
            duration,
            cover,
            path_s,
            source,
            track.volume as f64,
            album_id,
            track.color,
            track.user_label,
            track.invalid as i64,
        ],
    )?;

    let (id, hash): (i64, String) = conn.query_row(
        "SELECT id, hash FROM tracks WHERE path = ?1",
        [&path_s],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // rewrite the track artist list entirely.
    conn.execute("DELETE FROM track_artists WHERE track_id = ?1", [id])?;
    for name in &meta.artist {
        conn.execute(
            "INSERT INTO artists (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            [name],
        )?;
        let artist_id: i64 =
            conn.query_row("SELECT id FROM artists WHERE name = ?1", [name], |r| {
                r.get(0)
            })?;
        conn.execute(
            "INSERT OR IGNORE INTO track_artists (track_id, artist_id) VALUES (?1, ?2)",
            params![id, artist_id],
        )?;
    }

    // rewrite the track genre list entirely (M:N via track_genres).
    write_track_genres(conn, id, &meta.genres)?;

    Ok(hash)
}

/// a track already stored at the path of `track` takes `hash` (its file was
/// replaced by an imported one), and playlist references move from the old
/// hash to the new one. Nothing happens for a new path.
fn adopt_hash(conn: &Connection, track: &TrackVirtual, hash: &str) -> Result<(), Box<dyn Error>> {
    let Some(path) = track.get_path() else {
        return Ok(());
    };
    let old: Option<String> = conn
        .query_row(
            "SELECT hash FROM tracks WHERE path = ?1",
            [path.to_string_lossy()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old.filter(|old| old != hash) {
        conn.execute("UPDATE tracks SET hash = ?1 WHERE hash = ?2", [hash, &old])?;
        conn.execute(
            "UPDATE playlist_tracks SET song_hash = ?1 WHERE song_hash = ?2",
            [hash, &old],
        )?;
    }
    Ok(())
}

/// upserts an album name, returns its id (or `None` if the album is empty).
fn upsert_album(conn: &Connection, album: Option<&str>) -> Result<Option<i64>, Box<dyn Error>> {
    let Some(name) = album.filter(|a| !a.is_empty()) else {
        return Ok(None);
    };
    conn.execute(
        "INSERT INTO albums (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
        [name],
    )?;
    let id: i64 = conn.query_row("SELECT id FROM albums WHERE name = ?1", [name], |r| {
        r.get(0)
    })?;
    Ok(Some(id))
}

/// rewrites the track genres: clears `track_genres` for it and inserts anew.
fn write_track_genres(
    conn: &Connection,
    track_id: i64,
    genres: &[String],
) -> Result<(), Box<dyn Error>> {
    conn.execute("DELETE FROM track_genres WHERE track_id = ?1", [track_id])?;
    for name in genres.iter().filter(|g| !g.is_empty()) {
        conn.execute(
            "INSERT INTO genres (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            [name],
        )?;
        let genre_id: i64 =
            conn.query_row("SELECT id FROM genres WHERE name = ?1", [name], |r| {
                r.get(0)
            })?;
        conn.execute(
            "INSERT OR IGNORE INTO track_genres (track_id, genre_id) VALUES (?1, ?2)",
            params![track_id, genre_id],
        )?;
    }
    Ok(())
}

/// metadata to write: first what the track already holds, then a re-read of
/// the file, and finally a title from the file name — so a track without tags
/// can still be saved.
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
        id: None,
        title,
        artist: vec!["Unknown".into()],
        album: None,
        genres: Vec::new(),
        params: None,
    })
}

/// source type label of a track for the `source_type` column.
fn source_label(track: &TrackVirtual) -> &'static str {
    if track.index_id().is_some() {
        "index"
    } else if track.get_path().is_some() {
        "file"
    } else {
        "out"
    }
}

/// 128-bit random hash in hex.
fn random_hash() -> String {
    format!("{:016x}{:016x}", fastrand::u64(..), fastrand::u64(..))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use audio_structs::{
        playlist::Playlist,
        track_metadata::{TrackMetadata, TrackMetadataParams},
        track_virtual::TrackVirtual,
    };
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::*;
    use crate::schema::DB_FILE_NAME;

    /// creates a real file (so the track has an existing `path`) and a track
    /// with explicit metadata — without decoding audio.
    fn fixture_track(dir: &Path, file: &str, title: &str, artist: &str) -> TrackVirtual {
        let path = dir.join(file);
        fs::write(&path, format!("dummy-{file}")).unwrap();
        let metadata = TrackMetadata {
            id: None,
            title: title.into(),
            artist: vec![artist.into()],
            album: None,
            genres: Vec::new(),
            params: Some(TrackMetadataParams {
                duration_sec: 180,
                sample_rate: 44_100,
                bitrate: 320,
                cover_art: None,
                listen_count: 0,
            }),
        };
        TrackVirtual::new(0, path, metadata)
    }

    /// a playlist `name` of the given tracks with an explicit update time.
    fn save_named(db: &Db, name: &str, tracks: Vec<TrackVirtual>, updated: u64) {
        let mut pl = Playlist::from_tracks(tracks);
        pl.set_name(name.into());
        pl.set_updated_at(updated);
        db.save_playlist(pl).unwrap();
    }

    fn live_names(db: &Db) -> Vec<String> {
        db.get_playlist_hashes(None).unwrap().into_iter().map(|p| p.name).collect()
    }

    fn deleted(db: &Db) -> Vec<(String, u64)> {
        db.get_deleted_playlists(None)
            .unwrap()
            .into_iter()
            .map(|d| (d.name, d.deleted_at))
            .collect()
    }

    #[test]
    fn removing_a_playlist_soft_deletes_it() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        save_named(&db, "mix", vec![fixture_track(dir.path(), "a.mp3", "A", "X")], 100);
        db.remove_playlist("mix").unwrap();

        assert!(live_names(&db).is_empty());
        assert!(db.get_playlists(None, None).unwrap().is_empty());
        let gone = deleted(&db);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].0, "mix");
        // its song is now in no playlist.
        assert_eq!(db.get_orphan_tracks(OrphanFilter::All).unwrap().len(), 1);

        // the same name saved again comes back.
        save_named(&db, "mix", vec![], 0);
        assert_eq!(live_names(&db), vec!["mix".to_string()]);
        assert!(deleted(&db).is_empty());
    }

    #[test]
    fn a_local_save_is_dated() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        let mut pl = Playlist::from_tracks(vec![]);
        pl.set_name("undated".into());
        db.save_playlist(pl).unwrap();
        let row = db.get_playlist_hashes(None).unwrap().remove(0);
        assert!(row.updated_at.unwrap() > 1_600_000_000);
        assert!(row.created_at.is_some());
    }

    #[test]
    fn imported_deletions_follow_the_later_change() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        save_named(&db, "older", vec![], 100);
        save_named(&db, "newer", vec![], 500);
        save_named(&db, "already", vec![], 10);
        db.remove_playlist("already").unwrap();
        let already_at = deleted(&db)[0].1;
        let gone = |name: &str, at: u64| DeletedPlaylistRow {
            name: name.into(),
            deleted_at: at,
        };
        db.save_import(
            &[],
            &[],
            &[gone("older", 300), gone("newer", 300), gone("unknown", 42), gone("already", 1)],
        )
        .unwrap();

        // changed before the deletion — deleted; after it — kept.
        assert_eq!(live_names(&db), vec!["newer".to_string()]);
        let mut got = deleted(&db);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("already".to_string(), already_at),
                ("older".to_string(), 300),
                ("unknown".to_string(), 42),
            ]
        );
    }

    #[test]
    fn an_imported_playlist_brings_a_deleted_one_back() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        save_named(&db, "mix", vec![], 100);
        db.remove_playlist("mix").unwrap();
        let pl = PlaylistHashRow {
            id: 0,
            name: "mix".into(),
            cover_art: None,
            created_at: Some(1),
            updated_at: Some(u64::from(u32::MAX)),
            hashes: Vec::new(),
        };
        db.save_import(&[], &[pl], &[]).unwrap();
        assert_eq!(live_names(&db), vec!["mix".to_string()]);
        assert!(deleted(&db).is_empty());
    }

    #[test]
    fn orphans_kept_on_purpose_are_filtered() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "Alpha", "X")).unwrap();
        db.save_track(fixture_track(dir.path(), "b.mp3", "Beta", "X")).unwrap();
        let beta = db.get_tracks(Some("Beta".into()), None, None, None).unwrap();
        let beta_id = beta[0].index_id().unwrap();
        db.set_track_ignore_orphan(beta_id, true).unwrap();

        let titles = |f| -> Vec<String> {
            db.get_orphan_tracks(f)
                .unwrap()
                .iter()
                .map(|t| t.get_metadata().unwrap().title.clone())
                .collect()
        };
        assert_eq!(titles(OrphanFilter::NotIgnored), vec!["Alpha".to_string()]);
        assert_eq!(titles(OrphanFilter::Ignored), vec!["Beta".to_string()]);
        assert_eq!(titles(OrphanFilter::All), vec!["Alpha".to_string(), "Beta".to_string()]);
        assert!(db.get_tracks(Some("Beta".into()), None, None, None).unwrap()[0].ignore_orphan);
    }

    #[test]
    fn orphan_tracks_are_the_ones_in_no_playlist() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "Alpha", "X")).unwrap();
        db.save_track(fixture_track(dir.path(), "b.mp3", "Beta", "X")).unwrap();
        let alpha = db
            .get_tracks(None, None, None, None)
            .unwrap()
            .into_iter()
            .find(|t| t.get_metadata().unwrap().title == "Alpha")
            .unwrap();
        let mut pl = Playlist::from_tracks(vec![alpha]);
        pl.set_name("only alpha".into());
        db.save_playlist(pl).unwrap();

        let orphans = db.get_orphan_tracks(OrphanFilter::NotIgnored).unwrap();
        let titles: Vec<String> = orphans.iter().map(|t| t.get_metadata().unwrap().title.clone()).collect();
        assert_eq!(titles, vec!["Beta".to_string()]);
    }

    #[test]
    fn add_tracks_skips_non_music_and_marks_invalid_in_batch() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        let song = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test_data/2.mp3");
        let junk = dir.path().join("notes.mp3");
        fs::write(&junk, "not audio").unwrap();

        let added = db.add_tracks(&[song.clone(), junk, dir.path().join("missing.mp3")]).unwrap();
        assert_eq!(added, 1, "only the real song is written");
        // the same path again is an update, not a second row.
        assert_eq!(db.add_tracks(&[song]).unwrap(), 1);
        let pool = db.get_tracks(None, None, None, None).unwrap();
        assert_eq!(pool.len(), 1);

        let id = pool[0].index_id().unwrap();
        assert_eq!(db.set_tracks_invalid(&[id, 9999], true).unwrap(), 1);
        let pool = db.get_tracks(None, None, None, None).unwrap();
        assert!(pool[0].invalid);
    }

    #[test]
    fn deploys_full_structure_on_init() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(DB_FILE_NAME);
        let _db = Db::init(path.clone()).unwrap();
        assert!(path.exists());

        let conn = Connection::open(&path).unwrap();
        for table in [
            "tracks",
            "artists",
            "track_artists",
            "playlists",
            "playlist_tracks",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "table `{table}` missing");
        }
    }

    #[test]
    fn migrates_old_db_keeping_data() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(DB_FILE_NAME);
        let db = Db::init(path.clone()).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "Hello", "World"))
            .unwrap();
        save_named(&db, "mix", vec![], 100);
        // roll back to the pre-migration db of older app versions.
        db.conn
            .execute_batch(
                "DROP TABLE paired_devices; \
                 ALTER TABLE playlists DROP COLUMN deleted_at; \
                 ALTER TABLE tracks DROP COLUMN ignore_orphan; \
                 PRAGMA user_version = 0;",
            )
            .unwrap();
        drop(db);

        let db = Db::init(path.clone()).unwrap();
        assert!(!path.with_file_name(format!("{DB_FILE_NAME}_bak")).exists());
        assert_eq!(db.get_tracks(None, None, None, None).unwrap().len(), 1);
        // migration 2: the playlist is live, the new marks have their defaults.
        assert_eq!(live_names(&db), vec!["mix".to_string()]);
        assert!(!db.get_tracks(None, None, None, None).unwrap()[0].ignore_orphan);
        let id = db.save_paired_device(&[7; 32]).unwrap();
        assert_eq!(db.get_paired_device_id(&[7; 32]).unwrap(), Some(id));
    }

    #[test]
    fn paired_device_ids_are_not_reused() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        let a = db.save_paired_device(&[1; 32]).unwrap();
        // saving a known key keeps its id.
        assert_eq!(db.save_paired_device(&[1; 32]).unwrap(), a);
        db.remove_paired_device(a).unwrap();
        let b = db.save_paired_device(&[2; 32]).unwrap();
        assert!(b > a);
    }

    #[test]
    fn backs_up_invalid_structure() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(DB_FILE_NAME);
        // not an sqlite file -> invalid structure.
        fs::write(&path, b"this is definitely not a sqlite file").unwrap();

        let db = Db::init(path.clone()).unwrap();
        // the old file is moved to *_bak, a new db is deployed and empty.
        assert!(path.with_file_name(format!("{DB_FILE_NAME}_bak")).exists());
        assert!(db.hash_exist("whatever".into()) == false);
    }

    #[test]
    fn saves_and_loads_a_track() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        let track = fixture_track(dir.path(), "a.mp3", "Hello", "World");
        db.save_track(track).unwrap();

        // take the hash straight from the db and load the track by it.
        let hash: String = db
            .conn
            .query_row("SELECT hash FROM tracks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(db.hash_exist(hash.clone()));

        let loaded = db.get_track(hash).unwrap();
        let meta = loaded.get_metadata().unwrap();
        assert_eq!(meta.title, "Hello");
        assert_eq!(meta.artist, vec!["World".to_string()]);
    }

    #[test]
    fn re_saving_same_path_keeps_hash() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        let track = fixture_track(dir.path(), "a.mp3", "Hello", "World");
        db.save_track(track).unwrap();
        let first: String = db
            .conn
            .query_row("SELECT hash FROM tracks LIMIT 1", [], |r| r.get(0))
            .unwrap();

        // same path, changed title — the hash must stay the same.
        let edited = fixture_track(dir.path(), "a.mp3", "Hello (edited)", "World");
        db.save_track(edited).unwrap();
        let count: i64 = db
            .conn
            .query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let second: String = db
            .conn
            .query_row("SELECT hash FROM tracks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn finds_tracks_by_params() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        db.save_track(fixture_track(dir.path(), "1.mp3", "Hello", "World"))
            .unwrap();
        db.save_track(fixture_track(dir.path(), "2.mp3", "Goodbye", "World"))
            .unwrap();
        db.save_track(fixture_track(dir.path(), "3.mp3", "Hello Again", "Other"))
            .unwrap();

        // the whole pool.
        assert_eq!(db.get_tracks(None, None, None, None).unwrap().len(), 3);
        // by title (substring).
        assert_eq!(
            db.get_tracks(Some("hello".into()), None, None, None)
                .unwrap()
                .len(),
            2
        );
        // by artist.
        assert_eq!(
            db.get_tracks(None, Some("world".into()), None, None)
                .unwrap()
                .len(),
            2
        );
        // name + artist combined.
        assert_eq!(
            db.get_tracks(Some("hello".into()), Some("other".into()), None, None)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn lists_and_finds_playlists_with_cover() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        let mut p1 = Playlist::from_tracks(vec![fixture_track(dir.path(), "1.mp3", "A", "X")]);
        p1.set_name("rock".into());
        p1.set_cover_art(Some(dir.path().join("rock.png")));
        p1.set_created_at(1_000);
        p1.set_updated_at(2_000);
        db.save_playlist(p1).unwrap();

        let mut p2 = Playlist::from_tracks(vec![fixture_track(dir.path(), "2.mp3", "B", "Y")]);
        p2.set_name("jazz".into());
        db.save_playlist(p2).unwrap();

        // get all.
        let all = db.get_playlists(None, None).unwrap();
        assert_eq!(all.len(), 2);
        // ordered by name: jazz, rock.
        assert_eq!(all[0].get_name().as_deref(), Some("jazz"));
        assert_eq!(all[1].get_name().as_deref(), Some("rock"));

        // selection by parameter + the cover comes with the playlist.
        let found = db.get_playlists(Some("rock".into()), None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].get_cover_art(),
            Some(dir.path().join("rock.png").as_path())
        );
        // timestamps come with the playlist too.
        assert_eq!(found[0].get_created_at(), Some(1_000));
        assert_eq!(found[0].get_updated_at(), Some(2_000));
        // jazz has no cover; saved without timestamps, it is dated by the save.
        let jazz = &db.get_playlists(Some("jazz".into()), None).unwrap()[0];
        assert!(jazz.get_cover_art().is_none());
        assert!(jazz.get_created_at().is_some());
        assert!(jazz.get_updated_at().is_some());
    }

    #[test]
    fn track_and_playlist_hashes() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        let mut p = Playlist::from_tracks(vec![
            fixture_track(dir.path(), "1.mp3", "First", "A"),
            fixture_track(dir.path(), "2.mp3", "Second", "B"),
        ]);
        p.set_name("mix".into());
        p.set_cover_art(Some(dir.path().join("mix.png")));
        p.set_created_at(100);
        p.set_updated_at(200);
        db.save_playlist(p).unwrap();
        let mut empty = Playlist::from_tracks(vec![]);
        empty.set_name("empty".into());
        db.save_playlist(empty).unwrap();

        // the whole library: by ascending id.
        let all = db.get_track_hashes(None).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all[0].0 < all[1].0);
        assert!(all.iter().all(|(_, h, _)| h.len() == 32));

        // by id: request order, ids not found are skipped.
        let picked = db
            .get_track_hashes(Some(&[all[1].0, 9_999, all[0].0]))
            .unwrap();
        assert_eq!(picked, vec![all[1].clone(), all[0].clone()]);

        // the playlist references the same hashes in its own order.
        let pls = db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls.len(), 2);
        assert_eq!(pls[0].name, "mix");
        assert_eq!(pls[0].cover_art, Some(dir.path().join("mix.png")));
        assert_eq!((pls[0].created_at, pls[0].updated_at), (Some(100), Some(200)));
        // a playlist saved without dates is dated by the save.
        assert!(pls[1].hashes.is_empty() && pls[1].created_at.is_some());

        // by name: request order, names not found are skipped.
        let picked = db
            .get_playlist_hashes(Some(&["empty".into(), "nope".into(), "mix".into()]))
            .unwrap();
        assert_eq!(
            picked.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["empty", "mix"]
        );
        let by_path = |file: &str| {
            all.iter()
                .find(|(_, _, p)| p.ends_with(file))
                .unwrap()
                .1
                .clone()
        };
        assert_eq!(pls[0].hashes, vec![by_path("1.mp3"), by_path("2.mp3")]);

        // by hashes: request order, hashes not found are skipped.
        let by_hash = db
            .get_track_hashes_by_hash(&[by_path("2.mp3"), "nope".into(), by_path("1.mp3")])
            .unwrap();
        assert_eq!(by_hash, vec![all[1].clone(), all[0].clone()]);
    }

    #[test]
    fn save_import_keeps_given_hashes() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        let mut old = Playlist::from_tracks(vec![]);
        old.set_name("mix".into());
        old.set_cover_art(Some(dir.path().join("old.png")));
        old.set_created_at(10);
        db.save_playlist(old).unwrap();

        let track = fixture_track(dir.path(), "1.mp3", "First", "A");
        let hash = "0123456789abcdef0123456789abcdef".to_string();
        let pl = PlaylistHashRow {
            id: 0,
            name: "mix".into(),
            cover_art: None,
            created_at: None,
            updated_at: Some(20),
            hashes: vec![hash.clone(), "ffffffffffffffffffffffffffffffff".into()],
        };
        db.save_import(&[(track, hash.clone())], &[pl], &[]).unwrap();

        // the track is saved with the hash from the packet.
        assert!(db.hash_exist(hash.clone()));
        // the playlist is rewritten, but None didn't wipe the cover and created_at.
        let pls = db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls.len(), 1);
        assert_eq!(pls[0].hashes.len(), 2);
        assert_eq!(pls[0].cover_art, Some(dir.path().join("old.png")));
        assert_eq!((pls[0].created_at, pls[0].updated_at), (Some(10), Some(20)));
    }

    #[test]
    fn import_over_known_path_takes_packet_hash() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        let track = fixture_track(dir.path(), "song.mp3", "Song", "A");
        let mut local = Playlist::from_tracks(vec![track]);
        local.set_name("local".into());
        db.save_playlist(local).unwrap();
        let old = db.get_track_hashes(None).unwrap()[0].1.clone();

        // the imported file was written over the local one.
        let track = fixture_track(dir.path(), "song.mp3", "Song", "A");
        let new = "0123456789abcdef0123456789abcdef".to_string();
        db.save_import(&[(track, new.clone())], &[], &[]).unwrap();

        let tracks = db.get_track_hashes(None).unwrap();
        assert_eq!(tracks.len(), 1, "replaced, not duplicated");
        assert_eq!(tracks[0].1, new);
        assert!(!db.hash_exist(old));
        let pls = db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls[0].hashes, vec![new]);
    }

    #[test]
    fn saves_and_loads_a_playlist_in_order() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        let tracks = vec![
            fixture_track(dir.path(), "1.mp3", "First", "A"),
            fixture_track(dir.path(), "2.mp3", "Second", "B"),
        ];
        let mut playlist = Playlist::from_tracks(tracks);
        playlist.set_name("mix".into());
        db.save_playlist(playlist).unwrap();

        let loaded = db.get_playlist("mix".into()).unwrap();
        assert_eq!(loaded.get_count(), 2);
        let titles: Vec<_> = loaded
            .tracks()
            .iter()
            .map(|t| t.get_metadata().unwrap().title.clone())
            .collect();
        assert_eq!(titles, vec!["First".to_string(), "Second".to_string()]);
    }

    /// id of the single library track (in tests there is always one track).
    fn only_track_id(db: &Db) -> i64 {
        db.conn
            .query_row("SELECT id FROM tracks LIMIT 1", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn updates_track_title_and_artists() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "Old", "A"))
            .unwrap();
        let id = only_track_id(&db);

        db.set_track_meta(id, Some("New"), Some(&["X".into(), "Y".into()]))
            .unwrap();

        let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
        let meta = track.get_metadata().unwrap();
        assert_eq!(meta.title, "New");
        assert_eq!(meta.artist, vec!["X".to_string(), "Y".to_string()]);

        // passing None leaves the field alone: only artists change, the title stays.
        db.set_track_meta(id, None, Some(&["Z".into()])).unwrap();
        let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
        assert_eq!(track.get_metadata().unwrap().title, "New");
        assert_eq!(track.get_metadata().unwrap().artist, vec!["Z".to_string()]);
    }

    #[test]
    fn rename_keeps_hash_and_updates_path() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "T", "A"))
            .unwrap();
        let id = only_track_id(&db);
        let before: String = db
            .conn
            .query_row("SELECT hash FROM tracks WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();

        let new_path = dir.path().join("renamed.mp3");
        db.set_track_path(id, &new_path).unwrap();

        let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
        assert_eq!(track.get_path(), Some(new_path.as_path()));
        let after: String = db
            .conn
            .query_row("SELECT hash FROM tracks WHERE id = ?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn set_cover_roundtrips() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "T", "A"))
            .unwrap();
        let id = only_track_id(&db);

        let cover = dir.path().join("cover.png");
        db.set_track_cover(id, Some(&cover)).unwrap();
        let stored: Option<String> = db
            .conn
            .query_row("SELECT cover_art FROM tracks WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(stored.as_deref(), Some(cover.to_string_lossy().as_ref()));

        db.set_track_cover(id, None).unwrap();
        let cleared: Option<String> = db
            .conn
            .query_row("SELECT cover_art FROM tracks WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(cleared.is_none());
    }

    #[test]
    fn remove_track_drops_row_and_artists() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "T", "Solo"))
            .unwrap();
        let id = only_track_id(&db);

        db.remove_track(id).unwrap();
        assert_eq!(db.get_tracks(None, None, None, None).unwrap().len(), 0);
        let links: i64 = db
            .conn
            .query_row("SELECT count(*) FROM track_artists", [], |r| r.get(0))
            .unwrap();
        assert_eq!(links, 0);
    }

    #[test]
    fn track_paths_all_and_per_playlist() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        let mut p = Playlist::from_tracks(vec![
            fixture_track(dir.path(), "1.mp3", "One", "A"),
            fixture_track(dir.path(), "2.mp3", "Two", "B"),
        ]);
        p.set_name("mix".into());
        db.save_playlist(p).unwrap();
        db.save_track(fixture_track(dir.path(), "3.mp3", "Three", "C"))
            .unwrap();

        // the whole pool — three tracks.
        assert_eq!(db.get_track_paths().unwrap().len(), 3);
        // the playlist — only two, in position order.
        let pl = db.get_playlist_track_paths("mix").unwrap();
        assert_eq!(pl.len(), 2);
        assert!(pl[0].1.ends_with("1.mp3"));
        assert!(pl[1].1.ends_with("2.mp3"));
    }

    #[test]
    fn album_and_genres_roundtrip() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "T", "A"))
            .unwrap();
        let id = only_track_id(&db);

        db.set_track_album(id, Some("Nevermind")).unwrap();
        db.set_track_genres(id, &["Grunge".into(), "Rock".into()])
            .unwrap();

        let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
        let meta = track.get_metadata().unwrap();
        assert_eq!(meta.album.as_deref(), Some("Nevermind"));
        assert_eq!(meta.genres, vec!["Grunge".to_string(), "Rock".to_string()]);

        // clearing the album and genres.
        db.set_track_album(id, None).unwrap();
        db.set_track_genres(id, &[]).unwrap();
        let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
        let meta = track.get_metadata().unwrap();
        assert!(meta.album.is_none());
        assert!(meta.genres.is_empty());
    }

    #[test]
    fn color_label_and_invalid_roundtrip() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "T", "A"))
            .unwrap();
        let id = only_track_id(&db);

        db.set_track_color(id, Some("red")).unwrap();
        db.set_track_label(id, Some("favourite")).unwrap();
        db.set_track_invalid(id, true).unwrap();

        let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
        assert_eq!(track.color.as_deref(), Some("red"));
        assert_eq!(track.user_label.as_deref(), Some("favourite"));
        assert!(track.invalid);
    }

    #[test]
    fn rescan_preserves_user_state() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "a.mp3", "T", "A"))
            .unwrap();
        let id = only_track_id(&db);
        db.set_track_color(id, Some("blue")).unwrap();
        db.set_track_invalid(id, true).unwrap();
        db.set_track_volume_id(id, 0.3).unwrap();

        // a repeated upsert of the same path (as on a re-scan) must not wipe
        // the user color/invalid/volume (a freshly read track has the default
        // volume 1.0 — it must NOT overwrite the saved one).
        db.save_track(fixture_track(dir.path(), "a.mp3", "T2", "A"))
            .unwrap();
        let track = &db.get_tracks(None, None, Some(id), None).unwrap()[0];
        assert_eq!(track.get_metadata().unwrap().title, "T2");
        assert_eq!(track.color.as_deref(), Some("blue"));
        assert!(track.invalid);
        assert_eq!(track.volume, 0.3);
    }

    #[test]
    fn remove_track_cascades_to_playlists() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        let mut p = Playlist::from_tracks(vec![
            fixture_track(dir.path(), "1.mp3", "One", "A"),
            fixture_track(dir.path(), "2.mp3", "Two", "B"),
        ]);
        p.set_name("mix".into());
        db.save_playlist(p).unwrap();

        let id: i64 = db
            .conn
            .query_row("SELECT id FROM tracks WHERE title = 'One'", [], |r| {
                r.get(0)
            })
            .unwrap();
        db.remove_track(id).unwrap();

        // the track is gone from both the index and the playlist.
        assert_eq!(db.get_tracks(None, None, None, None).unwrap().len(), 1);
        assert_eq!(db.get_playlist_track_paths("mix").unwrap().len(), 1);
        let links: i64 = db
            .conn
            .query_row("SELECT count(*) FROM playlist_tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(links, 1);
    }

    #[test]
    fn reassign_path_dedups_and_clears_invalid() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        // the original track (we'll mark it invalid) and a duplicate at the future path.
        db.save_track(fixture_track(dir.path(), "old.mp3", "Song", "A"))
            .unwrap();
        db.save_track(fixture_track(dir.path(), "new.mp3", "Song Dup", "A"))
            .unwrap();
        let old_id: i64 = db
            .conn
            .query_row("SELECT id FROM tracks WHERE title = 'Song'", [], |r| {
                r.get(0)
            })
            .unwrap();
        db.set_track_invalid(old_id, true).unwrap();

        let new_path = dir.path().join("new.mp3");
        db.reassign_track_path(old_id, &new_path).unwrap();

        // the duplicate is removed, the old row now has the new path and no invalid mark.
        let all = db.get_tracks(None, None, None, None).unwrap();
        assert_eq!(all.len(), 1);
        let track = &db.get_tracks(None, None, Some(old_id), None).unwrap()[0];
        assert_eq!(track.get_path(), Some(new_path.as_path()));
        assert!(!track.invalid);
    }

    #[test]
    fn remove_invalid_purges_all_flagged() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();
        db.save_track(fixture_track(dir.path(), "1.mp3", "Keep", "A"))
            .unwrap();
        db.save_track(fixture_track(dir.path(), "2.mp3", "Drop", "A"))
            .unwrap();
        let drop_id: i64 = db
            .conn
            .query_row("SELECT id FROM tracks WHERE title = 'Drop'", [], |r| {
                r.get(0)
            })
            .unwrap();
        db.set_track_invalid(drop_id, true).unwrap();

        assert_eq!(db.remove_invalid_tracks().unwrap(), 1);
        let all = db.get_tracks(None, None, None, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].get_metadata().unwrap().title, "Keep");
    }
}
