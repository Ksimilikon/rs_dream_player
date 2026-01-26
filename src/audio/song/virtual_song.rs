use std::{
    error::Error,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::audio::{
    song::{metadata::Metadata, track::Track},
    types::Volume,
};

enum TypeSource {
    ///contain path file like id
    Inner(PathBuf),
    ///contain any str like id
    Outer(String),
}

pub struct VirtualSong {
    id: TypeSource,
    volume: Volume,
    metadata: Metadata,
    track: Option<Track>,
}

impl VirtualSong {
    pub fn from_file(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            id: TypeSource::Inner(path.clone()),
            volume: Volume::MAX,
            metadata: Track::from_file(path).unwrap().get_metadata()?,
            track: None,
        })
    }
    // pub fn from_out();
    // pub fn from_index();

    pub fn get_track(&self) -> Result<&Track, Box<dyn std::error::Error>> {
        match &self.track {
            Some(t) => Ok(t),
            None => Err("track is unloaded".into()),
        }
    }
    // TODO: async
    pub fn load_track(&mut self) {
        match &self.id {
            TypeSource::Inner(path) => {
                // TODO: handle unwrap
                let res = Track::from_file(path).unwrap();
                self.track = Some(res);
            }
            TypeSource::Outer(id) => {}
        }
    }
    pub fn unload_track(&mut self) {
        self.track = None;
    }
}

impl VirtualSong {
    pub fn debug_get_size(&self) -> usize {
        match &self.track {
            Some(v) => v.debug_get().capacity(),
            None => 0,
        }
    }
}
