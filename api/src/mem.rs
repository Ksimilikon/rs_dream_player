//! freeing memory handed out via FFI. Each owning type has its own
//! `idx_free_*`. Every function is idempotent for null and takes ownership of
//! the passed buffer (a second call is UB).

use std::ffi::{CString, c_char};

use crate::cdc::FFIPacket;
use crate::idx::{
    FFIPlaylist, FFIPlaylistsList, FFIStringList, FFITrackMetadata, FFITracksList,
};

/// frees one C string previously handed out by the core. Null is ignored.
///
/// # Safety
/// `s` is either null or a pointer obtained from `idx_*` and not freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_string(s: *mut c_char) {
    crate::panic::guard_unit(|| {
        if !s.is_null() {
            unsafe { drop(CString::from_raw(s)) };
        }
    })
}

/// frees a `*mut *mut c_char` of length `len` together with every string.
unsafe fn free_string_array(ptr: *mut *mut c_char, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(ptr, len)) };
    for s in slice.iter() {
        unsafe { idx_free_string(*s) };
    }
}

/// frees the metadata contents (strings and arrays). The struct itself
/// usually lives on the caller's stack or inside a list buffer.
unsafe fn free_metadata_fields(meta: &FFITrackMetadata) {
    unsafe {
        idx_free_string(meta.path);
        idx_free_string(meta.title);
        idx_free_string(meta.album);
        idx_free_string(meta.params.cover_art);
        idx_free_string(meta.color);
        idx_free_string(meta.user_label);
        free_string_array(meta.artists, meta.artists_len);
        free_string_array(meta.genres, meta.genres_len);
    }
}

/// frees track metadata (from `idx_get_track`).
///
/// # Safety
/// `meta` comes from `idx_*` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_track_metadata(meta: FFITrackMetadata) {
    crate::panic::guard_unit(|| {
        unsafe { free_metadata_fields(&meta) };
    })
}

/// frees a track list (from `idx_get_tracks`/`idx_get_playlist_tracks`).
///
/// # Safety
/// `list` comes from `idx_*` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_tracks_list(list: FFITracksList) {
    crate::panic::guard_unit(|| {
        if list.ptr.is_null() {
            return;
        }
        let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(list.ptr, list.len)) };
        for meta in slice.iter() {
            unsafe { free_metadata_fields(meta) };
        }
    })
}

/// frees the contents of one playlist (strings and the id array).
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

/// frees a string list (from `engine_get_output_devices`).
///
/// # Safety
/// `list` comes from `engine_*` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn engine_free_output_devices(list: FFIStringList) {
    crate::panic::guard_unit(|| {
        unsafe { free_string_array(list.ptr, list.len) };
    })
}

/// frees a playlist list (from `idx_get_playlists`).
///
/// # Safety
/// `list` comes from `idx_*` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_playlists_list(list: FFIPlaylistsList) {
    crate::panic::guard_unit(|| {
        if list.ptr.is_null() {
            return;
        }
        let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(list.ptr, list.len)) };
        for pl in slice.iter() {
            unsafe { free_playlist_fields(pl) };
        }
    })
}

/// frees packet bytes (from `cdc_encode`/`cdc_pack_*`). Null is ignored.
///
/// # Safety
/// `ptr`/`len` come from `cdc_*` and weren't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_free_bytes(ptr: *mut u8, len: usize) {
    crate::panic::guard_unit(|| {
        if !ptr.is_null() {
            unsafe { drop(Box::from_raw(std::slice::from_raw_parts_mut(ptr, len))) };
        }
    })
}

/// frees an unpacked packet (from `cdc_decode`) together with its songs
/// (names, audio) and playlists (names, hashes, covers and their names).
///
/// # Safety
/// `packet` comes from `cdc_decode` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_free_packet(packet: FFIPacket) {
    crate::panic::guard_unit(|| {
        if !packet.songs.is_null() {
            let songs =
                unsafe { Box::from_raw(std::slice::from_raw_parts_mut(packet.songs, packet.songs_len)) };
            for song in songs.iter() {
                unsafe {
                    idx_free_string(song.name);
                    cdc_free_bytes(song.data, song.data_len);
                }
            }
        }
        if !packet.playlists.is_null() {
            let playlists = unsafe {
                Box::from_raw(std::slice::from_raw_parts_mut(
                    packet.playlists,
                    packet.playlists_len,
                ))
            };
            for pl in playlists.iter() {
                unsafe {
                    idx_free_string(pl.name);
                    idx_free_string(pl.cover_name);
                    if !pl.tracks.is_null() {
                        drop(Box::from_raw(std::slice::from_raw_parts_mut(
                            pl.tracks,
                            pl.tracks_len,
                        )));
                    }
                    cdc_free_bytes(pl.cover, pl.cover_len);
                }
            }
        }
    })
}
