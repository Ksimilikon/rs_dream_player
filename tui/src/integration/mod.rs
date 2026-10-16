//! system media integration (moved from the removed `dbus` crate): MPRIS on
//! Linux, SMTC on Windows, a stub elsewhere. Works on top of the engine FFI —
//! system key commands come into the app, current track metadata goes out to
//! the system.
//!
//! the [`Media`] facade owns two channels: `cmd_rx` — commands from the OS
//! (Next/…), `meta_tx` — metadata to the OS. The backend runs on its own
//! thread.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod fallback;

/// a command from the system media keys.
pub enum MediaCommand {
    Next,
    Prev,
    PlayPause,
    Play,
    Pause,
    Stop,
}

/// current track metadata for the system transport.
#[derive(Default, Clone)]
pub struct MediaMeta {
    pub title: String,
    pub artists: Vec<String>,
    /// path to the cover file on disk (given as a `file://` URL).
    pub art_path: Option<PathBuf>,
}

/// the app-side media integration facade.
pub struct Media {
    cmd_rx: Receiver<MediaCommand>,
    meta_tx: Sender<MediaMeta>,
}

impl Media {
    /// starts the backend on a separate thread. If the integration is
    /// unavailable (no bus / error), the thread exits quietly and the facade
    /// keeps working "idle".
    pub fn start() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<MediaCommand>();
        let (meta_tx, meta_rx) = mpsc::channel::<MediaMeta>();

        #[cfg(target_os = "linux")]
        linux::spawn(cmd_tx, meta_rx);
        #[cfg(target_os = "windows")]
        windows::spawn(cmd_tx, meta_rx);
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        fallback::spawn(cmd_tx, meta_rx);

        Self { cmd_rx, meta_tx }
    }

    /// takes the next command from the OS without blocking (if any).
    pub fn poll(&self) -> Option<MediaCommand> {
        self.cmd_rx.try_recv().ok()
    }

    /// publishes current track metadata to the system transport.
    pub fn set_metadata(&self, meta: MediaMeta) {
        let _ = self.meta_tx.send(meta);
    }
}
