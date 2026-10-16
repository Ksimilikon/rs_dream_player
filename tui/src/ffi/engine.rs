//! safe wrapper over the core engine (`engine_*`) and initialization (`init`).
//!
//! the end of a track arrives through an optional C callback that
//! `engine_add_*` takes from us: it raises an atomic flag, and the app loop
//! polls it ([`take_track_ended`]) and moves on to the next track (autoplay).

use std::ffi::CString;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use api::player;

use super::types::opt_string;

/// raised by the core callback when a track has played to the end.
static TRACK_ENDED: AtomicBool = AtomicBool::new(false);

/// end-of-track C callback (called from the core audio thread).
extern "C" fn on_track_end() {
    TRACK_ENDED.store(true, Ordering::SeqCst);
}

/// takes the "track ended" flag (resetting it). `true` — time to play the next.
pub fn take_track_ended() -> bool {
    TRACK_ENDED.swap(false, Ordering::SeqCst)
}

fn path_cstr(p: &Path) -> CString {
    CString::new(p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// initializes the core: directories + the needs-a-db flag. Call once before
/// anything else.
pub fn init(cache: &Path, config: &Path, data: &Path, music: &Path, need_db: bool) {
    let (c, cfg, d, m) = (
        path_cstr(cache),
        path_cstr(config),
        path_cstr(data),
        path_cstr(music),
    );
    api::init(c.as_ptr(), cfg.as_ptr(), d.as_ptr(), m.as_ptr(), need_db);
}

/// plays a track from the index by id (autoplaying the next one when done).
pub fn play_track_id(id: i64) -> bool {
    player::engine_add_track_id(id, Some(on_track_end))
}

/// plays a track by file path (autoplaying the next one when done).
pub fn play_track_path(path: &str) -> bool {
    let c = CString::new(path).unwrap_or_default();
    // reset a possibly "stale" end flag of the previous track.
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

/// names of the available output devices.
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

/// name of the current output device.
pub fn current_output_device() -> Option<String> {
    let ptr = player::engine_get_output_device();
    let out = unsafe { opt_string(ptr) };
    unsafe { api::mem::idx_free_string(ptr) };
    out
}

/// switches the output device by name.
pub fn set_output_device(name: &str) -> bool {
    let c = CString::new(name).unwrap_or_default();
    unsafe { player::engine_set_output_device(c.as_ptr()) }
}
