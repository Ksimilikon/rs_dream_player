use std::{
    error::Error,
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

use lofty::{
    file::{AudioFile, TaggedFileExt},
    probe::Probe,
    tag::Accessor,
};
use serde::{Deserialize, Serialize};

use crate::audio::song::metadata::Metadata;

/// contain byte-sequencea
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Track {
    // TODO: make field with type file music
    // type: TypeFile
    data: Vec<u8>,
}
impl Track {
    pub fn new(bytes: &[u8]) -> Self {
        Track {
            data: bytes.to_vec(),
        }
    }
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        let mut file = File::open(path.as_ref())?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        if !Self::is_music(&buf) {
            return Err(format!("{} isnt music", path.as_ref().display()).into());
        }

        Ok(Track { data: buf })
    }

    pub fn get(&self) -> &Vec<u8> {
        self.data.as_ref()
    }
    pub fn get_metadata(&self) -> Result<Metadata, Box<dyn Error>> {
        let mut reader = Cursor::new(&self.data);

        let probed = Probe::new(&mut reader).guess_file_type()?;
        let tagged_file = probed.read()?;

        let properties = tagged_file.properties();

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
            .unwrap();

        Ok(Metadata {
            title: tag.title().map_or("Unknown".into(), |v| v.to_string()),
            artist: tag.artist().map_or("Unknown".into(), |v| v.to_string()),
            album: tag.album().map_or("Unknown".into(), |v| v.to_string()),
            duration_sec: properties.duration().as_secs(),
            sample_rate: properties.sample_rate().unwrap_or(0),
            bitrate: properties.audio_bitrate().unwrap_or(0),
            track_number: tag.track(),
        })
    }
    pub fn get_cover_art(&self) -> Option<Vec<u8>> {
        let mut reader = Cursor::new(&self.data);

        let probed = Probe::new(&mut reader).guess_file_type().unwrap();
        let tagged_file = probed.read().unwrap();
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
            .unwrap();

        tag.pictures().first().map(|p| p.data().to_vec())
    }
}

impl Track {
    pub fn is_music(bytes: &[u8]) -> bool {
        if bytes.len() < 4 {
            return false;
        }

        // MP3: начинается с "ID3" или специфического кадра (0xFF 0xFB)
        let is_mp3 = &bytes[0..3] == b"ID3" || (bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0);

        // WAV: "RIFF" в начале и "WAVE" на 8-й позиции
        let is_wav = bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE";

        // FLAC: "fLaC"
        let is_flac = &bytes[0..4] == b"fLaC";

        // OGG: "OggS"
        let is_ogg = &bytes[0..4] == b"OggS";

        // MIDI: "MThd"
        let is_midi = &bytes[0..4] == b"MThd";

        // M4A/AAC: "ftypM4A" или "ftypmp42" на 4-й позиции
        let is_m4a = bytes.len() > 11 && &bytes[4..11] == b"ftypM4A";

        is_mp3 || is_wav || is_flac || is_ogg || is_midi || is_m4a
    }
}

#[cfg(debug_assertions)]
impl Track {
    pub fn debug_get(&self) -> &Vec<u8> {
        &self.data
    }
}
