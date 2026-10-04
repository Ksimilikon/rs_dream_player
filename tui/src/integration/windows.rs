//! Windows-бэкенд: системный медиа-транспорт (SMTC) через souvlaki (перенесён из
//! крейта `dbus`). Создаём собственное скрытое окно нашего процесса как опору для
//! SMTC и качаем насос оконных сообщений на том же потоке — без этого система не
//! доставляет нажатия медиа-клавиш. Параллельно неблокирующе вычитываем
//! метаданные из `meta_rx`.

use std::error::Error;
use std::ffi::c_void;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
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
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
    RegisterClassExW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_QUIT, WNDCLASSEXW,
};
use ::windows::core::w;

use super::{MediaCommand, MediaMeta};

const DISPLAY_NAME: &str = "dream_player";
const TICK: Duration = Duration::from_millis(50);

/// запускает SMTC-интеграцию на отдельном потоке. Ошибки завершают поток тихо.
pub fn spawn(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) {
    std::thread::spawn(move || {
        let _ = run(cmd_tx, meta_rx);
    });
}

fn run(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) -> Result<(), Box<dyn Error>> {
    // собственное скрытое окно нашего процесса — на нём и держится SMTC.
    let window = DummyWindow::new()?;

    let config = PlatformConfig {
        dbus_name: DISPLAY_NAME,
        display_name: DISPLAY_NAME,
        hwnd: Some(window.handle.0 as *mut c_void),
    };

    let mut controls =
        MediaControls::new(config).map_err(|e| boxed(format!("souvlaki init: {e:?}")))?;

    // системные кнопки -> внутренние команды.
    controls
        .attach(move |event: MediaControlEvent| {
            if let Some(cmd) = map_event(event) {
                let _ = cmd_tx.send(cmd);
            }
        })
        .map_err(|e| boxed(format!("souvlaki attach: {e:?}")))?;

    // свой доступ к тому же DisplayUpdater — чтобы уметь чистить обложку
    // (souvlaki при cover_url=None старый thumbnail не убирает).
    let updater = display_updater(window.handle)?;

    let _ = controls.set_playback(MediaPlayback::Playing { progress: None });

    // главный цикл: нельзя блокироваться на recv — иначе не качаются оконные
    // сообщения и SMTC не доставляет нажатия. Качаем сообщения, затем
    // неблокирующе выгребаем метаданные, затем короткий сон.
    loop {
        pump_messages();

        loop {
            match meta_rx.try_recv() {
                Ok(data) => {
                    let _ = updater.ClearAll();
                    let _ = updater.SetType(MediaPlaybackType::Music);

                    let artist = data.artists.join(", ");
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
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        std::thread::sleep(TICK);
    }
}

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

fn map_event(event: MediaControlEvent) -> Option<MediaCommand> {
    match event {
        MediaControlEvent::Play => Some(MediaCommand::Play),
        MediaControlEvent::Pause => Some(MediaCommand::Pause),
        MediaControlEvent::Toggle => Some(MediaCommand::PlayPause),
        MediaControlEvent::Next => Some(MediaCommand::Next),
        MediaControlEvent::Previous => Some(MediaCommand::Prev),
        MediaControlEvent::Stop => Some(MediaCommand::Stop),
        _ => None,
    }
}

/// скрытое окно-«пустышка» нашего процесса: опора для SMTC и приёмник его
/// сообщений. Уничтожается на Drop.
struct DummyWindow {
    handle: HWND,
}

impl DummyWindow {
    fn new() -> Result<Self, Box<dyn Error>> {
        let class_name = w!("dream_player_smtc");
        unsafe {
            let instance =
                GetModuleHandleW(None).map_err(|e| boxed(format!("GetModuleHandleW: {e}")))?;
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

            let handle = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                hinstance,
                None,
            )
            .map_err(|e| boxed(format!("CreateWindowExW: {e}")))?;

            Ok(DummyWindow { handle })
        }
    }

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

/// прокачивает накопившиеся оконные сообщения потока — без этого SMTC не
/// вызывает `attach`-колбэк при нажатии системных клавиш.
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

fn boxed(msg: String) -> Box<dyn Error> {
    msg.into()
}
