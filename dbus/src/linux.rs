#![cfg(target_os = "linux")]
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::Sender;

use zbus::{
    connection, interface,
    zvariant::{ObjectPath, Value},
};

use super::*;

pub const DBUS_NAME: &str = "org.mpris.MediaPlayer2.dream_player";
pub const DBUS_PATH: &str = "/org/mpris/MediaPlayer2";

impl DBus {
    /// блокирует поток: поднимает соединение и качает обновления метаданных
    /// из `rx` в свойства, эмитя `PropertiesChanged`. Входящие команды от DE
    /// обслуживает внутренний executor zbus на своём потоке.
    pub fn run(self) -> zbus::Result<()> {
        let DBus { tx, rx } = self;
        // общий источник правды: пишет этот цикл, читают геттеры интерфейса.
        let data = Arc::new(Mutex::new(DBusData::default()));

        let player = DBusPlayer {
            commands: Mutex::new(tx),
            data: data.clone(),
        };

        let conn = zbus::block_on(async {
            connection::Builder::session()?
                .name(DBUS_NAME)?
                .serve_at(DBUS_PATH, player)?
                .serve_at(DBUS_PATH, DBusRoot::default())?
                .build()
                .await
        })?;

        let player_ref =
            zbus::block_on(conn.object_server().interface::<_, DBusPlayer>(DBUS_PATH))?;

        while let Ok(new) = rx.recv() {
            *data.lock().unwrap() = new;
            zbus::block_on(async {
                let p = player_ref.get().await;
                p.metadata_changed(player_ref.signal_emitter()).await
            })?;
        }

        drop(conn);
        Ok(())
    }
}

pub struct DBusPlayer {
    commands: Mutex<Sender<DBusEvent>>,
    data: Arc<Mutex<DBusData>>,
}

impl DBusPlayer {
    fn send(&self, cmd: DBusEvent) {
        if let Ok(tx) = self.commands.lock() {
            let _ = tx.send(cmd);
        }
    }
}

pub struct DBusRoot {
    can_quit: bool,
    can_raise: bool,
    has_track_list: bool,
    identity: String,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl DBusRoot {
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

impl Default for DBusRoot {
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
impl DBusPlayer {
    fn play(&self) {
        self.send(DBusEvent::Play);
    }
    fn pause(&self) {
        self.send(DBusEvent::Pause);
    }
    fn stop(&self) {
        self.send(DBusEvent::Stop);
    }
    fn next(&self) {
        self.send(DBusEvent::Next);
    }
    fn previous(&self) {
        self.send(DBusEvent::Prev);
    }
    fn play_pause(&self) {
        self.send(DBusEvent::PlayPause);
    }
    fn seek(&self, _offset: i64) {}

    // --- Свойства (Properties) ---
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        "Playing"
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
        m.insert("xesam:artist".to_string(), Value::from(data.artists.clone()));
        if let Some(bytes) = &data.art {
            if let Some(url) = write_art_tmp(bytes) {
                m.insert("mpris:artUrl".to_string(), Value::from(url));
            }
        }

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
        false
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }
}

/// MPRIS принимает обложку только как URL, поэтому сырые байты пишем во
/// временный файл и отдаём `file://`-ссылку.
fn write_art_tmp(bytes: &[u8]) -> Option<String> {
    let path = std::env::temp_dir().join("dream_player_art");
    let mut f = std::fs::File::create(&path).ok()?;
    f.write_all(bytes).ok()?;
    Some(format!("file://{}", path.display()))
}
