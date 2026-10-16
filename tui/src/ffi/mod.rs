//! the layer working with the core through its FFI (`api`). Split by purpose:
//! [`engine`] — playback, [`db`] — index and playlists, [`cdc`] — packets
//! (export / import); [`types`] — owning Rust mirrors of the FFI structs.

pub mod cdc;
pub mod db;
pub mod engine;
pub mod types;

pub use types::TrackMeta;
