use std::{error::Error, path::PathBuf, sync::Arc};

use crate::{
    song::{track::Track, track_metadata::TrackMetadata},
    types::Volume,
};

/// struct for general using song
/// can unload and load raw track
#[derive(Debug)]
pub struct TrackVirtual {
    src: TypeSource,
    metadata: Arc<TrackMetadata>,
    track: Option<Track>,
    pub volume: Volume,
}

#[derive(Debug)]
enum TypeSource {
    Index(PathBuf),
    // TODO: String to special type from string with strict struct
    Out(String),
    File(PathBuf),
    // // TODO: url
    // Stream(String),
}

#[derive(Debug)]
pub struct ErrorTrackUnload(String);
impl std::fmt::Display for ErrorTrackUnload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ErrorTrackUnload {}

impl TrackVirtual {
    // pub fn from_index(id: i32)-> Result<Self, Box<dyn Error>>{}
    /// arg: with_load: loaded or unloaded track in struct
    pub fn from_file(path: PathBuf, with_load: bool) -> Result<Self, Box<dyn Error>> {
        let track = Track::from_file(&path)?;
        Ok(Self {
            src: TypeSource::File(path),
            metadata: Arc::new(track.get_metadata()?),
            track: if with_load { Some(track) } else { None },
            volume: 1.,
        })
    }

    // TODO: async
    pub fn get_track(&self) -> Result<&Track, ErrorTrackUnload> {
        match &self.track {
            Some(t) => Ok(t),
            None => Err(ErrorTrackUnload("track is unload".into())),
        }
    }
    // TODO: async
    // TODO: finish impl
    pub fn load_track(&mut self) -> Result<(), Box<dyn Error>> {
        match &self.src {
            TypeSource::Index(p) => {}
            TypeSource::Out(s) => {}
            TypeSource::File(p) => self.track = Some(Track::from_file(p)?),
        }
        Ok(())
    }
    pub fn unload_track(&mut self) {
        self.track = None;
    }
    pub fn get_metadata(&self) -> Arc<TrackMetadata> {
        self.metadata.clone()
    }
}
