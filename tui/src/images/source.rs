//! image extraction for display: from a separate cover file on disk and, as a
//! fallback, straight from the audio file tags (the embedded cover).
//!
//! Tag parsing is not duplicated: the embedded cover is extracted by
//! [`audio_structs::track_metadata::TrackMetadata::read_cover`].

use std::path::Path;

use audio_structs::track_metadata::TrackMetadata;
use image::DynamicImage;

/// reads and decodes an image by the path to a picture file. `None` if the
/// file is missing or isn't a supported picture. For GIF the FIRST frame is
/// taken: sixel shows a still image, animation would need frame-by-frame
/// redrawing by hand.
pub fn load_image(path: &str) -> Option<DynamicImage> {
    let bytes = std::fs::read(path).ok()?;
    image::load_from_memory(&bytes).ok()
}

/// extracts the embedded cover from the audio file tags and decodes it. The
/// byte extraction from the tags is implemented in audio_structs. `None` if
/// there are no tags / cover or the format isn't recognized.
pub fn load_embedded(audio_path: &str) -> Option<DynamicImage> {
    let bytes = TrackMetadata::read_cover(Path::new(audio_path))?;
    image::load_from_memory(&bytes).ok()
}
