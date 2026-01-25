use std::{
    error::Error,
    hash::{DefaultHasher, Hash, Hasher},
};

use audiotags::Tag;
use serde::{Deserialize, Serialize};

use crate::audio::types::Volume;

#[derive(Serialize, Deserialize, Debug)]
pub struct VirtualSong {
    sources: Vec<String>,
    volume: Option<Volume>,
}
