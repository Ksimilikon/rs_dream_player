/// struct for metadata tracks
#[derive(Debug)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: Vec<String>,
    pub albums: Vec<String>,
    pub params: Option<TrackMetadataParams>,
}

/// init for loaded track
#[derive(Debug)]
pub struct TrackMetadataParams {
    pub duration_sec: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub cover_art: Option<Vec<u8>>,
}
