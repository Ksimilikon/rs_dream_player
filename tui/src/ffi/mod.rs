//! распределённый слой работы с ядром через его FFI (`api`). Разделён по
//! назначению: [`engine`] — воспроизведение, [`db`] — индекс и плейлисты;
//! [`types`] — владеющие Rust-зеркала FFI-структур.

pub mod db;
pub mod engine;
pub mod types;

pub use types::TrackMeta;
