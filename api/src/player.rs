use std::{
    ffi::{CStr, c_char},
    ptr,
    time::Duration,
};

use audio::AudioEngine;
use audio_structs::track_virtual::TrackVirtual;

use super::GLOBAL_STATE;
use crate::idx::{FFIStringList, to_c, vec_to_raw};

/// колбэк конца трека, передаётся фронтом. `None` (null-указатель) — без
/// уведомления. Вызывается из внутреннего аудио-потока ровно один раз, когда
/// трек доиграл до конца (не при `stop`/смене устройства).
pub type OnTrackEnd = Option<extern "C" fn()>;

/// загружает трек в движок и запускает воспроизведение. `true` при успехе.
fn play_track(engine: &mut AudioEngine, mut track: TrackVirtual, on_end: OnTrackEnd) -> bool {
    if track.load_track().is_err() {
        return false;
    }
    let Ok(bytes) = track.take_track() else {
        return false;
    };
    let volume = track.volume;
    // extern "C" fn не реализует FnOnce напрямую — оборачиваем в замыкание.
    let cb = on_end.map(|f| move || f());
    engine.load(bytes, volume, cb).is_ok()
}

/// играть трек из индекса по его id. `on_end` — необязательный колбэк конца трека.
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

/// играть трек по пути к файлу. Если файл уже в индексе — берём запись оттуда
/// (с сохранённой громкостью), иначе играем файл напрямую. `on_end` —
/// необязательный колбэк конца трека.
///
/// # Safety
/// `path` — валидный C-указатель на нуль-терминированную строку (не null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_add_track_path(path: *const c_char, on_end: OnTrackEnd) -> bool {
    let Ok(path) = crate::ptr_to_path(path) else {
        return false;
    };
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();

    // приоритет — запись из индекса по этому пути (несёт громкость/метадату).
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
/// остановить воспроизведение и очистить очередь.
#[unsafe(no_mangle)]
pub extern "C" fn engine_stop() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().stop();
}
/// скорость воспроизведения (1.0 = нормальная).
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
/// полная длительность текущего трека в секундах (`0` — неизвестно/ничего не
/// загружено; некоторые форматы её не сообщают).
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_duration() -> u64 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().get_duration().as_secs()
}
/// `true`, если воспроизведение на паузе.
#[unsafe(no_mangle)]
pub extern "C" fn engine_is_pause() -> bool {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().is_pause()
}
/// `true`, если очередь пуста (трек доиграл или ничего не загружено).
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
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_volume_master(volume: f32) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().set_volume_master(volume);
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

/// имена доступных устройств вывода. Освобождать `engine_free_output_devices`.
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_output_devices() -> FFIStringList {
    let raw: Vec<*mut c_char> = AudioEngine::output_devices()
        .iter()
        .map(|s| to_c(s))
        .collect();
    let (ptr, len) = vec_to_raw(raw);
    FFIStringList { ptr, len }
}

/// имя текущего устройства вывода (null, если неизвестно). Освобождать
/// `idx_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_output_device() -> *mut c_char {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard
        .get_engine()
        .get_output_device()
        .map_or(ptr::null_mut(), to_c)
}

/// переключает вывод на устройство с именем `name`. Возвращает `true` при успехе.
///
/// # Safety
/// `name` — либо null, либо валидный C-указатель на нуль-терминированную строку.
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
