//! извлечение изображений для показа: из отдельного файла обложки на диске и,
//! как запасной вариант, прямо из тегов аудиофайла (встроенная обложка).
//!
//! Разбор тегов не дублируем: встроенную обложку достаёт
//! [`audio_structs::track_metadata::TrackMetadata::read_cover`].

use std::path::Path;

use audio_structs::track_metadata::TrackMetadata;
use image::DynamicImage;

/// читает и декодирует изображение по пути к файлу-картинке. `None`, если файла
/// нет или это не поддерживаемая картинка. Для GIF берётся ПЕРВЫЙ кадр: sixel
/// показывает статику, анимация потребовала бы покадровой перерисовки вручную.
pub fn load_image(path: &str) -> Option<DynamicImage> {
    let bytes = std::fs::read(path).ok()?;
    image::load_from_memory(&bytes).ok()
}

/// извлекает встроенную обложку из тегов аудиофайла и декодирует её. Само
/// извлечение байтов из тегов реализовано в audio_structs. `None`, если тегов/
/// обложки нет или формат не распознан.
pub fn load_embedded(audio_path: &str) -> Option<DynamicImage> {
    let bytes = TrackMetadata::read_cover(Path::new(audio_path))?;
    image::load_from_memory(&bytes).ok()
}
