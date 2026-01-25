use std::{error::Error, fs::File, path::Path};

use serde::{Deserialize, Serialize};

use crate::audio::song::{track::Track, virtual_song::VirtualSong};

#[derive(Debug, Serialize, Deserialize)]
pub struct Playlist {
    // songs: Vec<VirtualSong>,
    songs: Box<Vec<Track>>,
    cur_song: usize,
}

impl Playlist {
    pub fn from_dir<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        let mut audio_files = Box::new(Vec::new());
        let dir = path.as_ref();
        if !dir.is_dir() {
            return Err(format!("'{}' isnt dirictory", dir.display()).into());
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                //skip subdirs
                continue;
            }

            match Track::from_file(path) {
                Ok(s) => audio_files.push(s),
                Err(e) => println!("{}", e),
            }
        }
        Ok(Self {
            songs: audio_files,
            cur_song: 0,
        })
    }

    pub fn get_song(&self) -> &Track {
        &self.songs[self.cur_song]
    }
    pub fn next(&mut self) {
        let temp = self.cur_song + 1;
        if temp < self.songs.len() {
            self.cur_song = temp;
        } else {
            self.cur_song = 0;
        }
    }
    pub fn prev(&mut self) {
        if self.cur_song == 0 {
            self.cur_song = self.songs.len() - 1;
        } else {
            self.cur_song -= 1;
        }
    }
}
