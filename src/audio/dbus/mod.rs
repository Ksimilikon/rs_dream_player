use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
};

use zbus::{
    blocking::connection,
    interface,
    zvariant::{ObjectPath, Value},
};

use crate::audio::metadata::Metadata;

pub mod linux;

pub const DBUS_NAME: &str = "org.mpris.MediaPlayer2.dream_player";
pub const DBUS_PATH: &str = "/org/mpris/MediaPlayer2";
#[derive(Default)]
pub struct DbusData {
    pub title: String,
    pub artist: Vec<String>,
    pub cover_art: Option<Vec<u8>>,
}
pub struct DbusPlayer {
    pub data: Arc<Mutex<DbusData>>,
}
pub struct DbusRoot {
    can_quit: bool,
    can_raise: bool,
    has_track_list: bool,
    identity: String,
}
pub struct Dbus {}

impl Dbus {
    pub fn start_server(
        mut rx: tokio::sync::mpsc::Receiver<Metadata>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let t = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let data = Arc::new(Mutex::new(DbusData::default()));
                let dbus_player = DbusPlayer { data: data.clone() };
                let root = DbusRoot::default();
                // WARN: unwrap
                let _conn = zbus::connection::Builder::session()
                    .unwrap()
                    .name(DBUS_NAME)
                    .unwrap()
                    .serve_at(DBUS_PATH, root)
                    .unwrap()
                    .serve_at(DBUS_PATH, dbus_player)
                    .unwrap()
                    .build()
                    .await
                    .unwrap();
                let interface_ref = _conn
                    .object_server()
                    .interface::<_, DbusPlayer>(DBUS_PATH)
                    .await
                    .unwrap();

                // WARN: block
                loop {
                    if let Some(meta) = rx.recv().await {
                        println!("send title {}", meta.title);
                        {
                            let mut guard = data.lock().unwrap();
                            guard.title = meta.title;
                            guard.artist = meta.artist;
                        }
                        let ctx = interface_ref.signal_emitter();
                        interface_ref
                            .get()
                            .await
                            .metadata_changed(ctx)
                            .await
                            .unwrap();
                        println!("signal post");
                    }
                }
            });
        });

        Ok(())
    }
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl DbusRoot {
    #[zbus(property)]
    fn identity(&self) -> &str {
        &self.identity
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        self.can_quit
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        self.can_raise
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        self.has_track_list
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".to_string(), "http".to_string(), "https".to_string()]
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![
            "audio/mpeg".to_string(),
            "audio/ogg".to_string(),
            "audio/flac".to_string(),
        ]
    }
}
impl Default for DbusRoot {
    fn default() -> Self {
        Self {
            can_quit: true,
            can_raise: true,
            has_track_list: false,
            identity: String::from("dream_player"),
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl DbusPlayer {
    fn play(&self) {
        println!("Команда: Play");
    }
    fn pause(&self) {
        println!("Команда: Pause");
    }
    fn stop(&self) {
        println!("Команда: Stop");
    }
    fn next(&self) {
        println!("Команда: Next");
    }

    fn previous(&self) {
        println!("Команда: Previous");
    }

    // Это самая часто используемая команда в виджетах Linux
    fn play_pause(&self) {
        println!("Команда: PlayPause");
        // Логика переключения здесь
    }

    fn seek(&self, offset: i64) {
        println!("Команда: Seek на {} мкс", offset);
    }

    // --- Свойства (Properties) ---
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        "Playing" // Или "Paused", "Stopped"
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, Value<'_>> {
        let data = self.data.lock().unwrap();
        let mut m = HashMap::new();

        // trackid обязателен и должен быть в формате D-Bus ObjectPath
        m.insert(
            "mpris:trackid".to_string(),
            Value::ObjectPath(ObjectPath::try_from("/org/mpris/MediaPlayer2/Track/0").unwrap()),
        );

        m.insert("xesam:title".to_string(), Value::from(data.title.clone()));

        m.insert("xesam:artist".to_string(), Value::from(data.artist.clone()));

        m
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
}
