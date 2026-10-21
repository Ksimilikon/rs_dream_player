//! section tags.

/// tag byte at the start of every section. On the wire it is a plain `u8`:
/// the discriminant of the variant. Never renumber, only append; a decoder
/// skips tags it does not know.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionTag {
    /// songs: hashes, file names, music files; v0.12+ also dates, metadata
    /// and covers.
    Songs = 1,
    /// playlists: names, dates, ordered lists of song hashes; v0.12+ also
    /// covers.
    Playlists = 2,
    /// playlist covers (v0.1 / v0.11 only); always the last section.
    Covers = 3,
    /// deleted playlists: names and deletion times (v0.11+).
    Deleted = 4,
    // 5 — reserved: the song info section of a v0.12 draft, never reuse.
}

impl From<SectionTag> for u8 {
    fn from(t: SectionTag) -> u8 {
        t as u8
    }
}

impl TryFrom<u8> for SectionTag {
    /// the unknown tag itself.
    type Error = u8;

    fn try_from(n: u8) -> Result<Self, Self::Error> {
        match n {
            1 => Ok(SectionTag::Songs),
            2 => Ok(SectionTag::Playlists),
            3 => Ok(SectionTag::Covers),
            4 => Ok(SectionTag::Deleted),
            _ => Err(n),
        }
    }
}
