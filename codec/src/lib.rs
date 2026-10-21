//! this crate makes the binary packet used to move songs and playlists
//! between databases. Public surface: [`encode`], [`decode`], [`Version`],
//! [`SectionTag`], [`CodecError`] and the plain data types [`Song`] /
//! [`SongMeta`] / [`Playlist`] / [`Packet`].
//!
//! # packet structure
//!
//! all integers are little-endian.
//!
//! 1) start word: 4 bytes, [`START_WORD`] (`b"DPLR"`)
//! 2) version: `u32`, simple digit (see [`Version`])
//! 3) count sections: `u8`
//! 4) sections, each: tag `u8` ([`SectionTag`]) + length `u64` + payload.
//!    Every section appears at most once; unknown tags are skipped by length.
//!
//! ## songs section ([`SectionTag::Songs`])
//!
//! everything about the music; the other sections refer to songs by hash.
//!
//! 1. count rows: `u32`, number of songs `N`
//! 2. hashes: `N` × `u128`, fixed size
//! 3. file names: `N` × (`u16` length + UTF-8 payload), can be 0 length
//! 4. music files: `N` × (`u32` length + raw payload), can be 0 length
//!
//! v0.12+ only:
//!
//! 5. info: `N` × (flags `u8` + `updated_at` `u64` + `updated_at_file`
//!    `u64`), fixed size. Flag bit 0: `updated_at` (the db record changed) is
//!    set, bit 1: `updated_at_file` (the file itself changed) is set, bit 2:
//!    the song has metadata in 6, bit 3: the song has a cover in 7 / 8. An
//!    unset date is written as 0 and must be ignored. Other bits are reserved
//!    (written as 0, ignored on read).
//! 6. metadata: for every song with flag bit 2, in song order: title
//!    (`u16` length + UTF-8), album (same, 0 length = none), artists (`u16`
//!    count + that many names), genres (same), color (name, 0 length =
//!    none), user label (same), volume (`f32`).
//! 7. cover file names: for every song with flag bit 3, in song order
//!    (`u16` length + UTF-8 payload), can be 0 length
//! 8. covers: for every song with flag bit 3, in song order (`u32` length +
//!    raw image). Only a cover that is not embedded into the music file
//!    travels here.
//!
//! song `i` is `(hashes[i], names[i], files[i], info[i], ...)`.
//!
//! ## playlists section ([`SectionTag::Playlists`])
//!
//! playlists only: songs are listed by hash, their files and data are in the
//! songs section.
//!
//! 1. count rows: `u32`, number of playlists `M`
//! 2. names: `M` × (`u16` length + UTF-8 payload), can be 0 length
//! 3. dates: `M` × (flags `u8` + `created_at` `u64` + `updated_at` `u64`),
//!    fixed size. Flag bit 0: `created_at` is set, bit 1: `updated_at` is
//!    set, bit 2 (v0.12+): the playlist has a cover in 5 / 6; an unset date
//!    is written as 0 and must be ignored. Other bits are reserved (written as
//!    0, ignored on read).
//! 4. track lists: `M` × (`u32` count `K` + `K` × `u128` song hash), in
//!    playlist order. Hashes may point to songs that are not in the packet.
//!
//! v0.12+ only:
//!
//! 5. cover file names: for every playlist with flag bit 2, in playlist
//!    order (`u16` length + UTF-8 payload), can be 0 length
//! 6. covers: for every playlist with flag bit 2, in playlist order (`u32`
//!    length + raw image)
//!
//! ## deleted playlists section ([`SectionTag::Deleted`])
//!
//! v0.11+, written after the playlists and only if some playlist is deleted.
//! A deleted playlist is soft-deleted: only its name (the identifier) and
//! when it was deleted travel, so the receiver deletes its copy too unless
//! that copy was changed later.
//!
//! 1. count rows: `u32`, number of deleted playlists `D`
//! 2. names: `D` × (`u16` length + UTF-8 payload)
//! 3. deletion times: `D` × `u64` (unix seconds)
//!
//! ## covers section ([`SectionTag::Covers`])
//!
//! v0.1 / v0.11 only (v0.12+ keeps the covers in the songs and playlists
//! sections): written last and only if at least one playlist has a cover.
//!
//! 1. count rows: `u32`, must equal `M`
//! 2. file names: `M` × (`u16` length + UTF-8 payload), can be 0 length
//! 3. covers: `M` × (`u32` length + raw image), 0 length = no cover
//!
//! a missing section decodes as empty.
//!
//! # code style
//!
//! - comments and docs are in English and start with a lowercase letter,
//!   except type / function names in backticks.
//! - the packet structure is described once, here; code refers to it by
//!   number (`// 4) sections`, `// songs 2. hashes`).
//! - no `unsafe`, no `unwrap` / `expect` / panics on input data: every
//!   failure is a [`CodecError`] variant.
//! - errors are fieldless `#[repr(u32)]` enums so they cross FFI as a plain
//!   number; no payloads, no `Box<dyn Error>`.
//! - wire enums (`Version`, `SectionTag`) are `#[repr]` with explicit
//!   discriminants; never renumber, only append.
//! - all sizes read from input are checked against the remaining bytes before
//!   allocating, so a corrupted header can't request a huge buffer.
//! - integers are written / read only through `to_le_bytes` /
//!   `from_le_bytes` (in `wire`); never native endianness.
//! - no external dependencies.

mod error;
mod section;
mod version;
mod wire;

pub use error::CodecError;
pub use section::SectionTag;
pub use version::Version;

use wire::{Reader, put_blob, put_count, put_hash, put_name, put_names};

/// magic bytes at the start of every packet.
pub const START_WORD: [u8; 4] = *b"DPLR";

/// one song inside a packet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Song {
    /// song hash (128-bit, same value as the hex hash in the database).
    pub hash: u128,
    /// file name, max `u16::MAX` bytes of UTF-8, can be empty.
    pub name: String,
    /// raw music file, max `u32::MAX` bytes, can be empty.
    pub data: Vec<u8>,
    /// last change of the db record (unix seconds), v0.12+.
    pub updated_at: Option<u64>,
    /// last change of the file itself (unix seconds), v0.12+.
    pub updated_at_file: Option<u64>,
    /// metadata from the db, v0.12+; `None` — not carried.
    pub meta: Option<SongMeta>,
    /// cover file name (format hint for the receiver), v0.12+; travels only
    /// together with `cover`.
    pub cover_name: String,
    /// raw cover image that is not embedded into the music file, v0.12+,
    /// max `u32::MAX` bytes; empty = no cover.
    pub cover: Vec<u8>,
}

/// song metadata as the db keeps it (v0.12+). Names are max `u16::MAX`
/// bytes of UTF-8, lists max `u16::MAX` names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SongMeta {
    pub title: String,
    /// empty = no album.
    pub album: String,
    pub artists: Vec<String>,
    pub genres: Vec<String>,
    /// color label, empty = none.
    pub color: String,
    /// text label, empty = none.
    pub label: String,
    /// saved per-track volume.
    pub volume: f32,
}

/// one playlist inside a packet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playlist {
    /// playlist name, max `u16::MAX` bytes of UTF-8, can be empty.
    pub name: String,
    /// song hashes in playlist order.
    pub tracks: Vec<u128>,
    /// creation time (unix seconds).
    pub created_at: Option<u64>,
    /// last update time (unix seconds).
    pub updated_at: Option<u64>,
    /// cover file name (format hint for the receiver), can be empty.
    pub cover_name: String,
    /// raw cover image, max `u32::MAX` bytes; empty = no cover.
    pub cover: Vec<u8>,
}

/// a deleted playlist inside a packet (v0.11+).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeletedPlaylist {
    /// playlist name, max `u16::MAX` bytes of UTF-8.
    pub name: String,
    /// deletion time (unix seconds).
    pub deleted_at: u64,
}

/// decoded packet.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub version: Version,
    pub songs: Vec<Song>,
    pub playlists: Vec<Playlist>,
    /// deleted playlists; always empty in v0.1.
    pub deleted: Vec<DeletedPlaylist>,
}

/// encodes `songs` and `playlists` into a packet of the given `version`
/// (without deleted playlists).
pub fn encode(
    version: Version,
    songs: &[Song],
    playlists: &[Playlist],
) -> Result<Vec<u8>, CodecError> {
    encode_with(version, songs, playlists, &[])
}

/// encodes `songs`, `playlists` and the `deleted` playlists into a packet of
/// the given `version`. v0.1 has no deleted playlists section: `deleted` is
/// left out there; before v0.12 the song dates, metadata and covers are left
/// out.
pub fn encode_with(
    version: Version,
    songs: &[Song],
    playlists: &[Playlist],
    deleted: &[DeletedPlaylist],
) -> Result<Vec<u8>, CodecError> {
    let full = version.full_sections();
    let with_covers = !full && playlists.iter().any(|p| !p.cover.is_empty());
    let with_deleted = version.has_deleted() && !deleted.is_empty();

    // reserve close to the final size: audio dominates, and growing a
    // multi-gigabyte `Vec` by doubling would briefly need twice the memory.
    let size = 64
        + songs
            .iter()
            .map(|s| 45 + s.name.len() + s.data.len() + s.cover_name.len() + s.cover.len())
            .sum::<usize>()
        + playlists
            .iter()
            .map(|p| {
                33 + p.name.len() + 16 * p.tracks.len() + p.cover_name.len() + p.cover.len()
            })
            .sum::<usize>()
        + deleted.iter().map(|d| 10 + d.name.len()).sum::<usize>();
    let mut out = Vec::with_capacity(size);

    // 1) start word
    out.extend_from_slice(&START_WORD);
    // 2) version
    out.extend_from_slice(&u32::from(version).to_le_bytes());
    // 3) count sections
    out.push(2 + u8::from(with_deleted) + u8::from(with_covers));
    // 4) sections
    section(&mut out, SectionTag::Songs, |out| write_songs(out, songs, full))?;
    section(&mut out, SectionTag::Playlists, |out| {
        write_playlists(out, playlists, full)
    })?;
    if with_deleted {
        section(&mut out, SectionTag::Deleted, |out| write_deleted(out, deleted))?;
    }
    if with_covers {
        section(&mut out, SectionTag::Covers, |out| write_covers(out, playlists))?;
    }

    Ok(out)
}

/// decodes a whole packet from `bytes`.
pub fn decode(bytes: &[u8]) -> Result<Packet, CodecError> {
    let mut r = Reader::new(bytes);

    // 1) start word
    if r.take(START_WORD.len())? != START_WORD {
        return Err(CodecError::BadStartWord);
    }
    // 2) version
    let version = Version::try_from(r.u32()?)?;
    let full = version.full_sections();
    // 3) count sections
    let count = r.u8()?;

    // 4) sections
    let mut songs = None;
    let mut playlists = None;
    let mut covers = None;
    let mut deleted = None;
    for _ in 0..count {
        let tag = r.u8()?;
        let len = usize::try_from(r.u64()?).map_err(|_| CodecError::UnexpectedEof)?;
        let mut s = Reader::new(r.take(len)?);
        match SectionTag::try_from(tag) {
            Ok(SectionTag::Songs) => set_once(&mut songs, read_songs(&mut s, full)?)?,
            Ok(SectionTag::Playlists) => {
                set_once(&mut playlists, read_playlists(&mut s, full)?)?
            }
            Ok(SectionTag::Covers) => set_once(&mut covers, read_covers(&mut s)?)?,
            Ok(SectionTag::Deleted) => set_once(&mut deleted, read_deleted(&mut s)?)?,
            // unknown section: already skipped by `take(len)`.
            Err(_) => continue,
        }
        s.finish()?;
    }
    r.finish()?;

    let mut playlists = playlists.unwrap_or_default();
    if let Some(covers) = covers {
        if covers.len() != playlists.len() {
            return Err(CodecError::CountMismatch);
        }
        for (p, (name, cover)) in playlists.iter_mut().zip(covers) {
            p.cover_name = name;
            p.cover = cover;
        }
    }

    Ok(Packet {
        version,
        songs: songs.unwrap_or_default(),
        playlists,
        deleted: deleted.unwrap_or_default(),
    })
}

// ──────────────────────────── sections ─────────────────────────────

/// writes one section: tag + `u64` length (patched after `body`) + payload.
fn section(
    out: &mut Vec<u8>,
    tag: SectionTag,
    body: impl FnOnce(&mut Vec<u8>) -> Result<(), CodecError>,
) -> Result<(), CodecError> {
    out.push(tag.into());
    let len_at = out.len();
    out.extend_from_slice(&0u64.to_le_bytes());
    body(out)?;
    let len = (out.len() - len_at - 8) as u64;
    out[len_at..len_at + 8].copy_from_slice(&len.to_le_bytes());
    Ok(())
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), CodecError> {
    match slot {
        Some(_) => Err(CodecError::DuplicateSection),
        None => {
            *slot = Some(value);
            Ok(())
        }
    }
}

/// the items whose flag is set.
fn flagged<'a, T>(items: &'a mut [T], flags: &'a [bool]) -> impl Iterator<Item = &'a mut T> {
    items.iter_mut().zip(flags).filter(|(_, f)| **f).map(|(t, _)| t)
}

/// `full` — v0.12+: with the song dates, metadata and covers.
fn write_songs(out: &mut Vec<u8>, songs: &[Song], full: bool) -> Result<(), CodecError> {
    // songs 1. count rows
    put_count(out, songs.len())?;
    // songs 2. hashes
    for song in songs {
        put_hash(out, song.hash);
    }
    // songs 3. file names
    for song in songs {
        put_name(out, &song.name)?;
    }
    // songs 4. music files
    for song in songs {
        put_blob(out, &song.data)?;
    }
    if !full {
        return Ok(());
    }
    // songs 5. info
    for s in songs {
        let flags = u8::from(s.updated_at.is_some())
            | u8::from(s.updated_at_file.is_some()) << 1
            | u8::from(s.meta.is_some()) << 2
            | u8::from(!s.cover.is_empty()) << 3;
        out.push(flags);
        out.extend_from_slice(&s.updated_at.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&s.updated_at_file.unwrap_or(0).to_le_bytes());
    }
    // songs 6. metadata
    for m in songs.iter().filter_map(|s| s.meta.as_ref()) {
        put_name(out, &m.title)?;
        put_name(out, &m.album)?;
        put_names(out, &m.artists)?;
        put_names(out, &m.genres)?;
        put_name(out, &m.color)?;
        put_name(out, &m.label)?;
        out.extend_from_slice(&m.volume.to_le_bytes());
    }
    let covered = || songs.iter().filter(|s| !s.cover.is_empty());
    // songs 7. cover file names
    for s in covered() {
        put_name(out, &s.cover_name)?;
    }
    // songs 8. covers
    for s in covered() {
        put_blob(out, &s.cover)?;
    }
    Ok(())
}

/// `full` — v0.12+: with the song dates, metadata and covers.
fn read_songs(r: &mut Reader<'_>, full: bool) -> Result<Vec<Song>, CodecError> {
    // songs 1. count rows: every song takes at least 16 + 2 + 4 bytes
    // (+ 17 of info in v0.12+).
    let (count, cap) = r.count(if full { 39 } else { 22 })?;
    // songs 2. hashes
    let mut songs = Vec::with_capacity(cap);
    for _ in 0..count {
        songs.push(Song {
            hash: r.hash()?,
            ..Song::default()
        });
    }
    // songs 3. file names
    for song in &mut songs {
        song.name = r.name()?;
    }
    // songs 4. music files
    for song in &mut songs {
        song.data = r.blob()?;
    }
    if !full {
        return Ok(songs);
    }
    // songs 5. info
    let mut covered = Vec::with_capacity(songs.len());
    for s in &mut songs {
        let flags = r.u8()?;
        let (updated, updated_file) = (r.u64()?, r.u64()?);
        s.updated_at = (flags & 1 != 0).then_some(updated);
        s.updated_at_file = (flags & 2 != 0).then_some(updated_file);
        s.meta = (flags & 4 != 0).then(SongMeta::default);
        covered.push(flags & 8 != 0);
    }
    // songs 6. metadata
    for m in songs.iter_mut().filter_map(|s| s.meta.as_mut()) {
        m.title = r.name()?;
        m.album = r.name()?;
        m.artists = r.names()?;
        m.genres = r.names()?;
        m.color = r.name()?;
        m.label = r.name()?;
        m.volume = r.f32()?;
    }
    // songs 7. cover file names
    for s in flagged(&mut songs, &covered) {
        s.cover_name = r.name()?;
    }
    // songs 8. covers
    for s in flagged(&mut songs, &covered) {
        s.cover = r.blob()?;
    }
    Ok(songs)
}

/// `full` — v0.12+: with the playlist covers.
fn write_playlists(
    out: &mut Vec<u8>,
    playlists: &[Playlist],
    full: bool,
) -> Result<(), CodecError> {
    // playlists 1. count rows
    put_count(out, playlists.len())?;
    // playlists 2. names
    for p in playlists {
        put_name(out, &p.name)?;
    }
    // playlists 3. dates
    for p in playlists {
        let flags = u8::from(p.created_at.is_some())
            | u8::from(p.updated_at.is_some()) << 1
            | u8::from(full && !p.cover.is_empty()) << 2;
        out.push(flags);
        out.extend_from_slice(&p.created_at.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&p.updated_at.unwrap_or(0).to_le_bytes());
    }
    // playlists 4. track lists
    for p in playlists {
        put_count(out, p.tracks.len())?;
        for &hash in &p.tracks {
            put_hash(out, hash);
        }
    }
    if !full {
        return Ok(());
    }
    let covered = || playlists.iter().filter(|p| !p.cover.is_empty());
    // playlists 5. cover file names
    for p in covered() {
        put_name(out, &p.cover_name)?;
    }
    // playlists 6. covers
    for p in covered() {
        put_blob(out, &p.cover)?;
    }
    Ok(())
}

/// `full` — v0.12+: with the playlist covers.
fn read_playlists(r: &mut Reader<'_>, full: bool) -> Result<Vec<Playlist>, CodecError> {
    // playlists 1. count rows: every playlist takes at least 2 + 17 + 4 bytes.
    let (count, cap) = r.count(23)?;
    // playlists 2. names
    let mut playlists = Vec::with_capacity(cap);
    for _ in 0..count {
        playlists.push(Playlist {
            name: r.name()?,
            ..Playlist::default()
        });
    }
    // playlists 3. dates
    let mut covered = Vec::with_capacity(playlists.len());
    for p in &mut playlists {
        let flags = r.u8()?;
        let (created, updated) = (r.u64()?, r.u64()?);
        p.created_at = (flags & 1 != 0).then_some(created);
        p.updated_at = (flags & 2 != 0).then_some(updated);
        // bit 2 is reserved before v0.12.
        covered.push(full && flags & 4 != 0);
    }
    // playlists 4. track lists
    for p in &mut playlists {
        let (len, cap) = r.count(16)?;
        p.tracks.reserve_exact(cap);
        for _ in 0..len {
            p.tracks.push(r.hash()?);
        }
    }
    // playlists 5. cover file names
    for p in flagged(&mut playlists, &covered) {
        p.cover_name = r.name()?;
    }
    // playlists 6. covers
    for p in flagged(&mut playlists, &covered) {
        p.cover = r.blob()?;
    }
    Ok(playlists)
}

fn write_covers(out: &mut Vec<u8>, playlists: &[Playlist]) -> Result<(), CodecError> {
    // covers 1. count rows
    put_count(out, playlists.len())?;
    // covers 2. file names
    for p in playlists {
        put_name(out, &p.cover_name)?;
    }
    // covers 3. covers
    for p in playlists {
        put_blob(out, &p.cover)?;
    }
    Ok(())
}

fn write_deleted(out: &mut Vec<u8>, deleted: &[DeletedPlaylist]) -> Result<(), CodecError> {
    // deleted 1. count rows
    put_count(out, deleted.len())?;
    // deleted 2. names
    for d in deleted {
        put_name(out, &d.name)?;
    }
    // deleted 3. deletion times
    for d in deleted {
        out.extend_from_slice(&d.deleted_at.to_le_bytes());
    }
    Ok(())
}

fn read_deleted(r: &mut Reader<'_>) -> Result<Vec<DeletedPlaylist>, CodecError> {
    // deleted 1. count rows: every entry takes at least 2 + 8 bytes.
    let (count, cap) = r.count(10)?;
    // deleted 2. names
    let mut deleted = Vec::with_capacity(cap);
    for _ in 0..count {
        deleted.push(DeletedPlaylist {
            name: r.name()?,
            deleted_at: 0,
        });
    }
    // deleted 3. deletion times
    for d in &mut deleted {
        d.deleted_at = r.u64()?;
    }
    Ok(deleted)
}

/// `(file name, image)` per playlist.
fn read_covers(r: &mut Reader<'_>) -> Result<Vec<(String, Vec<u8>)>, CodecError> {
    // covers 1. count rows: every cover takes at least 2 + 4 bytes.
    let (count, cap) = r.count(6)?;
    // covers 2. file names
    let mut covers = Vec::with_capacity(cap);
    for _ in 0..count {
        covers.push((r.name()?, Vec::new()));
    }
    // covers 3. covers
    for (_, cover) in &mut covers {
        *cover = r.blob()?;
    }
    Ok(covers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn songs() -> Vec<Song> {
        vec![
            Song {
                hash: 0x0123_4567_89ab_cdef_0011_2233_4455_6677,
                name: "track.flac".into(),
                data: vec![1, 2, 3, 4, 5],
                ..Song::default()
            },
            Song {
                hash: u128::MAX,
                name: "песня.mp3".into(),
                data: vec![],
                ..Song::default()
            },
            Song {
                hash: 0,
                name: String::new(),
                data: vec![0xff; 300],
                ..Song::default()
            },
        ]
    }

    /// `songs()` with the v0.12 extras: one with everything, one with a
    /// file date only, one with a cover only.
    fn songs_full() -> Vec<Song> {
        let mut songs = songs();
        songs[0].updated_at = Some(1_700_000_000);
        songs[0].updated_at_file = Some(1_700_000_500);
        songs[0].meta = Some(SongMeta {
            title: "Заголовок".into(),
            album: "album".into(),
            artists: vec!["a".into(), "б".into()],
            genres: vec!["rock".into()],
            color: "#ff0000".into(),
            label: String::new(),
            volume: 0.75,
        });
        songs[0].cover_name = "front.webp".into();
        songs[0].cover = b"RIFF\0\0\0\0WEBP".to_vec();
        songs[1].updated_at_file = Some(0);
        songs[2].cover = vec![0xFF, 0xD8, 0xFF];
        songs
    }

    fn playlists() -> Vec<Playlist> {
        vec![
            Playlist {
                name: "favourites".into(),
                tracks: vec![u128::MAX, 0, 0x0123_4567_89ab_cdef_0011_2233_4455_6677],
                created_at: Some(0),
                updated_at: Some(1_700_000_000),
                cover_name: "cover.png".into(),
                cover: vec![0x89, b'P', b'N', b'G'],
            },
            Playlist {
                name: "пустой".into(),
                created_at: None,
                updated_at: Some(5),
                ..Playlist::default()
            },
        ]
    }

    /// byte offset of the section with `tag` (its tag byte).
    fn section_at(bytes: &[u8], tag: SectionTag) -> usize {
        let mut at = 9;
        loop {
            let len = u64::from_le_bytes(bytes[at + 1..at + 9].try_into().unwrap()) as usize;
            if bytes[at] == u8::from(tag) {
                return at;
            }
            at += 9 + len;
        }
    }

    #[test]
    fn roundtrip() {
        let bytes = encode(Version::V0_1, &songs(), &playlists()).unwrap();
        let packet = decode(&bytes).unwrap();
        assert_eq!(packet.version, Version::V0_1);
        assert_eq!(packet.songs, songs());
        assert_eq!(packet.playlists, playlists());
    }

    fn deleted() -> Vec<DeletedPlaylist> {
        vec![
            DeletedPlaylist {
                name: "старый".into(),
                deleted_at: 1_700_000_100,
            },
            DeletedPlaylist {
                name: "gone".into(),
                deleted_at: 7,
            },
        ]
    }

    #[test]
    fn deleted_roundtrip_v0_11() {
        let bytes = encode_with(Version::V0_11, &songs(), &playlists(), &deleted()).unwrap();
        let packet = decode(&bytes).unwrap();
        assert_eq!(packet.version, Version::V0_11);
        assert_eq!(packet.playlists, playlists());
        assert_eq!(packet.deleted, deleted());
        // the deleted section comes before the covers, which stay last.
        assert!(section_at(&bytes, SectionTag::Deleted) < section_at(&bytes, SectionTag::Covers));
        assert_eq!(bytes[8], 4, "songs, playlists, deleted, covers");
    }

    #[test]
    fn full_roundtrip_v0_12() {
        let bytes = encode_with(Version::V0_12, &songs_full(), &playlists(), &deleted()).unwrap();
        let packet = decode(&bytes).unwrap();
        assert_eq!(packet.version, Version::V0_12);
        assert_eq!(packet.songs, songs_full());
        assert_eq!(packet.playlists, playlists());
        assert_eq!(packet.deleted, deleted());
        // the covers live in their sections: no covers section.
        assert_eq!(bytes[8], 3, "songs, playlists, deleted");
        let s = section_at(&bytes, SectionTag::Songs);
        let p = section_at(&bytes, SectionTag::Playlists);
        assert!(s < p && p < section_at(&bytes, SectionTag::Deleted));
    }

    #[test]
    fn playlists_section_has_no_song_files() {
        let bytes = encode(Version::V0_12, &songs_full(), &playlists()).unwrap();
        let p = section_at(&bytes, SectionTag::Playlists);
        let len = u64::from_le_bytes(bytes[p + 1..p + 9].try_into().unwrap()) as usize;
        // count 4 + names + dates 2 × 17 + track lists (4 + 3 × 16, 4) +
        // the cover of the first playlist (name + image).
        let pls = playlists();
        let expected = 4
            + (2 + pls[0].name.len()) + (2 + pls[1].name.len())
            + 2 * 17
            + (4 + 3 * 16) + 4
            + (2 + pls[0].cover_name.len()) + (4 + pls[0].cover.len());
        assert_eq!(len, expected);
    }

    #[test]
    fn songs_info_flags_layout_v0_12() {
        let one = [Song {
            updated_at_file: Some(9),
            cover_name: "c.png".into(),
            cover: vec![1, 2],
            ..Song::default()
        }];
        let bytes = encode(Version::V0_12, &one, &[]).unwrap();
        // section header 9 + count 4 + hash 16 + empty name 2 + empty file 4
        // -> flags.
        let d = section_at(&bytes, SectionTag::Songs) + 9 + 4 + 16 + 2 + 4;
        assert_eq!(bytes[d], 0b1010);
        assert_eq!(&bytes[d + 1..d + 9], &0u64.to_le_bytes());
        assert_eq!(&bytes[d + 9..d + 17], &9u64.to_le_bytes());
        // no metadata: the cover name and the image follow.
        assert_eq!(&bytes[d + 17..d + 19], &5u16.to_le_bytes());
        assert_eq!(&bytes[d + 19..d + 24], b"c.png");
        assert_eq!(&bytes[d + 24..d + 28], &2u32.to_le_bytes());
        assert_eq!(&bytes[d + 28..d + 30], &[1, 2]);
        assert_eq!(d + 30, section_at(&bytes, SectionTag::Playlists));
        assert_eq!(decode(&bytes).unwrap().songs, one);
    }

    #[test]
    fn playlist_cover_flag_layout_v0_12() {
        let pls = [Playlist {
            cover_name: "p.gif".into(),
            cover: vec![7],
            ..Playlist::default()
        }];
        let bytes = encode(Version::V0_12, &[], &pls).unwrap();
        // section header 9 + count 4 + empty name 2 -> flags.
        let d = section_at(&bytes, SectionTag::Playlists) + 9 + 4 + 2;
        assert_eq!(bytes[d], 0b100);
        // dates 17 + empty track list 4 -> the cover name and the image.
        assert_eq!(&bytes[d + 21..d + 23], &5u16.to_le_bytes());
        assert_eq!(&bytes[d + 28..d + 32], &1u32.to_le_bytes());
        assert_eq!(bytes[d + 32], 7);
        assert_eq!(d + 33, bytes.len());
        assert_eq!(decode(&bytes).unwrap().playlists, pls);
    }

    #[test]
    fn a_cover_name_without_an_image_is_not_carried() {
        let one = [Song {
            cover_name: "c.png".into(),
            ..Song::default()
        }];
        let bytes = encode(Version::V0_12, &one, &[]).unwrap();
        assert_eq!(decode(&bytes).unwrap().songs, [Song::default()]);
    }

    #[test]
    fn older_versions_leave_out_the_v0_12_extras() {
        let bytes = encode(Version::V0_11, &songs_full(), &[]).unwrap();
        assert_eq!(bytes[8], 2, "songs, playlists");
        assert_eq!(decode(&bytes).unwrap().songs, songs());
        // the playlist covers still travel in the covers section.
        let bytes = encode(Version::V0_11, &[], &playlists()).unwrap();
        assert_eq!(bytes[8], 3, "songs, playlists, covers");
        assert_eq!(decode(&bytes).unwrap().playlists, playlists());
    }

    #[test]
    fn v0_12_truncated_at_every_point() {
        let bytes = encode_with(Version::V0_12, &songs_full(), &playlists(), &deleted()).unwrap();
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn v0_1_has_no_deleted_section() {
        let bytes = encode_with(Version::V0_1, &songs(), &playlists(), &deleted()).unwrap();
        assert_eq!(bytes[8], 3, "songs, playlists, covers");
        assert!(decode(&bytes).unwrap().deleted.is_empty());
    }

    #[test]
    fn deleted_truncated_at_every_point() {
        let bytes = encode_with(Version::V0_11, &[], &[], &deleted()).unwrap();
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn empty_packet() {
        let bytes = encode(Version::V0_1, &[], &[]).unwrap();
        // header 9 + two sections × (9 + count 4).
        assert_eq!(bytes.len(), 9 + 2 * 13);
        let packet = decode(&bytes).unwrap();
        assert!(packet.songs.is_empty() && packet.playlists.is_empty());
    }

    #[test]
    fn header_layout() {
        let bytes = encode(Version::V0_1, &songs(), &playlists()).unwrap();
        assert_eq!(&bytes[0..4], b"DPLR");
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
        assert_eq!(bytes[8], 3);
        assert_eq!(bytes[9], u8::from(SectionTag::Songs));
    }

    #[test]
    fn sections_order_and_covers_last() {
        let bytes = encode(Version::V0_1, &songs(), &playlists()).unwrap();
        let s = section_at(&bytes, SectionTag::Songs);
        let p = section_at(&bytes, SectionTag::Playlists);
        let c = section_at(&bytes, SectionTag::Covers);
        assert!(s < p && p < c);
        let c_len = u64::from_le_bytes(bytes[c + 1..c + 9].try_into().unwrap()) as usize;
        assert_eq!(c + 9 + c_len, bytes.len());
    }

    #[test]
    fn no_covers_section_without_covers() {
        let mut pls = playlists();
        pls[0].cover.clear();
        pls[0].cover_name.clear();
        let bytes = encode(Version::V0_1, &songs(), &pls).unwrap();
        assert_eq!(bytes[8], 2);
        assert_eq!(decode(&bytes).unwrap().playlists, pls);
    }

    #[test]
    fn unknown_section_is_skipped() {
        let mut bytes = encode(Version::V0_1, &songs(), &playlists()).unwrap();
        bytes[8] += 1;
        bytes.push(200);
        bytes.extend_from_slice(&3u64.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3]);
        let packet = decode(&bytes).unwrap();
        assert_eq!(packet.songs, songs());
        assert_eq!(packet.playlists, playlists());
    }

    #[test]
    fn duplicate_section() {
        let mut bytes = encode(Version::V0_1, &[], &[]).unwrap();
        let p = section_at(&bytes, SectionTag::Playlists);
        bytes[p] = u8::from(SectionTag::Songs);
        assert_eq!(decode(&bytes), Err(CodecError::DuplicateSection));
    }

    #[test]
    fn covers_count_mismatch() {
        let bytes = encode(Version::V0_1, &[], &playlists()).unwrap();
        let mut bad = bytes.clone();
        // claim one cover less: cut the last (empty, 2-byte) name and the
        // last (empty, 4-byte) cover.
        let c = section_at(&bad, SectionTag::Covers);
        bad[c + 9..c + 13].copy_from_slice(&1u32.to_le_bytes());
        bad.truncate(bad.len() - 4);
        let last_name = c + 13 + 2 + "cover.png".len();
        bad.drain(last_name..last_name + 2);
        let new_len = (bad.len() - c - 9) as u64;
        bad[c + 1..c + 9].copy_from_slice(&new_len.to_le_bytes());
        assert_eq!(decode(&bad), Err(CodecError::CountMismatch));
    }

    #[test]
    fn bad_start_word() {
        let mut bytes = encode(Version::V0_1, &songs(), &[]).unwrap();
        bytes[0] = b'X';
        assert_eq!(decode(&bytes), Err(CodecError::BadStartWord));
    }

    #[test]
    fn unsupported_version() {
        let mut bytes = encode(Version::V0_1, &songs(), &[]).unwrap();
        bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(decode(&bytes), Err(CodecError::UnsupportedVersion));
    }

    #[test]
    fn truncated_at_every_point() {
        let bytes = encode(Version::V0_1, &songs(), &playlists()).unwrap();
        for len in 0..bytes.len() {
            assert_eq!(
                decode(&bytes[..len]),
                Err(CodecError::UnexpectedEof),
                "len {len}"
            );
        }
    }

    #[test]
    fn trailing_bytes() {
        let mut bytes = encode(Version::V0_1, &songs(), &playlists()).unwrap();
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(CodecError::TrailingBytes));
    }

    #[test]
    fn trailing_bytes_inside_section() {
        let mut bytes = encode(Version::V0_1, &[], &[]).unwrap();
        // grow the songs section by one junk byte.
        let s = section_at(&bytes, SectionTag::Songs);
        bytes[s + 1..s + 9].copy_from_slice(&5u64.to_le_bytes());
        bytes.insert(s + 13, 0);
        assert_eq!(decode(&bytes), Err(CodecError::TrailingBytes));
    }

    #[test]
    fn invalid_utf8_name() {
        let one = [Song {
            hash: 1,
            name: "a".into(),
            ..Song::default()
        }];
        let mut bytes = encode(Version::V0_1, &one, &[]).unwrap();
        // section header 9 + count 4 + hash 16 + name length 2.
        let s = section_at(&bytes, SectionTag::Songs);
        bytes[s + 9 + 4 + 16 + 2] = 0xff;
        assert_eq!(decode(&bytes), Err(CodecError::InvalidUtf8));
    }

    #[test]
    fn huge_counts_do_not_allocate() {
        for version in [Version::V0_1, Version::V0_12] {
            let mut bytes = encode(version, &[], &[]).unwrap();
            let s = section_at(&bytes, SectionTag::Songs);
            bytes[s + 9..s + 13].copy_from_slice(&u32::MAX.to_le_bytes());
            assert_eq!(decode(&bytes), Err(CodecError::UnexpectedEof));
        }

        let mut bytes = encode(Version::V0_1, &[], &[]).unwrap();
        let p = section_at(&bytes, SectionTag::Playlists);
        bytes[p + 9..p + 13].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), Err(CodecError::UnexpectedEof));
    }

    #[test]
    fn name_too_long() {
        let long = "a".repeat(u16::MAX as usize + 1);
        let s = [Song {
            name: long.clone(),
            ..Song::default()
        }];
        assert_eq!(encode(Version::V0_1, &s, &[]), Err(CodecError::NameTooLong));
        let p = [Playlist {
            name: long,
            ..Playlist::default()
        }];
        assert_eq!(encode(Version::V0_1, &[], &p), Err(CodecError::NameTooLong));
    }

    #[test]
    fn dates_flags_layout() {
        let pls = [Playlist {
            created_at: None,
            updated_at: Some(7),
            ..Playlist::default()
        }];
        let bytes = encode(Version::V0_1, &[], &pls).unwrap();
        // section header 9 + count 4 + empty name 2 -> flags.
        let d = section_at(&bytes, SectionTag::Playlists) + 9 + 4 + 2;
        assert_eq!(bytes[d], 0b10);
        assert_eq!(&bytes[d + 1..d + 9], &0u64.to_le_bytes());
        assert_eq!(&bytes[d + 9..d + 17], &7u64.to_le_bytes());
        assert_eq!(decode(&bytes).unwrap().playlists, pls);
    }

    #[test]
    fn section_tag_conversion() {
        for tag in [
            SectionTag::Songs,
            SectionTag::Playlists,
            SectionTag::Covers,
            SectionTag::Deleted,
        ] {
            assert_eq!(SectionTag::try_from(u8::from(tag)), Ok(tag));
        }
        assert_eq!(SectionTag::try_from(0), Err(0));
        assert_eq!(SectionTag::try_from(5), Err(5));
    }
}
