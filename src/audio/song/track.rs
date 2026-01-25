use std::{error::Error, fs::File, io::Read, path::Path};

use serde::{Deserialize, Serialize};

/// contain byte-sequencea
#[derive(Serialize, Deserialize, Debug)]
pub struct Track {
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
