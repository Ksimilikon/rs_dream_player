//! Заглушка медиа-интеграции для платформ без неё (macOS/iOS/Android/прочие):
//! команд наружу не порождает, обновления метаданных просто поглощает, пока
//! приложение живо.

use std::sync::mpsc::{Receiver, Sender};

use super::{MediaCommand, MediaMeta};

pub fn spawn(_cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) {
    std::thread::spawn(move || {
        // держим приёмник живым: обновления метаданных отбрасываем.
        while meta_rx.recv().is_ok() {}
    });
}
