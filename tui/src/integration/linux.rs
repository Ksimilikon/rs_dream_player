//! Linux-бэкенд: MPRIS через zbus (перенесён из крейта `dbus`). Команды DE
//! (Next/Prev/Play/…) уходят в `cmd_tx`, метаданные из `meta_rx` публикуются как
//! свойства с эмитом `PropertiesChanged`.

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::{
    connection, interface,
    zvariant::{ObjectPath, Value},
};

use super::{MediaCommand, MediaMeta};

const DBUS_NAME: &str = "org.mpris.MediaPlayer2.dream_player";
const DBUS_PATH: &str = "/org/mpris/MediaPlayer2";

/// как часто опрашиваем обновления метаданных (фоновые dbus-вызовы движок tokio
/// обрабатывает между опросами, пока ждём таймер).
const POLL: Duration = Duration::from_millis(150);

/// запускает MPRIS-сервер на ОДНОМ потоке: поднимает однопоточный tokio-рантайм,
/// на нём живут и zbus (feature `tokio`, без async-io), и наш цикл метаданных —
/// никаких дополнительных вспомогательных потоков. Ошибки (нет шины, имя занято)
/// завершают поток тихо.
pub fn spawn(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) {
    std::thread::spawn(move || {
        let _ = run(cmd_tx, meta_rx);
    });
}

fn run(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) -> Result<(), Box<dyn Error>> {
    // общий источник правды: пишет этот цикл, читают геттеры интерфейса.
    let data = Arc::new(Mutex::new(MediaMeta::default()));

    let player = Player {
        commands: Mutex::new(cmd_tx),
        data: data.clone(),
    };

    // current_thread: рантайм не поднимает рабочих потоков — всё на этом потоке.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move {
        let conn = connection::Builder::session()?
            .name(DBUS_NAME)?
            .serve_at(DBUS_PATH, player)?
            .serve_at(DBUS_PATH, Root::default())?
            .build()
            .await?;
        let player_ref = conn.object_server().interface::<_, Player>(DBUS_PATH).await?;

        let mut ticker = tokio::time::interval(POLL);
        loop {
            // уступаем рантайму: пока ждём тик, tokio обрабатывает входящие
            // dbus-вызовы (медиа-клавиши) на этом же потоке.
            ticker.tick().await;

            // неблокирующе выгребаем накопившиеся обновления метаданных.
            let mut changed = false;
            loop {
                match meta_rx.try_recv() {
                    Ok(new) => {
                        *data.lock().unwrap() = new;
                        changed = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    // приложение завершилось и закрыло канал — выходим.
                    Err(TryRecvError::Disconnected) => return Ok::<(), Box<dyn Error>>(()),
                }
            }
            if changed {
                let p = player_ref.get().await;
                p.metadata_changed(player_ref.signal_emitter()).await?;
            }
        }
    })
}

struct Player {
    commands: Mutex<Sender<MediaCommand>>,
    data: Arc<Mutex<MediaMeta>>,
}

impl Player {
    fn send(&self, cmd: MediaCommand) {
        if let Ok(tx) = self.commands.lock() {
            let _ = tx.send(cmd);
        }
    }
}

struct Root {
    can_quit: bool,
    can_raise: bool,
    has_track_list: bool,
    identity: String,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
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

impl Default for Root {
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
impl Player {
    fn play(&self) {
        self.send(MediaCommand::Play);
    }
    fn pause(&self) {
        self.send(MediaCommand::Pause);
    }
    fn stop(&self) {
        self.send(MediaCommand::Stop);
    }
    fn next(&self) {
        self.send(MediaCommand::Next);
    }
    fn previous(&self) {
        self.send(MediaCommand::Prev);
    }
    fn play_pause(&self) {
        self.send(MediaCommand::PlayPause);
    }
    fn seek(&self, _offset: i64) {}

    #[zbus(property)]
    fn playback_status(&self) -> &str {
        "Playing"
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, Value<'_>> {
        let data = self.data.lock().unwrap();
        let mut m = HashMap::new();
        // trackid обязателен и должен быть в формате D-Bus ObjectPath.
        m.insert(
            "mpris:trackid".to_string(),
            Value::ObjectPath(ObjectPath::try_from("/org/mpris/MediaPlayer2/Track/0").unwrap()),
        );
        m.insert("xesam:title".to_string(), Value::from(data.title.clone()));
        m.insert("xesam:artist".to_string(), Value::from(data.artists.clone()));
        // обложку отдаём как file://-URL; если её нет — ключ не добавляем (map
        // пересобирается на каждый metadata_changed, обложка прошлого затирается).
        if let Some(path) = &data.art_path {
            m.insert(
                "mpris:artUrl".to_string(),
                Value::from(format!("file://{}", path.display())),
            );
        }
        m
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
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
}
