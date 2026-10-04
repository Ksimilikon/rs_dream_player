//! системная медиа-интеграция (перенесена из удалённого крейта `dbus`): MPRIS на
//! Linux, SMTC на Windows, заглушка на остальных. Работает поверх FFI движка —
//! команды системных клавиш приходят в приложение, метаданные текущего трека
//! уходят в систему.
//!
//! фасад [`Media`] владеет двумя каналами: `cmd_rx` — команды от ОС (Next/…),
//! `meta_tx` — метаданные в ОС. Бэкенд крутится на своём потоке.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod fallback;

/// команда от системных медиа-клавиш.
pub enum MediaCommand {
    Next,
    Prev,
    PlayPause,
    Play,
    Pause,
    Stop,
}

/// метаданные текущего трека для системного транспорта.
#[derive(Default, Clone)]
pub struct MediaMeta {
    pub title: String,
    pub artists: Vec<String>,
    /// путь к файлу обложки на диске (отдаётся как `file://`-URL).
    pub art_path: Option<PathBuf>,
}

/// фасад медиа-интеграции на стороне приложения.
pub struct Media {
    cmd_rx: Receiver<MediaCommand>,
    meta_tx: Sender<MediaMeta>,
}

impl Media {
    /// запускает бэкенд на отдельном потоке. Если интеграция недоступна (нет
    /// шины/ошибка) — поток тихо завершится, фасад останется рабочим «вхолостую».
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

    /// неблокирующе забирает очередную команду от ОС (если есть).
    pub fn poll(&self) -> Option<MediaCommand> {
        self.cmd_rx.try_recv().ok()
    }

    /// публикует метаданные текущего трека в системный транспорт.
    pub fn set_metadata(&self, meta: MediaMeta) {
        let _ = self.meta_tx.send(meta);
    }
}
