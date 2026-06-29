use std::sync::mpsc::{Receiver, Sender};

pub mod linux;
pub mod traits;

/// команды, которые DE присылает плееру через MPRIS.
pub enum DBusEvent {
    Next,
    Prev,
    PlayPause,
    Play,
    Pause,
    Stop,
}

/// фасад dbus-слоя: владеет каналами и потоком с соединением.
/// `tx` — команды наружу (в ядро), `rx` — обновления метаданных внутрь.
pub struct DBus {
    tx: Sender<DBusEvent>,
    rx: Receiver<DBusData>,
}

/// единственный источник правды по текущему треку — ровно три поля,
/// которые показывает MPRIS.
#[derive(Debug, Default, Clone)]
pub struct DBusData {
    pub title: String,
    pub artists: Vec<String>,
    pub art: Option<Vec<u8>>,
}

impl DBus {
    pub fn new(tx: Sender<DBusEvent>, rx: Receiver<DBusData>) -> Self {
        Self { tx, rx }
    }
}
