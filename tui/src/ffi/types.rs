//! owning Rust mirrors of the core FFI structs. Conversion from
//! `api::idx::FFI*` lives here too; strings are copied, and the source FFI
//! buffers are freed by the caller (`db.rs`) with the paired `idx_free_*`.

use std::ffi::{CStr, c_char};

/// track metadata (an owning copy of `FFITrackMetadata`).
#[derive(Debug, Clone, Default)]
pub struct TrackMeta {
    /// id of the index row (`None` — the track is not from the db).
    pub id: Option<i64>,
    pub path: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub genres: Vec<String>,
    pub duration_s: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub listen_count: i64,
    pub cover_art: Option<String>,
    pub volume: f32,
    pub invalid: bool,
    pub color: Option<String>,
    pub label: Option<String>,
    /// kept on purpose though in no playlist (the orphan collector skips it).
    pub ignore_orphan: bool,
}

/// playlist (an owning copy of `FFIPlaylist`): name, track id order, cover.
#[derive(Debug, Clone, Default)]
pub struct PlaylistInfo {
    pub name: Option<String>,
    pub track_ids: Vec<i64>,
    pub cover_art: Option<String>,
    pub created_at: Option<u64>,
    pub updated_at: Option<u64>,
}

/// `*const c_char` -> `Option<String>` (null -> None). Copies the data.
///
/// # Safety
/// `ptr` is null or a valid pointer to a nul-terminated C string.
pub(super) unsafe fn opt_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
}

/// `*const c_char` -> `String` ("" on null).
unsafe fn string(ptr: *const c_char) -> String {
    unsafe { opt_string(ptr) }.unwrap_or_default()
}

/// array of C strings -> `Vec<String>`.
unsafe fn strings(ptr: *const *const c_char, len: usize) -> Vec<String> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(ptr, len) }
        .iter()
        .filter_map(|&p| unsafe { opt_string(p) })
        .collect()
}

impl TrackMeta {
    /// copies data from the FFI struct (frees nothing).
    pub(super) unsafe fn from_ffi(m: &api::idx::FFITrackMetadata) -> Self {
        let (duration_s, sample_rate, bitrate, listen_count, cover_art) = if m.has_params {
            (
                m.params.duration_s,
                m.params.sample_rate,
                m.params.bitrate,
                m.params.listen_count,
                unsafe { opt_string(m.params.cover_art) },
            )
        } else {
            (0, 0, 0, 0, None)
        };
        Self {
            id: m.has_id.then_some(m.id),
            path: unsafe { opt_string(m.path) },
            title: unsafe { string(m.title) },
            artists: unsafe { strings(m.artists as *const *const c_char, m.artists_len) },
            album: unsafe { opt_string(m.album) },
            genres: unsafe { strings(m.genres as *const *const c_char, m.genres_len) },
            duration_s,
            sample_rate,
            bitrate,
            listen_count,
            cover_art,
            volume: m.volume,
            invalid: m.invalid,
            color: unsafe { opt_string(m.color) },
            label: unsafe { opt_string(m.user_label) },
            ignore_orphan: m.ignore_orphan,
        }
    }

    /// artists as one comma-separated string.
    pub fn artists_line(&self) -> String {
        self.artists.join(", ")
    }
}

impl PlaylistInfo {
    pub(super) unsafe fn from_ffi(p: &api::idx::FFIPlaylist) -> Self {
        let track_ids = if p.tracks.is_null() || p.tracks_len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(p.tracks, p.tracks_len) }.to_vec()
        };
        Self {
            name: unsafe { opt_string(p.name) },
            track_ids,
            cover_art: unsafe { opt_string(p.cover_art) },
            created_at: p.has_created_at.then_some(p.created_at),
            updated_at: p.has_updated_at.then_some(p.updated_at),
        }
    }
}
