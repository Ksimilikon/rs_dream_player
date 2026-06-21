use std::{
    error::Error,
    path::{Path, PathBuf},
};

use lofty::{
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::{Accessor, ItemKey},
};

/// struct for metadata tracks
#[derive(Debug)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: Vec<String>,
    pub params: Option<TrackMetadataParams>,
}

/// init for loaded track
#[derive(Debug)]
pub struct TrackMetadataParams {
    pub duration_sec: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    /// path to a cover art file on disk (png/jpg/gif), extracted and validated
    /// by the indexer. `None` if the track has no (valid) cover art.
    pub cover_art: Option<PathBuf>,
}

impl TrackMetadata {
    // pub fn from_raw(bytes: Vec<u8>) -> Self {
    //     let probed =
    // }

    /// reads tags directly from `path` via lofty without loading the whole
    /// audio file into memory. `cover_art` is always `None` here — extracting,
    /// validating and saving embedded cover art is the indexer's job
    /// (see `crate::storage::index` and `crate::music::cover_art`).
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        let probed = Probe::open(path)?.guess_file_type()?;
        let tagged_file = probed.read()?;

        let properties = tagged_file.properties();

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
            .ok_or("No tags found")?;

        let mut artists: Vec<String> = tag
            .get_strings(ItemKey::TrackArtist)
            .map(|s| s.to_string())
            .collect();
        if artists.is_empty() {
            artists.push("Unknown".into());
        }

        Ok(TrackMetadata {
            title: tag.title().map_or("Unknown".into(), |v| v.to_string()),
            artist: artists,
            params: Some(TrackMetadataParams {
                duration_sec: properties.duration().as_secs(),
                sample_rate: properties.sample_rate().unwrap_or(0),
                bitrate: properties.audio_bitrate().unwrap_or(0),
                cover_art: None,
            }),
        })
    }
}
