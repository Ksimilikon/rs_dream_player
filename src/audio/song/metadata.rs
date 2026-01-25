use std::{
    error::Error,
    hash::{DefaultHasher, Hash, Hasher},
};

use audiotags::Tag;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct SongMetaData {
    #[serde(rename = "mpris:trackid")]
    track_id: String,
    #[serde(rename = "xesam:title")]
    title: String,
    #[serde(rename = "mpris:length")]
    length: u64,
    // cover_album: Option<>
    #[serde(rename = "xesam:artist")]
    artist: Option<String>,
}
impl SongMetaData {
    pub fn new(path: &String) -> Result<Self, Box<dyn Error>> {
        let tag = Tag::new().read_from_path(path)?;
        let title = tag
            .title()
            .map(|s| s.to_string())
            .unwrap_or("unknown".to_string());
        let artist = tag
            .artist()
            .map(|s| s.to_string())
            .unwrap_or("unknown".to_string());
        let track_id = SongMetaData::gen_track_id(path);
        let length_micsec = match tag.duration().map(|secs| (secs * 1_000_000.0) as u64) {
            Some(v) => v,
            None => return Err(format!("error::reading metadata::duration::{}", path).into()),
        };

        Ok(Self {
            track_id,
            title,
            length: length_micsec,
            artist: Some(artist),
        })
    }
    fn gen_track_id(path: &String) -> String {
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        let hash = hasher.finish();
        format!("/org/mpris/DreamPlayer/Track/{}", hash)
    }
}
