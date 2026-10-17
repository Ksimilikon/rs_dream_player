//! FFI over playback. Prefix `engine_` = audio engine operations.
//!
//! every function returns [`FFIEngineStatus`] (`0` = ok), values go through
//! out-parameters. On error the out-parameter holds an empty value (`0`,
//! `false`, null).

use std::{
    ffi::c_char,
    ptr,
    sync::MutexGuard,
    time::Duration,
};

use audio::AudioEngine;
use audio_structs::track_virtual::TrackVirtual;
use core::Core;

use crate::idx::{FFIStringList, to_c, vec_to_raw};
use crate::panic::Status;
use crate::{CStrError, with_out};

/// result of any `engine_*` function. `repr(u32)`: never renumber, only
/// append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIEngineStatus {
    Ok = 0,
    /// `init` hasn't succeeded yet.
    NotInitialized = 1,
    /// a required pointer (argument or out-parameter) is null.
    NullPointer = 2,
    /// a string argument is not valid UTF-8.
    InvalidUtf8 = 3,
    /// the core runs without a db (`need_db == false`).
    NoDatabase = 4,
    /// a db query failed.
    ReadFailed = 5,
    /// no track with this id in the db.
    TrackNotFound = 6,
    /// the track file can't be read / is not audio.
    LoadFailed = 7,
    /// the engine refused the track (can't decode it).
    PlaybackFailed = 8,
    /// seeking is not supported by the format / out of range.
    SeekFailed = 9,
    /// no output device with this name, or it failed to start.
    DeviceFailed = 10,
    /// the value was applied but the config couldn't be saved to disk.
    ConfigWriteFailed = 11,
    /// the core panicked (see `core_last_panic`).
    Panic = 12,
}

impl Status for FFIEngineStatus {
    const OK: Self = Self::Ok;
    const NULL_POINTER: Self = Self::NullPointer;
    const PANIC: Self = Self::Panic;
}

impl From<CStrError> for FFIEngineStatus {
    fn from(e: CStrError) -> Self {
        match e {
            CStrError::Null => Self::NullPointer,
            CStrError::InvalidUtf8 => Self::InvalidUtf8,
        }
    }
}

/// locks the core; `NotInitialized` before `init`.
fn core() -> Result<MutexGuard<'static, Core>, FFIEngineStatus> {
    crate::core().ok_or(FFIEngineStatus::NotInitialized)
}

/// runs `f` on the core and turns the result into a status.
fn run(f: impl FnOnce(&mut Core) -> Result<(), FFIEngineStatus>) -> FFIEngineStatus {
    match core().and_then(|mut c| f(&mut c)) {
        Ok(()) => FFIEngineStatus::Ok,
        Err(e) => e,
    }
}

/// reads a value from the core into `out`.
///
/// # Safety
/// `out` is null or valid for writes.
unsafe fn get<T>(out: *mut T, empty: T, f: impl FnOnce(&Core) -> T) -> FFIEngineStatus {
    unsafe {
        with_out(
            out,
            empty,
            || Ok(f(&*core()?)),
        )
    }
}

/// end-of-track callback passed by the frontend. `None` (null pointer) — no
/// notification. Called from the internal audio thread exactly once, when the
/// track has played to the end (not on `stop` / device change).
pub type OnTrackEnd = Option<extern "C" fn()>;

/// loads a track into the engine and starts playback.
fn play_track(
    engine: &mut AudioEngine,
    mut track: TrackVirtual,
    on_end: OnTrackEnd,
) -> Result<(), FFIEngineStatus> {
    track.load_track().map_err(|_| FFIEngineStatus::LoadFailed)?;
    let bytes = track.take_track().map_err(|_| FFIEngineStatus::LoadFailed)?;
    let volume = track.volume;
    // an extern "C" fn doesn't implement FnOnce directly — wrap it in a closure.
    let cb = on_end.map(|f| move || f());
    engine
        .load(bytes, volume, cb)
        .map_err(|_| FFIEngineStatus::PlaybackFailed)
}

/// plays a track from the index by its id. `on_end` is an optional
/// end-of-track callback.
#[unsafe(no_mangle)]
pub extern "C" fn engine_add_track_id(id_track: i64, on_end: OnTrackEnd) -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| {
            let db = core.get_db().ok_or(FFIEngineStatus::NoDatabase)?;
            let track = db
                .get_tracks(None, None, Some(id_track), None)
                .map_err(|_| FFIEngineStatus::ReadFailed)?
                .into_iter()
                .next()
                .ok_or(FFIEngineStatus::TrackNotFound)?;
            play_track(core.get_engine_mut(), track, on_end)
        })
    })
}

/// plays a track by file path. If the file is already in the index, its row
/// is used (with the saved volume), otherwise the file is played directly.
/// `on_end` is an optional end-of-track callback.
///
/// # Safety
/// `path` is null or a valid C pointer to a nul-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_add_track_path(
    path: *const c_char,
    on_end: OnTrackEnd,
) -> FFIEngineStatus {
    crate::panic::guard(|| {
        let path = match crate::ptr_to_path(path) {
            Ok(p) => p,
            Err(e) => return e.into(),
        };
        run(|core| {
            // priority — the index row for this path (carries volume / metadata).
            // Without a db (or if the query fails) the file is played directly.
            let indexed = core.get_db().and_then(|db| {
                db.get_tracks(None, None, None, None)
                    .ok()?
                    .into_iter()
                    .find(|t| t.get_path() == Some(path.as_path()))
            });
            let track = match indexed {
                Some(t) => t,
                None => TrackVirtual::from_file(path, false)
                    .map_err(|_| FFIEngineStatus::LoadFailed)?,
            };
            play_track(core.get_engine_mut(), track, on_end)
        })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn engine_play_pause() -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| Ok(core.get_engine_mut().play_pause()))
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_play() -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| Ok(core.get_engine_mut().play()))
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_pause() -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| Ok(core.get_engine_mut().pause()))
    })
}
/// stops playback and clears the queue.
#[unsafe(no_mangle)]
pub extern "C" fn engine_stop() -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| Ok(core.get_engine_mut().stop()))
    })
}
/// playback speed (1.0 = normal).
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_speed(speed: f32) -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| Ok(core.get_engine_mut().set_speed(speed)))
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_seek(seek_sec: u64) -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| {
            core.get_engine_mut()
                .seek(Duration::from_secs(seek_sec))
                .map_err(|_| FFIEngineStatus::SeekFailed)
        })
    })
}
/// current position in seconds.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_get_seek(out: *mut u64) -> FFIEngineStatus {
    crate::panic::guard(|| {
        unsafe { get(out, 0, |c| c.get_engine().get_pos().as_secs()) }
    })
}
/// full duration of the current track in seconds (`0` — unknown / nothing
/// loaded; some formats don't report it).
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_get_duration(out: *mut u64) -> FFIEngineStatus {
    crate::panic::guard(|| {
        unsafe { get(out, 0, |c| c.get_engine().get_duration().as_secs()) }
    })
}
/// `true` if playback is paused.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_is_pause(out: *mut bool) -> FFIEngineStatus {
    crate::panic::guard(|| {
        unsafe { get(out, false, |c| c.get_engine().is_pause()) }
    })
}
/// `true` if the queue is empty (the track finished or nothing is loaded).
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_is_empty(out: *mut bool) -> FFIEngineStatus {
    crate::panic::guard(|| {
        unsafe { get(out, false, |c| c.get_engine().is_empty()) }
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_volume_track(volume: f32) -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| Ok(core.get_engine_mut().set_volume_track(volume)))
    })
}
/// master volume: applied to the engine AND saved to the core config on disk
/// (survives a restart). `ConfigWriteFailed` — applied, but not saved.
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_volume_master(volume: f32) -> FFIEngineStatus {
    crate::panic::guard(|| {
        run(|core| {
            core.set_master_volume(volume)
                .map_err(|_| FFIEngineStatus::ConfigWriteFailed)
        })
    })
}
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_get_volume_track(out: *mut f32) -> FFIEngineStatus {
    crate::panic::guard(|| {
        unsafe { get(out, 0.0, |c| c.get_engine().get_volume_track()) }
    })
}
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_get_volume_master(out: *mut f32) -> FFIEngineStatus {
    crate::panic::guard(|| {
        unsafe { get(out, 0.0, |c| c.get_engine().get_volume_master()) }
    })
}

/// names of the available output devices, the system default
/// (`"Default Device"`, follows the system routing) first. Works before
/// `init` too. Free with
/// `engine_free_output_devices`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_get_output_devices(out: *mut FFIStringList) -> FFIEngineStatus {
    crate::panic::guard(|| {
        let empty = FFIStringList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        unsafe {
            with_out(
                out,
                empty,
                || {
                    let raw: Vec<*mut c_char> = AudioEngine::output_devices()
                        .iter()
                        .map(|s| to_c(s))
                        .collect();
                    let (ptr, len) = vec_to_raw(raw);
                    Ok(FFIStringList { ptr, len })
                },
            )
        }
    })
}

/// name of the current output device (`*out` == null if unknown). Free with
/// `idx_free_string`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_get_output_device(out: *mut *mut c_char) -> FFIEngineStatus {
    crate::panic::guard(|| {
        unsafe {
            get(out, ptr::null_mut(), |c| {
                c.get_engine()
                    .get_output_device()
                    .map_or(ptr::null_mut(), to_c)
            })
        }
    })
}

/// switches output to the device named `name`.
///
/// # Safety
/// `name` is either null or a valid C pointer to a nul-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_set_output_device(name: *const c_char) -> FFIEngineStatus {
    crate::panic::guard(|| {
        let name = match unsafe { crate::idx::c_to_string(name) } {
            Ok(n) => n,
            Err(e) => return e.into(),
        };
        run(|core| {
            core.get_engine_mut()
                .set_output_device(&name)
                .map_err(|_| FFIEngineStatus::DeviceFailed)
        })
    })
}
