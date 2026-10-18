//! safe wrapper over the core index (`idx_*`). Every call marshals strings to
//! C, reads the returned FFI structs into owning [`super::types`] and frees
//! the source buffers right away with the paired `idx_free_*`.

use std::ffi::CString;
use std::ptr;

use api::idx::{self, FFIIdxStatus};
use api::mem;

use super::types::{PlaylistInfo, TrackMeta};

/// `&str` -> owned `CString` (to pass to FFI). Drops data after a NUL.
fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// turns `Option<&str>` into a pointer (null for None); keeps the `CString` alive.
fn opt_cstr(s: Option<&str>) -> Option<CString> {
    s.map(cstr)
}

fn as_ptr(c: &Option<CString>) -> *const std::os::raw::c_char {
    c.as_ref().map_or(ptr::null(), |c| c.as_ptr())
}

/// `true` for `Ok`. The TUI shows failed writes as "not saved"; the exact
/// code is not shown.
fn ok(st: FFIIdxStatus) -> bool {
    st == FFIIdxStatus::Ok
}

// ───────────────────────────── reads ─────────────────────────────

/// every library playlist (empty on error).
pub fn get_playlists() -> Vec<PlaylistInfo> {
    let mut list = idx::FFIPlaylistsList {
        ptr: ptr::null_mut(),
        len: 0,
    };
    if !ok(unsafe { idx::idx_get_playlists(&mut list) }) {
        return Vec::new();
    }
    let out = if list.ptr.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(list.ptr, list.len) }
            .iter()
            .map(|p| unsafe { PlaylistInfo::from_ffi(p) })
            .collect()
    };
    unsafe { mem::idx_free_playlists_list(list) };
    out
}

/// the whole library track pool (empty on error).
pub fn get_tracks() -> Vec<TrackMeta> {
    collect_tracks(|out| unsafe { idx::idx_get_tracks(out) })
}

/// which songs in no playlist [`get_orphan_tracks`] lists.
pub use api::idx::FFIOrphanFilter as OrphanFilter;

/// songs that are in no playlist (`None` — error).
pub fn get_orphan_tracks(filter: OrphanFilter) -> Option<Vec<TrackMeta>> {
    let mut failed = false;
    let tracks = collect_tracks(|out| {
        let st = unsafe { idx::idx_get_orphan_tracks(filter as u32, out) };
        failed = st != FFIIdxStatus::Ok;
        st
    });
    (!failed).then_some(tracks)
}

/// the mark "ignore orphanhood" (the orphan collector skips the song).
pub fn set_track_ignore_orphan(id: i64, ignore: bool) -> bool {
    ok(idx::idx_set_track_ignore_orphan(id, ignore))
}

/// deletes a song from the device: its file (already gone is fine), then its
/// library row. The error — what failed, for the status line.
pub fn delete_from_device(track: &TrackMeta) -> Result<(), String> {
    if let Some(path) = &track.path {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{path}: {e}")),
        }
    }
    match track.id {
        Some(id) if !remove_track(id) => Err(format!("{}: file deleted, library row kept", track.title)),
        _ => Ok(()),
    }
}

/// tracks of a playlist by its id, in playlist order (empty on error).
pub fn get_playlist_tracks(id: i64) -> Vec<TrackMeta> {
    collect_tracks(|out| unsafe { idx::idx_get_playlist_tracks(id, out) })
}

/// metadata of one track by id or path (the path wins if given).
pub fn get_track(id: i64, path: Option<&str>) -> Option<TrackMeta> {
    let path_c = opt_cstr(path);
    let mut meta = std::mem::MaybeUninit::uninit();
    if !ok(unsafe { idx::idx_get_track(id, as_ptr(&path_c), meta.as_mut_ptr()) }) {
        // on error the core still writes empty metadata; nothing to free.
        return None;
    }
    let meta = unsafe { meta.assume_init() };
    let out = unsafe { TrackMeta::from_ffi(&meta) };
    unsafe { mem::idx_free_track_metadata(meta) };
    Some(out)
}

/// common `FFITracksList` -> owning Vec conversion, freeing the buffer.
fn collect_tracks(get: impl FnOnce(*mut idx::FFITracksList) -> FFIIdxStatus) -> Vec<TrackMeta> {
    let mut list = idx::FFITracksList {
        ptr: ptr::null_mut(),
        len: 0,
    };
    if !ok(get(&mut list)) {
        return Vec::new();
    }
    let out = if list.ptr.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(list.ptr, list.len) }
            .iter()
            .map(|m| unsafe { TrackMeta::from_ffi(m) })
            .collect()
    };
    unsafe { mem::idx_free_tracks_list(list) };
    out
}

// ───────────────────────────── writes ─────────────────────────────

/// indexes a directory (None -> the music directory from the configuration).
pub fn index_dir(dir: Option<&str>) -> bool {
    let c = opt_cstr(dir);
    ok(unsafe { idx::idx_index_dir(as_ptr(&c)) })
}

/// edits the title and/or the artist list (None — leave the field alone).
pub fn set_track_meta(id: i64, title: Option<&str>, artists: Option<&[String]>) -> bool {
    let title_c = opt_cstr(title);
    // keep the artist array alive together with the pointers into it.
    let artist_cs: Option<Vec<CString>> =
        artists.map(|a| a.iter().map(|s| cstr(s)).collect());
    let artist_ptrs: Option<Vec<*const std::os::raw::c_char>> =
        artist_cs.as_ref().map(|v| v.iter().map(|c| c.as_ptr()).collect());
    let (aptr, alen) = match &artist_ptrs {
        Some(v) => (v.as_ptr(), v.len()),
        None => (ptr::null(), 0),
    };
    ok(unsafe { idx::idx_set_track_meta(id, as_ptr(&title_c), aptr, alen) })
}

/// album (None -> clear).
pub fn set_track_album(id: i64, album: Option<&str>) -> bool {
    let c = opt_cstr(album);
    ok(unsafe { idx::idx_set_track_album(id, as_ptr(&c)) })
}

/// genres (rewrite entirely; empty -> clear).
pub fn set_track_genres(id: i64, genres: &[String]) -> bool {
    let cs: Vec<CString> = genres.iter().map(|s| cstr(s)).collect();
    let ptrs: Vec<*const std::os::raw::c_char> = cs.iter().map(|c| c.as_ptr()).collect();
    ok(unsafe { idx::idx_set_track_genres(id, ptrs.as_ptr(), ptrs.len()) })
}

/// cover path (None -> clear).
pub fn set_track_cover(id: i64, cover_path: Option<&str>) -> bool {
    let c = opt_cstr(cover_path);
    ok(unsafe { idx::idx_set_track_cover(id, as_ptr(&c)) })
}

/// color label (None -> clear).
pub fn set_track_color(id: i64, color: Option<&str>) -> bool {
    let c = opt_cstr(color);
    ok(unsafe { idx::idx_set_track_color(id, as_ptr(&c)) })
}

/// text label (None -> clear).
pub fn set_track_label(id: i64, label: Option<&str>) -> bool {
    let c = opt_cstr(label);
    ok(unsafe { idx::idx_set_track_label(id, as_ptr(&c)) })
}

pub fn set_track_invalid(id: i64, invalid: bool) -> bool {
    ok(idx::idx_set_track_invalid(id, invalid))
}

pub fn set_track_volume(id: i64, volume: f32) -> bool {
    ok(idx::idx_set_track_volume(id, volume))
}

/// new track file path.
pub fn set_track_path(id: i64, path: &str) -> bool {
    let c = cstr(path);
    ok(unsafe { idx::idx_set_track_path(id, c.as_ptr()) })
}

pub fn remove_track(id: i64) -> bool {
    ok(idx::idx_remove_track(id))
}

/// removes every invalid track; returns the number removed (None — error).
pub fn remove_invalid_tracks() -> Option<usize> {
    let mut n = 0u64;
    ok(unsafe { idx::idx_remove_invalid_tracks(&mut n) }).then_some(n as usize)
}

/// creates / updates playlist `name` from tracks `ids` (in this order).
pub fn save_playlist(name: &str, ids: &[i64], cover: Option<&str>) -> bool {
    let name_c = cstr(name);
    let cover_c = opt_cstr(cover);
    let (iptr, ilen) = if ids.is_empty() {
        (ptr::null(), 0)
    } else {
        (ids.as_ptr(), ids.len())
    };
    ok(unsafe { idx::idx_save_playlist(name_c.as_ptr(), iptr, ilen, as_ptr(&cover_c)) })
}

/// removes a playlist by name.
pub fn remove_playlist(name: &str) -> bool {
    let c = cstr(name);
    ok(unsafe { idx::idx_remove_playlist(c.as_ptr()) })
}
