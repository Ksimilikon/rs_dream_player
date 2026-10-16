use std::{
    ffi::{CStr, c_char},
    ptr,
    time::Duration,
};

use audio::AudioEngine;
use audio_structs::track_virtual::TrackVirtual;

use super::GLOBAL_STATE;
use crate::idx::{FFIStringList, to_c, vec_to_raw};

/// end-of-track callback passed by the frontend. `None` (null pointer) — no
/// notification. Called from the internal audio thread exactly once, when the
/// track has played to the end (not on `stop` / device change).
pub type OnTrackEnd = Option<extern "C" fn()>;

/// loads a track into the engine and starts playback. `true` on success.
fn play_track(engine: &mut AudioEngine, mut track: TrackVirtual, on_end: OnTrackEnd) -> bool {
    if track.load_track().is_err() {
        return false;
    }
    let Ok(bytes) = track.take_track() else {
        return false;
    };
    let volume = track.volume;
    // an extern "C" fn doesn't implement FnOnce directly — wrap it in a closure.
    let cb = on_end.map(|f| move || f());
    engine.load(bytes, volume, cb).is_ok()
}

/// plays a track from the index by its id. `on_end` is an optional
/// end-of-track callback.
#[unsafe(no_mangle)]
pub extern "C" fn engine_add_track_id(id_track: i64, on_end: OnTrackEnd) -> bool {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let track = guard.get_db().and_then(|db| {
        db.get_tracks(None, None, Some(id_track), None)
            .ok()?
            .into_iter()
            .next()
    });
    match track {
        Some(t) => play_track(guard.get_engine_mut(), t, on_end),
        None => false,
    }
}

/// plays a track by file path. If the file is already in the index, its row
/// is used (with the saved volume), otherwise the file is played directly.
/// `on_end` is an optional end-of-track callback.
///
/// # Safety
/// `path` is a valid C pointer to a nul-terminated string (not null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_add_track_path(path: *const c_char, on_end: OnTrackEnd) -> bool {
    let Ok(path) = crate::ptr_to_path(path) else {
        return false;
    };
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();

    // priority — the index row for this path (carries volume / metadata).
    let indexed = guard.get_db().and_then(|db| {
        db.get_tracks(None, None, None, None)
            .ok()?
            .into_iter()
            .find(|t| t.get_path() == Some(path.as_path()))
    });
    let track = match indexed {
        Some(t) => t,
        None => match TrackVirtual::from_file(path, false) {
            Ok(t) => t,
            Err(_) => return false,
        },
    };
    play_track(guard.get_engine_mut(), track, on_end)
}

#[unsafe(no_mangle)]
pub extern "C" fn engine_play_pause() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().play_pause();
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_play() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().play();
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_pause() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().pause();
}
/// stops playback and clears the queue.
#[unsafe(no_mangle)]
pub extern "C" fn engine_stop() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().stop();
}
/// playback speed (1.0 = normal).
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_speed(speed: f32) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().set_speed(speed);
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_seek(seek_sec: u64) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let _ = guard.get_engine_mut().seek(Duration::from_secs(seek_sec));
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_seek() -> u64 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().get_pos().as_secs()
}
/// full duration of the current track in seconds (`0` — unknown / nothing
/// loaded; some formats don't report it).
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_duration() -> u64 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().get_duration().as_secs()
}
/// `true` if playback is paused.
#[unsafe(no_mangle)]
pub extern "C" fn engine_is_pause() -> bool {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().is_pause()
}
/// `true` if the queue is empty (the track finished or nothing is loaded).
#[unsafe(no_mangle)]
pub extern "C" fn engine_is_empty() -> bool {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().is_empty()
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_volume_track(volume: f32) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().set_volume_track(volume);
}
/// master volume: applied to the engine AND saved to the core config on disk
/// (survives a restart).
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_volume_master(volume: f32) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.set_master_volume(volume);
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_volume_track() -> f32 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().get_volume_track()
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_volume_master() -> f32 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().get_volume_master()
}

/// names of the available output devices. Free with
/// `engine_free_output_devices`.
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_output_devices() -> FFIStringList {
    let raw: Vec<*mut c_char> = AudioEngine::output_devices()
        .iter()
        .map(|s| to_c(s))
        .collect();
    let (ptr, len) = vec_to_raw(raw);
    FFIStringList { ptr, len }
}

/// name of the current output device (null if unknown). Free with
/// `idx_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_output_device() -> *mut c_char {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard
        .get_engine()
        .get_output_device()
        .map_or(ptr::null_mut(), to_c)
}

/// switches output to the device named `name`. Returns `true` on success.
///
/// # Safety
/// `name` is either null or a valid C pointer to a nul-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_set_output_device(name: *const c_char) -> bool {
    if name.is_null() {
        return false;
    }
    let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else {
        return false;
    };
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().set_output_device(name).is_ok()
}
