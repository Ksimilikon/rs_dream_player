use std::{ffi::c_char, ptr::null};

use super::GLOBAL_STATE;

#[repr(C)]
pub struct FFITrackMetadata {
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
    pub cover_art: *mut c_char, // maybe null
}
#[repr(C)]
pub struct FFIPlaylist {
    pub name: *mut c_char, // null = anonymus

    pub tracks: *mut i64,
    pub tracks_len: usize,

    pub cover_art: *mut c_char, // maybe null

    pub has_updated_at: bool, // Option<u64>
    pub updated_at: u64,

    pub has_created_at: bool, // Option<u64>
    pub created_at: u64,
}

#[repr(C)]
pub struct FFIPlaylistsList {
    pub ptr: *const FFIPlaylist,
    pub len: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn idx_get_list_playlists() -> FFIPlaylistsList {
    if let Some(db) = GLOBAL_STATE.get().unwrap().lock().unwrap().get_db() {
        match db.list_playlists() {
            Ok(list) => {
                let playlists = db.list_playlists().unwrap();
                let result: Vec<FFIPlaylist> = Vec::with_capacity(playlists.capacity());

                for playlist in playlists {
                    let name = match playlist.get_name() {
                        Some(s) => {
                            std::ffi::CString::new(s).map_or(std::ptr::null_mut(), |c| c.into_raw())
                        }
                        None => std::ptr::null_mut(),
                    };

                    // 2. Треки (Vec<TrackVirtual> -> *mut i64)
                    let track_ids: Vec<i64> = playlist
                        .get_tracks()
                        .into_iter()
                        .map(|track| track.index_id())
                        .collect();
                    let tracks_len = track_ids.len();
                    let tracks = if tracks_len > 0 {
                        track_ids.into_raw_parts().0
                    } else {
                        std::ptr::null_mut()
                    };

                    // 3. Обложка (Option<PathBuf> -> *mut c_char)
                    let cover_art = match playlist.get_cover_art() {
                        Some(path) => {
                            let path_str = path.to_string_lossy().into_owned();
                            std::ffi::CString::new(path_str)
                                .map_or(std::ptr::null_mut(), |c| c.into_raw())
                        }
                        None => std::ptr::null_mut(),
                    };

                    // 4. Даты (Option<u64> -> bool + u64)
                    let (has_updated_at, updated_at) =
                        playlist.get_updated_at().map_or((false, 0), |v| (true, v));
                    let (has_created_at, created_at) =
                        playlist.get_created_at().map_or((false, 0), |v| (true, v));

                    // 5. Сборка структуры
                    let ffi_playlist = FFIPlaylist {
                        name,
                        tracks,
                        tracks_len,
                        cover_art,
                        has_updated_at,
                        updated_at,
                        has_created_at,
                        created_at,
                    };
                }
                let (ptr, len, cap) = result.into_raw_parts();
                return FFIPlaylistsList { ptr, len };
            }
            Err(err) => {}
        }
    }
    FFIPlaylistsList {
        ptr: null(),
        len: 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn idx_list_track(id_playlist: i64) {}

#[unsafe(no_mangle)]
pub extern "C" fn idx_metadata_track(id_track: i64) {}
