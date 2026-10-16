//! импорт пакета в бд в два шага. Ядро само файлы не пишет (на мобильных у
//! него нет прав на общие каталоги) — это делает фронтенд средствами своей
//! платформы. Набор функций одинаков на всех платформах:
//!
//! 1. `cdc_import_prepare` — распаковка, проверка дат, список файлов на запись
//!    (песни и обложки: предлагаемое имя + байты). Бд не меняется.
//! 2. фронтенд пишет файлы куда может (MediaStore, SAF, выбранная папка,
//!    песочница, ...) и собирает итоговые пути.
//! 3. `cdc_import_commit` — повторная проверка дат и запись в бд одной
//!    транзакцией: в бд сохраняются переданные пути. Либо `cdc_import_cancel`.
//!
//! правила:
//! - песни: хеш уже есть в бд -> в список не попадает (локальный файл не
//!   трогаем). Новая песня сохраняется в бд с хешем из пакета (хеши
//!   становятся общими для устройств). Песня без аудио или не-музыка ->
//!   пропуск. Песня без тегов не пропускается: поля = "Unknown".
//! - плейлисты сопоставляются по имени; без имени -> пропуск. Список песен
//!   переписывается целиком. Если обложки/дат в пакете нет (или файл обложки
//!   не записан), локальные значения сохраняются.
//! - даты (без `force`): пакет отклоняется целиком (`Outdated`), если хотя бы
//!   у одного плейлиста локальный `updated_at` задан и пакетный либо не
//!   задан, либо меньше. Без локальной даты сравнивать не с чем — принимаем.

use std::{
    collections::HashMap,
    ffi::{CString, c_char},
    path::{Path, PathBuf},
    ptr,
};

use audio_structs::{track::Track, track_virtual::TrackVirtual};
use codec::{Playlist, Song};
use storage::{Db, db::PlaylistHashRow, traits::indexator::Indexator};

use super::{FFICdcStatus, raw_slice};
use crate::GLOBAL_STATE;

/// что за файл нужно записать. `repr(u32)`: не перенумеровывать.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIImportFileKind {
    /// аудиофайл песни.
    Song = 1,
    /// обложка плейлиста.
    Cover = 2,
}

/// один файл, который фронтенд должен записать.
#[repr(C)]
pub struct FFIImportFile {
    pub kind: FFIImportFileKind,
    /// предлагаемое имя файла: безопасное на любой ОС (без каталогов и
    /// запрещённых символов), всегда с расширением. Уникальность в целевом
    /// каталоге не гарантирована — коллизии решает фронтенд.
    pub name: *const c_char,
    pub data: *const u8,
    pub data_len: usize,
}

/// список файлов на запись. Принадлежит сессии: валиден до
/// `cdc_import_commit`/`cdc_import_cancel`, отдельно не освобождается.
#[repr(C)]
pub struct FFIImportFiles {
    pub ptr: *const FFIImportFile,
    pub len: usize,
}

/// файл из плана: песня `songs[i]` или обложка `playlists[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Planned {
    Song(usize),
    Cover(usize),
}

/// распакованный пакет между `prepare` и `commit`. Для C — непрозрачный
/// указатель; владеет данными, на которые указывает [`FFIImportFiles`].
pub struct ImportSession {
    force: bool,
    /// только новые песни (с аудио, хеша нет в бд).
    songs: Vec<Song>,
    playlists: Vec<Playlist>,
    plan: Vec<Planned>,
    /// владеет строками, на которые указывают `files[i].name`.
    _names: Vec<CString>,
    files: Vec<FFIImportFile>,
}

// ───────────────────────────────── API ─────────────────────────────────────

/// шаг 1: распаковывает пакет `bytes[..len]`, проверяет даты и отдаёт список
/// файлов на запись (`out_files`) и сессию (`out_session`). Бд не меняется.
/// `force` — принять, даже если пакет устарел (по умолчанию `false`).
///
/// возвращает `Ok`, `Outdated` (пакет старше локальных данных), код ошибки
/// декодирования (1..=99) или ошибки ядра (200..). При ошибке сессии нет
/// (`*out_session == null`). Пустой список файлов — тоже нормальный случай:
/// `cdc_import_commit` всё равно нужен (плейлисты).
///
/// # Safety
/// `bytes` валиден на `len` байт (null допустим при `len == 0`);
/// `out_session`/`out_files` — валидные указатели.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_import_prepare(
    bytes: *const u8,
    len: usize,
    force: bool,
    out_session: *mut *mut ImportSession,
    out_files: *mut FFIImportFiles,
) -> FFICdcStatus {
    if out_session.is_null() || out_files.is_null() {
        return FFICdcStatus::NullPointer;
    }
    unsafe {
        *out_session = ptr::null_mut();
        *out_files = FFIImportFiles {
            ptr: ptr::null(),
            len: 0,
        };
    }
    let Some(bytes) = (unsafe { raw_slice(bytes, len) }) else {
        return FFICdcStatus::NullPointer;
    };
    match prepare(&Global, bytes, force) {
        Ok(session) => {
            let session = Box::new(session);
            unsafe {
                *out_files = FFIImportFiles {
                    ptr: session.files.as_ptr(),
                    len: session.files.len(),
                };
                *out_session = Box::into_raw(session);
            }
            FFICdcStatus::Ok
        }
        Err(e) => e,
    }
}

/// шаг 3: сохраняет импорт в бд. `paths[i]` — путь, по которому записан
/// файл `files[i]` из `cdc_import_prepare`; null = файл не записан (песня не
/// попадёт в бд, обложка плейлиста останется прежней). `paths_len` должен
/// равняться числу файлов, иначе `InvalidArgument`.
///
/// сессия освобождается всегда (и при ошибке). Если вернулся не `Ok`, бд не
/// изменена, а записанные файлы фронтенд удаляет сам.
///
/// # Safety
/// `session` получена из `cdc_import_prepare` и ещё не освобождалась (или
/// null); `paths` валиден на `paths_len` элементов, строки нуль-терминированы.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_import_commit(
    session: *mut ImportSession,
    paths: *const *const c_char,
    paths_len: usize,
) -> FFICdcStatus {
    if session.is_null() {
        return FFICdcStatus::NullPointer;
    }
    let session = unsafe { Box::from_raw(session) };
    let Some(raw) = (unsafe { raw_slice(paths, paths_len) }) else {
        return FFICdcStatus::NullPointer;
    };
    let mut parsed = Vec::with_capacity(raw.len());
    for &p in raw {
        parsed.push(match p.is_null() {
            true => None,
            false => match crate::ptr_to_path(p) {
                Ok(path) => Some(path),
                Err(_) => return FFICdcStatus::InvalidUtf8,
            },
        });
    }
    match commit(&Global, &session, &parsed) {
        Ok(()) => FFICdcStatus::Ok,
        Err(e) => e,
    }
}

/// отменяет импорт и освобождает сессию. Null игнорируется. Записанные
/// файлы фронтенд удаляет сам.
///
/// # Safety
/// `session` получена из `cdc_import_prepare` и ещё не освобождалась (или
/// null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_import_cancel(session: *mut ImportSession) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

// ─────────────────────────────── logic ───────────────────────────────

/// доступ к бд под блокировкой (подменяется в тестах).
trait Store {
    fn with<R>(&self, f: impl FnOnce(&Db) -> Result<R, FFICdcStatus>) -> Result<R, FFICdcStatus>;
}

/// глобальное ядро.
struct Global;

impl Store for Global {
    fn with<R>(&self, f: impl FnOnce(&Db) -> Result<R, FFICdcStatus>) -> Result<R, FFICdcStatus> {
        let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
        f(guard.get_db().ok_or(FFICdcStatus::NoDatabase)?)
    }
}

fn prepare(store: &impl Store, bytes: &[u8], force: bool) -> Result<ImportSession, FFICdcStatus> {
    let packet = codec::decode(bytes)?;

    let songs: Vec<Song> = store.with(|db| {
        check_dates(db, &packet.playlists, force)?;
        Ok(packet
            .songs
            .into_iter()
            .filter(|s| Track::is_music(&s.data) && !db.hash_exist(hex(s.hash)))
            .collect())
    })?;
    let playlists: Vec<Playlist> = packet
        .playlists
        .into_iter()
        .filter(|p| !p.name.is_empty())
        .collect();

    let mut plan = Vec::new();
    let mut names = Vec::new();
    for (i, s) in songs.iter().enumerate() {
        plan.push(Planned::Song(i));
        names.push(file_name_or(&s.name, &hex(s.hash), audio_ext(&s.data)));
    }
    for (i, p) in playlists.iter().enumerate().filter(|(_, p)| !p.cover.is_empty()) {
        plan.push(Planned::Cover(i));
        names.push(file_name_or(&p.cover_name, "cover", image_ext(&p.cover)));
    }
    // safe_file_name убирает управляющие символы, так что NUL внутри нет.
    let names: Vec<CString> = names
        .into_iter()
        .map(|n| CString::new(n).unwrap_or_default())
        .collect();

    // буферы `Vec` не двигаются при перемещении сессии, поэтому указатели
    // на них остаются валидными, пока сессия жива и не меняется.
    let files = plan
        .iter()
        .zip(&names)
        .map(|(&planned, name)| {
            let (kind, data) = match planned {
                Planned::Song(i) => (FFIImportFileKind::Song, &songs[i].data),
                Planned::Cover(i) => (FFIImportFileKind::Cover, &playlists[i].cover),
            };
            FFIImportFile {
                kind,
                name: name.as_ptr(),
                data: data.as_ptr(),
                data_len: data.len(),
            }
        })
        .collect();

    Ok(ImportSession {
        force,
        songs,
        playlists,
        plan,
        _names: names,
        files,
    })
}

fn commit(
    store: &impl Store,
    session: &ImportSession,
    paths: &[Option<PathBuf>],
) -> Result<(), FFICdcStatus> {
    if paths.len() != session.plan.len() {
        return Err(FFICdcStatus::InvalidArgument);
    }

    // метаданные читаются из записанных файлов — до блокировки.
    let mut tracks = Vec::new();
    let mut covers = HashMap::new();
    for (planned, path) in session.plan.iter().zip(paths) {
        let Some(path) = path else { continue };
        match *planned {
            Planned::Song(i) => {
                // нечитаемый файл (заголовок похож на музыку, а дальше мусор)
                // в бд не попадает.
                if let Ok(track) = TrackVirtual::from_file(path.clone(), false) {
                    tracks.push((track, hex(session.songs[i].hash)));
                }
            }
            Planned::Cover(i) => {
                covers.insert(i, path.clone());
            }
        }
    }
    let playlists: Vec<PlaylistHashRow> = session
        .playlists
        .iter()
        .enumerate()
        .map(|(i, p)| PlaylistHashRow {
            id: 0,
            name: p.name.clone(),
            cover_art: covers.remove(&i),
            created_at: p.created_at,
            updated_at: p.updated_at,
            hashes: p.tracks.iter().map(|&h| hex(h)).collect(),
        })
        .collect();

    store.with(|db| {
        // бд могла измениться, пока фронтенд писал файлы.
        check_dates(db, &session.playlists, session.force)?;
        // песня могла появиться за это время — второй вставки с тем же
        // хешем не будет.
        let tracks: Vec<_> = tracks
            .into_iter()
            .filter(|(_, hash)| !db.hash_exist(hash.clone()))
            .collect();
        db.save_import(&tracks, &playlists)
            .map_err(|_| FFICdcStatus::WriteFailed)
    })
}

/// `Outdated`, если пакет старше локальных плейлистов (см. правила модуля).
fn check_dates(db: &Db, playlists: &[Playlist], force: bool) -> Result<(), FFICdcStatus> {
    if force {
        return Ok(());
    }
    let local: HashMap<String, Option<u64>> = db
        .get_playlist_hashes(None)
        .map_err(|_| FFICdcStatus::ReadFailed)?
        .into_iter()
        .map(|p| (p.name, p.updated_at))
        .collect();
    for p in playlists {
        if let Some(Some(local)) = local.get(&p.name) {
            match p.updated_at {
                Some(remote) if remote >= *local => {}
                _ => return Err(FFICdcStatus::Outdated),
            }
        }
    }
    Ok(())
}

/// хеш -> hex-строка в формате бд.
fn hex(hash: u128) -> String {
    format!("{hash:032x}")
}

// ─────────────────────────────── names ───────────────────────────────

/// безопасное имя файла из пакета, иначе `fallback.ext`. Без расширения в
/// имени добавляется `ext` (определённое по содержимому).
fn file_name_or(name: &str, fallback: &str, ext: &str) -> String {
    let name = safe_file_name(name).unwrap_or_else(|| fallback.to_owned());
    match Path::new(&name).extension() {
        Some(_) => name,
        None => format!("{name}.{ext}"),
    }
}

/// имя из чужого пакета -> имя файла, безопасное на любой ОС: без каталогов
/// (`../`), запрещённых символов Windows, управляющих символов, ведущих /
/// хвостовых точек и пробелов, зарезервированных имён (`CON`, `NUL`, ...).
fn safe_file_name(name: &str) -> Option<String> {
    let base = Path::new(name).file_name()?.to_str()?;
    let clean: String = base
        .chars()
        .map(|c| match c.is_control() || r#"<>:"/\|?*"#.contains(c) {
            true => '_',
            false => c,
        })
        .collect();
    let clean = clean.trim_matches(|c| c == ' ' || c == '.');
    if clean.is_empty() {
        return None;
    }
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = clean.split('.').next().unwrap_or_default().to_ascii_uppercase();
    match RESERVED.contains(&stem.as_str()) {
        true => Some(format!("_{clean}")),
        false => Some(clean.to_owned()),
    }
}

/// расширение аудио по сигнатуре (те же форматы, что `Track::is_music`).
fn audio_ext(data: &[u8]) -> &'static str {
    match data {
        [b'f', b'L', b'a', b'C', ..] => "flac",
        [b'O', b'g', b'g', b'S', ..] => "ogg",
        [b'M', b'T', b'h', b'd', ..] => "mid",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E', ..] => "wav",
        [_, _, _, _, b'f', b't', b'y', b'p', ..] => "m4a",
        _ => "mp3",
    }
}

/// расширение картинки по сигнатуре.
fn image_ext(data: &[u8]) -> &'static str {
    match data {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xFF, 0xD8, 0xFF, ..] => "jpg",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        _ => "img",
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::CStr, fs, slice};

    use super::*;
    use audio_structs::playlist::Playlist as DbPlaylist;
    use codec::Version;

    /// бд и «каталог фронтенда» во временной папке.
    struct Local {
        db: Db,
        root: PathBuf,
    }

    impl Local {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("cdc_import_{tag}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            let db = Db::init(root.join("db.sqlite")).unwrap();
            Local { db, root }
        }

        fn local_playlist(&self, name: &str, updated: Option<u64>) {
            let mut p = DbPlaylist::from_tracks(vec![]);
            p.set_name(name.into());
            if let Some(t) = updated {
                p.set_updated_at(t);
            }
            self.db.save_playlist(p).unwrap();
        }

        /// то, что делает фронтенд: пишет все файлы сессии, возвращает пути.
        fn write_all(&self, session: &ImportSession) -> Vec<Option<PathBuf>> {
            session
                .files
                .iter()
                .map(|f| {
                    let name = unsafe { CStr::from_ptr(f.name) }.to_str().unwrap();
                    let data = unsafe { slice::from_raw_parts(f.data, f.data_len) };
                    let path = self.root.join(name);
                    fs::write(&path, data).unwrap();
                    Some(path)
                })
                .collect()
        }
    }

    impl Drop for Local {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    impl Store for Local {
        fn with<R>(
            &self,
            f: impl FnOnce(&Db) -> Result<R, FFICdcStatus>,
        ) -> Result<R, FFICdcStatus> {
            f(&self.db)
        }
    }

    /// реальный mp3-семпл из `test_data/`.
    fn mp3() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test_data/2.mp3");
        fs::read(path).unwrap()
    }

    /// минимальный WAV без тегов (44-байтный заголовок + тишина).
    fn tagless_wav() -> Vec<u8> {
        let data_len = 400u32;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&8000u32.to_le_bytes());
        v.extend_from_slice(&16000u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        v.resize(v.len() + data_len as usize, 0);
        v
    }

    fn packet(playlists: Vec<Playlist>) -> Vec<u8> {
        let songs = [
            Song {
                hash: 1,
                name: "one.mp3".into(),
                data: mp3(),
            },
            // без аудио — пропуск.
            Song {
                hash: 2,
                name: "two.mp3".into(),
                data: vec![],
            },
        ];
        codec::encode(Version::V0_1, &songs, &playlists).unwrap()
    }

    fn playlist(name: &str, updated: Option<u64>) -> Playlist {
        Playlist {
            name: name.into(),
            tracks: vec![1, 2],
            updated_at: updated,
            cover_name: "c.png".into(),
            cover: vec![0x89, b'P', b'N', b'G'],
            ..Playlist::default()
        }
    }

    fn file_names(session: &ImportSession) -> Vec<(FFIImportFileKind, String)> {
        session
            .files
            .iter()
            .map(|f| {
                let name = unsafe { CStr::from_ptr(f.name) };
                (f.kind, name.to_string_lossy().into_owned())
            })
            .collect()
    }

    #[test]
    fn prepare_lists_files_and_commit_saves_paths() {
        let l = Local::new("ok");
        let bytes = packet(vec![playlist("mix", Some(5))]);
        let session = prepare(&l, &bytes, false).unwrap();
        assert_eq!(
            file_names(&session),
            [
                (FFIImportFileKind::Song, "one.mp3".into()),
                (FFIImportFileKind::Cover, "c.png".into())
            ]
        );
        // prepare бд не трогает.
        assert!(!l.db.hash_exist(hex(1)));

        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));

        let tracks = l.db.get_track_hashes(None).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].1, hex(1));
        assert_eq!(tracks[0].2, l.root.join("one.mp3"));
        let pls = l.db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls[0].hashes, [hex(1), hex(2)]);
        assert_eq!(pls[0].updated_at, Some(5));
        assert_eq!(pls[0].cover_art, Some(l.root.join("c.png")));

        // повторный импорт: песня уже есть — в списке только обложка.
        let session = prepare(&l, &bytes, false).unwrap();
        assert_eq!(
            file_names(&session),
            [(FFIImportFileKind::Cover, "c.png".into())]
        );
    }

    #[test]
    fn unwritten_files_are_skipped() {
        let l = Local::new("skip");
        let session = prepare(&l, &packet(vec![playlist("mix", None)]), false).unwrap();
        assert_eq!(commit(&l, &session, &[None, None]), Ok(()));
        assert!(!l.db.hash_exist(hex(1)));
        let pls = l.db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls[0].hashes.len(), 2);
        assert!(pls[0].cover_art.is_none());
    }

    #[test]
    fn paths_count_must_match() {
        let l = Local::new("count");
        let session = prepare(&l, &packet(vec![]), false).unwrap();
        assert_eq!(
            commit(&l, &session, &[]),
            Err(FFICdcStatus::InvalidArgument)
        );
    }

    #[test]
    fn outdated_is_rejected_in_prepare() {
        let l = Local::new("outdated");
        l.local_playlist("mix", Some(10));
        for remote in [Some(9), None] {
            let bytes = packet(vec![playlist("mix", remote)]);
            assert_eq!(
                prepare(&l, &bytes, false).err(),
                Some(FFICdcStatus::Outdated)
            );
        }
        // равная или более новая дата — ok.
        assert!(prepare(&l, &packet(vec![playlist("mix", Some(10))]), false).is_ok());
    }

    #[test]
    fn outdated_is_rechecked_in_commit() {
        let l = Local::new("recheck");
        let session = prepare(&l, &packet(vec![playlist("mix", Some(5))]), false).unwrap();
        // пока фронтенд писал файлы, локальный плейлист обновился.
        l.local_playlist("mix", Some(10));
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Err(FFICdcStatus::Outdated));
        assert!(!l.db.hash_exist(hex(1)));
    }

    #[test]
    fn force_overrides_outdated() {
        let l = Local::new("force");
        l.local_playlist("mix", Some(10));
        let session = prepare(&l, &packet(vec![playlist("mix", Some(1))]), true).unwrap();
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));
        let pls = l.db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls[0].updated_at, Some(1));
    }

    #[test]
    fn local_without_date_accepts() {
        let l = Local::new("nodate");
        l.local_playlist("mix", None);
        assert!(prepare(&l, &packet(vec![playlist("mix", None)]), false).is_ok());
    }

    #[test]
    fn song_appeared_between_steps_is_not_duplicated() {
        let l = Local::new("race");
        let bytes = packet(vec![]);
        let first = prepare(&l, &bytes, false).unwrap();
        let second = prepare(&l, &bytes, false).unwrap();
        let paths = l.write_all(&first);
        assert_eq!(commit(&l, &first, &paths), Ok(()));
        // второй commit с той же песней не падает на UNIQUE(hash).
        assert_eq!(commit(&l, &second, &[None]), Ok(()));
        assert_eq!(l.db.get_track_hashes(None).unwrap().len(), 1);
    }

    #[test]
    fn tagless_song_gets_unknown() {
        let l = Local::new("tagless");
        let songs = [Song {
            hash: 3,
            name: "raw.wav".into(),
            data: tagless_wav(),
        }];
        let bytes = codec::encode(Version::V0_1, &songs, &[]).unwrap();
        let session = prepare(&l, &bytes, false).unwrap();
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));
        let tracks = l.db.get_tracks(None, None, None, Some(hex(3))).unwrap();
        let meta = tracks[0].get_metadata().unwrap();
        assert_eq!(meta.title, "Unknown");
        assert_eq!(meta.artist, ["Unknown"]);
    }

    #[test]
    fn decode_error_passes_through() {
        let l = Local::new("decode");
        assert_eq!(
            prepare(&l, b"DPL", false).err(),
            Some(FFICdcStatus::UnexpectedEof)
        );
        assert_eq!(
            prepare(&l, b"junk", false).err(),
            Some(FFICdcStatus::BadStartWord)
        );
    }

    #[test]
    fn ffi_null_handling() {
        let mut session = ptr::null_mut();
        let mut files = FFIImportFiles {
            ptr: ptr::null(),
            len: 0,
        };
        let st = unsafe { cdc_import_prepare(ptr::null(), 5, false, &mut session, &mut files) };
        assert_eq!(st, FFICdcStatus::NullPointer);
        assert!(session.is_null());
        assert_eq!(
            unsafe { cdc_import_commit(ptr::null_mut(), ptr::null(), 0) },
            FFICdcStatus::NullPointer
        );
        unsafe { cdc_import_cancel(ptr::null_mut()) };
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_file_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert!(safe_file_name(r"..\..\x.mp3").unwrap().ends_with("x.mp3"));
        assert_eq!(safe_file_name("a:b?.mp3").as_deref(), Some("a_b_.mp3"));
        assert_eq!(safe_file_name("con.mp3").as_deref(), Some("_con.mp3"));
        assert_eq!(safe_file_name(".."), None);
        assert_eq!(safe_file_name(" . "), None);
        assert_eq!(file_name_or("", "abc", "flac"), "abc.flac");
        assert_eq!(file_name_or("song", "abc", "flac"), "song.flac");
    }

    #[test]
    fn sniffed_extensions() {
        assert_eq!(audio_ext(&mp3()), "mp3");
        assert_eq!(audio_ext(&tagless_wav()), "wav");
        assert_eq!(image_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), "jpg");
    }
}
