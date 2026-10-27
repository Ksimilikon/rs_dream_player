//! packet format versions.

use crate::CodecError;

/// packet format version. On the wire it is a plain `u32` number: the
/// discriminant of the variant (`V0_1` → `1`). New versions get the next
/// number; existing numbers never change.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Version {
    /// v0.1: layout described in the crate docs, without the deleted
    /// playlists section.
    V0_1 = 1,
    /// v0.11: v0.1 + the deleted playlists section (soft deletion).
    V0_11 = 2,
    /// v0.12: v0.11 where every section carries everything about its
    /// entity: the songs section — the change dates, db metadata and covers
    /// of the songs; the playlists section — the playlist covers (there is
    /// no covers section).
    V0_12 = 3,
    /// v0.13: v0.12 where every playlist entry has its own id and the
    /// removed entries travel too, so playlists edited on two devices merge
    /// entry by entry.
    V0_13 = 4,
}

impl Version {
    /// the newest version, used for new packets.
    pub const LATEST: Version = Version::V0_13;

    /// the version carries the deleted playlists section.
    pub fn has_deleted(self) -> bool {
        self != Version::V0_1
    }

    /// the songs section carries the song dates, metadata and covers, the
    /// playlists section — the playlist covers (no covers section).
    pub fn full_sections(self) -> bool {
        self as u32 >= Version::V0_12 as u32
    }

    /// the playlists section carries the entries and the removed entries.
    pub fn has_entries(self) -> bool {
        self as u32 >= Version::V0_13 as u32
    }
}

impl From<Version> for u32 {
    fn from(v: Version) -> u32 {
        v as u32
    }
}

impl TryFrom<u32> for Version {
    type Error = CodecError;

    fn try_from(n: u32) -> Result<Self, Self::Error> {
        match n {
            1 => Ok(Version::V0_1),
            2 => Ok(Version::V0_11),
            3 => Ok(Version::V0_12),
            4 => Ok(Version::V0_13),
            _ => Err(CodecError::UnsupportedVersion),
        }
    }
}
