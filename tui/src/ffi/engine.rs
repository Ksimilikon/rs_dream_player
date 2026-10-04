//! безопасная обёртка над движком ядра (`engine_*`) и инициализацией (`init`).
//!
//! конец трека приходит через необязательный C-колбэк, который `engine_add_*`
//! принимает от нас: он поднимает атомарный флаг, а цикл приложения опрашивает
//! его ([`take_track_ended`]) и переходит к следующему треку (автоплей).

use std::ffi::CString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use api::player;

use super::types::opt_string;

/// поднимается колбэком ядра, когда трек доиграл до конца.
static TRACK_ENDED: AtomicBool = AtomicBool::new(false);

/// C-колбэк конца трека (вызывается из аудио-потока ядра).
extern "C" fn on_track_end() {
    TRACK_ENDED.store(true, Ordering::SeqCst);
}

/// забирает флаг «трек закончился» (сбрасывая его). `true` — пора играть следующий.
pub fn take_track_ended() -> bool {
    TRACK_ENDED.swap(false, Ordering::SeqCst)
}

fn path_cstr(p: &Path) -> CString {
    CString::new(p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// инициализирует ядро: каталоги + флаг нужды в бд. Вызывать один раз до всего.
pub fn init(cache: &Path, config: &Path, data: &Path, music: &Path, need_db: bool) {
    let (c, cfg, d, m) = (
        path_cstr(cache),
        path_cstr(config),
        path_cstr(data),
        path_cstr(music),
    );
    api::init(c.as_ptr(), cfg.as_ptr(), d.as_ptr(), m.as_ptr(), need_db);
}

/// играть трек из индекса по id (с автоплеем следующего по завершении).
pub fn play_track_id(id: i64) -> bool {
    player::engine_add_track_id(id, Some(on_track_end))
}

/// играть трек по пути к файлу (с автоплеем следующего по завершении).
pub fn play_track_path(path: &str) -> bool {
    let c = CString::new(path).unwrap_or_default();
    // сбрасываем возможный «висящий» флаг конца прошлого трека.
    TRACK_ENDED.store(false, Ordering::SeqCst);
    unsafe { player::engine_add_track_path(c.as_ptr(), Some(on_track_end)) }
}

pub fn play() {
    player::engine_play();
}
pub fn pause() {
    player::engine_pause();
}
pub fn play_pause() {
    player::engine_play_pause();
}
pub fn stop() {
    player::engine_stop();
}
pub fn set_speed(speed: f32) {
    player::engine_set_speed(speed);
}
pub fn seek(secs: u64) {
    player::engine_set_seek(secs);
}
pub fn position() -> u64 {
    player::engine_get_seek()
}
pub fn duration() -> u64 {
    player::engine_get_duration()
}
pub fn is_paused() -> bool {
    player::engine_is_pause()
}
pub fn is_empty() -> bool {
    player::engine_is_empty()
}
pub fn set_master_volume(v: f32) {
    player::engine_set_volume_master(v);
}
pub fn master_volume() -> f32 {
    player::engine_get_volume_master()
}
pub fn set_track_volume(v: f32) {
    player::engine_set_volume_track(v);
}
pub fn track_volume() -> f32 {
    player::engine_get_volume_track()
}

/// имена доступных устройств вывода.
pub fn output_devices() -> Vec<String> {
    let list = player::engine_get_output_devices();
    let out = if list.ptr.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(list.ptr, list.len) }
            .iter()
            .filter_map(|&p| unsafe { opt_string(p) })
            .collect()
    };
    unsafe { api::mem::engine_free_output_devices(list) };
    out
}

/// имя текущего устройства вывода.
pub fn current_output_device() -> Option<String> {
    let ptr = player::engine_get_output_device();
    let out = unsafe { opt_string(ptr) };
    unsafe { api::mem::idx_free_string(ptr) };
    out
}

/// переключает устройство вывода по имени.
pub fn set_output_device(name: &str) -> bool {
    let c = CString::new(name).unwrap_or_default();
    unsafe { player::engine_set_output_device(c.as_ptr()) }
}
