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
use rusqlite::{Connection, params, params_from_iter, types::Value};

use crate::schema::{backup_path, deploy, verify_structure};
use crate::traits::indexator::Indexator;

/// sqlite-бд для хранения музыки и плейлистов. Владеет одним соединением;
/// все методы берут `&self`.
pub struct Db {
    conn: Connection,
    path: PathBuf,
}
impl Db {
    /// открывает бд по `path`. Если файла нет — создаёт его вместе со всей
    /// структурой. Если структура существующего файла неверна, текущий файл
    /// отодвигается в сторону (к его имени добавляется суффикс `_bak`), а на
    /// его месте разворачивается свежая бд.
    pub fn init(path: PathBuf) -> Result<Self, Box<dyn Error>> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        // нет файла — разворачиваем структуру с нуля.
        if !path.exists() {
            let conn = Connection::open(&path)?;
            deploy(&conn)?;
            return Ok(Self { conn, path });
        }

        let conn = Connection::open(&path)?;
        conn.pragma_update(None, "foreign_keys", true)?;

        match verify_structure(&conn) {
            Ok(()) => Ok(Self { conn, path }),
            // неверная структура: переименовываем текущую бд в `*_bak`
            // и поднимаем свежую на её месте.
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

    /// путь к файлу бд.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// собирает [`TrackVirtual`] из строки таблицы `tracks`, дотягивая список
    /// артистов.
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
                duration_sec: row.duration as u64,
                sample_rate: 0,
                bitrate: 0,
                cover_art: row.cover_art.map(PathBuf::from),
            }),
        };
        Ok(TrackVirtual::new(row.id, PathBuf::from(row.path), metadata))
    }

    /// загружает упорядоченные треки плейлиста по его id.
    fn load_playlist_tracks(&self, playlist_id: i64) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.title, t.duration, t.cover_art, t.path \
             FROM playlist_tracks pt JOIN tracks t ON t.hash = pt.song_hash \
             WHERE pt.playlist_id = ?1 ORDER BY pt.position",
        )?;
        let rows = stmt
            .query_map([playlist_id], TrackRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut tracks = Vec::with_capacity(rows.len());
        for row in rows {
            tracks.push(self.build_track(row)?);
        }
        Ok(tracks)
    }

    /// собирает полный [`Playlist`] (с треками и обложкой) из строки `playlists`.
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

    /// все плейлисты (полностью, с треками), упорядоченные по имени.
    pub fn list_playlists(&self) -> Result<Vec<Playlist>, Box<dyn Error>> {
        self.find_playlist(None, None)
    }

    /// выборка плейлистов по параметрам: `name` — подстрока без учёта регистра,
    /// `id` — точное совпадение. Без фильтров возвращает все плейлисты.
    pub fn find_playlist(
        &self,
        name: Option<String>,
        id: Option<i64>,
    ) -> Result<Vec<Playlist>, Box<dyn Error>> {
        let mut sql =
            String::from("SELECT id, name, cover_art, created_at, updated_at FROM playlists");
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
        if !conds.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conds.join(" AND "));
        }
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

    /// выборка песен из общего пула по параметрам. `name` и `artist` ищутся как
    /// подстроки без учёта регистра, `id` и `hash` — точное совпадение. Без
    /// фильтров возвращает всю библиотеку.
    pub fn find_track(
        &self,
        name: Option<String>,
        artist: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    ) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        let mut sql = String::from(
            "SELECT DISTINCT t.id, t.title, t.duration, t.cover_art, t.path FROM tracks t",
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
            .query_map(params_from_iter(vals), TrackRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(|row| self.build_track(row)).collect()
    }

    /// плейлист из всего пула песен библиотеки (все треки таблицы `tracks`).
    pub fn pool_playlist(&self) -> Result<Playlist, Box<dyn Error>> {
        let tracks = self.find_track(None, None, None, None)?;
        Ok(Playlist::from_tracks(tracks))
    }
}

impl Indexator for Db {
    fn save_track(&self, track: TrackVirtual) -> Result<(), Box<dyn Error>> {
        upsert_track(&self.conn, &track)?;
        Ok(())
    }

    /// `key` — хеш песни.
    fn load_track(&self, key: String) -> Result<TrackVirtual, Box<dyn Error>> {
        let row = self.conn.query_row(
            "SELECT id, title, duration, cover_art, path FROM tracks WHERE hash = ?1",
            [&key],
            TrackRow::from_row,
        )?;
        self.build_track(row)
    }

    fn save_playlist(&self, playlist: Playlist) -> Result<(), Box<dyn Error>> {
        let name = playlist
            .get_name()
            .ok_or("cannot save an anonymous playlist (it has no name)")?;

        let cover = playlist
            .get_cover_art()
            .map(|p| p.to_string_lossy().into_owned());
        let created = playlist.get_created_at();
        let updated = playlist.get_updated_at();

        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO playlists (name, cover_art, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(name) DO UPDATE SET \
                cover_art = excluded.cover_art, \
                created_at = excluded.created_at, \
                updated_at = excluded.updated_at",
            params![
                name,
                cover,
                created.map(|t| t as i64),
                updated.map(|t| t as i64)
            ],
        )?;
        let playlist_id: i64 =
            tx.query_row("SELECT id FROM playlists WHERE name = ?1", [&name], |r| {
                r.get(0)
            })?;

        // переписываем упорядоченную привязку треков к плейлисту.
        tx.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id],
        )?;
        for (pos, track) in playlist.tracks().iter().enumerate() {
            let hash = upsert_track(&tx, track)?;
            tx.execute(
                "INSERT INTO playlist_tracks (playlist_id, song_hash, position) \
                 VALUES (?1, ?2, ?3)",
                params![playlist_id, hash, pos as i64],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// `name` — имя плейлиста; загружает его вместе с упорядоченными треками.
    fn load_playlist(&self, name: String) -> Result<Playlist, Box<dyn Error>> {
        let (id, cover, created, updated): (i64, Option<String>, Option<i64>, Option<i64>) =
            self.conn.query_row(
                "SELECT id, cover_art, created_at, updated_at FROM playlists WHERE name = ?1",
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

    /// случайный хеш, генерируемый один раз при добавлении песни. Он не зависит
    /// от содержимого файла, поэтому редактирование метаданных трека не меняет
    /// хеш и не ломает ссылки плейлистов.
    /// TODO: для переноса между устройствами позже можно добавить контентный
    /// отпечаток (например, по аудио-сэмплам) отдельным полем.
    fn hash(_track: &TrackVirtual) -> String {
        random_hash()
    }

    fn hash_exist(&self, hash: String) -> bool {
        self.conn
            .query_row("SELECT 1 FROM tracks WHERE hash = ?1", [&hash], |_| Ok(()))
            .is_ok()
    }
}

/// сырые столбцы `tracks`, общие для всех запросов, возвращающих треки.
struct TrackRow {
    id: i64,
    title: String,
    duration: i64,
    cover_art: Option<String>,
    path: String,
}

impl TrackRow {
    fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            title: r.get(1)?,
            duration: r.get(2)?,
            cover_art: r.get(3)?,
            path: r.get(4)?,
        })
    }
}

/// вставляет/обновляет одну строку трека (и его артистов) на данном соединении
/// или транзакции, возвращая хранимый хеш. Уже известный трек (та же `path`)
/// сохраняет свой исходный хеш, чтобы ссылки плейлистов оставались валидными
/// даже после правки его метаданных.
fn upsert_track(conn: &Connection, track: &TrackVirtual) -> Result<String, Box<dyn Error>> {
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

    // на новый трек попадёт этот случайный хеш; при конфликте по `path`
    // существующий хеш не трогаем.
    let new_hash = random_hash();
    conn.execute(
        "INSERT INTO tracks (hash, title, duration, cover_art, path, source_type) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(path) DO UPDATE SET \
            title = excluded.title, duration = excluded.duration, \
            cover_art = excluded.cover_art, source_type = excluded.source_type",
        params![new_hash, meta.title, duration, cover, path_s, source],
    )?;

    let (id, hash): (i64, String) = conn.query_row(
        "SELECT id, hash FROM tracks WHERE path = ?1",
        [&path_s],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    // полностью переписываем список артистов трека.
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

    Ok(hash)
}

/// метаданные для записи: сперва то, что трек уже держит, затем повторное
/// чтение файла, и наконец заголовок из имени файла — чтобы трек без тегов всё
/// равно можно было сохранить.
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

/// метка типа источника трека для столбца `source_type`.
fn source_label(track: &TrackVirtual) -> &'static str {
    if track.index_id().is_some() {
        "index"
    } else if track.get_path().is_some() {
        "file"
    } else {
        "out"
    }
}

/// 128-битный случайный хеш в hex.
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

    /// создаёт реальный файл (чтобы у трека была существующая `path`) и трек с
    /// явными метаданными — без декодирования аудио.
    fn fixture_track(dir: &Path, file: &str, title: &str, artist: &str) -> TrackVirtual {
        let path = dir.join(file);
        fs::write(&path, format!("dummy-{file}")).unwrap();
        let metadata = TrackMetadata {
            title: title.into(),
            artist: vec![artist.into()],
            params: Some(TrackMetadataParams {
                duration_sec: 180,
                sample_rate: 44_100,
                bitrate: 320,
                cover_art: None,
            }),
        };
        TrackVirtual::new(0, path, metadata)
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
    fn backs_up_invalid_structure() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(DB_FILE_NAME);
        // не sqlite-файл -> неверная структура.
        fs::write(&path, b"this is definitely not a sqlite file").unwrap();

        let db = Db::init(path.clone()).unwrap();
        // старый файл отодвинут в *_bak, новая бд развёрнута и пуста.
        assert!(path.with_file_name(format!("{DB_FILE_NAME}_bak")).exists());
        assert!(db.hash_exist("whatever".into()) == false);
    }

    #[test]
    fn saves_and_loads_a_track() {
        let dir = tempdir().unwrap();
        let db = Db::init(dir.path().join(DB_FILE_NAME)).unwrap();

        let track = fixture_track(dir.path(), "a.mp3", "Hello", "World");
        db.save_track(track).unwrap();

        // достаём хеш напрямую из бд и грузим трек по нему.
        let hash: String = db
            .conn
            .query_row("SELECT hash FROM tracks LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(db.hash_exist(hash.clone()));

        let loaded = db.load_track(hash).unwrap();
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

        // та же path, изменённое название — хеш должен остаться прежним.
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

        // весь пул.
        assert_eq!(db.find_track(None, None, None, None).unwrap().len(), 3);
        // по названию (подстрока).
        assert_eq!(
            db.find_track(Some("hello".into()), None, None, None)
                .unwrap()
                .len(),
            2
        );
        // по артисту.
        assert_eq!(
            db.find_track(None, Some("world".into()), None, None)
                .unwrap()
                .len(),
            2
        );
        // комбинация name + artist.
        assert_eq!(
            db.find_track(Some("hello".into()), Some("other".into()), None, None)
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

        // получить все.
        let all = db.list_playlists().unwrap();
        assert_eq!(all.len(), 2);
        // упорядочены по имени: jazz, rock.
        assert_eq!(all[0].get_name().as_deref(), Some("jazz"));
        assert_eq!(all[1].get_name().as_deref(), Some("rock"));

        // выборка по параметру + обложка едет с плейлистом.
        let found = db.find_playlist(Some("rock".into()), None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].get_cover_art(),
            Some(dir.path().join("rock.png").as_path())
        );
        // таймштампы тоже едут с плейлистом.
        assert_eq!(found[0].get_created_at(), Some(1_000));
        assert_eq!(found[0].get_updated_at(), Some(2_000));
        // у jazz обложки и таймштампов нет.
        let jazz = &db.find_playlist(Some("jazz".into()), None).unwrap()[0];
        assert!(jazz.get_cover_art().is_none());
        assert!(jazz.get_created_at().is_none());
        assert!(jazz.get_updated_at().is_none());
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

        let loaded = db.load_playlist("mix".into()).unwrap();
        assert_eq!(loaded.get_count(), 2);
        let titles: Vec<_> = loaded
            .tracks()
            .iter()
            .map(|t| t.get_metadata().unwrap().title.clone())
            .collect();
        assert_eq!(titles, vec!["First".to_string(), "Second".to_string()]);
    }
}
