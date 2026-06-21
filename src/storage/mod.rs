pub mod config;
pub mod index;
pub mod index_autocreate;

use std::error::Error;

use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};

use crate::storage::{config::Config, index::Index};

/// owns the sqlite-backed [`Index`] and the persisted [`Config`].
/// Playlists are not kept fully in RAM - they're loaded/saved by name.
pub struct Storage {
    index: Index,
    cfg: Config,
}

impl Storage {
    /// opens the index at the platform default location, creating/repairing it
    /// as needed, then loads the persisted config (falling back to defaults).
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let defaults = Config::defaults()?;
        let index = Index::open(defaults.db_file())?;
        let cfg = index.load_config(defaults)?;
        Ok(Self { index, cfg })
    }

    /// opens an index at an explicit path — handy for tests and tooling.
    pub fn open_at(db_file: std::path::PathBuf, defaults: Config) -> Result<Self, Box<dyn Error>> {
        let index = Index::open(db_file)?;
        let cfg = index.load_config(defaults)?;
        Ok(Self { index, cfg })
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.cfg
    }

    /// persists the current config into the index.
    pub fn save_config(&mut self) -> Result<(), Box<dyn Error>> {
        self.index.save_config(&self.cfg)
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn index_track(&self, track: &TrackVirtual) -> Result<i64, Box<dyn Error>> {
        self.index.index_track(track)
    }

    pub fn save_playlist(&self, playlist: &Playlist) -> Result<i64, Box<dyn Error>> {
        self.index.save_playlist(playlist)
    }

    pub fn find_track(
        &self,
        name: Option<String>,
        artist: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    ) -> Result<Vec<TrackVirtual>, Box<dyn Error>> {
        self.index.find_track(name, artist, id, hash)
    }

    pub fn find_playlist(
        &self,
        name: Option<String>,
        id: Option<i64>,
        hash: Option<String>,
    ) -> Result<Vec<Playlist>, Box<dyn Error>> {
        self.index.find_playlist(name, id, hash)
    }

    pub fn list_playlists(&self) -> Result<Vec<String>, Box<dyn Error>> {
        self.index.list_playlists()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use audio_structs::{
        playlist::Playlist,
        track_metadata::{TrackMetadata, TrackMetadataParams},
        track_virtual::TrackVirtual,
    };
    use tempfile::tempdir;

    use super::config::Config;
    use super::index::Index;

    /// a config that points everything inside `dir`, avoiding any dependency on
    /// platform directories during tests.
    fn test_config(dir: &std::path::Path) -> Config {
        Config {
            dir_music: dir.join("music"),
            dir_config: dir.join("config"),
            dir_arts: dir.join("arts"),
            dir_db: dir.to_path_buf(),
            volume: 1.0,
            last_playlist: None,
            last_track: None,
        }
    }

    /// writes a dummy file so the indexer has a real, hashable path to point at,
    /// then builds a track with explicit metadata (so no audio decoding needed).
    fn fixture_track(dir: &std::path::Path, file: &str, title: &str, artist: &str) -> TrackVirtual {
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
    fn deploys_full_structure_on_open() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("index.db");
        let _index = Index::open(&db).unwrap();
        assert!(db.exists());

        // verify every expected table physically exists and the schema version
        // was stamped, by inspecting the file with an independent connection.
        let conn = rusqlite::Connection::open(&db).unwrap();
        for table in [
            "tracks",
            "artists",
            "track_artists",
            "playlists",
            "playlist_tracks",
            "settings",
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

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, super::index_autocreate::SCHEMA_VERSION);
    }

    #[test]
    fn recreates_corrupted_database() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("index.db");
        fs::write(&db, b"this is definitely not a sqlite file").unwrap();

        // opening must succeed by moving the bad file aside and starting fresh.
        let index = Index::open(&db).unwrap();
        assert!(index.find_track(None, None, None, None).unwrap().is_empty());
        assert!(db.with_extension("corrupt").exists());
    }

    #[test]
    fn indexes_and_finds_a_track() {
        let dir = tempdir().unwrap();
        let index = Index::open(dir.path().join("index.db")).unwrap();

        let track = fixture_track(dir.path(), "a.mp3", "Hello", "World");
        let id = index.index_track(&track).unwrap();
        assert!(id > 0);

        // re-indexing the same path updates rather than duplicating.
        let again = index.index_track(&track).unwrap();
        assert_eq!(id, again);

        let by_name = index.find_track(Some("hell".into()), None, None, None).unwrap();
        assert_eq!(by_name.len(), 1);
        let meta = by_name[0].get_metadata().unwrap();
        assert_eq!(meta.title, "Hello");
        assert_eq!(meta.artist, vec!["World".to_string()]);

        let by_artist = index.find_track(None, Some("world".into()), None, None).unwrap();
        assert_eq!(by_artist.len(), 1);
        assert_eq!(index.find_track(None, None, Some(id), None).unwrap().len(), 1);
    }

    #[test]
    fn saves_and_loads_a_playlist() {
        let dir = tempdir().unwrap();
        let index = Index::open(dir.path().join("index.db")).unwrap();

        let tracks = vec![
            fixture_track(dir.path(), "1.mp3", "First", "A"),
            fixture_track(dir.path(), "2.mp3", "Second", "B"),
        ];
        let mut playlist = Playlist::from_tracks(tracks);
        playlist.set_name("mix".into());
        index.save_playlist(&playlist).unwrap();

        assert_eq!(index.list_playlists().unwrap(), vec!["mix".to_string()]);

        let found = index.find_playlist(Some("mix".into()), None, None).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].get_count(), 2);
        // order is preserved.
        let titles: Vec<_> = found[0]
            .tracks()
            .iter()
            .map(|t| t.get_metadata().unwrap().title.clone())
            .collect();
        assert_eq!(titles, vec!["First".to_string(), "Second".to_string()]);
    }

    #[test]
    fn config_roundtrips_through_settings() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("index.db");

        {
            let index = Index::open(&db).unwrap();
            let mut cfg = test_config(dir.path());
            cfg.volume = 0.42;
            cfg.last_playlist = Some("mix".into());
            cfg.last_track = Some(3);
            index.save_config(&cfg).unwrap();
        }

        // re-open and reload to prove persistence across connections.
        let index = Index::open(&db).unwrap();
        let loaded = index.load_config(test_config(dir.path())).unwrap();
        assert_eq!(loaded.volume, 0.42);
        assert_eq!(loaded.last_playlist, Some("mix".to_string()));
        assert_eq!(loaded.last_track, Some(3));
        assert_eq!(loaded.dir_music, PathBuf::from(dir.path().join("music")));
    }
}
