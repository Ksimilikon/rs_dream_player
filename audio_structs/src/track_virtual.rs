use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{track::Track, track_metadata::TrackMetadata, types::Volume};

/// struct for general using song
/// can unload and load both the raw track bytes and its metadata,
/// so a playlist doesn't have to keep everything in RAM at once.
#[derive(Debug)]
pub struct TrackVirtual {
    src: TypeSource,
    metadata: Option<Arc<TrackMetadata>>,
    track: Option<Vec<u8>>,
    pub volume: Volume,
}

#[derive(Debug)]
enum TypeSource {
    /// track backed by a row in the sqlite index: db id + file path
    Index {
        id: i64,
        path: PathBuf,
    },
    // TODO: String to special type from string with strict struct
    Out(String),
    File(PathBuf),
    // // TODO: url
    // Stream(String),
}

#[derive(Debug)]
pub struct ErrorTrackUnload(String);
impl fmt::Display for ErrorTrackUnload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ErrorTrackUnload {}

#[derive(Debug)]
pub struct ErrorMetadataUnload(String);
impl fmt::Display for ErrorMetadataUnload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ErrorMetadataUnload {}

#[derive(Debug)]
pub struct ErrorUnsupportedSource(String);
impl fmt::Display for ErrorUnsupportedSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ErrorUnsupportedSource {}

impl TrackVirtual {
    /// builds a track backed by the sqlite index. `metadata` normally comes
    /// from a single bulk query over the whole playlist, so it's cheap to
    /// keep loaded for every track.
    pub fn new(id: i64, path: PathBuf, metadata: TrackMetadata) -> Self {
        Self {
            src: TypeSource::Index { id, path },
            metadata: Some(Arc::new(metadata)),
            track: None,
            volume: 1.,
        }
    }

    /// arg: with_load: loaded or unloaded track in struct
    pub fn from_file(path: PathBuf, with_load: bool) -> Result<Self, Box<dyn Error>> {
        if with_load {
            let track = Track::from_file(&path)?;
            let metadata = Track::get_metadata(&track)?;
            Ok(Self {
                src: TypeSource::File(path),
                metadata: Some(Arc::new(metadata)),
                track: Some(track),
                volume: 1.,
            })
        } else {
            if !Track::is_music_file(&path)? {
                return Err(format!("{}: isnt music", path.display()).into());
            }
            let metadata = TrackMetadata::from_path(&path)?;
            Ok(Self {
                src: TypeSource::File(path),
                metadata: Some(Arc::new(metadata)),
                track: None,
                volume: 1.,
            })
        }
    }

    /// move Vec<u8> and track is unloeded in struct
    pub fn take_track(&mut self) -> Result<Vec<u8>, ErrorTrackUnload> {
        match self.track.take() {
            Some(t) => Ok(t),
            None => Err(ErrorTrackUnload("track is unload".into())),
        }
    }

    /// load track Vec<u8> to RAM
    pub fn load_track(&mut self) -> Result<(), Box<dyn Error>> {
        if self.track.is_some() {
            return Ok(());
        }
        match &self.src {
            TypeSource::Index { path, .. } => self.track = Some(Track::from_file(path)?),
            TypeSource::File(path) => self.track = Some(Track::from_file(path)?),
            TypeSource::Out(_) => {
                return Err(Box::new(ErrorUnsupportedSource(
                    "loading raw audio from an external source isn't implemented yet".into(),
                )));
            }
        }
        Ok(())
    }
    pub fn unload_track(&mut self) {
        self.track = None;
    }

    pub fn get_metadata(&self) -> Result<Arc<TrackMetadata>, ErrorMetadataUnload> {
        match &self.metadata {
            Some(m) => Ok(m.clone()),
            None => Err(ErrorMetadataUnload("metadata is unload".into())),
        }
    }
    /// reloads metadata if it was unloaded, by re-probing the underlying file.
    /// Note: for `Index`-sourced tracks this won't restore `cover_art`, since
    /// that path lives in the sqlite index (see `crate::storage::index`).
    pub fn load_metadata(&mut self) -> Result<(), Box<dyn Error>> {
        if self.metadata.is_some() {
            return Ok(());
        }
        let path = self.get_path().ok_or_else(|| {
            ErrorUnsupportedSource("track has no path to load metadata from".into())
        })?;
        self.metadata = Some(Arc::new(TrackMetadata::from_path(path)?));
        Ok(())
    }
    pub fn unload_metadata(&mut self) {
        self.metadata = None;
    }

    /// path to the underlying audio file, if the source has one
    pub fn get_path(&self) -> Option<&Path> {
        match &self.src {
            TypeSource::Index { path, .. } => Some(path),
            TypeSource::File(path) => Some(path),
            TypeSource::Out(_) => None,
        }
    }

    /// row id in the sqlite index, if this track is backed by it
    pub fn index_id(&self) -> Option<i64> {
        match &self.src {
            TypeSource::Index { id, .. } => Some(*id),
            _ => None,
        }
    }
}
