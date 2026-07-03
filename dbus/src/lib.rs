use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

pub mod traits;

// платформенная реализация MPRIS-подобного транспорта. Каждая предоставляет
// `impl DBus { pub fn run(self) -> Result<(), Box<dyn Error>> }`.
// linux — zbus (не зависит от TUI);
// windows — souvlaki (SMTC), но ТОЛЬКО в TUI-сборке (`with_tui`), т.к. опирается
//   на окно консоли (`GetConsoleWindow`); без TUI решение неработоспособно;
// всё остальное (Windows без `with_tui`, macos/ios/android/прочее) — пустая
//   заглушка `fallback` (проект — ядро; нативная интеграция придёт с фронтендом).
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(all(target_os = "windows", feature = "with_tui"))]
pub mod windows;
#[cfg(not(any(
    target_os = "linux",
    all(target_os = "windows", feature = "with_tui")
)))]
pub mod fallback;

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
    /// путь к файлу обложки на диске (covers/<id>.<ext>), если у трека она есть.
    /// Передаём именно путь, а не байты: обложка уже лежит файлом, а `file://`-URL
    /// напрямую годится и для MPRIS (linux), и для SMTC (windows). `None` — у трека
    /// обложки нет, тогда картинку не шлём, но обложку прошлого трека затираем.
    pub art_path: Option<PathBuf>,
}

impl DBus {
    pub fn new(tx: Sender<DBusEvent>, rx: Receiver<DBusData>) -> Self {
        Self { tx, rx }
    }
}
