//! FFI over the `codec` crate. Prefix `cdc_` = packet pack / unpack.
//!
//! - the packet version is passed as a `u32` number (= `codec::Version`
//!   discriminant, currently `1` = v0.1); an unknown number ->
//!   `UnsupportedVersion`.
//! - every function returns [`FFICdcStatus`] (`0` = ok), results go through
//!   out-parameters. On error the out-parameters are zeroed (null/0).
//! - a song hash at the FFI boundary is `[u8; 16]` big-endian: the bytes go in
//!   the same order as the hex hash string in the db (`u128` is not portable
//!   in the C ABI).
//! - the `with_audio`/`with_names` flags exist in every function: `false` ->
//!   song audio / file names are not packed (length 0 in the packet) or not
//!   returned on unpack (null). Playlist names are not affected.
//! - the `with_covers` flag exists in every function that has playlists:
//!   playlist covers are read from disk by OS path and packed as the last
//!   section together with the file name (a format hint for the receiver).
//! - playlist dates (`created_at`/`updated_at`, unix seconds) are always
//!   packed — they later decide which playlist version is the genuine one
//!   across devices.
//! - returned buffers are freed by the paired `cdc_free_*` from
//!   [`crate::mem`].
//! - packet import into the db takes two steps — `cdc_import_prepare` ->
//!   (the frontend writes the files) -> `cdc_import_commit` /
//!   `cdc_import_cancel` (see [`import`]).

use std::{
    ffi::{CStr, c_char},
    path::Path,
    ptr, slice,
};

use codec::{CodecError, Playlist, Song, Version};
use storage::db::PlaylistHashRow;

use crate::idx::{to_c, vec_to_raw};

mod import;

pub use import::{
    FFIImportFile, FFIImportFileKind, FFIImportFiles, ImportSession, cdc_import_cancel,
    cdc_import_commit, cdc_import_prepare,
};

/// result of any `cdc_*` function — one shared code base for every pack /
/// unpack / import function. Ranges:
/// - `0` — ok;
/// - `1..=99` — `codec::CodecError` one to one;
/// - `100..=199` — import statuses (decisions, not failures: the packet is
///   valid but was not applied);
/// - `200..` — core-level errors.
///
/// one enum for everything — the compiler itself won't allow two statuses
/// with the same code. `repr(u32)`: on the C side it is a plain number; never
/// renumber, only append within the range.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFICdcStatus {
    Ok = 0,

    // ── codec::CodecError ──
    BadStartWord = 1,
    UnsupportedVersion = 2,
    UnexpectedEof = 3,
    TrailingBytes = 4,
    InvalidUtf8 = 5,
    TooManyRows = 6,
    NameTooLong = 7,
    DataTooLong = 8,
    DuplicateSection = 9,
    CountMismatch = 10,

    // ── import (100..=199) ──
    /// the packet is outdated: some playlist in it is older (`updated_at`)
    /// than the local one. The whole import is rejected, nothing is written;
    /// it can only be applied with `force`.
    Outdated = 100,

    // ── core (200..) ──
    /// a required pointer is null.
    NullPointer = 200,
    /// the core runs without a db (`need_db == false`).
    NoDatabase = 201,
    /// some id from the list is not in the db.
    TrackNotFound = 202,
    /// a hash in the db is not a 32-character hex string.
    InvalidHash = 203,
    /// failed to read an audio file / cover (or a db query failed).
    ReadFailed = 204,
    /// some playlist from the list is not in the db.
    PlaylistNotFound = 205,
    /// failed to save the import into the db.
    WriteFailed = 206,
    /// an argument is inconsistent with the others (e.g. the number of paths
    /// in `cdc_import_commit` differs from the files of `cdc_import_prepare`).
    InvalidArgument = 207,
    /// `init` hasn't succeeded yet.
    NotInitialized = 208,
    /// the core panicked (see `core_last_panic`).
    Panic = 209,
}

impl crate::panic::Status for FFICdcStatus {
    const OK: Self = Self::Ok;
    const NULL_POINTER: Self = Self::NullPointer;
    const PANIC: Self = Self::Panic;
}

impl From<CodecError> for FFICdcStatus {
    fn from(e: CodecError) -> Self {
        match e {
            CodecError::BadStartWord => Self::BadStartWord,
            CodecError::UnsupportedVersion => Self::UnsupportedVersion,
            CodecError::UnexpectedEof => Self::UnexpectedEof,
            CodecError::TrailingBytes => Self::TrailingBytes,
            CodecError::InvalidUtf8 => Self::InvalidUtf8,
            CodecError::TooManyRows => Self::TooManyRows,
            CodecError::NameTooLong => Self::NameTooLong,
            CodecError::DataTooLong => Self::DataTooLong,
            CodecError::DuplicateSection => Self::DuplicateSection,
            CodecError::CountMismatch => Self::CountMismatch,
        }
    }
}

/// one song of an unpacked packet.
#[repr(C)]
pub struct FFIPacketSong {
    pub hash: [u8; 16],    // big-endian
    pub name: *mut c_char, // null: no name / `with_names == false`
    pub data: *mut u8,     // null: no audio / `with_audio == false`
    pub data_len: usize,
}

/// one playlist of an unpacked packet.
#[repr(C)]
pub struct FFIPacketPlaylist {
    pub name: *mut c_char,     // null: empty name
    pub tracks: *mut [u8; 16], // song hashes (big-endian) in playlist order
    pub tracks_len: usize,

    pub has_created_at: bool, // Option<u64>
    pub created_at: u64,
    pub has_updated_at: bool, // Option<u64>
    pub updated_at: u64,

    pub cover_name: *mut c_char, // null: no name / `with_covers == false`
    pub cover: *mut u8,          // null: no cover / `with_covers == false`
    pub cover_len: usize,
}

/// unpacked packet (owns the song and playlist buffers).
#[repr(C)]
pub struct FFIPacket {
    pub version: u32,
    pub songs: *mut FFIPacketSong,
    pub songs_len: usize,
    pub playlists: *mut FFIPacketPlaylist,
    pub playlists_len: usize,
}

impl FFIPacket {
    fn empty() -> Self {
        FFIPacket {
            version: 0,
            songs: ptr::null_mut(),
            songs_len: 0,
            playlists: ptr::null_mut(),
            playlists_len: 0,
        }
    }
}

// ───────────────────────── helpers (crate-private) ─────────────────────────

/// zeroes the byte out-parameters; `false` if any of them is null.
///
/// # Safety
/// `out_ptr`/`out_len` are null or valid pointers.
unsafe fn reset_bytes(out_ptr: *mut *mut u8, out_len: *mut usize) -> bool {
    if out_ptr.is_null() || out_len.is_null() {
        return false;
    }
    unsafe {
        *out_ptr = ptr::null_mut();
        *out_len = 0;
    }
    true
}

/// pack result -> out-parameters + status.
///
/// # Safety
/// `out_ptr`/`out_len` are valid non-null pointers.
unsafe fn finish_bytes(
    res: Result<Vec<u8>, FFICdcStatus>,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    match res {
        Ok(bytes) => {
            let (p, l) = vec_to_raw(bytes);
            unsafe {
                *out_ptr = p;
                *out_len = l;
            }
            FFICdcStatus::Ok
        }
        Err(e) => e,
    }
}

/// `(ptr, len)` -> slice; null is allowed only when `len == 0`.
pub(crate) unsafe fn raw_slice<'a, T>(p: *const T, len: usize) -> Option<&'a [T]> {
    match (p.is_null(), len) {
        (true, 0) => Some(&[]),
        (true, _) => None,
        (false, _) => Some(unsafe { slice::from_raw_parts(p, len) }),
    }
}

/// C string -> `String`; null -> empty string.
unsafe fn c_to_name(p: *const c_char) -> Result<String, FFICdcStatus> {
    if p.is_null() {
        return Ok(String::new());
    }
    match unsafe { CStr::from_ptr(p) }.to_str() {
        Ok(s) => Ok(s.to_owned()),
        Err(_) => Err(FFICdcStatus::InvalidUtf8),
    }
}

/// hex hash from the db -> `u128`.
fn parse_hash(hex: &str) -> Option<u128> {
    (hex.len() == 32)
        .then(|| u128::from_str_radix(hex, 16).ok())
        .flatten()
}

/// file name of an OS path (empty if there is none).
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// cover by OS path: `(file name, bytes)`. A missing cover file is not an
/// error (a cover is decorative, and a broken link in the db must not block
/// the export): there is simply no cover then.
fn read_cover(path: Option<&Path>) -> (String, Vec<u8>) {
    path.and_then(|p| Some((file_name(p), std::fs::read(p).ok()?)))
        .unwrap_or_default()
}

/// what to pack from the db.
enum Selection<'a> {
    /// all tracks and all playlists.
    Library,
    /// tracks by id, without playlists.
    Tracks(&'a [i64]),
    /// playlists by name and all their songs.
    Playlists(&'a [String]),
}

/// common packing from the db.
fn pack_from_db(
    version: u32,
    selection: Selection<'_>,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
) -> Result<Vec<u8>, FFICdcStatus> {
    let version = Version::try_from(version)?;

    // only the db query runs under the lock; audio / covers are read without
    // it, so the core (player) isn't held while gigabytes are read from disk.
    let (tracks, playlists) = {
        let guard = crate::core().ok_or(FFICdcStatus::NotInitialized)?;
        let db = guard.get_db().ok_or(FFICdcStatus::NoDatabase)?;
        let read = |_| FFICdcStatus::ReadFailed;
        match selection {
            Selection::Library => (
                db.get_track_hashes(None).map_err(read)?,
                db.get_playlist_hashes(None).map_err(read)?,
            ),
            Selection::Tracks(ids) => {
                let tracks = db.get_track_hashes(Some(ids)).map_err(read)?;
                if tracks.len() != ids.len() {
                    return Err(FFICdcStatus::TrackNotFound);
                }
                (tracks, Vec::new())
            }
            Selection::Playlists(names) => {
                let playlists = db.get_playlist_hashes(Some(names)).map_err(read)?;
                if playlists.len() != names.len() {
                    return Err(FFICdcStatus::PlaylistNotFound);
                }
                // songs of all playlists without repeats, in order of first
                // appearance. Hashes without a track in the db don't become
                // songs but stay in the playlist list.
                let mut seen = std::collections::HashSet::new();
                let hashes: Vec<String> = playlists
                    .iter()
                    .flat_map(|p| &p.hashes)
                    .filter(|h| seen.insert(h.as_str()))
                    .cloned()
                    .collect();
                (db.get_track_hashes_by_hash(&hashes).map_err(read)?, playlists)
            }
        }
    };

    let songs = tracks
        .into_iter()
        .map(|(_, hash, path)| song_from_db(&hash, &path, with_audio, with_names))
        .collect::<Result<Vec<_>, _>>()?;
    let playlists = playlists
        .into_iter()
        .map(|row| playlist_from_db(row, with_covers))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(codec::encode(version, &songs, &playlists)?)
}

/// db row -> packet song.
fn song_from_db(
    hash: &str,
    path: &Path,
    with_audio: bool,
    with_names: bool,
) -> Result<Song, FFICdcStatus> {
    let hash = parse_hash(hash).ok_or(FFICdcStatus::InvalidHash)?;
    let name = match with_names {
        true => file_name(path),
        false => String::new(),
    };
    let data = match with_audio {
        true => std::fs::read(path).map_err(|_| FFICdcStatus::ReadFailed)?,
        false => Vec::new(),
    };
    Ok(Song { hash, name, data })
}

/// db playlist -> packet playlist.
fn playlist_from_db(row: PlaylistHashRow, with_covers: bool) -> Result<Playlist, FFICdcStatus> {
    let tracks = row
        .hashes
        .iter()
        .map(|h| parse_hash(h).ok_or(FFICdcStatus::InvalidHash))
        .collect::<Result<Vec<_>, _>>()?;
    let (cover_name, cover) = match with_covers {
        true => read_cover(row.cover_art.as_deref()),
        false => (String::new(), Vec::new()),
    };
    Ok(Playlist {
        name: row.name,
        tracks,
        created_at: row.created_at,
        updated_at: row.updated_at,
        cover_name,
        cover,
    })
}

/// packet song -> FFI (honouring the flags).
fn song_to_ffi(song: Song, with_audio: bool, with_names: bool) -> FFIPacketSong {
    let name = match with_names && !song.name.is_empty() {
        true => to_c(&song.name),
        false => ptr::null_mut(),
    };
    let (data, data_len) = match with_audio {
        true => vec_to_raw(song.data),
        false => (ptr::null_mut(), 0),
    };
    FFIPacketSong {
        hash: song.hash.to_be_bytes(),
        name,
        data,
        data_len,
    }
}

/// non-empty string -> C string, empty -> null.
fn non_empty_to_c(s: &str) -> *mut c_char {
    match s.is_empty() {
        true => ptr::null_mut(),
        false => to_c(s),
    }
}

/// packet playlist -> FFI (honouring the covers flag).
fn playlist_to_ffi(playlist: Playlist, with_covers: bool) -> FFIPacketPlaylist {
    let name = non_empty_to_c(&playlist.name);
    let (tracks, tracks_len) = vec_to_raw(
        playlist
            .tracks
            .into_iter()
            .map(u128::to_be_bytes)
            .collect::<Vec<_>>(),
    );
    let (cover_name, (cover, cover_len)) = match with_covers {
        true => (non_empty_to_c(&playlist.cover_name), vec_to_raw(playlist.cover)),
        false => (ptr::null_mut(), (ptr::null_mut(), 0)),
    };
    FFIPacketPlaylist {
        name,
        tracks,
        tracks_len,
        has_created_at: playlist.created_at.is_some(),
        created_at: playlist.created_at.unwrap_or(0),
        has_updated_at: playlist.updated_at.is_some(),
        updated_at: playlist.updated_at.unwrap_or(0),
        cover_name,
        cover,
        cover_len,
    }
}

/// songs from the parallel arrays of `cdc_encode`.
///
/// # Safety
/// see `cdc_encode`.
unsafe fn songs_from_c(
    count: usize,
    hashes: *const [u8; 16],
    names: *const *const c_char,
    data: *const *const u8,
    data_lens: *const usize,
    with_audio: bool,
    with_names: bool,
) -> Result<Vec<Song>, FFICdcStatus> {
    let hashes = unsafe { raw_slice(hashes, count) }.ok_or(FFICdcStatus::NullPointer)?;
    let names = match with_names {
        true => Some(unsafe { raw_slice(names, count) }.ok_or(FFICdcStatus::NullPointer)?),
        false => None,
    };
    let audio = match with_audio {
        true => match unsafe { (raw_slice(data, count), raw_slice(data_lens, count)) } {
            (Some(d), Some(l)) => Some((d, l)),
            _ => return Err(FFICdcStatus::NullPointer),
        },
        false => None,
    };

    let mut songs = Vec::with_capacity(count);
    for (i, hash) in hashes.iter().enumerate() {
        let name = match names {
            Some(n) => unsafe { c_to_name(n[i]) }?,
            None => String::new(),
        };
        let data = match audio {
            Some((d, l)) => unsafe { raw_slice(d[i], l[i]) }
                .ok_or(FFICdcStatus::NullPointer)?
                .to_vec(),
            None => Vec::new(),
        };
        songs.push(Song {
            hash: u128::from_be_bytes(*hash),
            name,
            data,
        });
    }
    Ok(songs)
}

/// playlists from the parallel arrays of `cdc_encode`.
///
/// # Safety
/// see `cdc_encode`.
#[allow(clippy::too_many_arguments)]
unsafe fn playlists_from_c(
    count: usize,
    names: *const *const c_char,
    tracks: *const [u8; 16],
    tracks_lens: *const usize,
    created_at: *const u64,
    updated_at: *const u64,
    covers: *const *const c_char,
    with_covers: bool,
) -> Result<Vec<Playlist>, FFICdcStatus> {
    let names = unsafe { raw_slice(names, count) }.ok_or(FFICdcStatus::NullPointer)?;
    let lens = unsafe { raw_slice(tracks_lens, count) }.ok_or(FFICdcStatus::NullPointer)?;
    let total = lens
        .iter()
        .try_fold(0usize, |acc, &l| acc.checked_add(l))
        .ok_or(FFICdcStatus::TooManyRows)?;
    let tracks = unsafe { raw_slice(tracks, total) }.ok_or(FFICdcStatus::NullPointer)?;
    // null dates array -> no playlist has dates; element 0 -> no date.
    let dates = |p: *const u64| match p.is_null() {
        true => None,
        false => Some(unsafe { slice::from_raw_parts(p, count) }),
    };
    let (created_at, updated_at) = (dates(created_at), dates(updated_at));
    let date = |d: Option<&[u64]>, i: usize| d.map(|d| d[i]).filter(|&v| v != 0);
    let covers = match with_covers {
        true => Some(unsafe { raw_slice(covers, count) }.ok_or(FFICdcStatus::NullPointer)?),
        false => None,
    };

    let mut playlists = Vec::with_capacity(count);
    let mut rest = tracks;
    for i in 0..count {
        let (own, tail) = rest.split_at(lens[i]);
        rest = tail;
        let (cover_name, cover) = match covers.map(|c| c[i]) {
            Some(p) if !p.is_null() => match crate::ptr_to_path(p) {
                Ok(path) => (
                    file_name(&path),
                    std::fs::read(&path).map_err(|_| FFICdcStatus::ReadFailed)?,
                ),
                Err(_) => return Err(FFICdcStatus::InvalidUtf8),
            },
            _ => (String::new(), Vec::new()),
        };
        playlists.push(Playlist {
            name: unsafe { c_to_name(names[i]) }?,
            tracks: own.iter().map(|h| u128::from_be_bytes(*h)).collect(),
            created_at: date(created_at, i),
            updated_at: date(updated_at, i),
            cover_name,
            cover,
        });
    }
    Ok(playlists)
}

// ───────────────────────────────── API ─────────────────────────────────────

/// packs songs and playlists passed from outside. All arrays are parallel.
///
/// songs (length `songs_len`):
/// - `hashes` — required when `songs_len > 0`;
/// - `names` — required with `with_names`; a null element = no name;
/// - `data`/`data_lens` — required with `with_audio`; a null `data[i]` is
///   allowed only when `data_lens[i] == 0`.
///
/// playlists (length `playlists_len`):
/// - `pl_names` — required when `playlists_len > 0`; a null element = no name;
/// - `pl_tracks_lens` — number of songs of each playlist;
/// - `pl_tracks` — song hashes of all playlists back to back (first the songs
///   of the 1st playlist, then the 2nd, ...), length `sum(pl_tracks_lens)`;
/// - `pl_created_at`/`pl_updated_at` — dates (unix seconds); a null array =
///   no playlist has dates, an element 0 = no date;
/// - `pl_covers` — OS paths to covers, required with `with_covers`; a null
///   element = no cover. The file bytes and its name go into the packet. The
///   path is given explicitly here, so an unreadable file is `ReadFailed`.
///
/// the packet is written to `out_ptr`/`out_len`, free it with
/// `cdc_free_bytes`.
///
/// # Safety
/// every non-null pointer is valid for the given number of elements; strings
/// are nul-terminated; `data[i]` is valid for `data_lens[i]` bytes.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_encode(
    version: u32,
    songs_len: usize,
    hashes: *const [u8; 16],
    names: *const *const c_char,
    data: *const *const u8,
    data_lens: *const usize,
    playlists_len: usize,
    pl_names: *const *const c_char,
    pl_tracks: *const [u8; 16],
    pl_tracks_lens: *const usize,
    pl_created_at: *const u64,
    pl_updated_at: *const u64,
    pl_covers: *const *const c_char,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    crate::panic::guard(|| {
        if !unsafe { reset_bytes(out_ptr, out_len) } {
            return FFICdcStatus::NullPointer;
        }
        let res = (|| {
            let version = Version::try_from(version)?;
            let songs = unsafe {
                songs_from_c(songs_len, hashes, names, data, data_lens, with_audio, with_names)
            }?;
            let playlists = unsafe {
                playlists_from_c(
                    playlists_len,
                    pl_names,
                    pl_tracks,
                    pl_tracks_lens,
                    pl_created_at,
                    pl_updated_at,
                    pl_covers,
                    with_covers,
                )
            }?;
            Ok(codec::encode(version, &songs, &playlists)?)
        })();
        unsafe { finish_bytes(res, out_ptr, out_len) }
    })
}

/// unpacks the packet `bytes[..len]` into `out`. With a flag off, the matching
/// fields are null. Free it with `cdc_free_packet`.
///
/// # Safety
/// `bytes` is valid for `len` bytes (null is allowed when `len == 0`); `out`
/// is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_decode(
    bytes: *const u8,
    len: usize,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out: *mut FFIPacket,
) -> FFICdcStatus {
    crate::panic::guard(|| {
        if out.is_null() {
            return FFICdcStatus::NullPointer;
        }
        unsafe { *out = FFIPacket::empty() };
        let Some(bytes) = (unsafe { raw_slice(bytes, len) }) else {
            return FFICdcStatus::NullPointer;
        };

        let packet = match codec::decode(bytes) {
            Ok(p) => p,
            Err(e) => return e.into(),
        };
        let (songs, songs_len) = vec_to_raw(
            packet
                .songs
                .into_iter()
                .map(|s| song_to_ffi(s, with_audio, with_names))
                .collect::<Vec<_>>(),
        );
        let (playlists, playlists_len) = vec_to_raw(
            packet
                .playlists
                .into_iter()
                .map(|p| playlist_to_ffi(p, with_covers))
                .collect::<Vec<_>>(),
        );
        unsafe {
            *out = FFIPacket {
                version: packet.version.into(),
                songs,
                songs_len,
                playlists,
                playlists_len,
            };
        }
        FFICdcStatus::Ok
    })
}

/// snapshot packet of the whole db: all tracks (by ascending id) and all
/// playlists. Free it with `cdc_free_bytes`.
///
/// # Safety
/// `out_ptr`/`out_len` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_pack_library(
    version: u32,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    crate::panic::guard(|| {
        if !unsafe { reset_bytes(out_ptr, out_len) } {
            return FFICdcStatus::NullPointer;
        }
        let res = pack_from_db(version, Selection::Library, with_audio, with_names, with_covers);
        unsafe { finish_bytes(res, out_ptr, out_len) }
    })
}

/// packet of db tracks by their ids (`ids[..ids_len]`, order is kept),
/// without playlists. Any id not found -> `TrackNotFound`. Free it with
/// `cdc_free_bytes`.
///
/// # Safety
/// `ids` is valid for `ids_len` elements (null is allowed when
/// `ids_len == 0`); `out_ptr`/`out_len` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_pack_tracks(
    version: u32,
    ids: *const i64,
    ids_len: usize,
    with_audio: bool,
    with_names: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    crate::panic::guard(|| {
        if !unsafe { reset_bytes(out_ptr, out_len) } {
            return FFICdcStatus::NullPointer;
        }
        let Some(ids) = (unsafe { raw_slice(ids, ids_len) }) else {
            return FFICdcStatus::NullPointer;
        };
        let res = pack_from_db(version, Selection::Tracks(ids), with_audio, with_names, false);
        unsafe { finish_bytes(res, out_ptr, out_len) }
    })
}

/// packet of db playlists by their names (`names[..names_len]`, order is
/// kept) together with all their songs (without repeats). The name is the
/// playlist identifier (local ids differ between devices). Any name not found
/// -> `PlaylistNotFound`. Free it with `cdc_free_bytes`.
///
/// # Safety
/// `names` is valid for `names_len` nul-terminated strings (null is allowed
/// when `names_len == 0`); `out_ptr`/`out_len` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_pack_playlists(
    version: u32,
    names: *const *const c_char,
    names_len: usize,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    crate::panic::guard(|| {
        if !unsafe { reset_bytes(out_ptr, out_len) } {
            return FFICdcStatus::NullPointer;
        }
        let Some(raw) = (unsafe { raw_slice(names, names_len) }) else {
            return FFICdcStatus::NullPointer;
        };
        let mut list = Vec::with_capacity(raw.len());
        for &p in raw {
            if p.is_null() {
                return FFICdcStatus::NullPointer;
            }
            match unsafe { c_to_name(p) } {
                Ok(name) => list.push(name),
                Err(e) => return e,
            }
        }
        let res = pack_from_db(version, Selection::Playlists(&list), with_audio, with_names, with_covers);
        unsafe { finish_bytes(res, out_ptr, out_len) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{cdc_free_bytes, cdc_free_packet};

    /// `cdc_encode` with songs only.
    unsafe fn encode_songs(
        version: u32,
        hashes: &[[u8; 16]],
        names: *const *const c_char,
        data: *const *const u8,
        lens: *const usize,
        with_audio: bool,
        with_names: bool,
    ) -> (FFICdcStatus, *mut u8, usize) {
        let (mut p, mut l) = (ptr::null_mut(), 0);
        let st = unsafe {
            cdc_encode(
                version,
                hashes.len(),
                hashes.as_ptr(),
                names,
                data,
                lens,
                0,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                with_audio,
                with_names,
                false,
                &mut p,
                &mut l,
            )
        };
        (st, p, l)
    }

    #[test]
    fn hash_parsing() {
        assert_eq!(parse_hash(&format!("{:032x}", 0xabcu128)), Some(0xabc));
        assert_eq!(parse_hash("abc"), None);
        assert_eq!(parse_hash(&"z".repeat(32)), None);
    }

    #[test]
    fn hash_bytes_match_hex_order() {
        let hex = "0123456789abcdef0011223344556677";
        let bytes = parse_hash(hex).unwrap().to_be_bytes();
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[15], 0x77);
    }

    #[test]
    fn songs_roundtrip_with_flags() {
        let hashes = [[1u8; 16], [2u8; 16]];
        let n0 = c"a.flac";
        let names = [n0.as_ptr(), ptr::null()];
        let d0 = [9u8, 8, 7];
        let data = [d0.as_ptr(), ptr::null()];
        let lens = [d0.len(), 0];
        let (st, p, l) = unsafe {
            encode_songs(1, &hashes, names.as_ptr(), data.as_ptr(), lens.as_ptr(), true, true)
        };
        assert_eq!(st, FFICdcStatus::Ok);

        // full unpack.
        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, true, true, true, &mut pk) },
            FFICdcStatus::Ok
        );
        assert_eq!(pk.version, 1);
        assert_eq!(pk.songs_len, 2);
        assert_eq!(pk.playlists_len, 0);
        let songs = unsafe { slice::from_raw_parts(pk.songs, pk.songs_len) };
        assert_eq!(songs[0].hash, [1; 16]);
        assert_eq!(unsafe { CStr::from_ptr(songs[0].name) }, n0);
        assert_eq!(
            unsafe { slice::from_raw_parts(songs[0].data, songs[0].data_len) },
            &d0
        );
        assert!(songs[1].name.is_null() && songs[1].data.is_null());
        unsafe { cdc_free_packet(pk) };

        // without audio and names.
        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, false, false, false, &mut pk) },
            FFICdcStatus::Ok
        );
        let songs = unsafe { slice::from_raw_parts(pk.songs, pk.songs_len) };
        assert!(songs[0].name.is_null() && songs[0].data.is_null());
        unsafe { cdc_free_packet(pk) };

        unsafe { cdc_free_bytes(p, l) };
    }

    #[test]
    fn playlists_roundtrip_with_covers() {
        let dir = std::env::temp_dir().join(format!("cdc_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cover_path = dir.join("cover.png");
        std::fs::write(&cover_path, [0x89, b'P', b'N', b'G']).unwrap();
        let cover_c = std::ffi::CString::new(cover_path.to_str().unwrap()).unwrap();

        let pl_names = [c"rock".as_ptr(), c"empty".as_ptr()];
        let pl_lens = [2usize, 0];
        let pl_tracks = [[7u8; 16], [8u8; 16]];
        let pl_created = [100u64, 0];
        let pl_updated = [200u64, 300];
        let pl_covers = [cover_c.as_ptr(), ptr::null()];
        let (mut p, mut l) = (ptr::null_mut(), 0);
        let st = unsafe {
            cdc_encode(
                1,
                0,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                2,
                pl_names.as_ptr(),
                pl_tracks.as_ptr(),
                pl_lens.as_ptr(),
                pl_created.as_ptr(),
                pl_updated.as_ptr(),
                pl_covers.as_ptr(),
                false,
                false,
                true,
                &mut p,
                &mut l,
            )
        };
        assert_eq!(st, FFICdcStatus::Ok);

        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, false, false, true, &mut pk) },
            FFICdcStatus::Ok
        );
        let pls = unsafe { slice::from_raw_parts(pk.playlists, pk.playlists_len) };
        assert_eq!(pls.len(), 2);
        assert_eq!(unsafe { CStr::from_ptr(pls[0].name) }, c"rock");
        assert_eq!(
            unsafe { slice::from_raw_parts(pls[0].tracks, pls[0].tracks_len) },
            &pl_tracks
        );
        assert_eq!(
            unsafe { slice::from_raw_parts(pls[0].cover, pls[0].cover_len) },
            &[0x89, b'P', b'N', b'G']
        );
        assert_eq!(unsafe { CStr::from_ptr(pls[0].cover_name) }, c"cover.png");
        assert!(pls[0].has_created_at && pls[0].created_at == 100);
        assert!(pls[0].has_updated_at && pls[0].updated_at == 200);
        assert!(!pls[1].has_created_at && pls[1].has_updated_at);
        assert!(pls[1].tracks.is_null() && pls[1].cover.is_null() && pls[1].cover_name.is_null());
        unsafe { cdc_free_packet(pk) };

        // without covers on unpack.
        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, false, false, false, &mut pk) },
            FFICdcStatus::Ok
        );
        let pls = unsafe { slice::from_raw_parts(pk.playlists, pk.playlists_len) };
        assert!(pls[0].cover.is_null() && pls[0].cover_name.is_null());
        assert!(pls[0].has_created_at);
        unsafe { cdc_free_packet(pk) };

        unsafe { cdc_free_bytes(p, l) };
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn encode_flags_allow_null_arrays() {
        let hashes = [[3u8; 16]];
        let (st, p, l) = unsafe {
            encode_songs(1, &hashes, ptr::null(), ptr::null(), ptr::null(), false, false)
        };
        assert_eq!(st, FFICdcStatus::Ok);
        unsafe { cdc_free_bytes(p, l) };
    }

    #[test]
    fn encode_errors() {
        let (st, p, l) = unsafe {
            encode_songs(7, &[], ptr::null(), ptr::null(), ptr::null(), false, false)
        };
        assert_eq!(st, FFICdcStatus::UnsupportedVersion);
        assert!(p.is_null() && l == 0);

        // names enabled, but the array is null.
        let (st, _, _) = unsafe {
            encode_songs(1, &[[0; 16]], ptr::null(), ptr::null(), ptr::null(), false, true)
        };
        assert_eq!(st, FFICdcStatus::NullPointer);
    }

    #[test]
    fn decode_error_codes_match_codec() {
        let mut pk = FFIPacket::empty();
        let junk = *b"NOPE\x01\0\0\0\0";
        let st = unsafe { cdc_decode(junk.as_ptr(), junk.len(), true, true, true, &mut pk) };
        assert_eq!(st, FFICdcStatus::BadStartWord);
        assert_eq!(st as u32, CodecError::BadStartWord.code());
        assert!(pk.songs.is_null() && pk.playlists.is_null());
        assert_eq!(
            FFICdcStatus::from(CodecError::CountMismatch) as u32,
            CodecError::CountMismatch.code()
        );
    }
}
