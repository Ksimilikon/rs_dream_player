use std::fmt;

pub struct Metadata {
    pub title: String,
    pub artist: Vec<String>,
    pub album: Option<String>,
    pub duration_sec: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub track_number: Option<u32>,
    pub cover_art: Option<Vec<u8>>,
}

impl fmt::Display for Metadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Metadata {{\n  \
               title: {},\n  \
               artist: {:?},\n  \
               album: {},\n  \
               duration: {}s,\n  \
               sample_rate: {} Hz,\n  \
               bitrate: {} kbps,\n  \
               track_number: {},\n  \
               cover_art: {}\n\
             }}",
            self.title,
            self.artist,
            self.album.as_deref().unwrap_or("None"),
            self.duration_sec,
            self.sample_rate,
            self.bitrate / 1000,
            self.track_number
                .map_or("None".to_string(), |t| t.to_string()),
            self.cover_art
                .as_ref()
                .map_or("None".to_string(), |c| format!("Some({} bytes)", c.len()))
        )
    }
}
