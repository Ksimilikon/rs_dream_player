//! FFI over library sync between paired devices. Prefix `sync_`.
//!
//! the core implements the protocol (frames, tags / statuses, pairing over
//! the stream, comparing libraries, import) but no transport: the frontend
//! moves bytes over whatever it has (TCP, Bluetooth, ADB, ...). One session
//! per connection; the listening side is the host:
//!
//! 1. `con_add` + `con_connect` once the transport is up, then `sync_new`
//!    (two-way sync on both sides) or `sync_new_send` on one side and
//!    `sync_new_receive` on the other (a one-way send of a chosen selection
//!    with the export flags);
//! 2. loop until `sync_finished`:
//!    - send what `sync_take_output` hands out;
//!    - handle events from `sync_poll_event` (until `None`); on `Import`
//!      write the files of `sync_import_files` (a non-null `replace` — write
//!      over that file, otherwise anywhere) and call `sync_import_done`;
//!    - feed received bytes to `sync_feed`; a closed stream — `sync_cancel`;
//! 3. `sync_free`, `con_disconnect` / `con_remove`.
//!
//! only songs that are in some playlist are synced; what goes over the wire
//! is described in [`frame`] and [`library`]. Frame tags and statuses are
//! open, the payloads are encrypted for the paired device (see `con_*`).

pub mod frame;
pub mod library;
pub mod session;

use std::{
    ffi::{CString, c_char},
    path::PathBuf,
    ptr,
};

use storage::Db;
use vpair::{DeviceManager, VPairState};

use crate::cdc::{self, FFIImportFileKind, raw_slice};
use crate::idx::vec_to_raw;
use crate::panic::Status;
use crate::{ptr_to_path, with_out};
use frame::WireStatus;
use library::{Flags, Plan, PlanError, Selection};
use session::{Env, Event, Kind, SessionError, SyncSession};

/// result of any `sync_*` function (except `sync_free*`). `repr(u32)`:
/// never renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFISyncStatus {
    Ok = 0,
    /// `init` hasn't succeeded yet.
    NotInitialized = 1,
    /// a required pointer (argument or out-parameter) is null.
    NullPointer = 2,
    /// an argument is inconsistent (e.g. the number of import paths).
    InvalidArgument = 3,
    /// the core runs without a db (`need_db == false`).
    NoDatabase = 4,
    /// the key pair of the current device can't be loaded / created.
    KeysFailed = 5,
    /// no connection entry with this id.
    UnknownDevice = 6,
    /// not allowed now: the entry is not `Connected` (`sync_new`) or no
    /// import is waiting (`sync_import_*`).
    WrongState = 7,
    /// a path is not valid UTF-8 (Windows / non-Unix only).
    InvalidUtf8 = 8,
    /// the core panicked (see `core_last_panic`).
    Panic = 9,
    /// some playlist / track of a send selection is not in the db.
    NotFound = 10,
    /// a db query failed.
    DbFailed = 11,
}

impl Status for FFISyncStatus {
    const OK: Self = Self::Ok;
    const NULL_POINTER: Self = Self::NullPointer;
    const PANIC: Self = Self::Panic;
}

/// kind of [`FFISyncEvent`]. `repr(u32)`: never renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFISyncEventKind {
    /// no event now.
    None = 0,
    /// keys are exchanged (`paired_id`).
    Paired = 1,
    /// files to write: `sync_import_files` -> `sync_import_done`.
    Import = 2,
    /// the peer accepted a data portion (`songs`, `playlists`).
    Sent = 3,
    /// a received data portion is saved (`songs`, `playlists`).
    Received = 4,
    /// the peer didn't accept a data portion (`status`); the session goes on.
    PeerRejected = 5,
    /// the sync is complete.
    Done = 6,
    /// the session is over with an error (`status`, `by_peer`).
    Failed = 7,
}

/// a session event. `status` — a [`frame::WireStatus`] number.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FFISyncEvent {
    pub kind: FFISyncEventKind,
    pub status: u32,
    pub by_peer: bool,
    pub paired_id: i64,
    pub songs: u32,
    pub playlists: u32,
}

/// one file of a received data portion.
#[repr(C)]
pub struct FFISyncFile {
    pub kind: FFIImportFileKind,
    /// suggested file name (safe on any OS, with an extension).
    pub name: *const c_char,
    pub data: *const u8,
    pub data_len: usize,
    /// a local file with the same name to write over (the received song
    /// replaces it); null — write anywhere.
    pub replace: *const c_char,
}

/// files of a received data portion. Owned by the session: valid until
/// `sync_import_done` / `sync_cancel` / `sync_free`.
#[repr(C)]
pub struct FFISyncFiles {
    pub ptr: *const FFISyncFile,
    pub len: usize,
}

/// a sync session for C — an opaque pointer.
pub struct FFISyncSession {
    inner: SyncSession,
    /// own the strings `files[i].name` / `.replace` point to.
    names: Vec<CString>,
    replace: Vec<Option<CString>>,
    files: Vec<FFISyncFile>,
}

/// the global core.
struct Global;

impl Env for Global {
    type Store = cdc::Global;

    fn run<R>(
        &self,
        f: impl FnOnce(&mut DeviceManager, &Db) -> Result<R, WireStatus>,
    ) -> Result<R, WireStatus> {
        let mut core = crate::core().ok_or(WireStatus::Internal)?;
        let (mgr, db) = core.get_vpair().map_err(|_| WireStatus::Internal)?;
        f(mgr, db.ok_or(WireStatus::Internal)?)
    }

    fn store(&self) -> &cdc::Global {
        &cdc::Global
    }
}

const EMPTY_EVENT: FFISyncEvent = FFISyncEvent {
    kind: FFISyncEventKind::None,
    status: 0,
    by_peer: false,
    paired_id: 0,
    songs: 0,
    playlists: 0,
};

fn event_to_ffi(ev: Event) -> FFISyncEvent {
    let (kind, status, by_peer, paired_id, stats) = match ev {
        Event::Paired { paired_id } => (
            FFISyncEventKind::Paired,
            WireStatus::Ok,
            false,
            paired_id,
            None,
        ),
        Event::Import => (FFISyncEventKind::Import, WireStatus::Ok, false, 0, None),
        Event::Sent(s) => (FFISyncEventKind::Sent, WireStatus::Ok, false, 0, Some(s)),
        Event::Received(s) => (
            FFISyncEventKind::Received,
            WireStatus::Ok,
            false,
            0,
            Some(s),
        ),
        Event::PeerRejected(st) => (FFISyncEventKind::PeerRejected, st, true, 0, None),
        Event::Done => (FFISyncEventKind::Done, WireStatus::Ok, false, 0, None),
        Event::Failed { status, by_peer } => (FFISyncEventKind::Failed, status, by_peer, 0, None),
    };
    let stats = stats.unwrap_or_default();
    FFISyncEvent {
        kind,
        status: status as u32,
        by_peer,
        paired_id,
        songs: stats.songs,
        playlists: stats.playlists,
    }
}

/// `session` -> the session; null -> `NullPointer`.
///
/// # Safety
/// `session` is null or comes from `sync_new` and wasn't freed yet.
unsafe fn session<'a>(
    session: *mut FFISyncSession,
) -> Result<&'a mut FFISyncSession, FFISyncStatus> {
    unsafe { session.as_mut() }.ok_or(FFISyncStatus::NullPointer)
}

fn status(res: Result<(), FFISyncStatus>) -> FFISyncStatus {
    res.err().unwrap_or(FFISyncStatus::Ok)
}

// ───────────────────────────────── API ─────────────────────────────────────

/// checks the entry and opens a session of the kind `make` builds (it runs
/// under the core lock with the db).
///
/// # Safety
/// `out` is null or a valid pointer.
unsafe fn open(
    con: u64,
    host: bool,
    out: *mut *mut FFISyncSession,
    make: impl FnOnce(&Db) -> Result<Kind, FFISyncStatus>,
) -> FFISyncStatus {
    crate::panic::guard(|| unsafe {
        with_out(out, ptr::null_mut(), || {
            let kind = {
                let mut core = crate::core().ok_or(FFISyncStatus::NotInitialized)?;
                let (mgr, db) = core.get_vpair().map_err(|_| FFISyncStatus::KeysFailed)?;
                let db = db.ok_or(FFISyncStatus::NoDatabase)?;
                let dev = mgr.device(con).ok_or(FFISyncStatus::UnknownDevice)?;
                if dev.state() != VPairState::Connected {
                    return Err(FFISyncStatus::WrongState);
                }
                make(db)?
            };
            let inner = SyncSession::new(&Global, con, host, kind);
            Ok(Box::into_raw(Box::new(FFISyncSession {
                inner,
                names: Vec::new(),
                replace: Vec::new(),
                files: Vec::new(),
            })))
        })
    })
}

/// a two-way library sync session over the connection entry `con` (it must
/// be `Connected`); `host` — this side listened for the connection. Into
/// `out`; free with `sync_free`. The other side runs `sync_new` too.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_new(
    con: u64,
    host: bool,
    out: *mut *mut FFISyncSession,
) -> FFISyncStatus {
    unsafe { open(con, host, out, |_| Ok(Kind::Sync)) }
}

/// a one-way send session: sends the chosen selection with the export flags
/// (the other side runs `sync_new_receive`). `what`: `0` — the whole library,
/// `1` — playlists by name (`names[..names_len]`) with their songs, `2` —
/// tracks by id (`ids[..ids_len]`) without playlists. `with_audio` /
/// `with_names` / `with_covers` — as in `cdc_pack_*`. `NotFound` — some
/// playlist / track is not in the db; `InvalidArgument` — an unknown `what`.
///
/// # Safety
/// `out` is a valid pointer; `names` is valid for `names_len` nul-terminated
/// strings and `ids` for `ids_len` elements (null is allowed with length 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_new_send(
    con: u64,
    host: bool,
    what: u32,
    names: *const *const c_char,
    names_len: usize,
    ids: *const i64,
    ids_len: usize,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out: *mut *mut FFISyncSession,
) -> FFISyncStatus {
    let selection = || -> Result<Selection, FFISyncStatus> {
        Ok(match what {
            0 => Selection::Library,
            1 => {
                let raw =
                    unsafe { raw_slice(names, names_len) }.ok_or(FFISyncStatus::NullPointer)?;
                let names = raw
                    .iter()
                    .map(|&p| unsafe { crate::idx::c_to_string(p) })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| match e {
                        crate::CStrError::Null => FFISyncStatus::NullPointer,
                        crate::CStrError::InvalidUtf8 => FFISyncStatus::InvalidUtf8,
                    })?;
                Selection::Playlists(names)
            }
            2 => {
                let ids = unsafe { raw_slice(ids, ids_len) }.ok_or(FFISyncStatus::NullPointer)?;
                Selection::Tracks(ids.to_vec())
            }
            _ => return Err(FFISyncStatus::InvalidArgument),
        })
    };
    let flags = Flags {
        audio: with_audio,
        names: with_names,
        covers: with_covers,
    };
    unsafe {
        open(con, host, out, |db| {
            let plan = Plan::selection(db, &selection()?, flags).map_err(|e| match e {
                PlanError::NotFound => FFISyncStatus::NotFound,
                PlanError::Db => FFISyncStatus::DbFailed,
            })?;
            Ok(Kind::Send(plan))
        })
    }
}

/// a one-way receive session: accepts what the other side sends with
/// `sync_new_send`. `force` — a playlist deleted here after the sent one
/// changed comes back too (as in `cdc_import_prepare`).
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_new_receive(
    con: u64,
    host: bool,
    force: bool,
    out: *mut *mut FFISyncSession,
) -> FFISyncStatus {
    unsafe { open(con, host, out, |_| Ok(Kind::Receive { force })) }
}

/// frees the session (null is ignored). An unfinished session is not
/// cancelled — call `sync_cancel` first to tell the peer.
///
/// # Safety
/// `session` is null or comes from `sync_new` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_free(session: *mut FFISyncSession) {
    crate::panic::guard_unit(|| {
        if !session.is_null() {
            drop(unsafe { Box::from_raw(session) });
        }
    })
}

/// bytes received from the peer (`data[..len]`; null only with `len == 0`).
///
/// # Safety
/// `session` comes from `sync_new`; `data` is valid for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_feed(
    session: *mut FFISyncSession,
    data: *const u8,
    len: usize,
) -> FFISyncStatus {
    crate::panic::guard(|| {
        status((|| {
            let s = unsafe { self::session(session) }?;
            let data = unsafe { raw_slice(data, len) }.ok_or(FFISyncStatus::NullPointer)?;
            s.inner.feed(&Global, data);
            Ok(())
        })())
    })
}

/// bytes to send to the peer into `out_ptr`/`out_len` (null/0 — nothing
/// now). Free with `sync_free_bytes`.
///
/// # Safety
/// `session` comes from `sync_new`; `out_ptr`/`out_len` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_take_output(
    session: *mut FFISyncSession,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFISyncStatus {
    crate::panic::guard(|| {
        if out_len.is_null() {
            return FFISyncStatus::NullPointer;
        }
        let mut len = 0;
        let st = unsafe {
            with_out(out_ptr, ptr::null_mut(), || {
                let (p, l) = vec_to_raw(self::session(session)?.inner.take_output());
                len = l;
                Ok(p)
            })
        };
        unsafe { out_len.write(len) };
        st
    })
}

/// the next event into `out` (`kind == None` — no event now).
///
/// # Safety
/// `session` comes from `sync_new`; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_poll_event(
    session: *mut FFISyncSession,
    out: *mut FFISyncEvent,
) -> FFISyncStatus {
    crate::panic::guard(|| unsafe {
        with_out(out, EMPTY_EVENT, || {
            let s = self::session(session)?;
            Ok(s.inner.poll_event().map_or(EMPTY_EVENT, event_to_ffi))
        })
    })
}

/// `true` — the session is over (`Done` or `Failed`); a null session counts
/// as over.
///
/// # Safety
/// `session` is null or comes from `sync_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_finished(session: *mut FFISyncSession) -> bool {
    crate::panic::catch(|| unsafe { session.as_ref() }.is_none_or(|s| s.inner.is_finished()))
        .unwrap_or(true)
}

/// the files of the waiting import into `out` (after an `Import` event);
/// `WrongState` — none is waiting.
///
/// # Safety
/// `session` comes from `sync_new`; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_import_files(
    session: *mut FFISyncSession,
    out: *mut FFISyncFiles,
) -> FFISyncStatus {
    let empty = FFISyncFiles {
        ptr: ptr::null(),
        len: 0,
    };
    crate::panic::guard(|| unsafe {
        with_out(out, empty, || {
            let s = self::session(session)?;
            let pending = s.inner.import_files().ok_or(FFISyncStatus::WrongState)?;
            // CString buffers don't move with the vector, so the pointers
            // stay valid while the strings live.
            s.names = pending
                .session
                .planned()
                .map(|(_, name, _)| CString::new(name).unwrap_or_default())
                .collect();
            s.replace = pending
                .replace
                .iter()
                .map(|p| {
                    p.as_ref()
                        .and_then(|p| CString::new(p.to_string_lossy().as_bytes()).ok())
                })
                .collect();
            s.files = pending
                .session
                .planned()
                .zip(&s.names)
                .zip(&s.replace)
                .map(|(((kind, _, data), name), replace)| FFISyncFile {
                    kind,
                    name: name.as_ptr(),
                    data: data.as_ptr(),
                    data_len: data.len(),
                    replace: replace.as_ref().map_or(ptr::null(), |r| r.as_ptr()),
                })
                .collect();
            Ok(FFISyncFiles {
                ptr: s.files.as_ptr(),
                len: s.files.len(),
            })
        })
    })
}

/// the import files are written: `paths[i]` — where file `i` of
/// `sync_import_files` went (null — not written). The portion is saved and
/// the peer is answered. `InvalidArgument` — `paths_len` differs from the
/// number of files.
///
/// # Safety
/// `session` comes from `sync_new`; `paths` is valid for `paths_len`
/// elements, each null or a nul-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_import_done(
    session: *mut FFISyncSession,
    paths: *const *const c_char,
    paths_len: usize,
) -> FFISyncStatus {
    crate::panic::guard(|| {
        status((|| {
            let s = unsafe { self::session(session) }?;
            let raw = unsafe { raw_slice(paths, paths_len) }.ok_or(FFISyncStatus::NullPointer)?;
            let paths = raw
                .iter()
                .map(|&p| match p.is_null() {
                    true => Ok(None),
                    false => ptr_to_path(p)
                        .map(Some)
                        .map_err(|_| FFISyncStatus::InvalidUtf8),
                })
                .collect::<Result<Vec<Option<PathBuf>>, _>>()?;
            s.inner.import_done(&Global, &paths).map_err(|e| match e {
                SessionError::NoImport => FFISyncStatus::WrongState,
                SessionError::PathCount => FFISyncStatus::InvalidArgument,
            })?;
            s.files.clear();
            s.names.clear();
            s.replace.clear();
            Ok(())
        })())
    })
}

/// breaks the session off: `Abort(Cancelled)` is queued for the peer (send
/// it with `sync_take_output`). No-op for a finished session.
///
/// # Safety
/// `session` comes from `sync_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_cancel(session: *mut FFISyncSession) -> FFISyncStatus {
    crate::panic::guard(|| {
        status((|| {
            let s = unsafe { self::session(session) }?;
            s.inner.cancel(&Global);
            s.files.clear();
            s.names.clear();
            s.replace.clear();
            Ok(())
        })())
    })
}

#[cfg(test)]
mod tests;
