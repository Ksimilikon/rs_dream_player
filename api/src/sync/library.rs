//! what the sync protocol carries, built from / compared with the db. Every
//! payload is a codec packet:
//! - index: every playlist (name, dates, song hashes) and the hashes of the
//!   songs that are in some playlist and exist locally — no files;
//! - want: song hashes and playlist names (empty track lists);
//! - data: a portion of songs with names and files; the last portion — the
//!   playlists with covers. A one-way send ([`Selection`]) chooses what and
//!   with which [`Flags`], like an export.
//!
//! only songs that are in some playlist are synced. A playlist is wanted if
//! it is missing locally or the remote one is newer (`updated_at`, an unset
//! date counts as 0). A song is wanted if a wanted remote playlist or a local
//! playlist refers to it, the remote side has it and the local side doesn't.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use codec::{Playlist, Song, Version};
use storage::{Db, db::PlaylistHashRow};

use super::frame::WireStatus;
use crate::cdc::{hex, parse_hash, read_cover};

/// the codec version of every sync payload.
const VERSION: Version = Version::V0_1;
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
        .get_track_hashes_by_hash(&refs)
        .map_err(internal)?
        .iter()
        .filter_map(|(_, hash, _)| parse_hash(hash))
        .map(|hash| Song {
            hash,
            ..Song::default()
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
    codec::encode(VERSION, &songs, &playlists).map_err(internal)
}

/// what the local side lacks compared to a remote index.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Want {
    pub songs: Vec<u128>,
    pub playlists: Vec<String>,
}

impl Want {
    pub fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.playlists.is_empty()
    }

    pub fn encode(&self) -> Result<Vec<u8>, WireStatus> {
        let songs: Vec<Song> = self
            .songs
            .iter()
            .map(|&hash| Song {
                hash,
                ..Song::default()
            })
            .collect();
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
        Ok(Self {
            songs: packet.songs.iter().map(|s| s.hash).collect(),
            playlists: packet.playlists.into_iter().map(|p| p.name).collect(),
        })
    }
}

/// compares a remote index with the local library.
pub fn want(db: &Db, remote_index: &[u8]) -> Result<Want, WireStatus> {
    let remote = codec::decode(remote_index).map_err(invalid)?;
    let local_playlists = db.get_playlist_hashes(None).map_err(internal)?;
    let local_songs: HashSet<u128> = db
        .get_track_hashes(None)
        .map_err(internal)?
        .iter()
        .filter_map(|(_, hash, _)| parse_hash(hash))
        .collect();
    let local_dates: HashMap<&str, u64> = local_playlists
        .iter()
        .map(|p| (p.name.as_str(), p.updated_at.unwrap_or(0)))
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

    let remote_songs: HashSet<u128> = remote.songs.iter().map(|s| s.hash).collect();
    let local_refs = local_playlists
        .iter()
        .flat_map(|p| &p.hashes)
        .filter_map(|h| parse_hash(h));
    let mut seen = HashSet::new();
    let songs = wanted
        .iter()
        .flat_map(|p| p.tracks.iter().copied())
        .chain(local_refs)
        .filter(|h| remote_songs.contains(h) && !local_songs.contains(h) && seen.insert(*h))
        .collect();

    Ok(Want {
        songs,
        playlists: wanted.into_iter().map(|p| p.name.clone()).collect(),
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
    /// (hash, local path) — songs not sent yet, in order.
    songs: Vec<(u128, PathBuf)>,
    next: usize,
    playlists: Vec<PlaylistHashRow>,
    playlists_sent: bool,
    flags: Flags,
}

impl Plan {
    fn with(songs: Vec<(u128, PathBuf)>, playlists: Vec<PlaylistHashRow>, flags: Flags) -> Self {
        Self {
            songs,
            next: 0,
            playlists,
            playlists_sent: false,
            flags,
        }
    }

    /// resolves a want against the local library; what is gone meanwhile is
    /// skipped.
    pub fn new(db: &Db, want: &Want) -> Result<Self, WireStatus> {
        let hashes: Vec<String> = want.songs.iter().map(|&h| hex(h)).collect();
        let songs = songs_of(&db.get_track_hashes_by_hash(&hashes).map_err(internal)?);
        let playlists = db
            .get_playlist_hashes(Some(&want.playlists))
            .map_err(internal)?;
        Ok(Self::with(songs, playlists, Flags::ALL))
    }

    /// a selection to send one way (like an export).
    pub fn selection(db: &Db, what: &Selection, flags: Flags) -> Result<Self, PlanError> {
        let db_err = |_| PlanError::Db;
        let (songs, playlists) = match what {
            Selection::Library => (
                db.get_track_hashes(None).map_err(db_err)?,
                db.get_playlist_hashes(None).map_err(db_err)?,
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
                )
            }
            Selection::Tracks(ids) => {
                let tracks = db.get_track_hashes(Some(ids)).map_err(db_err)?;
                if tracks.len() != ids.len() {
                    return Err(PlanError::NotFound);
                }
                (tracks, Vec::new())
            }
        };
        // a song without its file is skipped by the receiver anyway.
        let songs = match flags.audio {
            true => songs_of(&songs),
            false => Vec::new(),
        };
        Ok(Self::with(songs, playlists, flags))
    }

    /// the next data portion; `None` — everything is sent. Reads files, so it
    /// is called without the core lock. An unreadable song file is skipped.
    pub fn next_portion(&mut self) -> Option<Result<(Vec<u8>, PortionStats), WireStatus>> {
        if self.next < self.songs.len() {
            let mut songs = Vec::new();
            let mut size = 0;
            while self.next < self.songs.len() && (songs.is_empty() || size < PORTION_BYTES) {
                let (hash, path) = &self.songs[self.next];
                self.next += 1;
                // a plan without audio has no songs at all.
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
                    hash: *hash,
                    name,
                    data,
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
        if self.playlists_sent || self.playlists.is_empty() {
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
        let stats = PortionStats {
            songs: 0,
            playlists: playlists.len() as u32,
        };
        Some(
            codec::encode(VERSION, &[], &playlists)
                .map(|p| (p, stats))
                .map_err(internal),
        )
    }
}

/// db track rows -> (hash, path); a malformed hash is skipped.
fn songs_of(rows: &[storage::db::TrackHashRow]) -> Vec<(u128, PathBuf)> {
    rows.iter()
        .filter_map(|(_, hash, path)| Some((parse_hash(hash)?, path.clone())))
        .collect()
}
