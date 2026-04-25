use std::ffi::{CString, c_char};

use player::{
    playlist::Playlist,
    song::{metadata::Metadata, track::Track, virtual_song::VirtualSong},
    types::Volume,
};
#[repr(C)]
pub struct MetadataFFI {
    // 8-байтовые поля (указатели и u64)
    pub title: *mut c_char,
    pub artist: *mut *mut c_char,
    pub mb_album: *mut c_char,
    pub cover_art: *mut u8,
    pub duration_sec: u64,
    pub artist_len: usize,
    pub cover_art_len: usize,
    // 4-байтовые поля
    pub sample_rate: u32,
    pub bitrate: u32,
    pub track_number: u32,
    // 1-байтовые поля
    pub has_track_number: bool,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_metadata() {}

#[repr(C)]
pub struct VirtualSongFFI {
    pub metadata: MetadataFFI,
    pub volume: Volume,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_virtualsong() {}

#[repr(C)]
pub struct PlaylistFFI {
    pub songs: *mut VirtualSongFFI,
    pub songs_len: usize,
    pub cur_song: usize,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn apply_playlist() {}

impl MetadataFFI {
    pub fn from_core(core: &Metadata) -> Self {
        let title = CString::new(core.title.clone()).unwrap().into_raw();

        // Берем первый альбом из списка (т.к. FFI ждет один mb_album)
        let mb_album = core
            .albums
            .first()
            .map(|s| CString::new(s.clone()).unwrap().into_raw())
            .unwrap_or(std::ptr::null_mut());

        let artist_c_strs: Vec<*mut c_char> = core
            .artist
            .iter()
            .map(|s| CString::new(s.clone()).unwrap().into_raw())
            .collect();
        let artist_len = artist_c_strs.len();
        let artist = Box::into_raw(artist_c_strs.into_boxed_slice()) as *mut *mut c_char;

        // Извлекаем параметры, если они есть, иначе используем дефолты
        let (
            duration_sec,
            sample_rate,
            bitrate,
            track_number,
            has_track_number,
            cover_art,
            cover_art_len,
        ) = if let Some(p) = &core.params {
            let (ptr, len) = p
                .cover_art
                .as_ref()
                .map(|v| (v.clone().as_mut_ptr(), v.len()))
                .unwrap_or((std::ptr::null_mut(), 0));

            (
                p.duration_sec,
                p.sample_rate,
                p.bitrate,
                p.track_number.unwrap_or(0),
                p.track_number.is_some(),
                ptr,
                len,
            )
        } else {
            (0, 0, 0, 0, false, std::ptr::null_mut(), 0)
        };

        Self {
            title,
            artist,
            artist_len,
            mb_album,
            duration_sec,
            sample_rate,
            bitrate,
            track_number,
            has_track_number,
            cover_art,
            cover_art_len,
        }
    }

    pub unsafe fn free(self) {
        if !self.title.is_null() {
            drop(CString::from_raw(self.title));
        }
        if !self.mb_album.is_null() {
            drop(CString::from_raw(self.mb_album));
        }
        if !self.artist.is_null() {
            let artists = Vec::from_raw_parts(self.artist, self.artist_len, self.artist_len);
            for ptr in artists {
                if !ptr.is_null() {
                    drop(CString::from_raw(ptr));
                }
            }
        }
        // cover_art здесь не удаляем, если он ссылается на буфер внутри Vec,
        // который удалится вместе с Track или Metadata (зависит от логики владения)
    }
}

impl VirtualSongFFI {
    pub fn from_core(core: &VirtualSong) -> Self {
        Self {
            volume: core.volume,
            metadata: MetadataFFI::from_core(&core.get_metadata()),
        }
    }

    pub unsafe fn free(self) {
        self.metadata.free();
    }
}
impl PlaylistFFI {
    pub fn from_core(core: &Playlist) -> Self {
        let mut ffi_songs: Vec<VirtualSongFFI> = core
            .get_songs()
            .iter()
            .map(|s| VirtualSongFFI::from_core(s))
            .collect();

        let songs_ptr = ffi_songs.as_mut_ptr();
        let songs_len = ffi_songs.len();
        std::mem::forget(ffi_songs);

        Self {
            songs: songs_ptr,
            songs_len,
            cur_song: core.get_cur_song(),
        }
    }

    pub unsafe fn free(self) {
        if !self.songs.is_null() {
            let songs = Vec::from_raw_parts(self.songs, self.songs_len, self.songs_len);
            for s in songs {
                s.free();
            }
        }
    }
}
