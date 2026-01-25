use std::{error::Error, fs::File, io::Read, path::Path};

/// contain byte-sequence
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
        let mut file = File::open(path)?;
        let mut buf = Vec::new();
        _ = file.read_to_end(&mut buf);

        Ok(Track { data: buf })
    }

    pub fn get(&self) -> &Vec<u8> {
        self.data.as_ref()
    }
}
