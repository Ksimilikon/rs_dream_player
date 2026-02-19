pub struct Metadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_sec: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub track_number: Option<u32>,
}
