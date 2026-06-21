use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
};

/// image formats accepted for cover art stored on disk
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverArt {
    Png,
    Jpeg,
    Gif,
}

impl CoverArt {
    pub fn extension(&self) -> &'static str {
        match self {
            CoverArt::Png => "png",
            CoverArt::Jpeg => "jpg",
            CoverArt::Gif => "gif",
        }
    }
}

#[derive(Debug)]
pub struct ErrorInvalidImageType(String);
impl fmt::Display for ErrorInvalidImageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl Error for ErrorInvalidImageType {}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];
const GIF87A: &[u8; 6] = b"GIF87a";
const GIF89A: &[u8; 6] = b"GIF89a";

/// detects image format by magic bytes, only png/jpg/gif are accepted as cover art
pub fn detect_image_format(bytes: &[u8]) -> Result<CoverArt, ErrorInvalidImageType> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        Ok(CoverArt::Png)
    } else if bytes.starts_with(&JPEG_SIGNATURE) {
        Ok(CoverArt::Jpeg)
    } else if bytes.starts_with(GIF87A) || bytes.starts_with(GIF89A) {
        Ok(CoverArt::Gif)
    } else {
        Err(ErrorInvalidImageType(
            "cover art type mismatch: only png, jpg and gif are supported".into(),
        ))
    }
}

/// validates `bytes` are png/jpg/gif and writes them to `dest_dir/<stem>.<ext>`,
/// returning the resulting path. Returns `ErrorInvalidImageType` for any other format.
pub fn save_cover_art(
    bytes: &[u8],
    dest_dir: &Path,
    stem: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let format = detect_image_format(bytes)?;
    fs::create_dir_all(dest_dir)?;
    let path = dest_dir.join(format!("{stem}.{}", format.extension()));
    fs::write(&path, bytes)?;
    Ok(path)
}
