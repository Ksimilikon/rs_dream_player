//! владеющие Rust-зеркала FFI-структур ядра. Конвертация из `api::idx::FFI*`
//! живёт здесь же; строки копируются, а исходные FFI-буферы освобождаются
//! вызывающим кодом (`db.rs`) парными `idx_free_*`.

use std::ffi::{CStr, c_char};

/// метаданные трека (владеющая копия `FFITrackMetadata`).
#[derive(Debug, Clone, Default)]
pub struct TrackMeta {
    /// id записи в индексе (`None` — трек не из бд).
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
}

/// плейлист (владеющая копия `FFIPlaylist`): имя, порядок id треков, обложка.
#[derive(Debug, Clone, Default)]
pub struct PlaylistInfo {
    pub name: Option<String>,
    pub track_ids: Vec<i64>,
    pub cover_art: Option<String>,
    pub created_at: Option<u64>,
    pub updated_at: Option<u64>,
}

/// `*const c_char` -> `Option<String>` (null -> None). Копирует данные.
///
/// # Safety
/// `ptr` — null или валидный указатель на нуль-терминированную C-строку.
pub(super) unsafe fn opt_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
}

/// `*const c_char` -> `String` ("" при null).
unsafe fn string(ptr: *const c_char) -> String {
    unsafe { opt_string(ptr) }.unwrap_or_default()
}

/// массив C-строк -> `Vec<String>`.
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
    /// копирует данные из FFI-структуры (ничего не освобождает).
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
        }
    }

    /// артисты одной строкой через запятую.
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
