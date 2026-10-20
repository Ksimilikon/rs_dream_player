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
    /// v0.12: v0.11 + the song info section (change dates and db metadata
    /// of the songs).
    V0_12 = 3,
}

impl Version {
    /// the newest version, used for new packets.
    pub const LATEST: Version = Version::V0_12;

    /// the version carries the deleted playlists section.
    pub fn has_deleted(self) -> bool {
        self != Version::V0_1
    }

    /// the version carries the song info section.
    pub fn has_song_info(self) -> bool {
        self as u32 >= Version::V0_12 as u32
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
            _ => Err(CodecError::UnsupportedVersion),
        }
    }
}
