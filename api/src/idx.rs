//! FFI over the index (sqlite). Prefix `idx_` = db operations.
//! Every returned pointer / struct owns its memory and must be freed with the
//! paired `idx_free_*` from [`crate::mem`].

use std::error::Error;
use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::ptr;

use audio_structs::{
    playlist::Playlist,
    track_metadata::{TagEdit, TrackMetadata},
    track_virtual::TrackVirtual,
};
use storage::db::OrphanFilter;
use storage::traits::indexator::Indexator;

use crate::panic::Status;
use crate::{CStrError, with_out};

/// track metadata for the C ABI. Carries the song identifier: `id` (db row)
/// and/or `path` (file path) — either can serve as the key (see
/// `idx_get_track`).
#[repr(C)]
pub struct FFITrackMetadata {
    pub has_id: bool, // Option<i64>
    pub id: i64,
    pub path: *mut c_char, // maybe null (external source)

    pub title: *mut c_char,
    pub artists: *mut *mut c_char,
    pub artists_len: usize,

    pub album: *mut c_char, // maybe null
    pub genres: *mut *mut c_char,
    pub genres_len: usize,

    pub has_params: bool, // Option
    pub params: FFITrackMetadataParams,

    // db-only user state (not file tags); defaults for a track outside the db.
    pub volume: f32, // saved per-track volume (1.0 — default)
    pub invalid: bool,
    pub color: *mut c_char,      // maybe null
    pub user_label: *mut c_char, // maybe null
    /// kept on purpose though in no playlist (the orphan collector skips it).
    pub ignore_orphan: bool,
}

#[repr(C)]
pub struct FFITrackMetadataParams {
    pub duration_s: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub listen_count: i64,
    pub cover_art: *mut c_char, // maybe null
}

/// list of track metadata (owns the buffer).
#[repr(C)]
pub struct FFITracksList {
    pub ptr: *mut FFITrackMetadata,
    pub len: usize,
}

/// list of C strings (owns the buffer and the strings). Used e.g. for output
/// device names (`engine_get_output_devices`).
#[repr(C)]
pub struct FFIStringList {
    pub ptr: *mut *mut c_char,
    pub len: usize,
}

#[repr(C)]
pub struct FFIPlaylist {
    pub name: *mut c_char, // null = anonymus

    pub tracks: *mut i64, // track ids
    pub tracks_len: usize,

    pub cover_art: *mut c_char, // maybe null

    pub has_updated_at: bool, // Option<u64>
    pub updated_at: u64,

    pub has_created_at: bool, // Option<u64>
    pub created_at: u64,
}

#[repr(C)]
pub struct FFIPlaylistsList {
    pub ptr: *mut FFIPlaylist,
    pub len: usize,
}

// ───────────────────────── helpers (crate-private) ─────────────────────────

/// `&str` -> owned C string; `null` on an interior NUL.
pub(crate) fn to_c(s: &str) -> *mut c_char {
    CString::new(s).map_or(ptr::null_mut(), CString::into_raw)
}

/// `Option<&str>` -> C string or `null`.
pub(crate) fn opt_to_c(s: Option<&str>) -> *mut c_char {
    s.map_or(ptr::null_mut(), to_c)
}

/// `Vec<T>` -> (ptr, len) via a boxed slice (len == number of elements).
pub(crate) fn vec_to_raw<T>(v: Vec<T>) -> (*mut T, usize) {
    if v.is_empty() {
        return (ptr::null_mut(), 0);
    }
    let boxed = v.into_boxed_slice();
    let len = boxed.len();
    (Box::into_raw(boxed) as *mut T, len)
}

/// list of strings -> (`*mut *mut c_char`, len).
fn strings_to_raw(v: &[String]) -> (*mut *mut c_char, usize) {
    let raw: Vec<*mut c_char> = v.iter().map(|s| to_c(s)).collect();
    vec_to_raw(raw)
}

fn empty_params() -> FFITrackMetadataParams {
    FFITrackMetadataParams {
        duration_s: 0,
        sample_rate: 0,
        bitrate: 0,
        listen_count: 0,
        cover_art: ptr::null_mut(),
    }
}

/// empty metadata (track not found / no data), carries only the identifier.
fn empty_metadata(has_id: bool, id: i64, path: *mut c_char) -> FFITrackMetadata {
    FFITrackMetadata {
        has_id,
        id,
        path,
        title: ptr::null_mut(),
        artists: ptr::null_mut(),
        artists_len: 0,
        album: ptr::null_mut(),
        genres: ptr::null_mut(),
        genres_len: 0,
        has_params: false,
        params: empty_params(),
        volume: 1.0,
        invalid: false,
        color: ptr::null_mut(),
        user_label: ptr::null_mut(),
        ignore_orphan: false,
    }
}

/// `TrackVirtual` -> `FFITrackMetadata`. The identifier (`id`/`path`) comes
/// from the track itself, the rest from the metadata (if loaded).
pub(crate) fn track_to_ffi(track: &TrackVirtual) -> FFITrackMetadata {
    let path = opt_to_c(track.get_path().map(|p| p.to_string_lossy()).as_deref());

    let Ok(meta) = track.get_metadata() else {
        let (has_id, id) = track.index_id().map_or((false, 0), |i| (true, i));
        return FFITrackMetadata {
            volume: track.volume,
            invalid: track.invalid,
            color: opt_to_c(track.color.as_deref()),
            user_label: opt_to_c(track.user_label.as_deref()),
            ignore_orphan: track.ignore_orphan,
            ..empty_metadata(has_id, id, path)
        };
    };

    let (has_id, id) = meta.id.map_or((false, 0), |i| (true, i));
    let (artists, artists_len) = strings_to_raw(&meta.artist);
    let (genres, genres_len) = strings_to_raw(&meta.genres);

    let (has_params, params) = match &meta.params {
        Some(p) => (
            true,
            FFITrackMetadataParams {
                duration_s: p.duration_sec,
                sample_rate: p.sample_rate,
                bitrate: p.bitrate,
                listen_count: p.listen_count,
                cover_art: opt_to_c(p.cover_art.as_ref().map(|c| c.to_string_lossy()).as_deref()),
            },
        ),
        None => (false, empty_params()),
    };

    FFITrackMetadata {
        has_id,
        id,
        path,
        title: to_c(&meta.title),
        artists,
        artists_len,
        album: opt_to_c(meta.album.as_deref()),
        genres,
        genres_len,
        has_params,
        params,
        volume: track.volume,
        invalid: track.invalid,
        color: opt_to_c(track.color.as_deref()),
        user_label: opt_to_c(track.user_label.as_deref()),
        ignore_orphan: track.ignore_orphan,
    }
}

// ───────────────────────────────── status ──────────────────────────────────

/// result of any `idx_*` function (except `idx_free_*`). `repr(u32)`: never
/// renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIIdxStatus {
    Ok = 0,
    /// `init` hasn't succeeded yet.
    NotInitialized = 1,
    /// a required pointer (argument or out-parameter) is null.
    NullPointer = 2,
    /// a string argument is not valid UTF-8.
    InvalidUtf8 = 3,
    /// the core runs without a db (`need_db == false`).
    NoDatabase = 4,
    /// no track / playlist with this id / path / name.
    NotFound = 5,
    /// a db query failed.
    ReadFailed = 6,
    /// a db write failed.
    WriteFailed = 7,
    /// indexing the directory failed (unreadable directory or db error).
    IndexFailed = 8,
    /// the core panicked (see `core_last_panic`).
    Panic = 9,
    /// an argument is out of range (e.g. an unknown orphan filter).
    InvalidArgument = 10,
}

impl Status for FFIIdxStatus {
    const OK: Self = Self::Ok;
    const NULL_POINTER: Self = Self::NullPointer;
    const PANIC: Self = Self::Panic;
}

impl From<CStrError> for FFIIdxStatus {
    fn from(e: CStrError) -> Self {
        match e {
            CStrError::Null => Self::NullPointer,
            CStrError::InvalidUtf8 => Self::InvalidUtf8,
        }
    }
}

/// runs `f` on the db under the lock.
fn with_db<R>(f: impl FnOnce(&storage::Db) -> Result<R, FFIIdxStatus>) -> Result<R, FFIIdxStatus> {
    let core = crate::core().ok_or(FFIIdxStatus::NotInitialized)?;
    f(core.get_db().ok_or(FFIIdxStatus::NoDatabase)?)
}

/// a db write under the lock -> status; a db error becomes `WriteFailed`.
fn write<R>(f: impl FnOnce(&storage::Db) -> Result<R, Box<dyn Error>>) -> FFIIdxStatus {
    status(with_db(|db| f(db).map_err(|_| FFIIdxStatus::WriteFailed)))
}

fn status<R>(res: Result<R, FFIIdxStatus>) -> FFIIdxStatus {
    match res {
        Ok(_) => FFIIdxStatus::Ok,
        Err(e) => e,
    }
}

/// a db read error -> `ReadFailed`.
fn read_err<E>(_: E) -> FFIIdxStatus {
    FFIIdxStatus::ReadFailed
}

/// writes a list into `out` (an empty list on error).
///
/// # Safety
/// `out` is null or valid for writes.
unsafe fn out_tracks(
    out: *mut FFITracksList,
    select: impl FnOnce(&storage::Db) -> Result<Vec<TrackVirtual>, FFIIdxStatus>,
) -> FFIIdxStatus {
    let empty = FFITracksList {
        ptr: ptr::null_mut(),
        len: 0,
    };
    unsafe {
        with_out(out, empty, || {
            let items: Vec<FFITrackMetadata> = with_db(select)?.iter().map(track_to_ffi).collect();
            let (ptr, len) = vec_to_raw(items);
            Ok(FFITracksList { ptr, len })
        })
    }
}

fn playlist_to_ffi(playlist: &Playlist) -> FFIPlaylist {
    let (tracks, tracks_len) = vec_to_raw(
        playlist
            .get_tracks()
            .iter()
            .filter_map(TrackVirtual::index_id)
            .collect::<Vec<i64>>(),
    );
    let (has_updated_at, updated_at) = playlist.get_updated_at().map_or((false, 0), |v| (true, v));
    let (has_created_at, created_at) = playlist.get_created_at().map_or((false, 0), |v| (true, v));

    FFIPlaylist {
        name: opt_to_c(playlist.get_name().as_deref()),
        tracks,
        tracks_len,
        cover_art: opt_to_c(playlist.get_cover_art().map(|p| p.to_string_lossy()).as_deref()),
        has_updated_at,
        updated_at,
        has_created_at,
        created_at,
    }
}

// ───────────────────────────────── API ─────────────────────────────────────

/// every library playlist into `out`. Free with `idx_free_playlists_list`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_get_playlists(out: *mut FFIPlaylistsList) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let empty = FFIPlaylistsList {
            ptr: ptr::null_mut(),
            len: 0,
        };
        unsafe {
            with_out(out, empty, || {
                let playlists = with_db(|db| db.get_playlists(None, None).map_err(read_err))?;
                let items: Vec<FFIPlaylist> = playlists.iter().map(playlist_to_ffi).collect();
                let (ptr, len) = vec_to_raw(items);
                Ok(FFIPlaylistsList { ptr, len })
            })
        }
    })
}

/// the whole library track pool (metadata) into `out`. Free with
/// `idx_free_tracks_list`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_get_tracks(out: *mut FFITracksList) -> FFIIdxStatus {
    crate::panic::guard(|| {
        unsafe { out_tracks(out, |db| db.get_tracks(None, None, None, None).map_err(read_err)) }
    })
}

/// which songs in no playlist `idx_get_orphan_tracks` lists. `repr(u32)`:
/// never renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIOrphanFilter {
    /// the ones not marked "ignore orphanhood" (the collector's default).
    NotIgnored = 0,
    /// only the marked ones.
    Ignored = 1,
    /// both.
    All = 2,
}

/// songs that are in no playlist into `out` (by title) — candidates for
/// cleanup; `filter` — an [`FFIOrphanFilter`] number (unknown ->
/// `InvalidArgument`). Free with `idx_free_tracks_list`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_get_orphan_tracks(filter: u32, out: *mut FFITracksList) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let filter = match filter {
            f if f == FFIOrphanFilter::NotIgnored as u32 => OrphanFilter::NotIgnored,
            f if f == FFIOrphanFilter::Ignored as u32 => OrphanFilter::Ignored,
            f if f == FFIOrphanFilter::All as u32 => OrphanFilter::All,
            _ => return FFIIdxStatus::InvalidArgument,
        };
        unsafe { out_tracks(out, |db| db.get_orphan_tracks(filter).map_err(read_err)) }
    })
}

/// the mark "ignore orphanhood": the song stays though it is in no playlist,
/// and the orphan collector doesn't list it by default.
#[unsafe(no_mangle)]
pub extern "C" fn idx_set_track_ignore_orphan(id: i64, ignore: bool) -> FFIIdxStatus {
    crate::panic::guard(|| write(|db| db.set_track_ignore_orphan(id, ignore)))
}

/// tracks of a playlist by its id into `out`. `NotFound` — no such playlist.
/// Free with `idx_free_tracks_list`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_get_playlist_tracks(
    id_playlist: i64,
    out: *mut FFITracksList,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        unsafe {
            out_tracks(out, |db| {
                let playlist = db
                    .get_playlists(None, Some(id_playlist))
                    .map_err(read_err)?
                    .into_iter()
                    .next()
                    .ok_or(FFIIdxStatus::NotFound)?;
                Ok(playlist.into_tracks())
            })
        }
    })
}

/// metadata of one track into `out` by identifier: `path` (if not null)
/// takes priority over `id`. `NotFound` — no such track. Free with
/// `idx_free_track_metadata`.
///
/// # Safety
/// `path` is either null or a valid C pointer to a nul-terminated string;
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_get_track(
    id_track: i64,
    path: *const c_char,
    out: *mut FFITrackMetadata,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let empty = empty_metadata(false, 0, ptr::null_mut());
        unsafe {
            with_out(out, empty, || {
                let path = opt_path(path)?;
                find_track(id_track, path.as_deref())?
                    .as_ref()
                    .map(track_to_ffi)
                    .ok_or(FFIIdxStatus::NotFound)
            })
        }
    })
}

/// a nullable C string -> optional path.
fn opt_path(path: *const c_char) -> Result<Option<PathBuf>, FFIIdxStatus> {
    match path.is_null() {
        true => Ok(None),
        false => Ok(Some(crate::ptr_to_path(path)?)),
    }
}

/// a db track by identifier: `path` (if given) takes priority over `id`.
fn find_track(id_track: i64, path: Option<&Path>) -> Result<Option<TrackVirtual>, FFIIdxStatus> {
    with_db(|db| {
        Ok(match path {
            // lookup by path (exact match on a unique column).
            Some(p) => db
                .get_tracks(None, None, None, None)
                .map_err(read_err)?
                .into_iter()
                .find(|t| t.get_path() == Some(p)),
            None => db
                .get_tracks(None, None, Some(id_track), None)
                .map_err(read_err)?
                .into_iter()
                .next(),
        })
    })
}

/// cover bytes of a track to display, by identifier (as in `idx_get_track`):
/// the cover file from the db if it is set and readable, otherwise the cover
/// embedded in the audio file tags. A `path` missing from the db is read
/// directly (embedded cover only). No cover -> `Ok` with `*out_ptr == null`,
/// `*out_len == 0`; an `id` missing from the db -> `NotFound`. The image is
/// returned as stored (png / jpg / ...). Free with `idx_free_bytes`.
///
/// # Safety
/// `path` is either null or a valid C string; `out_ptr`/`out_len` are valid
/// pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_get_track_cover(
    id_track: i64,
    path: *const c_char,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        if out_ptr.is_null() || out_len.is_null() {
            return FFIIdxStatus::NullPointer;
        }
        unsafe {
            *out_ptr = ptr::null_mut();
            *out_len = 0;
        }
        match track_cover(id_track, path) {
            Ok(Some(bytes)) => {
                let (p, l) = vec_to_raw(bytes);
                unsafe {
                    *out_ptr = p;
                    *out_len = l;
                }
                FFIIdxStatus::Ok
            }
            Ok(None) => FFIIdxStatus::Ok,
            Err(e) => e,
        }
    })
}

/// see `idx_get_track_cover`. Only the db lookup runs under the lock; files
/// (the cover, the audio tags) are read without it.
fn track_cover(id_track: i64, path: *const c_char) -> Result<Option<Vec<u8>>, FFIIdxStatus> {
    let path = opt_path(path)?;
    let (cover_file, audio) = match find_track(id_track, path.as_deref())? {
        Some(track) => {
            let cover = track
                .get_metadata()
                .ok()
                .and_then(|m| m.params.as_ref()?.cover_art.clone());
            (cover, track.get_path().map(Path::to_path_buf))
        }
        None if path.is_some() => (None, path),
        None => return Err(FFIIdxStatus::NotFound),
    };
    Ok(cover_file
        .and_then(|p| std::fs::read(p).ok())
        .or_else(|| TrackMetadata::read_cover(audio.as_deref()?)))
}

// ───────────────────────── writes (db mutations) ───────────────────────────

/// reads a `*const c_char` into `Option<String>` (null -> None).
///
/// # Safety
/// `ptr` is either null or a valid C pointer to a nul-terminated string.
pub(crate) unsafe fn c_to_opt_string(ptr: *const c_char) -> Result<Option<String>, CStrError> {
    match ptr.is_null() {
        true => Ok(None),
        false => unsafe { c_to_string(ptr) }.map(Some),
    }
}

/// reads a required `*const c_char` into `String` (null -> `Null`).
///
/// # Safety
/// `ptr` is either null or a valid C pointer to a nul-terminated string.
pub(crate) unsafe fn c_to_string(ptr: *const c_char) -> Result<String, CStrError> {
    if ptr.is_null() {
        return Err(CStrError::Null);
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Ok(s.to_owned()),
        Err(_) => Err(CStrError::InvalidUtf8),
    }
}

/// reads an array of C strings into `Vec<String>` (null/0 -> empty). A null
/// element is an error.
unsafe fn c_to_strings(ptr: *const *const c_char, len: usize) -> Result<Vec<String>, CStrError> {
    if ptr.is_null() || len == 0 {
        return Ok(Vec::new());
    }
    unsafe { std::slice::from_raw_parts(ptr, len) }
        .iter()
        .map(|&p| unsafe { c_to_string(p) })
        .collect()
}

/// reads an optional array of C strings: null -> `None` (`len` is ignored),
/// otherwise as in [`c_to_strings`] (len 0 -> an empty list).
unsafe fn c_to_opt_strings(
    ptr: *const *const c_char,
    len: usize,
) -> Result<Option<Vec<String>>, CStrError> {
    match ptr.is_null() {
        true => Ok(None),
        false => unsafe { c_to_strings(ptr, len) }.map(Some),
    }
}

/// indexes directory `dir` into the library. `dir` == null -> the music
/// directory from the configuration.
///
/// # Safety
/// `dir` is null or a valid C string pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_index_dir(dir: *const c_char) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let dir = match dir.is_null() {
            true => None,
            false => match crate::ptr_to_path(dir) {
                Ok(p) => Some(p),
                Err(e) => return e.into(),
            },
        };
        let Some(core) = crate::core() else {
            return FFIIdxStatus::NotInitialized;
        };
        let Some(db) = core.get_db() else {
            return FFIIdxStatus::NoDatabase;
        };
        let dir = dir.unwrap_or_else(|| core.get_dirs().music_dir.clone());
        match db.index_dir(&dir) {
            Ok(_) => FFIIdxStatus::Ok,
            Err(_) => FFIIdxStatus::IndexFailed,
        }
    })
}

/// edits the track tags: writes the given fields into the audio file of
/// track `id`, then mirrors them into the db (and dates the file, so it goes
/// out on sync again) — so a re-index reads the edit back from the file.
/// Every argument is optional: a null pointer -> the field is left as is.
/// - `title`: an empty string -> `InvalidArgument`;
/// - `artists` / `genres`: the list is rewritten entirely; non-null with
///   `*_len` == 0 clears it (`*_len` is ignored for a null pointer);
/// - `album`: an empty string clears it.
///
/// All null -> `Ok`, nothing is touched. An unknown `id` or a track without
/// a file -> `NotFound`; a failed tag write (missing / read-only file,
/// unsupported format) -> `WriteFailed`, the db is left as is. The file is
/// written without the db lock.
///
/// # Safety
/// `title` / `album` are null or C strings; `artists` / `genres` are null or
/// arrays of `artists_len` / `genres_len` C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_meta(
    id: i64,
    title: *const c_char,
    artists: *const *const c_char,
    artists_len: usize,
    album: *const c_char,
    genres: *const *const c_char,
    genres_len: usize,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let parsed = (|| {
            Ok::<_, CStrError>((
                unsafe { c_to_opt_string(title) }?,
                unsafe { c_to_opt_strings(artists, artists_len) }?,
                unsafe { c_to_opt_string(album) }?,
                unsafe { c_to_opt_strings(genres, genres_len) }?,
            ))
        })();
        let (title, artists, album, genres) = match parsed {
            Ok(v) => v,
            Err(e) => return e.into(),
        };
        if title.as_deref() == Some("") {
            return FFIIdxStatus::InvalidArgument;
        }
        set_tags(
            id,
            &TagEdit {
                title: title.as_deref(),
                artists: artists.as_deref(),
                album: album.as_deref().map(|a| (!a.is_empty()).then_some(a)),
                genres: genres.as_deref(),
            },
        )
    })
}

/// track album (null or empty -> clear); written into the file tags as in
/// `idx_set_track_meta` (same statuses).
///
/// # Safety
/// `album` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_album(id: i64, album: *const c_char) -> FFIIdxStatus {
    crate::panic::guard(|| {
        match unsafe { c_to_opt_string(album) } {
            Ok(album) => {
                let album = album.as_deref().filter(|a| !a.is_empty());
                set_tags(id, &TagEdit { album: Some(album), ..TagEdit::default() })
            }
            Err(e) => e.into(),
        }
    })
}

/// track genres (rewrite entirely; null/0 -> clear); written into the file
/// tags as in `idx_set_track_meta` (same statuses).
///
/// # Safety
/// `genres` is null or an array of `genres_len` C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_genres(
    id: i64,
    genres: *const *const c_char,
    genres_len: usize,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        match unsafe { c_to_strings(genres, genres_len) } {
            Ok(genres) => set_tags(id, &TagEdit { genres: Some(&genres), ..TagEdit::default() }),
            Err(e) => e.into(),
        }
    })
}

/// a tag edit of track `id`: the audio file first (without the db lock),
/// then the db. An empty edit touches nothing.
fn set_tags(id: i64, edit: &TagEdit) -> FFIIdxStatus {
    if edit.is_empty() {
        return FFIIdxStatus::Ok;
    }
    let audio = match find_track(id, None) {
        Ok(Some(t)) => match t.get_path() {
            Some(p) => p.to_path_buf(),
            None => return FFIIdxStatus::NotFound,
        },
        Ok(None) => return FFIIdxStatus::NotFound,
        Err(e) => return e,
    };
    match TrackMetadata::write_tags(&audio, edit) {
        Ok(()) => write(|db| db.set_track_tags(id, edit)),
        Err(_) => FFIIdxStatus::WriteFailed,
    }
}

/// track cover path (null -> clear).
///
/// # Safety
/// `cover_path` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_cover(id: i64, cover_path: *const c_char) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let cover = match cover_path.is_null() {
            true => None,
            false => match crate::ptr_to_path(cover_path) {
                Ok(p) => Some(p),
                Err(e) => return e.into(),
            },
        };
        write(|db| db.set_track_cover(id, cover.as_deref()))
    })
}

/// embeds the picture at `image_path` (png / jpg only) as the front cover
/// into the tags of the audio file of track `id`; in the db only the file is
/// dated as changed (see `idx_set_track_file_changed`).
/// An unknown `id` or a track without a file -> `NotFound`; an unreadable
/// file or a picture of another format (gif / webp too) -> `InvalidArgument`; a failed tag
/// write -> `WriteFailed`. The file is read and written without the db lock.
///
/// # Safety
/// `image_path` is a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_embed_track_cover(
    id: i64,
    image_path: *const c_char,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let image = match crate::ptr_to_path(image_path) {
            Ok(p) => p,
            Err(e) => return e.into(),
        };
        let audio = match find_track(id, None) {
            Ok(Some(t)) => match t.get_path() {
                Some(p) => p.to_path_buf(),
                None => return FFIIdxStatus::NotFound,
            },
            Ok(None) => return FFIIdxStatus::NotFound,
            Err(e) => return e,
        };
        let Ok(bytes) = std::fs::read(&image) else {
            return FFIIdxStatus::InvalidArgument;
        };
        match audio_structs::cover_art::detect_image_format(&bytes) {
            Ok(f) if f.embeddable() => {}
            _ => return FFIIdxStatus::InvalidArgument,
        }
        match TrackMetadata::write_cover(&audio, &bytes) {
            Ok(()) => write(|db| db.set_track_file_changed(id)),
            Err(_) => FFIIdxStatus::WriteFailed,
        }
    })
}

/// dates a change of the audio file of track `id` itself (tags, embedded
/// cover, ...): on sync the file goes to the other devices again and is
/// written over their copy. `idx_embed_track_cover` does it by itself; a
/// frontend that changes a file with its own means calls this after.
#[unsafe(no_mangle)]
pub extern "C" fn idx_set_track_file_changed(id: i64) -> FFIIdxStatus {
    crate::panic::guard(|| {
        write(|db| db.set_track_file_changed(id))
    })
}

/// track color label (null -> clear).
///
/// # Safety
/// `color` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_color(id: i64, color: *const c_char) -> FFIIdxStatus {
    crate::panic::guard(|| {
        match unsafe { c_to_opt_string(color) } {
            Ok(color) => write(|db| db.set_track_color(id, color.as_deref())),
            Err(e) => e.into(),
        }
    })
}

/// track text label (null -> clear).
///
/// # Safety
/// `label` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_label(id: i64, label: *const c_char) -> FFIIdxStatus {
    crate::panic::guard(|| {
        match unsafe { c_to_opt_string(label) } {
            Ok(label) => write(|db| db.set_track_label(id, label.as_deref())),
            Err(e) => e.into(),
        }
    })
}

/// the "file is invalid" mark.
#[unsafe(no_mangle)]
pub extern "C" fn idx_set_track_invalid(id: i64, invalid: bool) -> FFIIdxStatus {
    crate::panic::guard(|| {
        write(|db| db.set_track_invalid(id, invalid))
    })
}

/// the "file is invalid" mark of several tracks in one transaction; unknown
/// ids are skipped.
///
/// # Safety
/// `ids` is null or an array of length `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_tracks_invalid(
    ids: *const i64,
    len: usize,
    invalid: bool,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let Some(ids) = (unsafe { crate::cdc::raw_slice(ids, len) }) else {
            return FFIIdxStatus::NullPointer;
        };
        write(|db| db.set_tracks_invalid(ids, invalid))
    })
}

/// adds the song files `paths` to the library in one transaction (the
/// frontend finds them itself, see `idx_index_dir` for the core's own walk).
/// Files that aren't music or can't be read are skipped; the number written
/// goes to `out`.
///
/// # Safety
/// `paths` is null or an array of `len` valid C strings (none null); `out` is
/// a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_add_tracks(
    paths: *const *const c_char,
    len: usize,
    out: *mut u64,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        unsafe {
            with_out(out, 0, || {
                let raw = crate::cdc::raw_slice(paths, len).ok_or(FFIIdxStatus::NullPointer)?;
                let paths = raw
                    .iter()
                    .map(|&p| crate::ptr_to_path(p))
                    .collect::<Result<Vec<_>, _>>()?;
                with_db(|db| {
                    db.add_tracks(&paths)
                        .map(|n| n as u64)
                        .map_err(|_| FFIIdxStatus::WriteFailed)
                })
            })
        }
    })
}

/// saved track volume.
#[unsafe(no_mangle)]
pub extern "C" fn idx_set_track_volume(id: i64, volume: f32) -> FFIIdxStatus {
    crate::panic::guard(|| {
        write(|db| db.set_track_volume_id(id, volume))
    })
}

/// new track file path (after a rename on disk). `path` != null.
///
/// # Safety
/// `path` is a valid C string pointer (not null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_path(id: i64, path: *const c_char) -> FFIIdxStatus {
    crate::panic::guard(|| {
        match crate::ptr_to_path(path) {
            Ok(path) => write(|db| db.set_track_path(id, &path)),
            Err(e) => e.into(),
        }
    })
}

/// removes a track from the index by id.
#[unsafe(no_mangle)]
pub extern "C" fn idx_remove_track(id: i64) -> FFIIdxStatus {
    crate::panic::guard(|| {
        write(|db| db.remove_track(id))
    })
}

/// removes every track marked "invalid"; the number removed goes to `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_remove_invalid_tracks(out: *mut u64) -> FFIIdxStatus {
    crate::panic::guard(|| {
        unsafe {
            with_out(out, 0, || {
                with_db(|db| {
                    db.remove_invalid_tracks()
                        .map(|n| n as u64)
                        .map_err(|_| FFIIdxStatus::WriteFailed)
                })
            })
        }
    })
}

/// creates / updates playlist `name` from the tracks with the given ids (in
/// this order; ids missing from the db are skipped). `cover` is null or a
/// cover path.
///
/// # Safety
/// `name` is a valid C string (not null); `track_ids` is null or an array of
/// length `track_ids_len`; `cover` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_save_playlist(
    name: *const c_char,
    track_ids: *const i64,
    track_ids_len: usize,
    cover: *const c_char,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let name = match unsafe { c_to_string(name) } {
            Ok(n) => n,
            Err(e) => return e.into(),
        };
        let Some(ids) = (unsafe { crate::cdc::raw_slice(track_ids, track_ids_len) }) else {
            return FFIIdxStatus::NullPointer;
        };
        let cover = match unsafe { c_to_opt_string(cover) } {
            Ok(c) => c,
            Err(e) => return e.into(),
        };

        status(with_db(|db| {
            // assemble the playlist tracks by id in the given order.
            let mut tracks = Vec::with_capacity(ids.len());
            for &id in ids {
                let found = db.get_tracks(None, None, Some(id), None).map_err(read_err)?;
                if let Some(t) = found.into_iter().next() {
                    tracks.push(t);
                }
            }
            let mut playlist = Playlist::from_tracks(tracks);
            playlist.set_name(name);
            playlist.set_cover_art(cover.map(Into::into));
            db.save_playlist(playlist)
                .map_err(|_| FFIIdxStatus::WriteFailed)
        }))
    })
}

/// cover path of playlist `name` (null -> clear), without touching its
/// tracks. It is a change of the playlist: the cover syncs. `NotFound` — no
/// such playlist (or it is deleted).
///
/// # Safety
/// `name` is a valid C string (not null); `cover` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_playlist_cover(
    name: *const c_char,
    cover: *const c_char,
) -> FFIIdxStatus {
    crate::panic::guard(|| {
        let name = match unsafe { c_to_string(name) } {
            Ok(n) => n,
            Err(e) => return e.into(),
        };
        let cover = match cover.is_null() {
            true => None,
            false => match crate::ptr_to_path(cover) {
                Ok(p) => Some(p),
                Err(e) => return e.into(),
            },
        };
        status(with_db(|db| {
            match db.set_playlist_cover(&name, cover.as_deref()) {
                Ok(true) => Ok(()),
                Ok(false) => Err(FFIIdxStatus::NotFound),
                Err(_) => Err(FFIIdxStatus::WriteFailed),
            }
        }))
    })
}

/// deletes a playlist by name — softly: it disappears from the playlists,
/// its mark with the deletion time stays and reaches other devices on sync /
/// in a library packet (v0.11+). Saving a playlist with this name again
/// brings it back.
///
/// # Safety
/// `name` is a valid C string (not null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_remove_playlist(name: *const c_char) -> FFIIdxStatus {
    crate::panic::guard(|| {
        match unsafe { c_to_string(name) } {
            Ok(name) => write(|db| db.remove_playlist(&name)),
            Err(e) => e.into(),
        }
    })
}
