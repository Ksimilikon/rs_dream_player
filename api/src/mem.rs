//! freeing memory handed out via FFI. Each owning type has its own
//! `idx_free_*` / `cdc_free_*` / `con_free_*`. Every function is idempotent
//! for null and takes ownership of the passed buffer (a second call is UB).

use std::ffi::{CString, c_char};

use crate::cdc::FFIPacket;
use crate::con::{FFIConDevice, FFIConDevicesList, FFIConPairedList};
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

/// frees bytes handed out by `idx_*` (`idx_get_track_cover`). Null is
/// ignored.
///
/// # Safety
/// `ptr`/`len` come from `idx_*` and weren't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn idx_free_bytes(ptr: *mut u8, len: usize) {
    unsafe { cdc_free_bytes(ptr, len) }
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
/// (names, audio, covers and their names) and playlists (names, hashes, covers and their names).
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
                    idx_free_string(song.cover_name);
                    cdc_free_bytes(song.cover, song.cover_len);
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

/// frees one device entry (from `con_get_device`).
///
/// # Safety
/// `device` comes from `con_*` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_free_device(device: FFIConDevice) {
    crate::panic::guard_unit(|| {
        unsafe { idx_free_string(device.name) };
    })
}

/// frees a device entry list (from `con_get_devices`).
///
/// # Safety
/// `list` comes from `con_*` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_free_devices_list(list: FFIConDevicesList) {
    crate::panic::guard_unit(|| {
        if list.ptr.is_null() {
            return;
        }
        let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(list.ptr, list.len)) };
        for dev in slice.iter() {
            unsafe { idx_free_string(dev.name) };
        }
    })
}

/// frees a paired device list (from `con_get_paired`).
///
/// # Safety
/// `list` comes from `con_*` and wasn't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_free_paired_list(list: FFIConPairedList) {
    crate::panic::guard_unit(|| {
        if list.ptr.is_null() {
            return;
        }
        let slice = unsafe { Box::from_raw(std::slice::from_raw_parts_mut(list.ptr, list.len)) };
        for dev in slice.iter() {
            unsafe { idx_free_string(dev.name) };
        }
    })
}

/// frees bytes handed out by `con_*` (`con_encrypt` / `con_decrypt`). Null is
/// ignored.
///
/// # Safety
/// `ptr`/`len` come from `con_*` and weren't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn con_free_bytes(ptr: *mut u8, len: usize) {
    unsafe { cdc_free_bytes(ptr, len) }
}

/// frees bytes handed out by `sync_take_output`. Null is ignored.
///
/// # Safety
/// `ptr`/`len` come from `sync_*` and weren't freed yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sync_free_bytes(ptr: *mut u8, len: usize) {
    unsafe { cdc_free_bytes(ptr, len) }
}
