#![cfg(all(target_os = "windows", feature = "with_tui"))]
//! Windows-реализация медиа-транспорта поверх системного SMTC (System Media
//! Transport Controls) через кроссплатформенную библиотеку `souvlaki`.
//!
//! ПОЧЕМУ РАНЬШЕ НЕ РАБОТАЛИ СИСТЕМНЫЕ КЛАВИШИ (Next/Prev и пр.):
//! SMTC на Windows доставляет нажатия кнопок не напрямую, а через очередь
//! оконных сообщений того HWND, который отдан в `PlatformConfig`. Чтобы колбэк
//! `attach` вообще вызвался, нужны ДВА условия:
//!   1) HWND должен принадлежать НАШЕМУ процессу (souvlaki навешивает на него
//!      WinRT-обработчик, а система шлёт события в очередь потока-владельца);
//!   2) этот поток обязан КАЧАТЬ оконные сообщения (`PeekMessage`/`Dispatch`).
//!
//! Старый код брал `GetConsoleWindow()` — но консольное окно принадлежит
//! conhost/терминалу (ЧУЖОЙ процесс), поэтому доставить в него события в наш
//! поток невозможно; вдобавок цикл блокировался на `rx.recv()` и сообщения не
//! качал вовсе. Итог — метаданные уходили в систему, а кнопки молчали.
//!
//! РЕШЕНИЕ (ровно приём из примера souvlaki `print_events.rs` «media keys on the
//! command line»): создаём собственное скрытое окно в нашем процессе, отдаём его
//! HWND в SMTC и крутим насос сообщений на этом же потоке, параллельно
//! неблокирующе вычитывая обновления метаданных из `rx`.

use std::error::Error;
use std::ffi::c_void;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

// абсолютные пути к внешнему крейту: наш модуль тоже называется `windows`.
use ::windows::Media::{
    MediaPlaybackType, SystemMediaTransportControls, SystemMediaTransportControlsDisplayUpdater,
};
use ::windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use ::windows::Win32::System::LibraryLoader::GetModuleHandleW;
use ::windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use ::windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, PeekMessageW,
    RegisterClassExW, TranslateMessage, MSG, PM_REMOVE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_QUIT,
    WNDCLASSEXW,
};
use ::windows::core::w;

use super::*;

/// имя/идентификатор плеера для SMTC.
const DISPLAY_NAME: &str = "dream_player";

/// как часто крутим цикл (насос сообщений + вычитка метаданных). Нажатие кнопки
/// обрабатывается в пределах этого интервала — 50 мс незаметны на слух.
const TICK: Duration = Duration::from_millis(50);

impl DBus {
    /// поднимает SMTC-интеграцию: создаёт своё скрытое окно как опору для SMTC,
    /// транслирует нажатия системных кнопок во внутренний канал и качает
    /// метаданные текущего трека из `rx` в системный транспорт.
    /// Блокирует поток (как и linux-версия).
    pub fn run(self) -> Result<(), Box<dyn Error>> {
        let DBus { tx, rx } = self;

        // собственное скрытое окно нашего процесса — на нём и держится SMTC.
        let window = DummyWindow::new()?;

        let config = PlatformConfig {
            dbus_name: DISPLAY_NAME,
            display_name: DISPLAY_NAME,
            // HWND -> сырой указатель, как ждёт souvlaki.
            hwnd: Some(window.handle.0 as *mut c_void),
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

        // souvlaki умеет ставить обложку (через cover_url), но НЕ умеет её
        // очищать: при cover_url=None старый thumbnail остаётся висеть. Поэтому
        // берём свой доступ к тому же DisplayUpdater (GetForWindow отдаёт тот же
        // объект SMTC для нашего окна) и перед каждым обновлением чистим его.
        let updater = display_updater(window.handle)?;

        // начинаем как «играет» — статус уточнит фронтенд, когда появится.
        let _ = controls.set_playback(MediaPlayback::Playing { progress: None });

        // ГЛАВНЫЙ ЦИКЛ. Нельзя блокироваться на `rx.recv()`: пока поток спит в
        // ожидании метаданных, он не качает оконные сообщения, а без этого SMTC
        // не доставляет нажатия кнопок. Поэтому: качаем сообщения, затем
        // неблокирующе выгребаем все накопившиеся метаданные, затем короткий сон.
        loop {
            pump_messages();

            // выгребаем все ожидающие обновления, не блокируясь.
            loop {
                match rx.try_recv() {
                    Ok(data) => {
                        // Сначала затираем всё, что осталось от прошлого трека
                        // (в первую очередь — обложку, которую souvlaki не чистит).
                        // ClearAll сбрасывает и тип медиа, поэтому возвращаем Music.
                        // Update здесь не зовём: финальный `set_metadata` ниже сам
                        // вызовет Update, и система увидит одно согласованное
                        // состояние без мигания старой обложкой.
                        let _ = updater.ClearAll();
                        let _ = updater.SetType(MediaPlaybackType::Music);

                        let artist = data.artists.join(", ");
                        // если у трека есть обложка на диске — отдаём file://-URL,
                        // souvlaki сам загрузит файл; иначе картинку не шлём.
                        let cover = data
                            .art_path
                            .as_ref()
                            .map(|p| format!("file://{}", p.display()));
                        let meta = MediaMetadata {
                            title: Some(&data.title),
                            artist: (!artist.is_empty()).then_some(artist.as_str()),
                            cover_url: cover.as_deref(),
                            ..Default::default()
                        };
                        let _ = controls.set_metadata(meta);
                    }
                    // очередь пуста — выходим к насосу сообщений.
                    Err(TryRecvError::Empty) => break,
                    // ядро завершилось и закрыло канал — завершаемся штатно.
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }

            std::thread::sleep(TICK);
        }
    }
}

/// достаёт `DisplayUpdater` того же SMTC, что использует souvlaki. `GetForWindow`
/// возвращает единый объект SMTC на окно, поэтому это ровно тот же транспорт —
/// нужен лишь чтобы уметь очищать обложку (`ClearAll`), чего souvlaki не даёт.
fn display_updater(
    hwnd: HWND,
) -> Result<SystemMediaTransportControlsDisplayUpdater, Box<dyn Error>> {
    let interop: ISystemMediaTransportControlsInterop = ::windows::core::factory::<
        SystemMediaTransportControls,
        ISystemMediaTransportControlsInterop,
    >()
    .map_err(|e| boxed(format!("SMTC interop factory: {e}")))?;

    let controls: SystemMediaTransportControls =
        unsafe { interop.GetForWindow(hwnd) }.map_err(|e| boxed(format!("GetForWindow: {e}")))?;

    controls
        .DisplayUpdater()
        .map_err(|e| boxed(format!("DisplayUpdater: {e}")))
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

/// скрытое окно-«пустышка» нашего процесса: не показывается и ничего не рисует,
/// служит лишь опорой для SMTC и приёмником его сообщений. Уничтожается на Drop.
struct DummyWindow {
    handle: HWND,
}

impl DummyWindow {
    fn new() -> Result<Self, Box<dyn Error>> {
        // имя класса окна; должно жить до снятия регистрации — литерал `w!`
        // указывает на статическую UTF-16 строку, так что живёт всю программу.
        let class_name = w!("dream_player_smtc");

        unsafe {
            let instance = GetModuleHandleW(None)
                .map_err(|e| boxed(format!("GetModuleHandleW: {e}")))?;
            let hinstance = HINSTANCE(instance.0);

            let wnd_class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                hInstance: hinstance,
                lpszClassName: class_name,
                lpfnWndProc: Some(Self::wnd_proc),
                ..Default::default()
            };

            if RegisterClassExW(&wnd_class) == 0 {
                return Err(boxed(format!(
                    "RegisterClassExW: {}",
                    std::io::Error::last_os_error()
                )));
            }

            // обычное невидимое окно нулевого размера (его не показываем): SMTC
            // требует настоящий HWND, message-only-окна ему не подходят.
            let handle = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,      // без родителя
                None,      // без меню
                hinstance, // модуль-владелец
                None,      // без доп. параметров
            )
            .map_err(|e| boxed(format!("CreateWindowExW: {e}")))?;

            Ok(DummyWindow { handle })
        }
    }

    /// оконная процедура: своей логики нет, всё отдаём системе по умолчанию.
    extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

impl Drop for DummyWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.handle);
        }
    }
}

/// прокачивает все накопившиеся оконные сообщения нашего потока. Именно этот
/// прогон заставляет SMTC вызывать навешенный `attach`-колбэк при нажатии
/// системных клавиш; без него кнопки не срабатывают.
fn pump_messages() {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// оборачивает строку в `Box<dyn Error>` (у souvlaki::Error нет гарантии
/// реализации `std::error::Error`).
fn boxed(msg: String) -> Box<dyn Error> {
    msg.into()
}
