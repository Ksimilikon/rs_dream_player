//! codec errors.

use std::fmt;

/// everything that can go wrong in [`crate::encode`] / [`crate::decode`].
///
/// this is a fieldless `#[repr(u32)]` enum on purpose: it maps 1:1 to a plain
/// error code on the FFI side (`0` is left free for "ok"), with no heap data
/// to free. Discriminants are part of the ABI: never renumber, only append.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecError {
    /// decode: input does not start with [`crate::START_WORD`].
    BadStartWord = 1,
    /// decode: version number is unknown to this build.
    UnsupportedVersion = 2,
    /// decode: input ends before a section or payload is complete.
    UnexpectedEof = 3,
    /// decode: extra bytes after the last field of a section, or after the
    /// last section.
    TrailingBytes = 4,
    /// decode: a file / playlist name is not valid UTF-8.
    InvalidUtf8 = 5,
    /// encode: more than `u32::MAX` songs, playlists or tracks in a playlist;
    /// or more than `u8::MAX` sections.
    TooManyRows = 6,
    /// encode: a file / playlist name is longer than `u16::MAX` bytes.
    NameTooLong = 7,
    /// encode: a music file / cover is longer than `u32::MAX` bytes.
    DataTooLong = 8,
    /// decode: the same section appears twice.
    DuplicateSection = 9,
    /// decode: number of covers differs from number of playlists.
    CountMismatch = 10,
}

impl CodecError {
    /// numeric error code for FFI.
    pub fn code(self) -> u32 {
        self as u32
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            CodecError::BadStartWord => "bad start word",
            CodecError::UnsupportedVersion => "unsupported packet version",
            CodecError::UnexpectedEof => "unexpected end of packet",
            CodecError::TrailingBytes => "trailing bytes after section or packet",
            CodecError::InvalidUtf8 => "name is not valid UTF-8",
            CodecError::TooManyRows => "too many rows or sections",
            CodecError::NameTooLong => "name too long (max u16::MAX bytes)",
            CodecError::DataTooLong => "file too long (max u32::MAX bytes)",
            CodecError::DuplicateSection => "duplicate section",
            CodecError::CountMismatch => "covers count differs from playlists count",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for CodecError {}
