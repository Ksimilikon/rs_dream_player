use std::{
    error::Error,
    io::Cursor,
    path::{Path, PathBuf},
};

use lofty::{
    config::WriteOptions,
    file::{AudioFile, TaggedFileExt},
    picture::{Picture, PictureType},
    probe::Probe,
    tag::{Accessor, ItemKey, ItemValue, Tag, TagExt, TagItem},
};

use crate::cover_art::detect_image_format;

/// struct for metadata tracks
#[derive(Debug)]
pub struct TrackMetadata {
    pub title: String,
    pub artist: Vec<String>,
    /// album name from the file tags, if any (1:N — one album per song).
    pub album: Option<String>,
    /// genre list from the file tags (M:N — may be empty).
    pub genres: Vec<String>,
    pub params: Option<TrackMetadataParams>,
}

/// init for loaded track
#[derive(Debug)]
pub struct TrackMetadataParams {
    pub duration_sec: u64,
    pub sample_rate: u32,
    pub bitrate: u32,
    /// path to a cover art file on disk (png/jpg/gif), extracted and validated
    /// by the indexer. `None` if the track has no (valid) cover art.
    pub cover_art: Option<PathBuf>,
}

impl TrackMetadata {
    // pub fn from_raw(bytes: Vec<u8>) -> Self {
    //     let probed =
    // }

    /// reads tags directly from `path` via lofty without loading the whole
    /// audio file into memory. `cover_art` is always `None` here — extracting,
    /// validating and saving embedded cover art is the indexer's job
    /// (see `crate::storage::index` and `crate::music::cover_art`).
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn Error>> {
        let probed = Probe::open(path)?.guess_file_type()?;
        let tagged_file = probed.read()?;

        let properties = tagged_file.properties();

        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
            .ok_or("No tags found")?;

        let mut artists: Vec<String> = tag
            .get_strings(ItemKey::TrackArtist)
            .map(|s| s.to_string())
            .collect();
        if artists.is_empty() {
            artists.push("Unknown".into());
        }

        let album = tag.get_string(ItemKey::AlbumTitle).map(str::to_string);
        let genres: Vec<String> = tag
            .get_strings(ItemKey::Genre)
            .map(str::to_string)
            .collect();

        Ok(TrackMetadata {
            title: tag.title().map_or("Unknown".into(), |v| v.to_string()),
            artist: artists,
            album,
            genres,
            params: Some(TrackMetadataParams {
                duration_sec: properties.duration().as_secs(),
                sample_rate: properties.sample_rate().unwrap_or(0),
                bitrate: properties.audio_bitrate().unwrap_or(0),
                cover_art: None,
            }),
        })
    }

    /// writes `title`, the `artists` list, `album` and the `genres` list into the
    /// file's tags at `path`. Creates a primary tag of the file's native type if
    /// one doesn't exist yet, so even an untagged file becomes editable.
    /// `album == None` clears the album tag.
    pub fn write_tags(
        path: &Path,
        title: &str,
        artists: &[String],
        album: Option<&str>,
        genres: &[String],
    ) -> Result<(), Box<dyn Error>> {
        let mut tagged_file = Probe::open(path)?.guess_file_type()?.read()?;
        ensure_primary_tag(&mut tagged_file);
        let tag = tagged_file
            .primary_tag_mut()
            .ok_or("no writable tag for this file type")?;

        tag.insert_text(ItemKey::TrackTitle, title.to_string());
        // rewrite the whole artist list: drop existing items, then push each one.
        tag.remove_key(ItemKey::TrackArtist);
        for artist in artists {
            tag.push(TagItem::new(
                ItemKey::TrackArtist,
                ItemValue::Text(artist.clone()),
            ));
        }

        // album: set or clear.
        match album {
            Some(a) => {
                tag.insert_text(ItemKey::AlbumTitle, a.to_string());
            }
            None => tag.remove_key(ItemKey::AlbumTitle),
        }

        // rewrite the whole genre list.
        tag.remove_key(ItemKey::Genre);
        for genre in genres {
            tag.push(TagItem::new(ItemKey::Genre, ItemValue::Text(genre.clone())));
        }

        tag.save_to_path(path, WriteOptions::default())?;
        Ok(())
    }

    /// embeds `image_bytes` as the front cover picture in the file's tags at
    /// `path`. Accepts only png/jpg/gif (validated via
    /// [`crate::cover_art::detect_image_format`]).
    pub fn write_cover(path: &Path, image_bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        // reject anything that isn't png/jpg/gif before touching the file.
        detect_image_format(image_bytes)?;

        let mut picture = Picture::from_reader(&mut Cursor::new(image_bytes))?;
        picture.set_pic_type(PictureType::CoverFront);

        let mut tagged_file = Probe::open(path)?.guess_file_type()?.read()?;
        ensure_primary_tag(&mut tagged_file);
        let tag = tagged_file
            .primary_tag_mut()
            .ok_or("no writable tag for this file type")?;

        tag.remove_picture_type(PictureType::CoverFront);
        tag.push_picture(picture);

        tag.save_to_path(path, WriteOptions::default())?;
        Ok(())
    }

    /// extracts the embedded cover art bytes from the file's tags, if any.
    /// Prefers the `CoverFront` picture, otherwise falls back to the first one.
    /// Returns `None` when the file has no readable tags or no picture. This is
    /// the canonical embedded-cover extractor used by consumers that only have
    /// the file path (e.g. the UI cover preview, the indexer).
    pub fn read_cover(path: &Path) -> Option<Vec<u8>> {
        let tagged_file = Probe::open(path).ok()?.guess_file_type().ok()?.read().ok()?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;
        let picture = tag
            .get_picture_type(PictureType::CoverFront)
            .or_else(|| tag.pictures().first())?;
        Some(picture.data().to_vec())
    }
}

/// ensures the file has a primary tag to write into, creating an empty one of
/// the file's native tag type when none is present.
fn ensure_primary_tag(tagged_file: &mut lofty::file::TaggedFile) {
    if tagged_file.primary_tag().is_none() {
        let tag_type = tagged_file.primary_tag_type();
        tagged_file.insert_tag(Tag::new(tag_type));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// путь к реальному mp3-семплу в репозитории (для тестов записи тегов).
    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../test_data/2.mp3")
    }

    #[test]
    fn write_tags_roundtrips_title_and_artists() {
        let src = fixture();
        if !src.exists() {
            eprintln!("skipping: fixture missing at {}", src.display());
            return;
        }
        let dir = tempdir().unwrap();
        let path = dir.path().join("copy.mp3");
        fs::copy(&src, &path).unwrap();

        TrackMetadata::write_tags(
            &path,
            "My New Title",
            &["Alice".into(), "Bob".into()],
            Some("Greatest Hits"),
            &["Rock".into(), "Pop".into()],
        )
        .unwrap();

        let meta = TrackMetadata::from_path(&path).unwrap();
        assert_eq!(meta.title, "My New Title");
        assert_eq!(meta.artist, vec!["Alice".to_string(), "Bob".to_string()]);
        assert_eq!(meta.album.as_deref(), Some("Greatest Hits"));
        assert_eq!(meta.genres, vec!["Rock".to_string(), "Pop".to_string()]);
    }

    /// валидный 1×1 PNG (красный пиксель) — для проверки записи/чтения обложки.
    const PNG_1X1: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8,
        2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192,
        0, 0, 3, 1, 1, 0, 201, 254, 146, 239, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    #[test]
    fn read_cover_roundtrips_embedded_picture() {
        let src = fixture();
        if !src.exists() {
            eprintln!("skipping: fixture missing at {}", src.display());
            return;
        }
        let dir = tempdir().unwrap();
        let path = dir.path().join("copy.mp3");
        fs::copy(&src, &path).unwrap();

        // без обложки read_cover возвращает None.
        assert!(TrackMetadata::read_cover(&path).is_none());

        // после встраивания — читаем те же байты обратно.
        TrackMetadata::write_cover(&path, PNG_1X1).unwrap();
        let got = TrackMetadata::read_cover(&path).expect("cover should be present");
        assert_eq!(got, PNG_1X1);
    }

    #[test]
    fn write_cover_rejects_non_image() {
        let src = fixture();
        if !src.exists() {
            return;
        }
        let dir = tempdir().unwrap();
        let path = dir.path().join("copy.mp3");
        fs::copy(&src, &path).unwrap();

        // не png/jpg/gif — должно вернуть ошибку до записи в файл.
        let err = TrackMetadata::write_cover(&path, b"not an image at all");
        assert!(err.is_err());
    }
}
