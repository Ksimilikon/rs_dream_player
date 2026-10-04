//! освобождение памяти, выданной через FFI. На каждый владеющий тип — свой
//! `idx_free_*`. Все функции идемпотентны к null и забирают владение переданным
//! буфером (повторный вызов — UB).

use std::ffi::{CString, c_char};

use crate::idx::{
    FFIPlaylist, FFIPlaylistsList, FFIStringList, FFITrackMetadata, FFITracksList,
};

/// освобождает одну C-строку, ранее выданную ядром. Null игнорируется.
///
/// # Safety
/// `s` — либо null, либо указатель, полученный из `idx_*` и ещё не освобождённый.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

/// освобождает `*mut *mut c_char` длины `len` вместе со всеми строками.
unsafe fn free_string_array(ptr: *mut *mut c_char, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(ptr, len)) };
    for s in slice.iter() {
        unsafe { idx_free_string(*s) };
    }
}

/// освобождает содержимое метадаты (строки и массивы). Сама структура обычно
/// лежит на стеке вызывающего либо внутри буфера списка.
unsafe fn free_metadata_fields(meta: &FFITrackMetadata) {
    unsafe {
        idx_free_string(meta.path);
        idx_free_string(meta.title);
        idx_free_string(meta.album);
        idx_free_string(meta.params.cover_art);
        free_string_array(meta.artists, meta.artists_len);
        free_string_array(meta.genres, meta.genres_len);
    }
}

/// освобождает метадату трека (из `idx_get_track`).
///
/// # Safety
/// `meta` получена из `idx_*` и ещё не освобождалась.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_track_metadata(meta: FFITrackMetadata) {
    unsafe { free_metadata_fields(&meta) };
}

/// освобождает список треков (из `idx_get_tracks`/`idx_get_playlist_tracks`).
///
/// # Safety
/// `list` получен из `idx_*` и ещё не освобождался.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_tracks_list(list: FFITracksList) {
    if list.ptr.is_null() {
        return;
    }
    let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(list.ptr, list.len)) };
    for meta in slice.iter() {
        unsafe { free_metadata_fields(meta) };
    }
}

/// освобождает содержимое одного плейлиста (строки и массив id).
unsafe fn free_playlist_fields(pl: &FFIPlaylist) {
    unsafe {
        idx_free_string(pl.name);
        idx_free_string(pl.cover_art);
        if !pl.tracks.is_null() {
            drop(Box::from_raw(std::slice::from_raw_parts_mut(
                pl.tracks,
                pl.tracks_len,
            )));
        }
    }
}

/// освобождает список строк (из `engine_get_output_devices`).
///
/// # Safety
/// `list` получен из `engine_*` и ещё не освобождался.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_free_output_devices(list: FFIStringList) {
    unsafe { free_string_array(list.ptr, list.len) };
}

/// освобождает список плейлистов (из `idx_get_playlists`).
///
/// # Safety
/// `list` получен из `idx_*` и ещё не освобождался.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_playlists_list(list: FFIPlaylistsList) {
    if list.ptr.is_null() {
        return;
    }
    let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(list.ptr, list.len)) };
    for pl in slice.iter() {
        unsafe { free_playlist_fields(pl) };
    }
}
