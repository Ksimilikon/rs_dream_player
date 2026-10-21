//! safe wrapper over a library sync session (`sync_*`). The session is
//! driven by one thread (the sync worker); the core itself is shared safely.

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};
use std::ptr;

use api::cdc::FFIImportFileKind;
use api::mem;
use crate::ffi::cdc::{Export, Flags};
use api::sync::{
    self, FFISyncEvent, FFISyncEventKind, FFISyncFiles, FFISyncSession, FFISyncStatus,
};

/// one file of a received data portion.
pub struct SyncFile<'a> {
    /// a song / playlist cover (not a song file).
    pub cover: bool,
    pub name: String,
    pub data: &'a [u8],
    /// a local song with the same name to write over.
    pub replace: Option<PathBuf>,
}

/// human-readable description of a core status.
pub fn describe(st: FFISyncStatus) -> &'static str {
    use FFISyncStatus as S;
    match st {
        S::Ok => "ok",
        S::NotInitialized => "core is not initialized",
        S::NullPointer => "internal error: null pointer",
        S::InvalidArgument => "internal error: paths don't match the files",
        S::NoDatabase => "core runs without a database",
        S::KeysFailed => "failed to load / create the device keys",
        S::UnknownDevice => "no such connection",
        S::WrongState => "not allowed in the current state",
        S::InvalidUtf8 => "path is not valid UTF-8",
        S::Panic => "core panicked (see core_panic.log in the data dir)",
        S::NotFound => "some playlist / track is not in the library",
        S::DbFailed => "database error",
    }
}

/// human-readable description of a protocol status (`Ack` / `Abort`).
pub fn describe_wire(status: u32) -> &'static str {
    match status {
        0 => "ok",
        1 => "invalid packet",
        2 => "unexpected packet",
        3 => "decryption failed (not paired with this device?)",
        4 => "pairing failed (the device key changed?)",
        5 => "rejected (local playlists are newer)",
        6 => "failed to store the data",
        7 => "device is busy",
        8 => "cancelled",
        9 => "internal error",
        10 => "the other side runs another mode (sync / send / receive)",
        _ => "unknown status",
    }
}

pub struct Session(*mut FFISyncSession);

// the session is used by one thread at a time (moved into the worker).
unsafe impl Send for Session {}

impl Session {
    /// a two-way sync session over a `Connected` entry; `host` — this side
    /// listened.
    pub fn new(con: u64, host: bool) -> Result<Self, FFISyncStatus> {
        let mut s = ptr::null_mut();
        Self::wrap(unsafe { sync::sync_new(con, host, &mut s) }, s)
    }

    /// a one-way send of a selection with the export flags.
    pub fn new_send(con: u64, host: bool, what: Export<'_>, flags: Flags) -> Result<Self, FFISyncStatus> {
        let names: Vec<CString> = match what {
            Export::Playlists(names) => names
                .iter()
                .map(|n| CString::new(n.as_str()).unwrap_or_default())
                .collect(),
            _ => Vec::new(),
        };
        let name_ptrs: Vec<*const std::os::raw::c_char> = names.iter().map(|c| c.as_ptr()).collect();
        let (kind, ids): (u32, &[i64]) = match what {
            Export::Library => (0, &[]),
            Export::Playlists(_) => (1, &[]),
            Export::Tracks(ids) => (2, ids),
        };
        let mut s = ptr::null_mut();
        let st = unsafe {
            sync::sync_new_send(
                con,
                host,
                kind,
                name_ptrs.as_ptr(),
                name_ptrs.len(),
                ids.as_ptr(),
                ids.len(),
                flags.audio,
                flags.names,
                flags.covers,
                &mut s,
            )
        };
        Self::wrap(st, s)
    }

    /// a one-way receive; `force` — accept outdated playlists too.
    pub fn new_receive(con: u64, host: bool, force: bool) -> Result<Self, FFISyncStatus> {
        let mut s = ptr::null_mut();
        Self::wrap(unsafe { sync::sync_new_receive(con, host, force, &mut s) }, s)
    }

    fn wrap(st: FFISyncStatus, s: *mut FFISyncSession) -> Result<Self, FFISyncStatus> {
        match st {
            FFISyncStatus::Ok => Ok(Self(s)),
            e => Err(e),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        unsafe { sync::sync_feed(self.0, bytes.as_ptr(), bytes.len()) };
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        let (mut p, mut l) = (ptr::null_mut(), 0usize);
        unsafe { sync::sync_take_output(self.0, &mut p, &mut l) };
        if p.is_null() {
            return Vec::new();
        }
        let bytes = unsafe { std::slice::from_raw_parts(p, l) }.to_vec();
        unsafe { mem::sync_free_bytes(p, l) };
        bytes
    }

    pub fn poll_event(&mut self) -> Option<FFISyncEvent> {
        let mut ev = FFISyncEvent {
            kind: FFISyncEventKind::None,
            status: 0,
            by_peer: false,
            paired_id: 0,
            songs: 0,
            playlists: 0,
        };
        unsafe { sync::sync_poll_event(self.0, &mut ev) };
        (ev.kind != FFISyncEventKind::None).then_some(ev)
    }

    pub fn finished(&self) -> bool {
        unsafe { sync::sync_finished(self.0) }
    }

    /// files of the waiting import (after an `Import` event).
    pub fn import_files(&mut self) -> Vec<SyncFile<'_>> {
        let mut files = FFISyncFiles {
            ptr: ptr::null(),
            len: 0,
        };
        let st = unsafe { sync::sync_import_files(self.0, &mut files) };
        if st != FFISyncStatus::Ok || files.ptr.is_null() {
            return Vec::new();
        }
        let str_of = |p: *const std::os::raw::c_char| {
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        };
        unsafe { std::slice::from_raw_parts(files.ptr, files.len) }
            .iter()
            .map(|f| SyncFile {
                cover: f.kind != FFIImportFileKind::Song,
                name: str_of(f.name),
                data: match f.data.is_null() {
                    true => &[],
                    false => unsafe { std::slice::from_raw_parts(f.data, f.data_len) },
                },
                replace: (!f.replace.is_null()).then(|| PathBuf::from(str_of(f.replace))),
            })
            .collect()
    }

    /// the import files are written: `paths[i]` — where file `i` went.
    pub fn import_done(&mut self, paths: &[Option<&Path>]) -> Result<(), FFISyncStatus> {
        let cs: Vec<Option<CString>> = paths
            .iter()
            .map(|p| p.map(|p| CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default()))
            .collect();
        let ptrs: Vec<*const std::os::raw::c_char> = cs
            .iter()
            .map(|c| c.as_ref().map_or(ptr::null(), |c| c.as_ptr()))
            .collect();
        match unsafe { sync::sync_import_done(self.0, ptrs.as_ptr(), ptrs.len()) } {
            FFISyncStatus::Ok => Ok(()),
            e => Err(e),
        }
    }

    /// breaks the session off; the `Abort` for the peer is in `take_output`.
    pub fn cancel(&mut self) {
        unsafe { sync::sync_cancel(self.0) };
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { sync::sync_free(self.0) };
    }
}
