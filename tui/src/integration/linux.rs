//! Linux backend: MPRIS via zbus (moved from the `dbus` crate). DE commands
//! (Next/Prev/Play/…) go to `cmd_tx`, metadata from `meta_rx` is published as
//! properties with a `PropertiesChanged` emit.

use std::collections::HashMap;
use std::error::Error;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use zbus::{
    connection, interface,
    zvariant::{ObjectPath, Value},
};

use super::{MediaCommand, MediaMeta};

const DBUS_NAME: &str = "org.mpris.MediaPlayer2.dream_player";
const DBUS_PATH: &str = "/org/mpris/MediaPlayer2";

/// how often metadata updates are polled (tokio handles background dbus calls
/// between polls while the timer is pending).
const POLL: Duration = Duration::from_millis(150);

/// starts the MPRIS server on ONE thread: brings up a single-threaded tokio
/// runtime that hosts both zbus (feature `tokio`, no async-io) and our
/// metadata loop — no extra helper threads. Errors (no bus, name taken) end
/// the thread silently.
pub fn spawn(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) {
    std::thread::spawn(move || {
        let _ = run(cmd_tx, meta_rx);
    });
}

fn run(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) -> Result<(), Box<dyn Error>> {
    // the shared source of truth: written by this loop, read by the interface
    // getters.
    let data = Arc::new(Mutex::new(MediaMeta::default()));

    let player = Player {
        commands: Mutex::new(cmd_tx),
        data: data.clone(),
    };

    // current_thread: the runtime spawns no worker threads — everything runs on
    // this thread.
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
            // yield to the runtime: while we wait for the tick, tokio handles
            // incoming dbus calls (media keys) on this same thread.
            ticker.tick().await;

            // drain pending metadata updates without blocking.
            let mut changed = false;
            loop {
                match meta_rx.try_recv() {
                    Ok(new) => {
                        *data.lock().unwrap_or_else(PoisonError::into_inner) = new;
                        changed = true;
                    }
                    Err(TryRecvError::Empty) => break,
                    // the application exited and closed the channel — leave.
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
        let data = self.data.lock().unwrap_or_else(PoisonError::into_inner);
        let mut m = HashMap::new();
        // trackid is required and must be a D-Bus ObjectPath.
        m.insert(
            "mpris:trackid".to_string(),
            // a constant valid path: no runtime validation needed.
            Value::ObjectPath(ObjectPath::from_static_str_unchecked("/org/mpris/MediaPlayer2/Track/0")),
        );
        m.insert("xesam:title".to_string(), Value::from(data.title.clone()));
        m.insert("xesam:artist".to_string(), Value::from(data.artists.clone()));
        // the cover is given as a file:// URL; without one the key isn't added
        // (the map is rebuilt on every metadata_changed, so the previous cover
        // is dropped).
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
