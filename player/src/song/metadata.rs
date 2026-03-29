#[derive(Debug)]
pub struct Metadata {
    pub title: String,
    pub artist: Vec<String>,
    pub albums: Vec<String>,
    pub params: Option<MetadataParams>,
}

/// init for loaded track
#[derive(Debug)]
pub struct MetadataParams {
    pub duration_sec: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub track_number: Option<u32>,
    pub cover_art: Option<Vec<u8>>,
}
