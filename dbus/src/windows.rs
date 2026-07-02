#![cfg(all(target_os = "windows", feature = "with_tui"))]
//! Windows-реализация медиа-транспорта поверх системного SMTC (System Media
//! Transport Controls) через кроссплатформенную библиотеку `souvlaki`.
//!
//! ВАЖНО — РАБОТАЕТ ТОЛЬКО С TUI: этот бэкенд опирается на окно консоли
//! (`GetConsoleWindow`) как на опору для SMTC, поэтому без TUI он
//! неработоспособен. Модуль и его зависимости (`souvlaki`, `windows`)
//! компилируются только под фичей `with_tui` (см. `lib.rs` и `Cargo.toml`); при
//! простой сборке под Windows без флага вместо него берётся `fallback`.
//!
//! ВНИМАНИЕ: не проверено на живой системе (разработка идёт на Linux). Проект —
//! это ядро плеера; полноценный фронтенд появится позже, и вместе с ним —
//! настоящее окно с насосом сообщений, чей хендл и нужно будет отдавать SMTC
//! вместо консольного. Обновление метаданных и проброс команд написаны, но
//! приём событий от системы зависит от прокачки оконных сообщений и в
//! TUI-режиме может не срабатывать. Это ожидаемо и заменится на уровне фронта.

use std::error::Error;
use std::ffi::c_void;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};
// абсолютный путь к внешнему крейту: модуль тоже называется `windows`.
use ::windows::Win32::System::Console::GetConsoleWindow;

use super::*;

/// имя/идентификатор плеера для SMTC.
const DISPLAY_NAME: &str = "dream_player";

impl DBus {
    /// поднимает SMTC-интеграцию: транслирует команды системы во внутренний
    /// канал и качает метаданные текущего трека из `rx` в системный транспорт.
    /// Блокирует поток (как и linux-версия).
    pub fn run(self) -> Result<(), Box<dyn Error>> {
        let DBus { tx, rx } = self;

        // SMTC на Windows требует валидный HWND. За неимением собственного окна
        // (TUI-ядро) берём окно консоли; при отсутствии — None (souvlaki может
        // отказать, но это лишь заглушка до появления фронтенда).
        let hwnd = console_hwnd();

        let config = PlatformConfig {
            dbus_name: DISPLAY_NAME,
            display_name: DISPLAY_NAME,
            hwnd,
        };

        let mut controls =
            MediaControls::new(config).map_err(|e| boxed(format!("souvlaki init: {e:?}")))?;

        // системные кнопки (Next/Prev/Play/Pause/Stop) -> внутренние команды ядра.
        controls
            .attach(move |event: MediaControlEvent| {
                if let Some(cmd) = map_event(event) {
                    let _ = tx.send(cmd);
                }
            })
            .map_err(|e| boxed(format!("souvlaki attach: {e:?}")))?;

        // начинаем как «играет» — статус уточнит фронтенд, когда появится.
        let _ = controls.set_playback(MediaPlayback::Playing { progress: None });

        // качаем обновления метаданных в системный транспорт, пока ядро живо.
        while let Ok(data) = rx.recv() {
            let artist = data.artists.join(", ");
            let meta = MediaMetadata {
                title: Some(&data.title),
                artist: (!artist.is_empty()).then_some(artist.as_str()),
                ..Default::default()
            };
            let _ = controls.set_metadata(meta);
        }

        Ok(())
    }
}

/// хендл консольного окна как `*mut c_void` для souvlaki (`None`, если недоступно).
fn console_hwnd() -> Option<*mut c_void> {
    let hwnd = unsafe { GetConsoleWindow() };
    (!hwnd.0.is_null()).then_some(hwnd.0)
}

/// сопоставляет событие SMTC внутренней команде плеера (лишние — игнорируем).
fn map_event(event: MediaControlEvent) -> Option<DBusEvent> {
    match event {
        MediaControlEvent::Play => Some(DBusEvent::Play),
        MediaControlEvent::Pause => Some(DBusEvent::Pause),
        MediaControlEvent::Toggle => Some(DBusEvent::PlayPause),
        MediaControlEvent::Next => Some(DBusEvent::Next),
        MediaControlEvent::Previous => Some(DBusEvent::Prev),
        MediaControlEvent::Stop => Some(DBusEvent::Stop),
        _ => None,
    }
}

/// оборачивает строку в `Box<dyn Error>` (у souvlaki::Error нет гарантии
/// реализации `std::error::Error`).
fn boxed(msg: String) -> Box<dyn Error> {
    msg.into()
}
