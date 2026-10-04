//! безопасная обёртка над индексом ядра (`idx_*`). Все вызовы маршалят строки в
//! C, читают возвращённые FFI-структуры в владеющие [`super::types`] и тут же
//! освобождают исходные буферы парными `idx_free_*`.

use std::ffi::CString;
use std::ptr;

use api::idx;
use api::mem;

use super::types::{PlaylistInfo, TrackMeta};

/// `&str` -> owned `CString` (для передачи в FFI). Теряет данные после NUL.
fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// превращает `Option<&str>` в указатель (null для None); хранит `CString` живым.
fn opt_cstr(s: Option<&str>) -> Option<CString> {
    s.map(cstr)
}

fn as_ptr(c: &Option<CString>) -> *const std::os::raw::c_char {
    c.as_ref().map_or(ptr::null(), |c| c.as_ptr())
}

// ───────────────────────────── reads ─────────────────────────────

/// все плейлисты библиотеки.
pub fn get_playlists() -> Vec<PlaylistInfo> {
    let list = idx::idx_get_playlists();
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

/// весь пул треков библиотеки.
pub fn get_tracks() -> Vec<TrackMeta> {
    collect_tracks(idx::idx_get_tracks())
}

/// треки плейлиста по его id (в порядке плейлиста).
pub fn get_playlist_tracks(id: i64) -> Vec<TrackMeta> {
    collect_tracks(idx::idx_get_playlist_tracks(id))
}

/// метадата одного трека по id или пути (путь приоритетнее, если задан).
pub fn get_track(id: i64, path: Option<&str>) -> Option<TrackMeta> {
    let path_c = opt_cstr(path);
    let meta = unsafe { idx::idx_get_track(id, as_ptr(&path_c)) };
    let found = meta.has_id || !meta.title.is_null() || !meta.path.is_null();
    let out = found.then(|| unsafe { TrackMeta::from_ffi(&meta) });
    unsafe { mem::idx_free_track_metadata(meta) };
    out
}

/// общий сбор `FFITracksList` -> владеющий Vec с освобождением буфера.
fn collect_tracks(list: idx::FFITracksList) -> Vec<TrackMeta> {
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

/// индексирует каталог (None -> каталог музыки из конфигурации).
pub fn index_dir(dir: Option<&str>) -> bool {
    let c = opt_cstr(dir);
    unsafe { idx::idx_index_dir(as_ptr(&c)) }
}

/// правит заголовок и/или список артистов (None — поле не трогаем).
pub fn set_track_meta(id: i64, title: Option<&str>, artists: Option<&[String]>) -> bool {
    let title_c = opt_cstr(title);
    // массив артистов держим живым вместе с указателями на него.
    let artist_cs: Option<Vec<CString>> =
        artists.map(|a| a.iter().map(|s| cstr(s)).collect());
    let artist_ptrs: Option<Vec<*const std::os::raw::c_char>> =
        artist_cs.as_ref().map(|v| v.iter().map(|c| c.as_ptr()).collect());
    let (aptr, alen) = match &artist_ptrs {
        Some(v) => (v.as_ptr(), v.len()),
        None => (ptr::null(), 0),
    };
    unsafe { idx::idx_set_track_meta(id, as_ptr(&title_c), aptr, alen) }
}

/// альбом (None -> очистить).
pub fn set_track_album(id: i64, album: Option<&str>) -> bool {
    let c = opt_cstr(album);
    unsafe { idx::idx_set_track_album(id, as_ptr(&c)) }
}

/// жанры (переписать целиком; пусто -> очистить).
pub fn set_track_genres(id: i64, genres: &[String]) -> bool {
    let cs: Vec<CString> = genres.iter().map(|s| cstr(s)).collect();
    let ptrs: Vec<*const std::os::raw::c_char> = cs.iter().map(|c| c.as_ptr()).collect();
    unsafe { idx::idx_set_track_genres(id, ptrs.as_ptr(), ptrs.len()) }
}

/// путь к обложке (None -> очистить).
pub fn set_track_cover(id: i64, cover_path: Option<&str>) -> bool {
    let c = opt_cstr(cover_path);
    unsafe { idx::idx_set_track_cover(id, as_ptr(&c)) }
}

/// цветовая метка (None -> очистить).
pub fn set_track_color(id: i64, color: Option<&str>) -> bool {
    let c = opt_cstr(color);
    unsafe { idx::idx_set_track_color(id, as_ptr(&c)) }
}

/// текстовая метка (None -> очистить).
pub fn set_track_label(id: i64, label: Option<&str>) -> bool {
    let c = opt_cstr(label);
    unsafe { idx::idx_set_track_label(id, as_ptr(&c)) }
}

pub fn set_track_invalid(id: i64, invalid: bool) -> bool {
    idx::idx_set_track_invalid(id, invalid)
}

pub fn set_track_volume(id: i64, volume: f32) -> bool {
    idx::idx_set_track_volume(id, volume)
}

/// новый путь к файлу трека.
pub fn set_track_path(id: i64, path: &str) -> bool {
    let c = cstr(path);
    unsafe { idx::idx_set_track_path(id, c.as_ptr()) }
}

pub fn remove_track(id: i64) -> bool {
    idx::idx_remove_track(id)
}

/// удаляет все недействительные треки; возвращает число удалённых (None — ошибка).
pub fn remove_invalid_tracks() -> Option<usize> {
    match idx::idx_remove_invalid_tracks() {
        n if n >= 0 => Some(n as usize),
        _ => None,
    }
}

/// создаёт/обновляет плейлист `name` из треков `ids` (в этом порядке).
pub fn save_playlist(name: &str, ids: &[i64], cover: Option<&str>) -> bool {
    let name_c = cstr(name);
    let cover_c = opt_cstr(cover);
    let (iptr, ilen) = if ids.is_empty() {
        (ptr::null(), 0)
    } else {
        (ids.as_ptr(), ids.len())
    };
    unsafe { idx::idx_save_playlist(name_c.as_ptr(), iptr, ilen, as_ptr(&cover_c)) }
}

/// удаляет плейлист по имени.
pub fn remove_playlist(name: &str) -> bool {
    let c = cstr(name);
    unsafe { idx::idx_remove_playlist(c.as_ptr()) }
}
