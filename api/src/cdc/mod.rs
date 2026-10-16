//! FFI поверх крейта `codec`. Префикс `cdc_` = упаковка/распаковка пакетов.
//!
//! - версия пакета передаётся числом `u32` (= дискриминант `codec::Version`,
//!   сейчас `1` = v0.1); неизвестное число -> `UnsupportedVersion`.
//! - все функции возвращают [`FFICdcStatus`] (`0` = ok), результат — через
//!   out-параметры. При ошибке out-параметры обнулены (null/0).
//! - хеш песни на границе FFI — `[u8; 16]` big-endian: байты идут в том же
//!   порядке, что и hex-строка хеша в бд (`u128` в C-ABI не переносим).
//! - флаги `with_audio`/`with_names` есть во всех функциях: `false` ->
//!   аудио/имена файлов песен не пакуются (в пакете длина 0) или не отдаются
//!   при распаковке (null). Имена плейлистов флаг не трогает.
//! - флаг `with_covers` — во всех функциях, где есть плейлисты: обложки
//!   плейлистов читаются с диска по пути ОС и пакуются последней секцией
//!   вместе с именем файла (подсказка формата для получателя).
//! - даты плейлистов (`created_at`/`updated_at`, unix-секунды) едут всегда —
//!   по ним потом выбирается подлинная версия плейлиста между устройствами.
//! - выданные буферы освобождаются парными `cdc_free_*` из [`crate::mem`].
//! - импорт пакета в бд в два шага — `cdc_import_prepare` ->
//!   (фронтенд пишет файлы) -> `cdc_import_commit` / `cdc_import_cancel`
//!   (см. [`import`]).

use std::{
    ffi::{CStr, c_char},
    path::Path,
    ptr, slice,
};

use codec::{CodecError, Playlist, Song, Version};
use storage::db::PlaylistHashRow;

use super::GLOBAL_STATE;
use crate::idx::{to_c, vec_to_raw};

mod import;

pub use import::{
    FFIImportFile, FFIImportFileKind, FFIImportFiles, ImportSession, cdc_import_cancel,
    cdc_import_commit, cdc_import_prepare,
};

/// результат любой `cdc_*` функции — единая база кодов для всех функций
/// упаковки/распаковки/импорта. Диапазоны:
/// - `0` — ok;
/// - `1..=99` — `codec::CodecError` один в один;
/// - `100..=199` — статусы импорта (решения, а не сбои: пакет корректен, но
///   применён не был);
/// - `200..` — ошибки уровня ядра.
///
/// один enum на всех — компилятор сам не даст выдать два статуса с одним
/// кодом. `repr(u32)`: на стороне C это просто число; коды не
/// перенумеровывать, только добавлять в свой диапазон.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFICdcStatus {
    Ok = 0,

    // ── codec::CodecError ──
    BadStartWord = 1,
    UnsupportedVersion = 2,
    UnexpectedEof = 3,
    TrailingBytes = 4,
    InvalidUtf8 = 5,
    TooManyRows = 6,
    NameTooLong = 7,
    DataTooLong = 8,
    DuplicateSection = 9,
    CountMismatch = 10,

    // ── импорт (100..=199) ──
    /// пакет устарел: какой-то плейлист в нём старше (`updated_at`), чем
    /// локальный. Импорт отклонён целиком, ничего не записано; применить
    /// можно только с `force`.
    Outdated = 100,

    // ── ядро (200..) ──
    /// обязательный указатель равен null.
    NullPointer = 200,
    /// ядро запущено без бд (`need_db == false`).
    NoDatabase = 201,
    /// какого-то id из списка нет в бд.
    TrackNotFound = 202,
    /// хеш в бд не является 32-символьным hex.
    InvalidHash = 203,
    /// не удалось прочитать аудиофайл / обложку (или запрос к бд упал).
    ReadFailed = 204,
    /// какого-то id плейлиста из списка нет в бд.
    PlaylistNotFound = 205,
    /// не удалось сохранить импорт в бд.
    WriteFailed = 206,
    /// аргумент не согласован с остальными (например, число путей в
    /// `cdc_import_commit` не равно числу файлов из `cdc_import_prepare`).
    InvalidArgument = 207,
}

impl From<CodecError> for FFICdcStatus {
    fn from(e: CodecError) -> Self {
        match e {
            CodecError::BadStartWord => Self::BadStartWord,
            CodecError::UnsupportedVersion => Self::UnsupportedVersion,
            CodecError::UnexpectedEof => Self::UnexpectedEof,
            CodecError::TrailingBytes => Self::TrailingBytes,
            CodecError::InvalidUtf8 => Self::InvalidUtf8,
            CodecError::TooManyRows => Self::TooManyRows,
            CodecError::NameTooLong => Self::NameTooLong,
            CodecError::DataTooLong => Self::DataTooLong,
            CodecError::DuplicateSection => Self::DuplicateSection,
            CodecError::CountMismatch => Self::CountMismatch,
        }
    }
}

/// одна песня распакованного пакета.
#[repr(C)]
pub struct FFIPacketSong {
    pub hash: [u8; 16],    // big-endian
    pub name: *mut c_char, // null: имени нет / `with_names == false`
    pub data: *mut u8,     // null: аудио нет / `with_audio == false`
    pub data_len: usize,
}

/// один плейлист распакованного пакета.
#[repr(C)]
pub struct FFIPacketPlaylist {
    pub name: *mut c_char,     // null: пустое имя
    pub tracks: *mut [u8; 16], // хеши песен (big-endian) в порядке плейлиста
    pub tracks_len: usize,

    pub has_created_at: bool, // Option<u64>
    pub created_at: u64,
    pub has_updated_at: bool, // Option<u64>
    pub updated_at: u64,

    pub cover_name: *mut c_char, // null: имени нет / `with_covers == false`
    pub cover: *mut u8,          // null: обложки нет / `with_covers == false`
    pub cover_len: usize,
}

/// распакованный пакет (владеет буферами песен и плейлистов).
#[repr(C)]
pub struct FFIPacket {
    pub version: u32,
    pub songs: *mut FFIPacketSong,
    pub songs_len: usize,
    pub playlists: *mut FFIPacketPlaylist,
    pub playlists_len: usize,
}

impl FFIPacket {
    fn empty() -> Self {
        FFIPacket {
            version: 0,
            songs: ptr::null_mut(),
            songs_len: 0,
            playlists: ptr::null_mut(),
            playlists_len: 0,
        }
    }
}

// ───────────────────────── helpers (crate-private) ─────────────────────────

/// обнуляет out-параметры байтов; `false`, если какой-то из них null.
///
/// # Safety
/// `out_ptr`/`out_len` — null или валидные указатели.
unsafe fn reset_bytes(out_ptr: *mut *mut u8, out_len: *mut usize) -> bool {
    if out_ptr.is_null() || out_len.is_null() {
        return false;
    }
    unsafe {
        *out_ptr = ptr::null_mut();
        *out_len = 0;
    }
    true
}

/// результат упаковки -> out-параметры + статус.
///
/// # Safety
/// `out_ptr`/`out_len` — валидные ненулевые указатели.
unsafe fn finish_bytes(
    res: Result<Vec<u8>, FFICdcStatus>,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    match res {
        Ok(bytes) => {
            let (p, l) = vec_to_raw(bytes);
            unsafe {
                *out_ptr = p;
                *out_len = l;
            }
            FFICdcStatus::Ok
        }
        Err(e) => e,
    }
}

/// `(ptr, len)` -> срез; null допустим только при `len == 0`.
pub(crate) unsafe fn raw_slice<'a, T>(p: *const T, len: usize) -> Option<&'a [T]> {
    match (p.is_null(), len) {
        (true, 0) => Some(&[]),
        (true, _) => None,
        (false, _) => Some(unsafe { slice::from_raw_parts(p, len) }),
    }
}

/// C-строка -> `String`; null -> пустая строка.
unsafe fn c_to_name(p: *const c_char) -> Result<String, FFICdcStatus> {
    if p.is_null() {
        return Ok(String::new());
    }
    match unsafe { CStr::from_ptr(p) }.to_str() {
        Ok(s) => Ok(s.to_owned()),
        Err(_) => Err(FFICdcStatus::InvalidUtf8),
    }
}

/// hex-хеш из бд -> `u128`.
fn parse_hash(hex: &str) -> Option<u128> {
    (hex.len() == 32)
        .then(|| u128::from_str_radix(hex, 16).ok())
        .flatten()
}

/// имя файла по пути ОС (пустое, если его нет).
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// обложка по пути ОС: `(имя файла, байты)`. Отсутствующий файл обложки — не
/// ошибка (обложка декоративна, а битая ссылка в бд не должна блокировать
/// экспорт): тогда обложки просто нет.
fn read_cover(path: Option<&Path>) -> (String, Vec<u8>) {
    path.and_then(|p| Some((file_name(p), std::fs::read(p).ok()?)))
        .unwrap_or_default()
}

/// что упаковать из бд.
enum Selection<'a> {
    /// все треки и все плейлисты.
    Library,
    /// треки по id, без плейлистов.
    Tracks(&'a [i64]),
    /// плейлисты по id и все их песни.
    Playlists(&'a [i64]),
}

/// общая упаковка из бд.
fn pack_from_db(
    version: u32,
    selection: Selection<'_>,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
) -> Result<Vec<u8>, FFICdcStatus> {
    let version = Version::try_from(version)?;

    // под блокировкой только выборка из бд; чтение аудио/обложек — уже без
    // неё, чтобы не держать ядро (плеер) на время чтения гигабайтов с диска.
    let (tracks, playlists) = {
        let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
        let db = guard.get_db().ok_or(FFICdcStatus::NoDatabase)?;
        let read = |_| FFICdcStatus::ReadFailed;
        match selection {
            Selection::Library => (
                db.get_track_hashes(None).map_err(read)?,
                db.get_playlist_hashes(None).map_err(read)?,
            ),
            Selection::Tracks(ids) => {
                let tracks = db.get_track_hashes(Some(ids)).map_err(read)?;
                if tracks.len() != ids.len() {
                    return Err(FFICdcStatus::TrackNotFound);
                }
                (tracks, Vec::new())
            }
            Selection::Playlists(ids) => {
                let playlists = db.get_playlist_hashes(Some(ids)).map_err(read)?;
                if playlists.len() != ids.len() {
                    return Err(FFICdcStatus::PlaylistNotFound);
                }
                // песни всех плейлистов без повторов, в порядке первого
                // появления. Хеши без трека в бд в песни не попадают, но в
                // списке плейлиста остаются.
                let mut seen = std::collections::HashSet::new();
                let hashes: Vec<String> = playlists
                    .iter()
                    .flat_map(|p| &p.hashes)
                    .filter(|h| seen.insert(h.as_str()))
                    .cloned()
                    .collect();
                (db.get_track_hashes_by_hash(&hashes).map_err(read)?, playlists)
            }
        }
    };

    let songs = tracks
        .into_iter()
        .map(|(_, hash, path)| song_from_db(&hash, &path, with_audio, with_names))
        .collect::<Result<Vec<_>, _>>()?;
    let playlists = playlists
        .into_iter()
        .map(|row| playlist_from_db(row, with_covers))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(codec::encode(version, &songs, &playlists)?)
}

/// строка бд -> песня пакета.
fn song_from_db(
    hash: &str,
    path: &Path,
    with_audio: bool,
    with_names: bool,
) -> Result<Song, FFICdcStatus> {
    let hash = parse_hash(hash).ok_or(FFICdcStatus::InvalidHash)?;
    let name = match with_names {
        true => file_name(path),
        false => String::new(),
    };
    let data = match with_audio {
        true => std::fs::read(path).map_err(|_| FFICdcStatus::ReadFailed)?,
        false => Vec::new(),
    };
    Ok(Song { hash, name, data })
}

/// плейлист бд -> плейлист пакета.
fn playlist_from_db(row: PlaylistHashRow, with_covers: bool) -> Result<Playlist, FFICdcStatus> {
    let tracks = row
        .hashes
        .iter()
        .map(|h| parse_hash(h).ok_or(FFICdcStatus::InvalidHash))
        .collect::<Result<Vec<_>, _>>()?;
    let (cover_name, cover) = match with_covers {
        true => read_cover(row.cover_art.as_deref()),
        false => (String::new(), Vec::new()),
    };
    Ok(Playlist {
        name: row.name,
        tracks,
        created_at: row.created_at,
        updated_at: row.updated_at,
        cover_name,
        cover,
    })
}

/// песня пакета -> FFI (с учётом флагов).
fn song_to_ffi(song: Song, with_audio: bool, with_names: bool) -> FFIPacketSong {
    let name = match with_names && !song.name.is_empty() {
        true => to_c(&song.name),
        false => ptr::null_mut(),
    };
    let (data, data_len) = match with_audio {
        true => vec_to_raw(song.data),
        false => (ptr::null_mut(), 0),
    };
    FFIPacketSong {
        hash: song.hash.to_be_bytes(),
        name,
        data,
        data_len,
    }
}

/// непустая строка -> C-строка, пустая -> null.
fn non_empty_to_c(s: &str) -> *mut c_char {
    match s.is_empty() {
        true => ptr::null_mut(),
        false => to_c(s),
    }
}

/// плейлист пакета -> FFI (с учётом флага обложек).
fn playlist_to_ffi(playlist: Playlist, with_covers: bool) -> FFIPacketPlaylist {
    let name = non_empty_to_c(&playlist.name);
    let (tracks, tracks_len) = vec_to_raw(
        playlist
            .tracks
            .into_iter()
            .map(u128::to_be_bytes)
            .collect::<Vec<_>>(),
    );
    let (cover_name, (cover, cover_len)) = match with_covers {
        true => (non_empty_to_c(&playlist.cover_name), vec_to_raw(playlist.cover)),
        false => (ptr::null_mut(), (ptr::null_mut(), 0)),
    };
    FFIPacketPlaylist {
        name,
        tracks,
        tracks_len,
        has_created_at: playlist.created_at.is_some(),
        created_at: playlist.created_at.unwrap_or(0),
        has_updated_at: playlist.updated_at.is_some(),
        updated_at: playlist.updated_at.unwrap_or(0),
        cover_name,
        cover,
        cover_len,
    }
}

/// песни из параллельных массивов `cdc_encode`.
///
/// # Safety
/// см. `cdc_encode`.
unsafe fn songs_from_c(
    count: usize,
    hashes: *const [u8; 16],
    names: *const *const c_char,
    data: *const *const u8,
    data_lens: *const usize,
    with_audio: bool,
    with_names: bool,
) -> Result<Vec<Song>, FFICdcStatus> {
    let hashes = unsafe { raw_slice(hashes, count) }.ok_or(FFICdcStatus::NullPointer)?;
    let names = match with_names {
        true => Some(unsafe { raw_slice(names, count) }.ok_or(FFICdcStatus::NullPointer)?),
        false => None,
    };
    let audio = match with_audio {
        true => match unsafe { (raw_slice(data, count), raw_slice(data_lens, count)) } {
            (Some(d), Some(l)) => Some((d, l)),
            _ => return Err(FFICdcStatus::NullPointer),
        },
        false => None,
    };

    let mut songs = Vec::with_capacity(count);
    for (i, hash) in hashes.iter().enumerate() {
        let name = match names {
            Some(n) => unsafe { c_to_name(n[i]) }?,
            None => String::new(),
        };
        let data = match audio {
            Some((d, l)) => unsafe { raw_slice(d[i], l[i]) }
                .ok_or(FFICdcStatus::NullPointer)?
                .to_vec(),
            None => Vec::new(),
        };
        songs.push(Song {
            hash: u128::from_be_bytes(*hash),
            name,
            data,
        });
    }
    Ok(songs)
}

/// плейлисты из параллельных массивов `cdc_encode`.
///
/// # Safety
/// см. `cdc_encode`.
#[allow(clippy::too_many_arguments)]
unsafe fn playlists_from_c(
    count: usize,
    names: *const *const c_char,
    tracks: *const [u8; 16],
    tracks_lens: *const usize,
    created_at: *const u64,
    updated_at: *const u64,
    covers: *const *const c_char,
    with_covers: bool,
) -> Result<Vec<Playlist>, FFICdcStatus> {
    let names = unsafe { raw_slice(names, count) }.ok_or(FFICdcStatus::NullPointer)?;
    let lens = unsafe { raw_slice(tracks_lens, count) }.ok_or(FFICdcStatus::NullPointer)?;
    let total = lens
        .iter()
        .try_fold(0usize, |acc, &l| acc.checked_add(l))
        .ok_or(FFICdcStatus::TooManyRows)?;
    let tracks = unsafe { raw_slice(tracks, total) }.ok_or(FFICdcStatus::NullPointer)?;
    // массив дат null -> дат нет ни у кого; элемент 0 -> даты нет.
    let dates = |p: *const u64| match p.is_null() {
        true => None,
        false => Some(unsafe { slice::from_raw_parts(p, count) }),
    };
    let (created_at, updated_at) = (dates(created_at), dates(updated_at));
    let date = |d: Option<&[u64]>, i: usize| d.map(|d| d[i]).filter(|&v| v != 0);
    let covers = match with_covers {
        true => Some(unsafe { raw_slice(covers, count) }.ok_or(FFICdcStatus::NullPointer)?),
        false => None,
    };

    let mut playlists = Vec::with_capacity(count);
    let mut rest = tracks;
    for i in 0..count {
        let (own, tail) = rest.split_at(lens[i]);
        rest = tail;
        let (cover_name, cover) = match covers.map(|c| c[i]) {
            Some(p) if !p.is_null() => match crate::ptr_to_path(p) {
                Ok(path) => (
                    file_name(&path),
                    std::fs::read(&path).map_err(|_| FFICdcStatus::ReadFailed)?,
                ),
                Err(_) => return Err(FFICdcStatus::InvalidUtf8),
            },
            _ => (String::new(), Vec::new()),
        };
        playlists.push(Playlist {
            name: unsafe { c_to_name(names[i]) }?,
            tracks: own.iter().map(|h| u128::from_be_bytes(*h)).collect(),
            created_at: date(created_at, i),
            updated_at: date(updated_at, i),
            cover_name,
            cover,
        });
    }
    Ok(playlists)
}

// ───────────────────────────────── API ─────────────────────────────────────

/// пакует песни и плейлисты, переданные снаружи. Все массивы параллельные.
///
/// песни (длины `songs_len`):
/// - `hashes` — обязателен при `songs_len > 0`;
/// - `names` — обязателен при `with_names`; элемент null = без имени;
/// - `data`/`data_lens` — обязательны при `with_audio`; элемент `data[i]`
///   null допустим только при `data_lens[i] == 0`.
///
/// плейлисты (длины `playlists_len`):
/// - `pl_names` — обязателен при `playlists_len > 0`; элемент null = без имени;
/// - `pl_tracks_lens` — число песен каждого плейлиста;
/// - `pl_tracks` — хеши песен всех плейлистов подряд (сначала песни 1-го
///   плейлиста, затем 2-го, ...), длины `sum(pl_tracks_lens)`;
/// - `pl_created_at`/`pl_updated_at` — даты (unix-секунды); массив null = дат
///   нет ни у одного плейлиста, элемент 0 = даты нет;
/// - `pl_covers` — пути ОС к обложкам, обязателен при `with_covers`; элемент
///   null = без обложки. В пакет едут байты файла и его имя. Здесь путь задан
///   явно, поэтому нечитаемый файл — ошибка `ReadFailed`.
///
/// пакет пишется в `out_ptr`/`out_len`, освобождать `cdc_free_bytes`.
///
/// # Safety
/// все ненулевые указатели валидны на указанное число элементов; строки
/// нуль-терминированы; `data[i]` валиден на `data_lens[i]` байт.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_encode(
    version: u32,
    songs_len: usize,
    hashes: *const [u8; 16],
    names: *const *const c_char,
    data: *const *const u8,
    data_lens: *const usize,
    playlists_len: usize,
    pl_names: *const *const c_char,
    pl_tracks: *const [u8; 16],
    pl_tracks_lens: *const usize,
    pl_created_at: *const u64,
    pl_updated_at: *const u64,
    pl_covers: *const *const c_char,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    if !unsafe { reset_bytes(out_ptr, out_len) } {
        return FFICdcStatus::NullPointer;
    }
    let res = (|| {
        let version = Version::try_from(version)?;
        let songs = unsafe {
            songs_from_c(songs_len, hashes, names, data, data_lens, with_audio, with_names)
        }?;
        let playlists = unsafe {
            playlists_from_c(
                playlists_len,
                pl_names,
                pl_tracks,
                pl_tracks_lens,
                pl_created_at,
                pl_updated_at,
                pl_covers,
                with_covers,
            )
        }?;
        Ok(codec::encode(version, &songs, &playlists)?)
    })();
    unsafe { finish_bytes(res, out_ptr, out_len) }
}

/// распаковывает пакет `bytes[..len]` в `out`. При выключенном флаге
/// соответствующие поля — null. Освобождать `cdc_free_packet`.
///
/// # Safety
/// `bytes` валиден на `len` байт (null допустим при `len == 0`); `out` —
/// валидный указатель.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_decode(
    bytes: *const u8,
    len: usize,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out: *mut FFIPacket,
) -> FFICdcStatus {
    if out.is_null() {
        return FFICdcStatus::NullPointer;
    }
    unsafe { *out = FFIPacket::empty() };
    let Some(bytes) = (unsafe { raw_slice(bytes, len) }) else {
        return FFICdcStatus::NullPointer;
    };

    let packet = match codec::decode(bytes) {
        Ok(p) => p,
        Err(e) => return e.into(),
    };
    let (songs, songs_len) = vec_to_raw(
        packet
            .songs
            .into_iter()
            .map(|s| song_to_ffi(s, with_audio, with_names))
            .collect::<Vec<_>>(),
    );
    let (playlists, playlists_len) = vec_to_raw(
        packet
            .playlists
            .into_iter()
            .map(|p| playlist_to_ffi(p, with_covers))
            .collect::<Vec<_>>(),
    );
    unsafe {
        *out = FFIPacket {
            version: packet.version.into(),
            songs,
            songs_len,
            playlists,
            playlists_len,
        };
    }
    FFICdcStatus::Ok
}

/// пакет-снапшот всей бд: все треки (по возрастанию id) и все плейлисты.
/// Освобождать `cdc_free_bytes`.
///
/// # Safety
/// `out_ptr`/`out_len` — валидные указатели.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_pack_library(
    version: u32,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    if !unsafe { reset_bytes(out_ptr, out_len) } {
        return FFICdcStatus::NullPointer;
    }
    let res = pack_from_db(version, Selection::Library, with_audio, with_names, with_covers);
    unsafe { finish_bytes(res, out_ptr, out_len) }
}

/// пакет из треков бд по их id (`ids[..ids_len]`, порядок сохраняется), без
/// плейлистов. Любой ненайденный id -> `TrackNotFound`. Освобождать
/// `cdc_free_bytes`.
///
/// # Safety
/// `ids` валиден на `ids_len` элементов (null допустим при `ids_len == 0`);
/// `out_ptr`/`out_len` — валидные указатели.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_pack_tracks(
    version: u32,
    ids: *const i64,
    ids_len: usize,
    with_audio: bool,
    with_names: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    if !unsafe { reset_bytes(out_ptr, out_len) } {
        return FFICdcStatus::NullPointer;
    }
    let Some(ids) = (unsafe { raw_slice(ids, ids_len) }) else {
        return FFICdcStatus::NullPointer;
    };
    let res = pack_from_db(version, Selection::Tracks(ids), with_audio, with_names, false);
    unsafe { finish_bytes(res, out_ptr, out_len) }
}

/// пакет из плейлистов бд по их id (`ids[..ids_len]`, порядок сохраняется)
/// вместе со всеми их песнями (без повторов). Любой ненайденный id ->
/// `PlaylistNotFound`. Освобождать `cdc_free_bytes`.
///
/// # Safety
/// `ids` валиден на `ids_len` элементов (null допустим при `ids_len == 0`);
/// `out_ptr`/`out_len` — валидные указатели.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_pack_playlists(
    version: u32,
    ids: *const i64,
    ids_len: usize,
    with_audio: bool,
    with_names: bool,
    with_covers: bool,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> FFICdcStatus {
    if !unsafe { reset_bytes(out_ptr, out_len) } {
        return FFICdcStatus::NullPointer;
    }
    let Some(ids) = (unsafe { raw_slice(ids, ids_len) }) else {
        return FFICdcStatus::NullPointer;
    };
    let res = pack_from_db(version, Selection::Playlists(ids), with_audio, with_names, with_covers);
    unsafe { finish_bytes(res, out_ptr, out_len) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::{cdc_free_bytes, cdc_free_packet};

    /// `cdc_encode` только с песнями.
    unsafe fn encode_songs(
        version: u32,
        hashes: &[[u8; 16]],
        names: *const *const c_char,
        data: *const *const u8,
        lens: *const usize,
        with_audio: bool,
        with_names: bool,
    ) -> (FFICdcStatus, *mut u8, usize) {
        let (mut p, mut l) = (ptr::null_mut(), 0);
        let st = unsafe {
            cdc_encode(
                version,
                hashes.len(),
                hashes.as_ptr(),
                names,
                data,
                lens,
                0,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                with_audio,
                with_names,
                false,
                &mut p,
                &mut l,
            )
        };
        (st, p, l)
    }

    #[test]
    fn hash_parsing() {
        assert_eq!(parse_hash(&format!("{:032x}", 0xabcu128)), Some(0xabc));
        assert_eq!(parse_hash("abc"), None);
        assert_eq!(parse_hash(&"z".repeat(32)), None);
    }

    #[test]
    fn hash_bytes_match_hex_order() {
        let hex = "0123456789abcdef0011223344556677";
        let bytes = parse_hash(hex).unwrap().to_be_bytes();
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[15], 0x77);
    }

    #[test]
    fn songs_roundtrip_with_flags() {
        let hashes = [[1u8; 16], [2u8; 16]];
        let n0 = c"a.flac";
        let names = [n0.as_ptr(), ptr::null()];
        let d0 = [9u8, 8, 7];
        let data = [d0.as_ptr(), ptr::null()];
        let lens = [d0.len(), 0];
        let (st, p, l) = unsafe {
            encode_songs(1, &hashes, names.as_ptr(), data.as_ptr(), lens.as_ptr(), true, true)
        };
        assert_eq!(st, FFICdcStatus::Ok);

        // полная распаковка.
        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, true, true, true, &mut pk) },
            FFICdcStatus::Ok
        );
        assert_eq!(pk.version, 1);
        assert_eq!(pk.songs_len, 2);
        assert_eq!(pk.playlists_len, 0);
        let songs = unsafe { slice::from_raw_parts(pk.songs, pk.songs_len) };
        assert_eq!(songs[0].hash, [1; 16]);
        assert_eq!(unsafe { CStr::from_ptr(songs[0].name) }, n0);
        assert_eq!(
            unsafe { slice::from_raw_parts(songs[0].data, songs[0].data_len) },
            &d0
        );
        assert!(songs[1].name.is_null() && songs[1].data.is_null());
        unsafe { cdc_free_packet(pk) };

        // без аудио и имён.
        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, false, false, false, &mut pk) },
            FFICdcStatus::Ok
        );
        let songs = unsafe { slice::from_raw_parts(pk.songs, pk.songs_len) };
        assert!(songs[0].name.is_null() && songs[0].data.is_null());
        unsafe { cdc_free_packet(pk) };

        unsafe { cdc_free_bytes(p, l) };
    }

    #[test]
    fn playlists_roundtrip_with_covers() {
        let dir = std::env::temp_dir().join(format!("cdc_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cover_path = dir.join("cover.png");
        std::fs::write(&cover_path, [0x89, b'P', b'N', b'G']).unwrap();
        let cover_c = std::ffi::CString::new(cover_path.to_str().unwrap()).unwrap();

        let pl_names = [c"rock".as_ptr(), c"empty".as_ptr()];
        let pl_lens = [2usize, 0];
        let pl_tracks = [[7u8; 16], [8u8; 16]];
        let pl_created = [100u64, 0];
        let pl_updated = [200u64, 300];
        let pl_covers = [cover_c.as_ptr(), ptr::null()];
        let (mut p, mut l) = (ptr::null_mut(), 0);
        let st = unsafe {
            cdc_encode(
                1,
                0,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                2,
                pl_names.as_ptr(),
                pl_tracks.as_ptr(),
                pl_lens.as_ptr(),
                pl_created.as_ptr(),
                pl_updated.as_ptr(),
                pl_covers.as_ptr(),
                false,
                false,
                true,
                &mut p,
                &mut l,
            )
        };
        assert_eq!(st, FFICdcStatus::Ok);

        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, false, false, true, &mut pk) },
            FFICdcStatus::Ok
        );
        let pls = unsafe { slice::from_raw_parts(pk.playlists, pk.playlists_len) };
        assert_eq!(pls.len(), 2);
        assert_eq!(unsafe { CStr::from_ptr(pls[0].name) }, c"rock");
        assert_eq!(
            unsafe { slice::from_raw_parts(pls[0].tracks, pls[0].tracks_len) },
            &pl_tracks
        );
        assert_eq!(
            unsafe { slice::from_raw_parts(pls[0].cover, pls[0].cover_len) },
            &[0x89, b'P', b'N', b'G']
        );
        assert_eq!(unsafe { CStr::from_ptr(pls[0].cover_name) }, c"cover.png");
        assert!(pls[0].has_created_at && pls[0].created_at == 100);
        assert!(pls[0].has_updated_at && pls[0].updated_at == 200);
        assert!(!pls[1].has_created_at && pls[1].has_updated_at);
        assert!(pls[1].tracks.is_null() && pls[1].cover.is_null() && pls[1].cover_name.is_null());
        unsafe { cdc_free_packet(pk) };

        // без обложек при распаковке.
        let mut pk = FFIPacket::empty();
        assert_eq!(
            unsafe { cdc_decode(p, l, false, false, false, &mut pk) },
            FFICdcStatus::Ok
        );
        let pls = unsafe { slice::from_raw_parts(pk.playlists, pk.playlists_len) };
        assert!(pls[0].cover.is_null() && pls[0].cover_name.is_null());
        assert!(pls[0].has_created_at);
        unsafe { cdc_free_packet(pk) };

        unsafe { cdc_free_bytes(p, l) };
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn encode_flags_allow_null_arrays() {
        let hashes = [[3u8; 16]];
        let (st, p, l) = unsafe {
            encode_songs(1, &hashes, ptr::null(), ptr::null(), ptr::null(), false, false)
        };
        assert_eq!(st, FFICdcStatus::Ok);
        unsafe { cdc_free_bytes(p, l) };
    }

    #[test]
    fn encode_errors() {
        let (st, p, l) = unsafe {
            encode_songs(7, &[], ptr::null(), ptr::null(), ptr::null(), false, false)
        };
        assert_eq!(st, FFICdcStatus::UnsupportedVersion);
        assert!(p.is_null() && l == 0);

        // имена включены, а массив null.
        let (st, _, _) = unsafe {
            encode_songs(1, &[[0; 16]], ptr::null(), ptr::null(), ptr::null(), false, true)
        };
        assert_eq!(st, FFICdcStatus::NullPointer);
    }

    #[test]
    fn decode_error_codes_match_codec() {
        let mut pk = FFIPacket::empty();
        let junk = *b"NOPE\x01\0\0\0\0";
        let st = unsafe { cdc_decode(junk.as_ptr(), junk.len(), true, true, true, &mut pk) };
        assert_eq!(st, FFICdcStatus::BadStartWord);
        assert_eq!(st as u32, CodecError::BadStartWord.code());
        assert!(pk.songs.is_null() && pk.playlists.is_null());
        assert_eq!(
            FFICdcStatus::from(CodecError::CountMismatch) as u32,
            CodecError::CountMismatch.code()
        );
    }
}
