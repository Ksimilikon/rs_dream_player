//! media integration stub for platforms without one (macOS/iOS/Android/
//! others): produces no commands and simply swallows metadata updates while
//! the app is alive.

use std::sync::mpsc::{Receiver, Sender};

use super::{MediaCommand, MediaMeta};

pub fn spawn(_cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) {
    std::thread::spawn(move || {
        // keep the receiver alive: metadata updates are discarded.
        while meta_rx.recv().is_ok() {}
    });
}
