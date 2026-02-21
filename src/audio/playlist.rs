use std::{error::Error, fs::File, io::Cursor, path::Path, sync::Arc};

use get_size::GetSize;
use rodio::{Decoder, Sink};
use serde::{Deserialize, Serialize};

use crate::audio::{
    song::{
        metadata::Metadata,
        track::{ErrorIsntMusic, Track},
        virtual_song::VirtualSong,
    },
    types::Volume,
};

pub struct Playlist {
    // songs: Vec<VirtualSong>,
    songs: Vec<VirtualSong>,
    cur_song: usize,
}
impl Playlist {
    pub fn from_dir<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        let mut audio_files = Vec::new();
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

            match VirtualSong::from_file(path) {
                Ok(s) => audio_files.push(s),
                Err(e) => {
                    if let Some(_e) = e.downcast_ref::<ErrorIsntMusic>() {
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Ok(Self {
            songs: audio_files,
            cur_song: 0,
        })
    }

    pub fn play(&mut self, sink: &Sink, volume: Volume) -> Result<(), Box<dyn Error>> {
        let visong = &mut self.songs[self.cur_song];
        visong.load_track();
        let track = visong.get_track()?;
        let cursor = Cursor::new(track.get().clone());
        let source = Decoder::new(cursor).unwrap();

        sink.set_volume(volume * visong.volume);

        sink.append(source);
        Ok(())
    }
    pub fn get_metadata(&self) -> Arc<Metadata> {
        self.songs[self.cur_song].get_metadata()
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

/// debug
// TODO: make caching
#[cfg(debug_assertions)]
impl Playlist {
    pub fn debug_songs_size(&self) {
        let list_structure_size = self.songs.capacity() * std::mem::size_of::<Track>();

        let total_audio_data: usize = self
            .songs
            .iter()
            .map(|visong| visong.debug_get_size())
            .sum();

        println!(
            "Структуры в списке: {} MB",
            list_structure_size as f32 / 1024. / 1024.
        );
        println!(
            "Чистые аудио-данные в куче: {} MB",
            total_audio_data as f32 / 1024. / 1024.
        );
        println!("Итого: {} bytes", list_structure_size + total_audio_data);
    }
}
