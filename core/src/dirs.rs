use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Dirs {
    pub cache: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,

    pub music_dir: PathBuf,
}
