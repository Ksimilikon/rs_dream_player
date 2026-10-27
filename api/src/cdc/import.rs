//! two-step packet import into the db. The core doesn't write files itself
//! (on mobile it has no rights to shared directories) — the frontend does it
//! with its platform's means. The set of functions is the same on every
//! platform:
//!
//! 1. `cdc_import_prepare` — unpack, check dates, list the files to write
//!    (songs, song covers and playlist covers: suggested name + bytes). The
//!    db doesn't change.
//! 2. the frontend writes the files wherever it can (MediaStore, SAF, a
//!    picked folder, the sandbox, ...) and collects the resulting paths.
//! 3. `cdc_import_commit` — re-check dates and write to the db in one
//!    transaction: the passed paths are saved in the db. Or
//!    `cdc_import_cancel`.
//!
//! rules:
//! - songs: hash already in the db -> not listed (the local file is left
//!   alone). A new song is saved in the db with the hash from the packet
//!   (hashes become shared between devices). A song without audio or not
//!   music -> skipped. A song without tags is not skipped: fields = "Unknown".
//! - song metadata (v0.12+): the db metadata from the packet goes over the
//!   tags of a new song; for a known song it is applied if the packet one
//!   changed later (`updated_at`, an unset date counts as 0). Sync also
//!   rewrites the file of a known song if the packet file changed later
//!   (`updated_at_file`) — over the local file (`ImportSession::replaces`);
//!   a packet import (`cdc_import_*`) leaves a known file alone. `force`
//!   concerns deleted playlists only.
//! - song covers (v0.12+, a cover file not embedded into the music file):
//!   listed for a new song and for a known one whose metadata is applied
//!   (the cover is a part of the dated db record). A song without a cover in
//!   the packet keeps its local cover.
//! - playlists are matched by name; without a name -> skipped. A known
//!   playlist is merged with the packet one entry by entry (see
//!   `storage::entries`): the entries of both, minus the ones removed in
//!   either (v0.13+ carries them), in the order of the copy changed later; a
//!   packet without entry ids (before v0.13) gets the derived ones. The
//!   cover (listed to be written only then) and the creation date come from
//!   the packet if it changed later (`updated_at`, an unset date counts as
//!   0) or the playlist isn't here; a packet without them keeps the local
//!   values. A packet is never rejected for its dates.
//! - a playlist deleted here at or after the packet one changed is skipped
//!   (the deletion wins); with `force` it comes back.
//! - deleted playlists (v0.11+): a local playlist changed at or before the
//!   deletion is soft-deleted, one changed later is kept; an unknown name is
//!   remembered as deleted.

use std::{
    collections::HashMap,
    ffi::{CString, c_char},
    path::{Path, PathBuf},
    ptr,
};

use audio_structs::{track::Track, track_virtual::TrackVirtual};
use codec::{DeletedPlaylist, Playlist, Song};
use storage::{
    Db,
    db::{DeletedPlaylistRow, PlaylistHashRow, TrackSyncRow},
    traits::indexator::Indexator,
};

use super::{FFICdcStatus, meta_from_packet, raw_slice};

/// what kind of file has to be written. `repr(u32)`: never renumber.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FFIImportFileKind {
    /// song audio file.
    Song = 1,
    /// playlist cover.
    Cover = 2,
    /// song cover (not embedded into the audio file), v0.12+.
    SongCover = 3,
}

/// one file the frontend has to write.
#[repr(C)]
pub struct FFIImportFile {
    pub kind: FFIImportFileKind,
    /// suggested file name: safe on any OS (no directories or forbidden
    /// characters), always with an extension. Uniqueness in the target
    /// directory is not guaranteed — the frontend resolves collisions.
    pub name: *const c_char,
    pub data: *const u8,
    pub data_len: usize,
}

/// list of files to write. Owned by the session: valid until
/// `cdc_import_commit`/`cdc_import_cancel`, not freed separately.
#[repr(C)]
pub struct FFIImportFiles {
    pub ptr: *const FFIImportFile,
    pub len: usize,
}

/// a planned file: song `songs[i]`, cover `playlists[i]` or song cover
/// `song_covers[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Planned {
    Song(usize),
    Cover(usize),
    SongCover(usize),
}

/// unpacked packet between `prepare` and `commit`. For C — an opaque
/// pointer; owns the data [`FFIImportFiles`] points to.
pub struct ImportSession {
    force: bool,
    /// songs with a file to write: new ones (with audio, hash not in the
    /// db) and known ones with a newer file (sync only).
    pub(crate) songs: Vec<Song>,
    /// for every song of `songs`: the local file of the known song to write
    /// over; `None` — a new song.
    local: Vec<Option<PathBuf>>,
    /// known songs that get only the newer metadata from the packet.
    metas: Vec<TrackSyncRow>,
    /// song covers to write: hash, cover name and image only.
    song_covers: Vec<Song>,
    pub(crate) playlists: Vec<Playlist>,
    /// playlists deleted on the other side (v0.11+).
    pub(crate) deleted: Vec<DeletedPlaylist>,
    plan: Vec<Planned>,
    /// owns the strings `files[i].name` points to.
    _names: Vec<CString>,
    files: Vec<FFIImportFile>,
}

// ───────────────────────────────── API ─────────────────────────────────────

/// step 1: unpacks the packet `bytes[..len]` and returns the list of files to
/// write (`out_files`) and the session (`out_session`). The db doesn't
/// change. `force` — a playlist deleted here after the packet one changed
/// comes back too (`false` by default; see the module rules).
///
/// returns `Ok`, a decode error code (1..=99) or a core error (200..). On error there is no session
/// (`*out_session == null`). An empty file list is a normal case too:
/// `cdc_import_commit` is still needed (playlists).
///
/// # Safety
/// `bytes` is valid for `len` bytes (null is allowed when `len == 0`);
/// `out_session`/`out_files` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_import_prepare(
    bytes: *const u8,
    len: usize,
    force: bool,
    out_session: *mut *mut ImportSession,
    out_files: *mut FFIImportFiles,
) -> FFICdcStatus {
    crate::panic::guard(|| {
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
        match prepare(&Global, bytes, force, false) {
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
    })
}

/// step 3: saves the import into the db. `paths[i]` is the path where
/// `files[i]` from `cdc_import_prepare` was written; null = the file wasn't
/// written (the song won't get into the db, the playlist cover stays as it
/// was). `paths_len` must equal the number of files, otherwise
/// `InvalidArgument`.
///
/// the session is always freed (on error too). If the result is not `Ok`, the
/// db is unchanged and the frontend deletes the written files itself.
///
/// # Safety
/// `session` comes from `cdc_import_prepare` and wasn't freed yet (or is
/// null); `paths` is valid for `paths_len` elements, strings are
/// nul-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_import_commit(
    session: *mut ImportSession,
    paths: *const *const c_char,
    paths_len: usize,
) -> FFICdcStatus {
    crate::panic::guard(|| {
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
    })
}

/// cancels the import and frees the session. Null is ignored. The frontend
/// deletes the written files itself.
///
/// # Safety
/// `session` comes from `cdc_import_prepare` and wasn't freed yet (or is
/// null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cdc_import_cancel(session: *mut ImportSession) {
    crate::panic::guard_unit(|| {
        if !session.is_null() {
            drop(unsafe { Box::from_raw(session) });
        }
    })
}

impl ImportSession {
    /// the files to write, in the order `commit` expects: kind, suggested
    /// name and bytes.
    pub(crate) fn planned(&self) -> impl Iterator<Item = (FFIImportFileKind, &str, &[u8])> {
        self.plan.iter().zip(&self._names).map(|(&planned, name)| {
            let (kind, data) = file_of(&self.songs, &self.song_covers, &self.playlists, planned);
            (kind, name.to_str().unwrap_or_default(), data)
        })
    }

    /// for every file of [`Self::planned`]: the local file of the same song
    /// to write over (its newer version); `None` — a new file.
    pub(crate) fn replaces(&self) -> impl Iterator<Item = Option<&Path>> {
        self.plan.iter().map(|&planned| match planned {
            Planned::Song(i) => self.local[i].as_deref(),
            Planned::Cover(_) | Planned::SongCover(_) => None,
        })
    }

    /// known songs that get only metadata.
    pub(crate) fn meta_updates(&self) -> usize {
        self.metas.len()
    }
}

// ─────────────────────────────── logic ───────────────────────────────

/// the kind and the bytes of a planned file.
fn file_of<'a>(
    songs: &'a [Song],
    song_covers: &'a [Song],
    playlists: &'a [Playlist],
    planned: Planned,
) -> (FFIImportFileKind, &'a [u8]) {
    match planned {
        Planned::Song(i) => (FFIImportFileKind::Song, &songs[i].data),
        Planned::Cover(i) => (FFIImportFileKind::Cover, &playlists[i].cover),
        Planned::SongCover(i) => (FFIImportFileKind::SongCover, &song_covers[i].cover),
    }
}

/// db access under the lock (replaced in tests).
pub(crate) trait Store {
    fn with<R>(&self, f: impl FnOnce(&Db) -> Result<R, FFICdcStatus>) -> Result<R, FFICdcStatus>;
}

/// the global core.
pub(crate) struct Global;

impl Store for Global {
    fn with<R>(&self, f: impl FnOnce(&Db) -> Result<R, FFICdcStatus>) -> Result<R, FFICdcStatus> {
        let guard = crate::core().ok_or(FFICdcStatus::NotInitialized)?;
        f(guard.get_db().ok_or(FFICdcStatus::NoDatabase)?)
    }
}

/// `rewrite_files` — a newer file of a known song is listed to be written
/// over the local one (sync); otherwise only its metadata is taken.
pub(crate) fn prepare(
    store: &impl Store,
    bytes: &[u8],
    force: bool,
    rewrite_files: bool,
) -> Result<ImportSession, FFICdcStatus> {
    let packet = codec::decode(bytes)?;

    let (songs, local, metas, song_covers, sorted) = store.with(|db| {
        let sorted = sort_playlists(db, &packet.playlists, force)?;
        let hashes: Vec<String> = packet.songs.iter().map(|s| hex(s.hash)).collect();
        let known: HashMap<String, TrackSyncRow> = db
            .get_track_sync(&hashes)
            .map_err(|_| FFICdcStatus::ReadFailed)?
            .into_iter()
            .map(|r| (r.hash.clone(), r))
            .collect();
        let (songs, local, metas, covers) = sort_songs(packet.songs, &known, rewrite_files);
        Ok((songs, local, metas, covers, sorted))
    })?;
    let playlists: Vec<Playlist> = packet
        .playlists
        .into_iter()
        .zip(sorted)
        .filter_map(|(mut p, sorted)| {
            match sorted {
                PlaylistFate::Skip => return None,
                PlaylistFate::Merge => {
                    p.cover.clear();
                    p.cover_name.clear();
                }
                PlaylistFate::Take => {}
            }
            Some(p)
        })
        .collect();
    let deleted: Vec<DeletedPlaylist> = packet
        .deleted
        .into_iter()
        .filter(|d| !d.name.is_empty())
        .collect();

    let mut plan = Vec::new();
    let mut names = Vec::new();
    for (i, s) in songs.iter().enumerate() {
        plan.push(Planned::Song(i));
        names.push(file_name_or(&s.name, &hex(s.hash), audio_ext(&s.data)));
    }
    for (i, c) in song_covers.iter().enumerate() {
        plan.push(Planned::SongCover(i));
        let fallback = format!("{}-cover", hex(c.hash));
        names.push(file_name_or(&c.cover_name, &fallback, image_ext(&c.cover)));
    }
    for (i, p) in playlists.iter().enumerate().filter(|(_, p)| !p.cover.is_empty()) {
        plan.push(Planned::Cover(i));
        names.push(file_name_or(&p.cover_name, "cover", image_ext(&p.cover)));
    }
    // safe_file_name removes control characters, so there is no NUL inside.
    let names: Vec<CString> = names
        .into_iter()
        .map(|n| CString::new(n).unwrap_or_default())
        .collect();

    // `Vec` buffers don't move when the session moves, so pointers into them
    // stay valid while the session is alive and unchanged.
    let files = plan
        .iter()
        .zip(&names)
        .map(|(&planned, name)| {
            let (kind, data) = file_of(&songs, &song_covers, &playlists, planned);
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
        local,
        metas,
        song_covers,
        playlists,
        deleted,
        plan,
        _names: names,
        files,
    })
}

pub(crate) fn commit(
    store: &impl Store,
    session: &ImportSession,
    paths: &[Option<PathBuf>],
) -> Result<(), FFICdcStatus> {
    if paths.len() != session.plan.len() {
        return Err(FFICdcStatus::InvalidArgument);
    }

    // metadata is read from the written files — before taking the lock.
    let mut tracks = Vec::new();
    let mut metas = session.metas.clone();
    let mut covers = HashMap::new();
    let mut song_covers = Vec::new();
    for (planned, path) in session.plan.iter().zip(paths) {
        let Some(path) = path else { continue };
        match *planned {
            Planned::Song(i) => {
                // an unreadable file (the header looks like music, garbage
                // follows) doesn't get into the db.
                let Ok(track) = TrackVirtual::from_file(path.clone(), false) else {
                    continue;
                };
                let song = &session.songs[i];
                tracks.push((track, hex(song.hash), session.local[i].is_some()));
                // the db metadata goes over the tags of the written file.
                if let Some(meta) = &song.meta {
                    metas.push(TrackSyncRow {
                        id: 0,
                        hash: hex(song.hash),
                        path: path.clone(),
                        cover_art: None,
                        updated_at: song.updated_at,
                        updated_at_file: song.updated_at_file,
                        meta: meta_from_packet(meta),
                    });
                }
            }
            Planned::Cover(i) => {
                covers.insert(i, path.clone());
            }
            Planned::SongCover(i) => {
                song_covers.push((hex(session.song_covers[i].hash), path.clone()));
            }
        }
    }
    let rows: Vec<PlaylistHashRow> = session
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
            entries: p.entries.iter().map(|e| (e.id, e.added_at)).collect(),
            removed: p.removed.iter().map(|e| (e.entry_id, e.removed_at)).collect(),
        })
        .collect();

    store.with(|db| {
        // the db may have changed while the frontend was writing files: a
        // playlist deleted here meanwhile is skipped.
        let fates = sort_playlists(db, &session.playlists, session.force)?;
        let playlists: Vec<PlaylistHashRow> = rows
            .into_iter()
            .zip(fates)
            .filter(|(_, fate)| *fate != PlaylistFate::Skip)
            .map(|(row, _)| row)
            .collect();
        // a new song may have appeared meanwhile — there will be no second
        // insert with the same hash (nor its metadata over the local one).
        let (tracks, late): (Vec<_>, Vec<_>) = tracks
            .into_iter()
            .partition(|(_, hash, known)| *known || !db.hash_exist(hash.clone()));
        let late: Vec<String> = late.into_iter().map(|(_, hash, _)| hash).collect();
        metas.retain(|m| !late.contains(&m.hash));
        song_covers.retain(|(hash, _)| !late.contains(hash));
        let tracks: Vec<_> = tracks.into_iter().map(|(t, hash, _)| (t, hash)).collect();
        let deleted: Vec<DeletedPlaylistRow> = session
            .deleted
            .iter()
            .map(|d| DeletedPlaylistRow {
                name: d.name.clone(),
                deleted_at: d.deleted_at,
            })
            .collect();
        db.save_import(&tracks, &metas, &song_covers, &playlists, &deleted)
            .map_err(|_| FFICdcStatus::WriteFailed)
    })
}

/// what [`sort_songs`] makes of the packet songs.
type SortedSongs = (Vec<Song>, Vec<Option<PathBuf>>, Vec<TrackSyncRow>, Vec<Song>);

/// the cover of `song` taken out of it: hash, cover name and image (`None`
/// — no cover).
fn take_cover(song: &mut Song) -> Option<Song> {
    (!song.cover.is_empty()).then(|| Song {
        hash: song.hash,
        cover_name: std::mem::take(&mut song.cover_name),
        cover: std::mem::take(&mut song.cover),
        ..Song::default()
    })
}

/// packet songs against the known ones (by hex hash): `(songs with a file
/// to write, the local file to write over for each, metadata-only updates,
/// song covers to write)` — see the module rules.
fn sort_songs(
    songs: Vec<Song>,
    known: &HashMap<String, TrackSyncRow>,
    rewrite_files: bool,
) -> SortedSongs {
    let newer = |remote: Option<u64>, local: Option<u64>| remote.unwrap_or(0) > local.unwrap_or(0);
    let (mut files, mut local, mut metas, mut covers) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for mut song in songs {
        let music = Track::is_music(&song.data);
        match known.get(&hex(song.hash)) {
            None if music => {
                covers.extend(take_cover(&mut song));
                files.push(song);
                local.push(None);
            }
            None => {}
            // a newer file is taken only with its metadata: they are dated
            // together, so the next sync doesn't want it again.
            Some(row)
                if rewrite_files
                    && music
                    && song.meta.is_some()
                    && newer(song.updated_at_file, row.updated_at_file) =>
            {
                covers.extend(take_cover(&mut song));
                files.push(song);
                local.push(Some(row.path.clone()));
            }
            Some(row) => {
                if let Some(meta) = &song.meta
                    && newer(song.updated_at, row.updated_at)
                {
                    metas.push(TrackSyncRow {
                        updated_at: song.updated_at,
                        // the local file stays.
                        updated_at_file: None,
                        meta: meta_from_packet(meta),
                        ..row.clone()
                    });
                    covers.extend(take_cover(&mut song));
                }
            }
        }
    }
    (files, local, metas, covers)
}

/// what becomes of a packet playlist (see the module rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaylistFate {
    /// new here, deleted here earlier or the packet one changed later: merged,
    /// its cover is taken.
    Take,
    /// merged, the local cover stays.
    Merge,
    /// no name, or deleted here at or after the packet one changed.
    Skip,
}

/// the fate of every packet playlist, in order.
fn sort_playlists(db: &Db, playlists: &[Playlist], force: bool) -> Result<Vec<PlaylistFate>, FFICdcStatus> {
    let local: HashMap<String, Option<u64>> = db
        .get_playlist_hashes(None)
        .map_err(|_| FFICdcStatus::ReadFailed)?
        .into_iter()
        .map(|p| (p.name, p.updated_at))
        .collect();
    let deleted: HashMap<String, u64> = db
        .get_deleted_playlists(None)
        .map_err(|_| FFICdcStatus::ReadFailed)?
        .into_iter()
        .map(|d| (d.name, d.deleted_at))
        .collect();
    let remote_at = |p: &Playlist| p.updated_at.unwrap_or(0);
    Ok(playlists
        .iter()
        .map(|p| match (local.get(&p.name), deleted.get(&p.name)) {
            _ if p.name.is_empty() => PlaylistFate::Skip,
            (Some(local), _) if remote_at(p) <= local.unwrap_or(0) => PlaylistFate::Merge,
            (Some(_), _) => PlaylistFate::Take,
            (None, Some(&at)) if remote_at(p) <= at && !force => PlaylistFate::Skip,
            (None, _) => PlaylistFate::Take,
        })
        .collect())
}

/// hash -> hex string in the db format.
pub(crate) fn hex(hash: u128) -> String {
    format!("{hash:032x}")
}

// ─────────────────────────────── names ───────────────────────────────

/// a safe file name from the packet, otherwise `fallback.ext`. A name without
/// an extension gets `ext` (detected from the content).
fn file_name_or(name: &str, fallback: &str, ext: &str) -> String {
    let name = safe_file_name(name).unwrap_or_else(|| fallback.to_owned());
    match Path::new(&name).extension() {
        Some(_) => name,
        None => format!("{name}.{ext}"),
    }
}

/// a name from a foreign packet -> a file name safe on any OS: no
/// directories (`../`), Windows-forbidden characters, control characters,
/// leading / trailing dots and spaces, reserved names (`CON`, `NUL`, ...).
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

/// audio extension by signature (the same formats as `Track::is_music`).
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

/// image extension by signature.
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

    /// db and a "frontend directory" in a temporary folder.
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
            let Some(t) = updated else {
                // an undated row (from before saves were dated): only an
                // import leaves the date unset.
                let row = PlaylistHashRow {
                    id: 0,
                    name: name.into(),
                    cover_art: None,
                    created_at: None,
                    updated_at: None,
                    hashes: Vec::new(),
                    entries: Vec::new(),
                    removed: Vec::new(),
                };
                self.db.save_import(&[], &[], &[], &[row], &[]).unwrap();
                return;
            };
            let mut p = DbPlaylist::from_tracks(vec![]);
            p.set_name(name.into());
            p.set_updated_at(t);
            self.db.save_playlist(p).unwrap();
        }

        /// what the frontend does: writes every session file, returns the paths.
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

    /// a real mp3 sample from `test_data/`.
    fn mp3() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test_data/2.mp3");
        fs::read(path).unwrap()
    }

    /// minimal WAV without tags (44-byte header + silence).
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
                ..Song::default()
            },
            // no audio — skipped.
            Song {
                hash: 2,
                name: "two.mp3".into(),
                data: vec![],
                ..Song::default()
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
        let session = prepare(&l, &bytes, false, false).unwrap();
        assert_eq!(
            file_names(&session),
            [
                (FFIImportFileKind::Song, "one.mp3".into()),
                (FFIImportFileKind::Cover, "c.png".into())
            ]
        );
        // prepare doesn't touch the db.
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

        // repeated import: the song already exists, the playlist didn't
        // change since — nothing to write, the playlist isn't doubled.
        let session = prepare(&l, &bytes, false, false).unwrap();
        assert!(file_names(&session).is_empty());
        assert_eq!(commit(&l, &session, &[]), Ok(()));
        let pls = l.db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls[0].hashes, [hex(1), hex(2)]);
    }

    #[test]
    fn unwritten_files_are_skipped() {
        let l = Local::new("skip");
        let session = prepare(&l, &packet(vec![playlist("mix", None)]), false, false).unwrap();
        assert_eq!(commit(&l, &session, &[None, None]), Ok(()));
        assert!(!l.db.hash_exist(hex(1)));
        let pls = l.db.get_playlist_hashes(None).unwrap();
        assert_eq!(pls[0].hashes.len(), 2);
        assert!(pls[0].cover_art.is_none());
    }

    #[test]
    fn paths_count_must_match() {
        let l = Local::new("count");
        let session = prepare(&l, &packet(vec![]), false, false).unwrap();
        assert_eq!(
            commit(&l, &session, &[]),
            Err(FFICdcStatus::InvalidArgument)
        );
    }

    #[test]
    fn an_older_packet_merges_and_keeps_the_local_cover() {
        let l = Local::new("older");
        l.local_playlist("mix", Some(10));
        for remote in [Some(9), None] {
            let session = prepare(&l, &packet(vec![playlist("mix", remote)]), false, false).unwrap();
            // the local copy changed later: its cover stays, none is listed.
            assert_eq!(file_names(&session), [(FFIImportFileKind::Song, "one.mp3".into())]);
            assert_eq!(commit(&l, &session, &[None]), Ok(()));
            let pls = l.db.get_playlist_hashes(None).unwrap();
            assert_eq!(pls[0].hashes, [hex(1), hex(2)], "the packet songs are merged in");
            assert_eq!(pls[0].updated_at, Some(10));
            assert!(pls[0].cover_art.is_none());
        }
    }

    #[test]
    fn a_playlist_deleted_meanwhile_is_skipped_in_commit() {
        let l = Local::new("recheck");
        let session = prepare(&l, &packet(vec![playlist("mix", Some(5))]), false, false).unwrap();
        // the playlist appeared and got deleted here while the frontend was
        // writing files: the deletion is later than the packet change.
        l.local_playlist("mix", Some(6));
        l.db.remove_playlist("mix").unwrap();
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));
        assert!(l.db.hash_exist(hex(1)), "the songs are still imported");
        assert!(l.db.get_playlist_hashes(None).unwrap().is_empty());
    }

    #[test]
    fn force_brings_back_a_playlist_deleted_later() {
        for force in [false, true] {
            let l = Local::new(if force { "force" } else { "no-force" });
            l.local_playlist("mix", Some(10));
            l.db.remove_playlist("mix").unwrap();
            let session = prepare(&l, &packet(vec![playlist("mix", Some(20))]), force, false).unwrap();
            let paths = l.write_all(&session);
            assert_eq!(commit(&l, &session, &paths), Ok(()));
            let live = l.db.get_playlist_hashes(None).unwrap();
            assert_eq!(live.len(), usize::from(force));
        }
    }

    /// a v0.11 packet with deleted playlists only.
    fn deletions(gone: &[(&str, u64)]) -> Vec<u8> {
        let deleted: Vec<DeletedPlaylist> = gone
            .iter()
            .map(|&(name, deleted_at)| DeletedPlaylist {
                name: name.into(),
                deleted_at,
            })
            .collect();
        codec::encode_with(Version::V0_11, &[], &[], &deleted).unwrap()
    }

    #[test]
    fn imported_deletion_soft_deletes_an_older_playlist() {
        let l = Local::new("deletion");
        l.local_playlist("old", Some(10));
        l.local_playlist("fresh", Some(500));
        let session = prepare(&l, &deletions(&[("old", 100), ("fresh", 100)]), false, false).unwrap();
        assert_eq!(commit(&l, &session, &[]), Ok(()));
        let live: Vec<String> = l.db.get_playlist_hashes(None).unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(live, vec!["fresh".to_string()]);
        let gone = l.db.get_deleted_playlists(None).unwrap();
        assert_eq!((gone[0].name.as_str(), gone[0].deleted_at), ("old", 100));
    }

    #[test]
    fn a_playlist_deleted_later_here_is_skipped() {
        let l = Local::new("deleted-later");
        l.local_playlist("mix", Some(10));
        l.db.remove_playlist("mix").unwrap();
        let session = prepare(&l, &packet(vec![playlist("mix", Some(20))]), false, false).unwrap();
        // the playlist (and its cover) is left out, the song still comes.
        assert_eq!(file_names(&session), [(FFIImportFileKind::Song, "one.mp3".into())]);
        assert!(session.playlists.is_empty());
    }

    /// a v0.13 packet with one playlist of `entries` (entry id, song hash;
    /// added at `updated`) and `removed` entry ids.
    fn packet_v0_13(updated: u64, entries: &[(u64, u128)], removed: &[u64]) -> Vec<u8> {
        let pl = Playlist {
            name: "mix".into(),
            tracks: entries.iter().map(|&(_, h)| h).collect(),
            entries: entries
                .iter()
                .map(|&(id, _)| codec::Entry { id, added_at: updated })
                .collect(),
            removed: removed
                .iter()
                .map(|&entry_id| codec::RemovedEntry { entry_id, removed_at: updated })
                .collect(),
            updated_at: Some(updated),
            ..Playlist::default()
        };
        codec::encode(Version::V0_13, &[], &[pl]).unwrap()
    }

    #[test]
    fn imports_merge_entries_and_respect_removals() {
        let l = Local::new("merge");
        let import = |bytes: Vec<u8>| {
            let session = prepare(&l, &bytes, false, false).unwrap();
            assert_eq!(commit(&l, &session, &[]), Ok(()));
            let pl = l.db.get_playlist_hashes(None).unwrap().remove(0);
            let ids: Vec<u64> = pl.entries.iter().map(|&(id, _)| id).collect();
            (ids, pl.hashes)
        };
        // the same song twice (two entries) and another one.
        let (ids, _) = import(packet_v0_13(10, &[(1, 7), (2, 7), (3, 8)], &[]));
        assert_eq!(ids, [1, 2, 3]);
        // an older copy that removed entry 2 and added entry 4 after entry 1.
        let (ids, hashes) = import(packet_v0_13(5, &[(1, 7), (4, 9), (3, 8)], &[2]));
        assert_eq!(ids, [1, 4, 3]);
        assert_eq!(hashes, [hex(7), hex(9), hex(8)]);
        // an even older copy that still has entry 2: it doesn't come back.
        let (ids, _) = import(packet_v0_13(1, &[(1, 7), (2, 7)], &[]));
        assert_eq!(ids, [1, 4, 3]);
    }

    #[test]
    fn local_without_date_accepts() {
        let l = Local::new("nodate");
        l.local_playlist("mix", None);
        assert!(prepare(&l, &packet(vec![playlist("mix", None)]), false, false).is_ok());
    }

    #[test]
    fn song_appeared_between_steps_is_not_duplicated() {
        let l = Local::new("race");
        let bytes = packet(vec![]);
        let first = prepare(&l, &bytes, false, false).unwrap();
        let second = prepare(&l, &bytes, false, false).unwrap();
        let paths = l.write_all(&first);
        assert_eq!(commit(&l, &first, &paths), Ok(()));
        // a second commit with the same song doesn't fail on UNIQUE(hash).
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
            ..Song::default()
        }];
        let bytes = codec::encode(Version::V0_1, &songs, &[]).unwrap();
        let session = prepare(&l, &bytes, false, false).unwrap();
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));
        let tracks = l.db.get_tracks(None, None, None, Some(hex(3))).unwrap();
        let meta = tracks[0].get_metadata().unwrap();
        assert_eq!(meta.title, "Unknown");
        assert_eq!(meta.artist, ["Unknown"]);
    }

    fn meta(title: &str) -> codec::SongMeta {
        codec::SongMeta {
            title: title.into(),
            artists: vec!["Artist".into()],
            genres: vec!["rock".into()],
            label: "fav".into(),
            volume: 0.5,
            ..codec::SongMeta::default()
        }
    }

    /// song 1 (`one.mp3`) with dates and metadata in a v0.12 packet.
    fn dated(title: &str, updated: u64, updated_file: u64, data: Vec<u8>) -> Vec<u8> {
        let songs = [Song {
            hash: 1,
            name: "one.mp3".into(),
            data,
            updated_at: Some(updated),
            updated_at_file: Some(updated_file),
            meta: Some(meta(title)),
            ..Song::default()
        }];
        codec::encode(Version::V0_12, &songs, &[]).unwrap()
    }

    fn title_of(l: &Local, hash: u128) -> (String, Option<u64>, Option<u64>) {
        let row = &l.db.get_track_sync(&[hex(hash)]).unwrap()[0];
        (row.meta.title.clone(), row.updated_at, row.updated_at_file)
    }

    #[test]
    fn new_song_takes_packet_metadata_over_tags() {
        let l = Local::new("new-meta");
        let session = prepare(&l, &dated("From db", 5, 6, mp3()), false, false).unwrap();
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));
        assert_eq!(title_of(&l, 1), ("From db".into(), Some(5), Some(6)));
        let row = &l.db.get_track_sync(&[hex(1)]).unwrap()[0];
        assert_eq!(row.meta.user_label.as_deref(), Some("fav"));
        assert_eq!(row.meta.genres, ["rock"]);
    }

    #[test]
    fn known_song_takes_newer_metadata_only() {
        let l = Local::new("known-meta");
        let session = prepare(&l, &dated("v1", 10, 10, mp3()), false, false).unwrap();
        let paths = l.write_all(&session);
        commit(&l, &session, &paths).unwrap();

        // older metadata — nothing to do.
        let session = prepare(&l, &dated("old", 9, 99, vec![]), false, false).unwrap();
        assert_eq!(session.meta_updates(), 0);
        // newer metadata: applied, the file stays (no files, the file date
        // stays local), even with a newer file in a packet import.
        let session = prepare(&l, &dated("v2", 20, 99, mp3()), false, false).unwrap();
        assert!(session.files.is_empty());
        assert_eq!(commit(&l, &session, &[]), Ok(()));
        assert_eq!(title_of(&l, 1), ("v2".into(), Some(20), Some(10)));
    }

    #[test]
    fn sync_rewrites_a_newer_file_over_the_local_one() {
        let l = Local::new("rewrite");
        let session = prepare(&l, &dated("v1", 10, 10, mp3()), false, true).unwrap();
        let paths = l.write_all(&session);
        commit(&l, &session, &paths).unwrap();
        let local = l.root.join("one.mp3");

        // the same file date: no rewrite.
        let session = prepare(&l, &dated("v1", 10, 10, mp3()), false, true).unwrap();
        assert!(session.files.is_empty());

        let session = prepare(&l, &dated("v2", 30, 30, mp3()), false, true).unwrap();
        assert_eq!(file_names(&session), [(FFIImportFileKind::Song, "one.mp3".into())]);
        assert_eq!(session.replaces().collect::<Vec<_>>(), [Some(local.as_path())]);
        // the frontend chose another place for it: the track follows.
        let moved = l.root.join("moved.mp3");
        fs::write(&moved, mp3()).unwrap();
        assert_eq!(commit(&l, &session, &[Some(moved.clone())]), Ok(()));
        let tracks = l.db.get_track_hashes(None).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!((tracks[0].1.clone(), tracks[0].2.clone()), (hex(1), moved));
        assert_eq!(title_of(&l, 1), ("v2".into(), Some(30), Some(30)));
    }

    /// `dated` with a cover file of song 1.
    fn dated_with_cover(updated: u64, data: Vec<u8>) -> Vec<u8> {
        let mut packet = codec::decode(&dated("t", updated, updated, data)).unwrap();
        packet.songs[0].cover_name = "front.gif".into();
        packet.songs[0].cover = b"GIF89a".to_vec();
        codec::encode(Version::V0_12, &packet.songs, &[]).unwrap()
    }

    fn cover_of(l: &Local, hash: u128) -> Option<PathBuf> {
        l.db.get_track_sync(&[hex(hash)]).unwrap()[0].cover_art.clone()
    }

    #[test]
    fn song_cover_is_written_for_a_new_song() {
        let l = Local::new("song-cover");
        let session = prepare(&l, &dated_with_cover(5, mp3()), false, false).unwrap();
        assert_eq!(
            file_names(&session),
            [
                (FFIImportFileKind::Song, "one.mp3".into()),
                (FFIImportFileKind::SongCover, "front.gif".into())
            ]
        );
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));
        assert_eq!(cover_of(&l, 1), Some(l.root.join("front.gif")));
    }

    #[test]
    fn song_cover_follows_newer_metadata_of_a_known_song() {
        let l = Local::new("song-cover-known");
        let session = prepare(&l, &dated("t", 10, 10, mp3()), false, false).unwrap();
        let paths = l.write_all(&session);
        commit(&l, &session, &paths).unwrap();

        // older metadata: the cover is not taken either.
        let session = prepare(&l, &dated_with_cover(9, vec![]), false, false).unwrap();
        assert!(session.files.is_empty());
        // newer: the cover comes with the metadata.
        let session = prepare(&l, &dated_with_cover(20, vec![]), false, false).unwrap();
        assert_eq!(file_names(&session), [(FFIImportFileKind::SongCover, "front.gif".into())]);
        let paths = l.write_all(&session);
        assert_eq!(commit(&l, &session, &paths), Ok(()));
        assert_eq!(cover_of(&l, 1), Some(l.root.join("front.gif")));
        // a newer packet without a cover keeps the local one.
        let session = prepare(&l, &dated("t", 30, 30, vec![]), false, false).unwrap();
        commit(&l, &session, &[]).unwrap();
        assert_eq!(cover_of(&l, 1), Some(l.root.join("front.gif")));
    }

    #[test]
    fn decode_error_passes_through() {
        let l = Local::new("decode");
        assert_eq!(
            prepare(&l, b"DPL", false, false).err(),
            Some(FFICdcStatus::UnexpectedEof)
        );
        assert_eq!(
            prepare(&l, b"junk", false, false).err(),
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
