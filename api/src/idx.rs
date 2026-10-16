//! FFI over the index (sqlite). Prefix `idx_` = db operations.
//! Every returned pointer / struct owns its memory and must be freed with the
//! paired `idx_free_*` from [`crate::mem`].

use std::error::Error;
use std::ffi::{CStr, CString, c_char};
use std::ptr;

use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};
use storage::traits::indexator::Indexator;

use super::GLOBAL_STATE;

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
    }
}

/// `TrackVirtual` -> `FFITrackMetadata`. The identifier (`id`/`path`) comes
/// from the track itself, the rest from the metadata (if loaded).
pub(crate) fn track_to_ffi(track: &TrackVirtual) -> FFITrackMetadata {
    let path = opt_to_c(track.get_path().map(|p| p.to_string_lossy()).as_deref());

    let Ok(meta) = track.get_metadata() else {
        let (has_id, id) = track.index_id().map_or((false, 0), |i| (true, i));
        return empty_metadata(has_id, id, path);
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
    }
}

// ───────────────────────────────── API ─────────────────────────────────────

/// every library playlist. Free with `idx_free_playlists_list`.
#[unsafe(no_mangle)]
pub extern "C" fn idx_get_playlists() -> FFIPlaylistsList {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let playlists = guard
        .get_db()
        .and_then(|db| db.get_playlists(None, None).ok())
        .unwrap_or_default();

    let items: Vec<FFIPlaylist> = playlists
        .iter()
        .map(|playlist| {
            let (tracks, tracks_len) = vec_to_raw(
                playlist
                    .get_tracks()
                    .iter()
                    .filter_map(TrackVirtual::index_id)
                    .collect::<Vec<i64>>(),
            );
            let (has_updated_at, updated_at) =
                playlist.get_updated_at().map_or((false, 0), |v| (true, v));
            let (has_created_at, created_at) =
                playlist.get_created_at().map_or((false, 0), |v| (true, v));

            FFIPlaylist {
                name: opt_to_c(playlist.get_name().as_deref()),
                tracks,
                tracks_len,
                cover_art: opt_to_c(
                    playlist.get_cover_art().map(|p| p.to_string_lossy()).as_deref(),
                ),
                has_updated_at,
                updated_at,
                has_created_at,
                created_at,
            }
        })
        .collect();

    let (ptr, len) = vec_to_raw(items);
    FFIPlaylistsList { ptr, len }
}

/// the whole library track pool (metadata). Free with `idx_free_tracks_list`.
#[unsafe(no_mangle)]
pub extern "C" fn idx_get_tracks() -> FFITracksList {
    tracks_list(|db| db.get_tracks(None, None, None, None).ok())
}

/// tracks of a playlist by its id. Free with `idx_free_tracks_list`.
#[unsafe(no_mangle)]
pub extern "C" fn idx_get_playlist_tracks(id_playlist: i64) -> FFITracksList {
    tracks_list(|db| {
        let playlist = db.get_playlists(None, Some(id_playlist)).ok()?.into_iter().next()?;
        Some(playlist.into_tracks())
    })
}

/// metadata of one track by identifier: `path` (if not null) takes priority
/// over `id`. The result fields are zero if the track isn't found.
///
/// # Safety
/// `path` is either null or a valid C pointer to a nul-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_get_track(id_track: i64, path: *const c_char) -> FFITrackMetadata {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let found = guard.get_db().and_then(|db| {
        if !path.is_null()
            && let Ok(p) = crate::ptr_to_path(path)
        {
            // lookup by path (exact match on a unique column).
            db.get_tracks(None, None, None, None)
                .ok()?
                .into_iter()
                .find(|t| t.get_path() == Some(p.as_path()))
        } else {
            db.get_tracks(None, None, Some(id_track), None)
                .ok()?
                .into_iter()
                .next()
        }
    });

    match found {
        Some(track) => track_to_ffi(&track),
        None => empty_metadata(false, 0, ptr::null_mut()),
    }
}

/// common assembly of an `FFITracksList` from a track selection.
fn tracks_list(select: impl FnOnce(&storage::Db) -> Option<Vec<TrackVirtual>>) -> FFITracksList {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let items: Vec<FFITrackMetadata> = guard
        .get_db()
        .and_then(select)
        .unwrap_or_default()
        .iter()
        .map(track_to_ffi)
        .collect();
    let (ptr, len) = vec_to_raw(items);
    FFITracksList { ptr, len }
}

// ───────────────────────── writes (db mutations) ───────────────────────────

/// reads a `*const c_char` into `Option<String>` (null -> None).
///
/// # Safety
/// `ptr` is either null or a valid C pointer to a nul-terminated string.
pub(crate) unsafe fn c_to_opt_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok().map(str::to_owned)
}

/// reads an array of C strings into `Vec<String>` (null/0 -> empty).
unsafe fn c_to_strings(ptr: *const *const c_char, len: usize) -> Vec<String> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, len) }
        .iter()
        .filter_map(|&p| unsafe { c_to_opt_string(p) })
        .collect()
}

/// runs an operation on the db under the lock; `None` if there is no db or on
/// error.
fn with_db<R>(f: impl FnOnce(&storage::Db) -> Result<R, Box<dyn Error>>) -> Option<R> {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    f(guard.get_db()?).ok()
}

/// indexes directory `dir` into the library. `dir` == null -> the music
/// directory from the configuration. Returns `true` on success.
///
/// # Safety
/// `dir` is null or a valid C string pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_index_dir(dir: *const c_char) -> bool {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let Some(db) = guard.get_db() else {
        return false;
    };
    let dir = if dir.is_null() {
        guard.get_dirs().music_dir.clone()
    } else {
        match crate::ptr_to_path(dir) {
            Ok(p) => p,
            Err(_) => return false,
        }
    };
    db.index_dir(&dir).is_ok()
}

/// edits track metadata: `title` and/or the artist list. A null field is left
/// alone (for artists: `artists` == null -> don't change; otherwise rewrite
/// entirely).
///
/// # Safety
/// `title` is null or a C string; `artists` is null or an array of
/// `artists_len` C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_meta(
    id: i64,
    title: *const c_char,
    artists: *const *const c_char,
    artists_len: usize,
) -> bool {
    let title = unsafe { c_to_opt_string(title) };
    let artists =
        (!artists.is_null()).then(|| unsafe { c_to_strings(artists, artists_len) });
    with_db(|db| db.set_track_meta(id, title.as_deref(), artists.as_deref())).is_some()
}

/// track album (null -> clear).
///
/// # Safety
/// `album` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_album(id: i64, album: *const c_char) -> bool {
    let album = unsafe { c_to_opt_string(album) };
    with_db(|db| db.set_track_album(id, album.as_deref())).is_some()
}

/// track genres (rewrite entirely; null/0 -> clear).
///
/// # Safety
/// `genres` is null or an array of `genres_len` C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_genres(
    id: i64,
    genres: *const *const c_char,
    genres_len: usize,
) -> bool {
    let genres = unsafe { c_to_strings(genres, genres_len) };
    with_db(|db| db.set_track_genres(id, &genres)).is_some()
}

/// track cover path (null -> clear).
///
/// # Safety
/// `cover_path` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_cover(id: i64, cover_path: *const c_char) -> bool {
    let cover = if cover_path.is_null() {
        None
    } else {
        match crate::ptr_to_path(cover_path) {
            Ok(p) => Some(p),
            Err(_) => return false,
        }
    };
    with_db(|db| db.set_track_cover(id, cover.as_deref())).is_some()
}

/// track color label (null -> clear).
///
/// # Safety
/// `color` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_color(id: i64, color: *const c_char) -> bool {
    let color = unsafe { c_to_opt_string(color) };
    with_db(|db| db.set_track_color(id, color.as_deref())).is_some()
}

/// track text label (null -> clear).
///
/// # Safety
/// `label` is null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_label(id: i64, label: *const c_char) -> bool {
    let label = unsafe { c_to_opt_string(label) };
    with_db(|db| db.set_track_label(id, label.as_deref())).is_some()
}

/// the "file is invalid" mark.
#[unsafe(no_mangle)]
pub extern "C" fn idx_set_track_invalid(id: i64, invalid: bool) -> bool {
    with_db(|db| db.set_track_invalid(id, invalid)).is_some()
}

/// saved track volume.
#[unsafe(no_mangle)]
pub extern "C" fn idx_set_track_volume(id: i64, volume: f32) -> bool {
    with_db(|db| db.set_track_volume_id(id, volume)).is_some()
}

/// new track file path (after a rename on disk). `path` != null.
///
/// # Safety
/// `path` is a valid C string pointer (not null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_set_track_path(id: i64, path: *const c_char) -> bool {
    let Ok(path) = crate::ptr_to_path(path) else {
        return false;
    };
    with_db(|db| db.set_track_path(id, &path)).is_some()
}

/// removes a track from the index by id.
#[unsafe(no_mangle)]
pub extern "C" fn idx_remove_track(id: i64) -> bool {
    with_db(|db| db.remove_track(id)).is_some()
}

/// removes every track marked "invalid". Returns the number removed (`-1` on
/// error / no db).
#[unsafe(no_mangle)]
pub extern "C" fn idx_remove_invalid_tracks() -> i64 {
    with_db(|db| db.remove_invalid_tracks()).map_or(-1, |n| n as i64)
}

/// creates / updates playlist `name` from the tracks with the given ids (in
/// this order). `cover` is null or a cover path.
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
) -> bool {
    let Some(name) = (unsafe { c_to_opt_string(name) }) else {
        return false;
    };
    let ids: &[i64] = if track_ids.is_null() || track_ids_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(track_ids, track_ids_len) }
    };
    let cover = unsafe { c_to_opt_string(cover) };

    with_db(|db| {
        // assemble the playlist tracks by id in the given order.
        let mut tracks = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Some(t) = db.get_tracks(None, None, Some(id), None)?.into_iter().next() {
                tracks.push(t);
            }
        }
        let mut playlist = Playlist::from_tracks(tracks);
        playlist.set_name(name);
        playlist.set_cover_art(cover.map(Into::into));
        db.save_playlist(playlist)
    })
    .is_some()
}

/// removes a playlist by name.
///
/// # Safety
/// `name` is a valid C string (not null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_remove_playlist(name: *const c_char) -> bool {
    let Some(name) = (unsafe { c_to_opt_string(name) }) else {
        return false;
    };
    with_db(|db| db.remove_playlist(&name)).is_some()
}
