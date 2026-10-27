//! what the sync protocol carries, built from / compared with the db. Every
//! payload is a codec packet:
//! - index: every playlist (name, dates, entries: song hashes with entry
//!   ids, removed entries) and the songs that are in some playlist and exist
//!   locally: hashes and change dates — no files, no metadata;
//! - want: song hashes and playlist names (empty track lists). A song with
//!   (empty) metadata is wanted for its metadata only, without one — whole;
//! - data: portions of songs with names, files, dates, metadata and covers;
//!   then portions of songs with dates, metadata and covers only (no files);
//!   the last portion — the playlists with covers (the songs only by hash).
//!   A one-way send ([`Selection`]) chooses what and with which [`Flags`],
//!   like an export (without audio — the metadata of the songs only).
//!
//! every payload is a codec packet of [`VERSION`]; a peer that sends another
//! version (an older app) is refused with [`WireStatus::VersionMismatch`].
//!
//! only songs that are in some playlist are synced. A playlist is wanted if
//! it is missing locally, the remote one is newer (`updated_at`, an unset
//! date counts as 0) or has entries this side doesn't know / removed entries
//! this side still has; a playlist deleted locally is wanted only if the
//! remote one is newer than the deletion. A wanted playlist is merged with
//! the local one entry by entry on import (see `cdc::import`), so edits made
//! on both sides are both kept. A remote deletion is wanted if the local
//! playlist changed at or before it, an older local deletion or no local
//! playlist at all (it travels on). A song is
//! wanted if a wanted remote playlist or a local playlist refers to it, the
//! remote side has it and the local side doesn't. A song both sides have is
//! wanted whole if the remote file changed later (`updated_at_file`, an
//! unset date counts as 0) — it is written over the local file — or for its
//! metadata only if the remote db record changed later (`updated_at`). A
//! song cover (a file not embedded into the music file) is a part of the db
//! record: it travels with the metadata.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use codec::{DeletedPlaylist, Playlist, Song, SongMeta, Version};
use storage::entries;
use storage::{
    Db,
    db::{DeletedPlaylistRow, PlaylistHashRow, TrackSyncRow},
};

use super::frame::WireStatus;
use crate::cdc::{
    deleted_from_db, entries_to_packet, hex, parse_hash, read_cover, read_song_cover,
    removed_to_packet, with_info,
};

/// the codec version of every sync payload; both sides must use it.
pub const VERSION: Version = Version::V0_13;
/// a data portion is cut once its files and covers reach this size (one song
/// is never split, so a single larger song makes a larger portion).
pub const PORTION_BYTES: u64 = 16 << 20;

fn internal<E>(_: E) -> WireStatus {
    WireStatus::Internal
}

fn invalid<E>(_: E) -> WireStatus {
    WireStatus::InvalidPacket
}

/// a received payload is a packet of [`VERSION`]: another version is a
/// peer of another app version (`VersionMismatch`), garbage — `InvalidPacket`.
pub fn check_version(bytes: &[u8]) -> Result<(), WireStatus> {
    match codec::peek_version(bytes) {
        Ok(v) if v == VERSION => Ok(()),
        Ok(_) | Err(codec::CodecError::UnsupportedVersion) => Err(WireStatus::VersionMismatch),
        Err(_) => Err(WireStatus::InvalidPacket),
    }
}

/// a db playlist -> a packet one: entries and removed entries, no cover.
fn playlist_of(p: &PlaylistHashRow) -> Playlist {
    Playlist {
        tracks: p.hashes.iter().filter_map(|h| parse_hash(h)).collect(),
        entries: entries_to_packet(&p.entries),
        removed: removed_to_packet(&p.removed),
        name: p.name.clone(),
        created_at: p.created_at,
        updated_at: p.updated_at,
        ..Playlist::default()
    }
}

/// the index of the local library.
pub fn index(db: &Db) -> Result<Vec<u8>, WireStatus> {
    let playlists = db.get_playlist_hashes(None).map_err(internal)?;
    let mut seen = HashSet::new();
    let refs: Vec<String> = playlists
        .iter()
        .flat_map(|p| &p.hashes)
        .filter(|h| seen.insert(h.as_str()))
        .cloned()
        .collect();
    let songs = db
        .get_track_sync(&refs)
        .map_err(internal)?
        .iter()
        .filter_map(|row| {
            Some(Song {
                hash: parse_hash(&row.hash)?,
                updated_at: row.updated_at,
                updated_at_file: row.updated_at_file,
                ..Song::default()
            })
        })
        .collect::<Vec<_>>();
    let playlists = playlists.iter().map(playlist_of).collect::<Vec<_>>();
    let deleted: Vec<DeletedPlaylist> = db
        .get_deleted_playlists(None)
        .map_err(internal)?
        .into_iter()
        .map(deleted_from_db)
        .collect();
    codec::encode_with(VERSION, &songs, &playlists, &deleted).map_err(internal)
}

/// what the local side lacks compared to a remote index.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Want {
    /// songs wanted whole: the file and the metadata.
    pub songs: Vec<u128>,
    /// songs wanted for their metadata only (the local file is up to date).
    pub metas: Vec<u128>,
    pub playlists: Vec<String>,
}

impl Want {
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.metas.is_empty() && self.playlists.is_empty()
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireStatus> {
        let whole = self.songs.iter().map(|&hash| Song {
            hash,
            ..Song::default()
        });
        let metas = self.metas.iter().map(|&hash| Song {
            hash,
            meta: Some(SongMeta::default()),
            ..Song::default()
        });
        let songs: Vec<Song> = whole.chain(metas).collect();
        let playlists: Vec<Playlist> = self
            .playlists
            .iter()
            .map(|name| Playlist {
                name: name.clone(),
                ..Playlist::default()
            })
            .collect();
        codec::encode(VERSION, &songs, &playlists).map_err(internal)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WireStatus> {
        check_version(bytes)?;
        let packet = codec::decode(bytes).map_err(invalid)?;
        let (metas, songs): (Vec<&Song>, Vec<&Song>) =
            packet.songs.iter().partition(|s| s.meta.is_some());
        Ok(Self {
            songs: songs.iter().map(|s| s.hash).collect(),
            metas: metas.iter().map(|s| s.hash).collect(),
            playlists: packet.playlists.into_iter().map(|p| p.name).collect(),
        })
    }
}

/// compares a remote index with the local library.
pub fn want(db: &Db, remote_index: &[u8]) -> Result<Want, WireStatus> {
    check_version(remote_index)?;
    let remote = codec::decode(remote_index).map_err(invalid)?;
    let local_playlists = db.get_playlist_hashes(None).map_err(internal)?;
    // the remote songs this side has, with their dates.
    let remote_hashes: Vec<String> = remote.songs.iter().map(|s| hex(s.hash)).collect();
    let local_songs: HashMap<u128, TrackSyncRow> = db
        .get_track_sync(&remote_hashes)
        .map_err(internal)?
        .into_iter()
        .filter_map(|row| Some((parse_hash(&row.hash)?, row)))
        .collect();
    let local_deleted = db.get_deleted_playlists(None).map_err(internal)?;
    // the time of the last local change: an update or the deletion.
    let local_dates: HashMap<&str, u64> = local_playlists
        .iter()
        .map(|p| (p.name.as_str(), p.updated_at.unwrap_or(0)))
        .chain(local_deleted.iter().map(|d| (d.name.as_str(), d.deleted_at)))
        .collect();
    let deleted_here: HashMap<&str, u64> = local_deleted
        .iter()
        .map(|d| (d.name.as_str(), d.deleted_at))
        .collect();

    let live: HashMap<&str, &PlaylistHashRow> =
        local_playlists.iter().map(|p| (p.name.as_str(), p)).collect();
    let wanted: Vec<&Playlist> = remote
        .playlists
        .iter()
        .filter(|p| !p.name.is_empty())
        .filter(|p| {
            let remote_at = p.updated_at.unwrap_or(0);
            match (live.get(p.name.as_str()), deleted_here.get(p.name.as_str())) {
                (Some(local), _) => remote_at > local.updated_at.unwrap_or(0) || has_news(p, local),
                (None, Some(&at)) => remote_at > at,
                (None, None) => true,
            }
        })
        .collect();
    // remote deletions this side hasn't applied: the local playlist changed
    // at or before it, an older local deletion, or nothing here at all.
    let wanted_deletions = remote
        .deleted
        .iter()
        .filter(|d| !d.name.is_empty())
        .filter(|d| match (deleted_here.get(d.name.as_str()), local_dates.get(d.name.as_str())) {
            (Some(&at), _) => d.deleted_at > at,
            (None, Some(&changed)) => changed <= d.deleted_at,
            (None, None) => true,
        })
        .map(|d| d.name.clone());

    let remote_songs: HashSet<u128> = remote.songs.iter().map(|s| s.hash).collect();
    let local_refs = local_playlists
        .iter()
        .flat_map(|p| &p.hashes)
        .filter_map(|h| parse_hash(h));
    let mut seen = HashSet::new();
    let mut songs: Vec<u128> = wanted
        .iter()
        .flat_map(|p| p.tracks.iter().copied())
        .chain(local_refs)
        .filter(|h| remote_songs.contains(h) && !local_songs.contains_key(h) && seen.insert(*h))
        .collect();
    // songs both sides have: a newer file — whole, newer metadata — only it.
    let newer = |remote: Option<u64>, local: Option<u64>| remote.unwrap_or(0) > local.unwrap_or(0);
    let mut metas = Vec::new();
    for s in &remote.songs {
        let Some(local) = local_songs.get(&s.hash) else {
            continue;
        };
        if !seen.insert(s.hash) {
            continue;
        }
        if newer(s.updated_at_file, local.updated_at_file) {
            songs.push(s.hash);
        } else if newer(s.updated_at, local.updated_at) {
            metas.push(s.hash);
        }
    }

    Ok(Want {
        songs,
        metas,
        playlists: wanted
            .into_iter()
            .map(|p| p.name.clone())
            .chain(wanted_deletions)
            .collect(),
    })
}

/// the remote copy `remote` of the live local playlist `local` has an entry
/// this side doesn't know (neither live nor removed) or has removed an entry
/// this side still has.
fn has_news(remote: &Playlist, local: &PlaylistHashRow) -> bool {
    let live: HashSet<u64> = local.entries.iter().map(|&(id, _)| id).collect();
    let removed: HashSet<u64> = local.removed.iter().map(|&(id, _)| id).collect();
    let ids: Vec<u64> = match remote.entries.len() == remote.tracks.len() {
        true => remote.entries.iter().map(|e| e.id).collect(),
        false => {
            let hashes: Vec<String> = remote.tracks.iter().map(|&h| hex(h)).collect();
            entries::legacy_entries(&remote.name, &hashes, 0).into_iter().map(|e| e.id).collect()
        }
    };
    ids.iter().any(|id| !live.contains(id) && !removed.contains(id))
        || remote.removed.iter().any(|e| live.contains(&e.entry_id))
}

/// what one data portion carries (for progress).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PortionStats {
    pub songs: u32,
    pub playlists: u32,
}

/// what to send one way — the same choice as an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// every track and playlist.
    Library,
    /// playlists by name with all their songs.
    Playlists(Vec<String>),
    /// tracks by id, without playlists.
    Tracks(Vec<i64>),
}

/// what goes into the data portions — the same flags as an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    /// song files (without them the receiver gets only the playlists).
    pub audio: bool,
    /// song file names (the receiver names the files by hash otherwise).
    pub names: bool,
    /// playlist covers and song covers (the ones not embedded into the
    /// music files).
    pub covers: bool,
}

impl Flags {
    pub const ALL: Self = Self {
        audio: true,
        names: true,
        covers: true,
    };
}

/// a selection can't be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    /// some playlist / track of the selection is not in the db.
    NotFound,
    Db,
}

/// a song to send: what is read from disk as it is sent + the rest.
struct Outgoing {
    /// local music file.
    path: PathBuf,
    /// local cover file.
    cover: Option<PathBuf>,
    /// the song without its files: hash, dates, metadata.
    song: Song,
}

/// songs and playlists to send, cut into data portions as they are sent.
pub struct Plan {
    /// songs to send with their files, in order.
    songs: Vec<Outgoing>,
    next: usize,
    /// songs to send without files (metadata and covers only).
    metas: Vec<Outgoing>,
    next_meta: usize,
    playlists: Vec<PlaylistHashRow>,
    /// deleted playlists, sent with the playlists.
    deleted: Vec<DeletedPlaylistRow>,
    playlists_sent: bool,
    flags: Flags,
}

impl Plan {
    fn with(
        songs: Vec<Outgoing>,
        metas: Vec<Outgoing>,
        playlists: Vec<PlaylistHashRow>,
        deleted: Vec<DeletedPlaylistRow>,
        flags: Flags,
    ) -> Self {
        Self {
            songs,
            next: 0,
            metas,
            next_meta: 0,
            playlists,
            deleted,
            playlists_sent: false,
            flags,
        }
    }

    /// resolves a want against the local library; what is gone meanwhile is
    /// skipped.
    pub fn new(db: &Db, want: &Want) -> Result<Self, WireStatus> {
        let rows = |hashes: &[u128]| {
            let hashes: Vec<String> = hashes.iter().map(|&h| hex(h)).collect();
            db.get_track_sync(&hashes).map_err(internal)
        };
        let songs = songs_of(rows(&want.songs)?);
        let metas = songs_of(rows(&want.metas)?);
        // a wanted name is a playlist here or a deletion here.
        let playlists = db
            .get_playlist_hashes(Some(&want.playlists))
            .map_err(internal)?;
        let deleted = db
            .get_deleted_playlists(Some(&want.playlists))
            .map_err(internal)?;
        Ok(Self::with(songs, metas, playlists, deleted, Flags::ALL))
    }

    /// a selection to send one way (like an export).
    pub fn selection(db: &Db, what: &Selection, flags: Flags) -> Result<Self, PlanError> {
        let db_err = |_| PlanError::Db;
        let (songs, playlists, deleted) = match what {
            // the whole library carries its deletions too.
            Selection::Library => (
                db.get_track_hashes(None).map_err(db_err)?,
                db.get_playlist_hashes(None).map_err(db_err)?,
                db.get_deleted_playlists(None).map_err(db_err)?,
            ),
            Selection::Playlists(names) => {
                let playlists = db.get_playlist_hashes(Some(names)).map_err(db_err)?;
                if playlists.len() != names.len() {
                    return Err(PlanError::NotFound);
                }
                let mut seen = HashSet::new();
                let hashes: Vec<String> = playlists
                    .iter()
                    .flat_map(|p| &p.hashes)
                    .filter(|h| seen.insert(h.as_str()))
                    .cloned()
                    .collect();
                (
                    db.get_track_hashes_by_hash(&hashes).map_err(db_err)?,
                    playlists,
                    Vec::new(),
                )
            }
            Selection::Tracks(ids) => {
                let tracks = db.get_track_hashes(Some(ids)).map_err(db_err)?;
                if tracks.len() != ids.len() {
                    return Err(PlanError::NotFound);
                }
                (tracks, Vec::new(), Vec::new())
            }
        };
        let hashes: Vec<String> = songs.iter().map(|(_, h, _)| h.clone()).collect();
        let songs = songs_of(db.get_track_sync(&hashes).map_err(db_err)?);
        // without audio only the metadata goes: a new song without its file
        // is skipped by the receiver, a known one gets the newer metadata.
        let (songs, metas) = match flags.audio {
            true => (songs, Vec::new()),
            false => (Vec::new(), songs),
        };
        Ok(Self::with(songs, metas, playlists, deleted, flags))
    }

    /// the next data portion; `None` — everything is sent. Reads files, so it
    /// is called without the core lock. An unreadable song file is skipped.
    pub fn next_portion(&mut self) -> Option<Result<(Vec<u8>, PortionStats), WireStatus>> {
        // the songs with their files, then the ones without.
        for with_files in [true, false] {
            let songs = self.cut(with_files);
            if songs.is_empty() {
                continue;
            }
            let stats = PortionStats {
                songs: songs.len() as u32,
                playlists: 0,
            };
            return Some(
                codec::encode(VERSION, &songs, &[])
                    .map(|p| (p, stats))
                    .map_err(internal),
            );
        }
        if self.playlists_sent || (self.playlists.is_empty() && self.deleted.is_empty()) {
            return None;
        }
        self.playlists_sent = true;
        let playlists: Vec<Playlist> = self
            .playlists
            .iter()
            .map(|p| {
                let (cover_name, cover) = match self.flags.covers {
                    true => read_cover(p.cover_art.as_deref()),
                    false => (String::new(), Vec::new()),
                };
                Playlist {
                    cover_name,
                    cover,
                    ..playlist_of(p)
                }
            })
            .collect();
        let deleted: Vec<DeletedPlaylist> = self.deleted.iter().cloned().map(deleted_from_db).collect();
        let stats = PortionStats {
            songs: 0,
            playlists: (playlists.len() + deleted.len()) as u32,
        };
        Some(
            codec::encode_with(VERSION, &[], &playlists, &deleted)
                .map(|p| (p, stats))
                .map_err(internal),
        )
    }
}

impl Plan {
    /// the next songs of `songs` (`with_files`) or of `metas`, until their
    /// files and covers reach [`PORTION_BYTES`]; empty — that list is sent.
    /// An unreadable song file skips the song.
    fn cut(&mut self, with_files: bool) -> Vec<Song> {
        let (list, next) = match with_files {
            true => (&self.songs, &mut self.next),
            false => (&self.metas, &mut self.next_meta),
        };
        let mut songs = Vec::new();
        let mut size = 0;
        while *next < list.len() && (songs.is_empty() || size < PORTION_BYTES) {
            let out = &list[*next];
            *next += 1;
            // a plan without audio has no songs with files at all.
            let (name, data) = match with_files {
                true => {
                    let Ok(data) = std::fs::read(&out.path) else {
                        continue;
                    };
                    let name = match self.flags.names {
                        true => out
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        false => String::new(),
                    };
                    (name, data)
                }
                false => Default::default(),
            };
            let (cover_name, cover) = match self.flags.covers {
                true => read_song_cover(out.cover.as_deref(), &out.path),
                false => Default::default(),
            };
            size += (data.len() + cover.len()) as u64;
            songs.push(Song {
                name,
                data,
                cover_name,
                cover,
                ..out.song.clone()
            });
        }
        songs
    }
}

/// db track rows -> songs to send; a malformed hash is skipped.
fn songs_of(rows: Vec<TrackSyncRow>) -> Vec<Outgoing> {
    rows.into_iter()
        .filter_map(|row| {
            let song = Song {
                hash: parse_hash(&row.hash)?,
                ..Song::default()
            };
            Some(Outgoing {
                song: with_info(song, &row),
                path: row.path,
                cover: row.cover_art,
            })
        })
        .collect()
}
