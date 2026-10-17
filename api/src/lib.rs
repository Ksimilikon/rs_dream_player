//! C ABI of the core. Every subsystem has its own status enum (`0` = ok),
//! returned as a plain number; results go through out-parameters:
//! - [`FFIInitStatus`] — `init`;
//! - [`player::FFIEngineStatus`] — `engine_*`;
//! - [`idx::FFIIdxStatus`] — `idx_*`;
//! - [`cdc::FFICdcStatus`] — `cdc_*`.
//!
//! no exported function lets a panic escape: every failure becomes a status
//! code, a panic — the `Panic` status (see [`panic`]). Status enums are
//! `repr(u32)`; discriminants are part of the ABI — never renumber, only
//! append.

use core::{Core, InitError, dirs::Dirs};
use std::{
    ffi::{CStr, c_char},
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock, PoisonError},
};

pub mod cdc;
pub mod idx;
pub mod mem;
pub mod panic;
pub mod player;

use panic::Status;

static GLOBAL_STATE: OnceLock<Mutex<Core>> = OnceLock::new();

/// locks the core; `None` before a successful `init`. A poisoned lock is
/// taken over: the core holds no invariants a half-finished call could break
/// badly enough to justify refusing every later call.
pub(crate) fn core() -> Option<MutexGuard<'static, Core>> {
    Some(GLOBAL_STATE.get()?.lock().unwrap_or_else(PoisonError::into_inner))
}

/// why a C string argument could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CStrError {
    Null,
    InvalidUtf8,
}

/// turns a C string into a path. On Unix the bytes are taken as is (paths are
/// arbitrary bytes), on Windows / others — as UTF-8.
pub(crate) fn ptr_to_path(ptr: *const c_char) -> Result<PathBuf, CStrError> {
    if ptr.is_null() {
        return Err(CStrError::Null);
    }
    let c_str = unsafe { CStr::from_ptr(ptr) };

    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(OsStr::from_bytes(c_str.to_bytes())))
    }
    #[cfg(not(unix))]
    {
        // Windows and others: the frontend passes the path in UTF-8.
        c_str
            .to_str()
            .map(PathBuf::from)
            .map_err(|_| CStrError::InvalidUtf8)
    }
}

/// writes the result into the out-parameter `out` and returns the status:
/// `Ok` with the value, or the error (a panic in `f` -> `Panic`) with `empty`
/// written (so the caller never reads garbage). A null `out` ->
/// `NullPointer` without running `f`.
///
/// # Safety
/// `out` is null or valid for writes.
pub(crate) unsafe fn with_out<T, S: Status>(
    out: *mut T,
    empty: T,
    f: impl FnOnce() -> Result<T, S>,
) -> S {
    if out.is_null() {
        return S::NULL_POINTER;
    }
    let (value, status) = match panic::catch(f) {
        Some(Ok(v)) => (v, S::OK),
        Some(Err(e)) => (empty, e),
        None => (empty, S::PANIC),
    };
    // `write`: the old contents are uninitialized garbage, never dropped.
    unsafe { out.write(value) };
    status
}

/// result of [`init`]. `repr(u32)`: never renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIInitStatus {
    Ok = 0,
    /// a directory pointer is null.
    NullPointer = 1,
    /// a directory is not valid UTF-8 (Windows / non-Unix only).
    InvalidUtf8 = 2,
    /// the core is already initialized; the first instance stays.
    AlreadyInitialized = 3,
    /// the db file can't be opened / created.
    DbFailed = 4,
    /// no audio output (no device, backend failed to start).
    AudioFailed = 5,
    /// the core panicked (see `core_last_panic`).
    Panic = 6,
}

impl Status for FFIInitStatus {
    const OK: Self = Self::Ok;
    const NULL_POINTER: Self = Self::NullPointer;
    const PANIC: Self = Self::Panic;
}

impl From<CStrError> for FFIInitStatus {
    fn from(e: CStrError) -> Self {
        match e {
            CStrError::Null => Self::NullPointer,
            CStrError::InvalidUtf8 => Self::InvalidUtf8,
        }
    }
}

impl From<InitError> for FFIInitStatus {
    fn from(e: InitError) -> Self {
        match e {
            InitError::Db => Self::DbFailed,
            InitError::Audio => Self::AudioFailed,
        }
    }
}

/// initializes the core. Call once before anything else; every other
/// function returns its `NotInitialized` status until this one returns `Ok`.
///
/// # Safety
/// every directory is null or a valid nul-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init(
    cache_dir: *const c_char,
    config_dir: *const c_char,
    data_dir: *const c_char,
    music_dir: *const c_char,
    need_db: bool,
) -> FFIInitStatus {
    panic::guard(|| match try_init(cache_dir, config_dir, data_dir, music_dir, need_db) {
        Ok(()) => FFIInitStatus::Ok,
        Err(e) => e,
    })
}

fn try_init(
    cache_dir: *const c_char,
    config_dir: *const c_char,
    data_dir: *const c_char,
    music_dir: *const c_char,
    need_db: bool,
) -> Result<(), FFIInitStatus> {
    // checked before building the core: a second `Core` would open a second
    // audio stream just to be thrown away.
    if GLOBAL_STATE.get().is_some() {
        return Err(FFIInitStatus::AlreadyInitialized);
    }
    let data = ptr_to_path(data_dir)?;
    // before the core starts: a panic while opening the db is logged too.
    panic::set_log_dir(&data);
    let dirs = Dirs {
        cache: ptr_to_path(cache_dir)?,
        config: ptr_to_path(config_dir)?,
        data,
        music_dir: ptr_to_path(music_dir)?,
    };
    let core = Core::new(dirs, need_db)?;
    // a concurrent `init` may have won the race since the check above.
    GLOBAL_STATE
        .set(Mutex::new(core))
        .map_err(|_| FFIInitStatus::AlreadyInitialized)
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;
    use crate::cdc::FFICdcStatus;
    use crate::idx::{FFIIdxStatus, FFITracksList};
    use crate::player::FFIEngineStatus;

    // nothing in this crate's tests calls `init`, so the core stays absent.

    #[test]
    fn calls_before_init_return_not_initialized() {
        assert_eq!(player::engine_play(), FFIEngineStatus::NotInitialized);
        let mut pos = 7u64;
        assert_eq!(
            unsafe { player::engine_get_seek(&mut pos) },
            FFIEngineStatus::NotInitialized
        );
        assert_eq!(pos, 0, "the out-parameter is reset on error");

        let mut list = FFITracksList {
            ptr: ptr::dangling_mut(),
            len: 3,
        };
        assert_eq!(
            unsafe { idx::idx_get_tracks(&mut list) },
            FFIIdxStatus::NotInitialized
        );
        assert!(list.ptr.is_null() && list.len == 0);
        assert_eq!(idx::idx_remove_track(1), FFIIdxStatus::NotInitialized);
        let (mut cover, mut cover_len) = (ptr::dangling_mut(), 5);
        assert_eq!(
            unsafe { idx::idx_get_track_cover(1, ptr::null(), &mut cover, &mut cover_len) },
            FFIIdxStatus::NotInitialized
        );
        assert!(cover.is_null() && cover_len == 0, "the out-parameters are reset");

        let (mut p, mut l) = (ptr::null_mut(), 0);
        assert_eq!(
            unsafe { cdc::cdc_pack_library(1, false, false, false, &mut p, &mut l) },
            FFICdcStatus::NotInitialized
        );
    }

    #[test]
    fn null_arguments_are_reported() {
        assert_eq!(
            unsafe { player::engine_get_seek(ptr::null_mut()) },
            FFIEngineStatus::NullPointer
        );
        assert_eq!(
            unsafe { player::engine_set_output_device(ptr::null()) },
            FFIEngineStatus::NullPointer
        );
        assert_eq!(
            unsafe { idx::idx_get_tracks(ptr::null_mut()) },
            FFIIdxStatus::NullPointer
        );
        assert_eq!(
            unsafe { idx::idx_remove_playlist(ptr::null()) },
            FFIIdxStatus::NullPointer
        );
        assert_eq!(
            unsafe { idx::idx_get_track_cover(1, ptr::null(), ptr::null_mut(), ptr::null_mut()) },
            FFIIdxStatus::NullPointer
        );
        assert_eq!(
            unsafe { init(ptr::null(), ptr::null(), ptr::null(), ptr::null(), false) },
            FFIInitStatus::NullPointer
        );
    }
}
