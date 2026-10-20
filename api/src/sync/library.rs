//! what the sync protocol carries, built from / compared with the db. Every
//! payload is a codec packet:
//! - index: every playlist (name, dates, song hashes) and the songs that are
//!   in some playlist and exist locally: hashes and change dates — no files,
//!   no metadata;
//! - want: song hashes and playlist names (empty track lists). A song with
//!   (empty) metadata is wanted for its metadata only, without one — whole;
//! - data: a portion of songs with names, files, dates and metadata; then a
//!   portion of songs with dates and metadata only; the last portion — the
//!   playlists with covers. A one-way send ([`Selection`]) chooses what and
//!   with which [`Flags`], like an export (without audio — the metadata of
//!   the songs only).
//!
//! only songs that are in some playlist are synced. A playlist is wanted if
//! it is missing locally or the remote one is newer (`updated_at`, an unset
//! date counts as 0) than the local one — or than its local deletion. A
//! remote deletion is wanted if the local playlist changed at or before it, an
//! older local deletion or no local playlist at all (it travels on). A song is
//! wanted if a wanted remote playlist or a local playlist refers to it, the
//! remote side has it and the local side doesn't. A song both sides have is
//! wanted whole if the remote file changed later (`updated_at_file`, an
//! unset date counts as 0) — it is written over the local file — or for its
//! metadata only if the remote db record changed later (`updated_at`).

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use codec::{DeletedPlaylist, Playlist, Song, SongMeta, Version};
use storage::{
    Db,
    db::{DeletedPlaylistRow, PlaylistHashRow, TrackSyncRow},
};

use super::frame::WireStatus;
use crate::cdc::{deleted_from_db, hex, parse_hash, read_cover, with_info};

/// the codec version of every sync payload.
const VERSION: Version = Version::V0_12;
/// a data portion is cut once its files reach this size (one song is never
/// split, so a single larger song makes a larger portion).
pub const PORTION_BYTES: u64 = 16 << 20;

fn internal<E>(_: E) -> WireStatus {
    WireStatus::Internal
}

fn invalid<E>(_: E) -> WireStatus {
    WireStatus::InvalidPacket
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
    let playlists = playlists
        .into_iter()
        .map(|p| Playlist {
            tracks: p.hashes.iter().filter_map(|h| parse_hash(h)).collect(),
            name: p.name,
            created_at: p.created_at,
            updated_at: p.updated_at,
            ..Playlist::default()
        })
        .collect::<Vec<_>>();
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

    let wanted: Vec<&Playlist> = remote
        .playlists
        .iter()
        .filter(|p| !p.name.is_empty())
        .filter(|p| match local_dates.get(p.name.as_str()) {
            None => true,
            Some(&local) => p.updated_at.unwrap_or(0) > local,
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
    /// playlist covers.
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

/// songs and playlists to send, cut into data portions as they are sent.
pub struct Plan {
    /// (local path, the song without its file: hash, dates, metadata) —
    /// songs to send with their files, in order.
    songs: Vec<(PathBuf, Song)>,
    next: usize,
    /// songs to send without files (metadata only), in one portion.
    metas: Vec<Song>,
    metas_sent: bool,
    playlists: Vec<PlaylistHashRow>,
    /// deleted playlists, sent with the playlists.
    deleted: Vec<DeletedPlaylistRow>,
    playlists_sent: bool,
    flags: Flags,
}

impl Plan {
    fn with(
        songs: Vec<(PathBuf, Song)>,
        metas: Vec<Song>,
        playlists: Vec<PlaylistHashRow>,
        deleted: Vec<DeletedPlaylistRow>,
        flags: Flags,
    ) -> Self {
        Self {
            songs,
            next: 0,
            metas,
            metas_sent: false,
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
        let metas = songs_of(rows(&want.metas)?).into_iter().map(|(_, s)| s).collect();
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
            false => (Vec::new(), songs.into_iter().map(|(_, s)| s).collect()),
        };
        Ok(Self::with(songs, metas, playlists, deleted, flags))
    }

    /// the next data portion; `None` — everything is sent. Reads files, so it
    /// is called without the core lock. An unreadable song file is skipped.
    pub fn next_portion(&mut self) -> Option<Result<(Vec<u8>, PortionStats), WireStatus>> {
        if self.next < self.songs.len() {
            let mut songs = Vec::new();
            let mut size = 0;
            while self.next < self.songs.len() && (songs.is_empty() || size < PORTION_BYTES) {
                let (path, song) = &self.songs[self.next];
                self.next += 1;
                // a plan without audio has no songs with files at all.
                let Ok(data) = std::fs::read(path) else {
                    continue;
                };
                let name = match self.flags.names {
                    true => path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    false => String::new(),
                };
                size += data.len() as u64;
                songs.push(Song {
                    name,
                    data,
                    ..song.clone()
                });
            }
            if !songs.is_empty() {
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
        }
        if !self.metas_sent && !self.metas.is_empty() {
            self.metas_sent = true;
            let stats = PortionStats {
                songs: self.metas.len() as u32,
                playlists: 0,
            };
            return Some(
                codec::encode(VERSION, &self.metas, &[])
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
                    name: p.name.clone(),
                    tracks: p.hashes.iter().filter_map(|h| parse_hash(h)).collect(),
                    created_at: p.created_at,
                    updated_at: p.updated_at,
                    cover_name,
                    cover,
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

/// db track rows -> (path, song without its file); a malformed hash is
/// skipped.
fn songs_of(rows: Vec<TrackSyncRow>) -> Vec<(PathBuf, Song)> {
    rows.into_iter()
        .filter_map(|row| {
            let song = Song {
                hash: parse_hash(&row.hash)?,
                ..Song::default()
            };
            Some((row.path.clone(), with_info(song, &row)))
        })
        .collect()
}
