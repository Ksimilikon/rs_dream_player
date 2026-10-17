//! safe wrapper over the core engine (`engine_*`) and initialization (`init`).
//!
//! the end of a track arrives through an optional C callback that
//! `engine_add_*` takes from us: it raises an atomic flag, and the app loop
//! polls it ([`take_track_ended`]) and moves on to the next track (autoplay).

use std::ffi::CString;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use api::FFIInitStatus;
use api::player::{self, FFIEngineStatus};

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

/// human-readable description of an init failure.
pub fn describe_init(st: FFIInitStatus) -> &'static str {
    use FFIInitStatus as S;
    match st {
        S::Ok => "ok",
        S::NullPointer => "internal error: null directory",
        S::InvalidUtf8 => "a directory path is not valid UTF-8",
        S::AlreadyInitialized => "core is already initialized",
        S::DbFailed => "failed to open the database",
        S::AudioFailed => "failed to start audio output",
        S::Panic => "core panicked (see core_panic.log in the data dir)",
    }
}

/// initializes the core: directories + the needs-a-db flag. Call once before
/// anything else.
pub fn init(
    cache: &Path,
    config: &Path,
    data: &Path,
    music: &Path,
    need_db: bool,
) -> Result<(), FFIInitStatus> {
    let (c, cfg, d, m) = (
        path_cstr(cache),
        path_cstr(config),
        path_cstr(data),
        path_cstr(music),
    );
    match unsafe { api::init(c.as_ptr(), cfg.as_ptr(), d.as_ptr(), m.as_ptr(), need_db) } {
        FFIInitStatus::Ok => Ok(()),
        e => Err(e),
    }
}

/// `true` for `Ok`.
fn ok(st: FFIEngineStatus) -> bool {
    st == FFIEngineStatus::Ok
}

/// reads a value through an out-parameter; `default` on error.
fn get<T: Copy>(default: T, f: unsafe extern "C" fn(*mut T) -> FFIEngineStatus) -> T {
    let mut v = default;
    match unsafe { f(&mut v) } {
        FFIEngineStatus::Ok => v,
        _ => default,
    }
}

/// plays a track from the index by id (autoplaying the next one when done).
pub fn play_track_id(id: i64) -> bool {
    ok(player::engine_add_track_id(id, Some(on_track_end)))
}

/// plays a track by file path (autoplaying the next one when done).
pub fn play_track_path(path: &str) -> bool {
    let c = CString::new(path).unwrap_or_default();
    // reset a possibly "stale" end flag of the previous track.
    TRACK_ENDED.store(false, Ordering::SeqCst);
    ok(unsafe { player::engine_add_track_path(c.as_ptr(), Some(on_track_end)) })
}

// the TUI only reacts to the outcome of `engine_add_*` / device switching; a
// failed play / pause / seek just leaves the state as it was — the next UI
// tick shows the real state.
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
    get(0, player::engine_get_seek)
}
pub fn duration() -> u64 {
    get(0, player::engine_get_duration)
}
pub fn is_paused() -> bool {
    get(false, player::engine_is_pause)
}
pub fn is_empty() -> bool {
    get(true, player::engine_is_empty)
}
pub fn set_master_volume(v: f32) {
    player::engine_set_volume_master(v);
}
pub fn master_volume() -> f32 {
    get(1.0, player::engine_get_volume_master)
}
pub fn set_track_volume(v: f32) {
    player::engine_set_volume_track(v);
}
pub fn track_volume() -> f32 {
    get(1.0, player::engine_get_volume_track)
}

/// names of the available output devices.
pub fn output_devices() -> Vec<String> {
    let mut list = api::idx::FFIStringList {
        ptr: ptr::null_mut(),
        len: 0,
    };
    if !ok(unsafe { player::engine_get_output_devices(&mut list) }) {
        return Vec::new();
    }
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
    let mut ptr = ptr::null_mut();
    if !ok(unsafe { player::engine_get_output_device(&mut ptr) }) {
        return None;
    }
    let out = unsafe { opt_string(ptr) };
    unsafe { api::mem::idx_free_string(ptr) };
    out
}

/// switches the output device by name.
pub fn set_output_device(name: &str) -> bool {
    let c = CString::new(name).unwrap_or_default();
    ok(unsafe { player::engine_set_output_device(c.as_ptr()) })
}
