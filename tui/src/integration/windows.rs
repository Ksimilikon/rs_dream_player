//! Windows backend: the system media transport (SMTC) via souvlaki (moved from
//! the `dbus` crate). We create our own hidden window in our process as an
//! anchor for SMTC and pump the window message loop on the same thread —
//! without it the system doesn't deliver media key presses. Meanwhile metadata
//! is read from `meta_rx` without blocking.

use std::error::Error;
use std::ffi::c_void;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, PlatformConfig};

// absolute paths to the external crate: our module is also called `windows`.
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

/// starts the SMTC integration on a separate thread. Errors end the thread
/// silently.
pub fn spawn(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) {
    std::thread::spawn(move || {
        let _ = run(cmd_tx, meta_rx);
    });
}

fn run(cmd_tx: Sender<MediaCommand>, meta_rx: Receiver<MediaMeta>) -> Result<(), Box<dyn Error>> {
    // our process's own hidden window — SMTC is anchored to it.
    let window = DummyWindow::new()?;

    let config = PlatformConfig {
        dbus_name: DISPLAY_NAME,
        display_name: DISPLAY_NAME,
        hwnd: Some(window.handle.0 as *mut c_void),
    };

    let mut controls =
        MediaControls::new(config).map_err(|e| boxed(format!("souvlaki init: {e:?}")))?;

    // system buttons -> internal commands.
    controls
        .attach(move |event: MediaControlEvent| {
            if let Some(cmd) = map_event(event) {
                let _ = cmd_tx.send(cmd);
            }
        })
        .map_err(|e| boxed(format!("souvlaki attach: {e:?}")))?;

    // our own access to the same DisplayUpdater — to be able to clear the cover
    // (souvlaki doesn't remove the old thumbnail when cover_url=None).
    let updater = display_updater(window.handle)?;

    let _ = controls.set_playback(MediaPlayback::Playing { progress: None });

    // main loop: it must not block on recv — otherwise window messages aren't
    // pumped and SMTC doesn't deliver presses. Pump messages, then drain
    // metadata without blocking, then a short sleep.
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

/// a hidden "dummy" window of our process: the SMTC anchor and the receiver
/// of its messages. Destroyed on Drop.
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

/// pumps the thread's pending window messages — without it SMTC doesn't call
/// the `attach` callback when system keys are pressed.
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
