//! safe wrapper over the core packets (`cdc_*`): packing (export) and the
//! two-step import. Core buffers are copied into owning `Vec`s and freed right
//! away with the paired `cdc_free_*`; the import session is closed in `Drop`.

use std::ffi::{CStr, CString};
use std::path::Path;
use std::ptr;

use api::cdc::{self, FFICdcStatus, FFIImportFileKind, FFIImportFiles, ImportSession};
use api::mem;

/// packet format version the frontend writes (`codec::Version::V0_1`).
const PACKET_VERSION: u32 = 1;

/// what to put into the packet.
#[derive(Debug, Clone, Copy)]
pub struct Flags {
    pub audio: bool,
    pub names: bool,
    pub covers: bool,
}

/// what to export.
pub enum Export<'a> {
    /// the whole db: every track and playlist.
    Library,
    /// playlists by name together with their songs.
    Playlists(&'a [String]),
    /// tracks by id (without playlists).
    Tracks(&'a [i64]),
}

/// human-readable description of a core status.
pub fn describe(st: FFICdcStatus) -> &'static str {
    use FFICdcStatus as S;
    match st {
        S::Ok => "ok",
        S::BadStartWord => "not a packet (bad start word)",
        S::UnsupportedVersion => "unsupported packet version",
        S::UnexpectedEof => "packet is truncated",
        S::TrailingBytes => "packet has trailing garbage",
        S::InvalidUtf8 => "packet has a non-UTF-8 name",
        S::TooManyRows => "too many rows for the packet",
        S::NameTooLong => "a name is too long for the packet",
        S::DataTooLong => "a file is too large for the packet (> 4 GiB)",
        S::DuplicateSection => "packet has a duplicate section",
        S::CountMismatch => "packet covers don't match playlists",
        S::Outdated => "packet is outdated: local playlists are newer (enable force to override)",
        S::NullPointer => "internal error: null pointer",
        S::NoDatabase => "core runs without a database",
        S::TrackNotFound => "some track is not in the library",
        S::InvalidHash => "library has a malformed song hash",
        S::ReadFailed => "failed to read a file or the database",
        S::PlaylistNotFound => "some playlist is not in the library",
        S::WriteFailed => "failed to save the import to the database",
        S::InvalidArgument => "internal error: paths don't match the file list",
    }
}

/// packs the selection into a packet.
pub fn pack(what: Export<'_>, flags: Flags) -> Result<Vec<u8>, FFICdcStatus> {
    let (mut p, mut l) = (ptr::null_mut(), 0usize);
    let st = unsafe {
        match what {
            Export::Library => cdc::cdc_pack_library(
                PACKET_VERSION,
                flags.audio,
                flags.names,
                flags.covers,
                &mut p,
                &mut l,
            ),
            Export::Playlists(names) => {
                let cs: Vec<CString> = names
                    .iter()
                    .map(|n| CString::new(n.as_str()).unwrap_or_default())
                    .collect();
                let ptrs: Vec<*const std::os::raw::c_char> =
                    cs.iter().map(|c| c.as_ptr()).collect();
                cdc::cdc_pack_playlists(
                    PACKET_VERSION,
                    ptrs.as_ptr(),
                    ptrs.len(),
                    flags.audio,
                    flags.names,
                    flags.covers,
                    &mut p,
                    &mut l,
                )
            }
            Export::Tracks(ids) => cdc::cdc_pack_tracks(
                PACKET_VERSION,
                ids.as_ptr(),
                ids.len(),
                flags.audio,
                flags.names,
                &mut p,
                &mut l,
            ),
        }
    };
    if st != FFICdcStatus::Ok {
        return Err(st);
    }
    let bytes = match p.is_null() {
        true => Vec::new(),
        false => unsafe { std::slice::from_raw_parts(p, l) }.to_vec(),
    };
    unsafe { mem::cdc_free_bytes(p, l) };
    Ok(bytes)
}

/// one file that has to be written for the import.
pub struct ImportFile<'a> {
    pub cover: bool,
    pub name: String,
    pub data: &'a [u8],
}

/// a prepared import (step 1). Files are read from the session memory; the
/// session is closed by [`PreparedImport::commit`] or in `Drop` (= cancel).
pub struct PreparedImport {
    session: *mut ImportSession,
    files: FFIImportFiles,
}

impl PreparedImport {
    /// step 1: unpack and check dates (the db doesn't change).
    pub fn prepare(bytes: &[u8], force: bool) -> Result<Self, FFICdcStatus> {
        let mut session = ptr::null_mut();
        let mut files = FFIImportFiles {
            ptr: ptr::null(),
            len: 0,
        };
        let st = unsafe {
            cdc::cdc_import_prepare(bytes.as_ptr(), bytes.len(), force, &mut session, &mut files)
        };
        match st {
            FFICdcStatus::Ok => Ok(Self { session, files }),
            e => Err(e),
        }
    }

    /// files to write (in the order `commit` expects).
    pub fn files(&self) -> Vec<ImportFile<'_>> {
        if self.files.ptr.is_null() {
            return Vec::new();
        }
        unsafe { std::slice::from_raw_parts(self.files.ptr, self.files.len) }
            .iter()
            .map(|f| ImportFile {
                cover: f.kind == FFIImportFileKind::Cover,
                name: unsafe { CStr::from_ptr(f.name) }
                    .to_string_lossy()
                    .into_owned(),
                data: match f.data.is_null() {
                    true => &[],
                    false => unsafe { std::slice::from_raw_parts(f.data, f.data_len) },
                },
            })
            .collect()
    }

    /// step 3: saves the import. `paths[i]` is where `files()[i]` was written
    /// (`None` — not written). The session is closed in any case.
    pub fn commit(mut self, paths: &[Option<&Path>]) -> Result<(), FFICdcStatus> {
        let cs: Vec<Option<CString>> = paths
            .iter()
            .map(|p| p.map(|p| CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default()))
            .collect();
        let ptrs: Vec<*const std::os::raw::c_char> = cs
            .iter()
            .map(|c| c.as_ref().map_or(ptr::null(), |c| c.as_ptr()))
            .collect();
        let session = std::mem::replace(&mut self.session, ptr::null_mut());
        let st = unsafe { cdc::cdc_import_commit(session, ptrs.as_ptr(), ptrs.len()) };
        match st {
            FFICdcStatus::Ok => Ok(()),
            e => Err(e),
        }
    }
}

impl Drop for PreparedImport {
    fn drop(&mut self) {
        // null after commit — then this is a no-op.
        unsafe { cdc::cdc_import_cancel(self.session) };
    }
}
