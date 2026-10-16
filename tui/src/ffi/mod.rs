//! распределённый слой работы с ядром через его FFI (`api`). Разделён по
//! назначению: [`engine`] — воспроизведение, [`db`] — индекс и плейлисты,
//! [`cdc`] — пакеты (экспорт/импорт); [`types`] — владеющие Rust-зеркала
//! FFI-структур.

pub mod cdc;
pub mod db;
pub mod engine;
pub mod types;

pub use types::TrackMeta;
