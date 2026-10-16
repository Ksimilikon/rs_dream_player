//! The player core as a dynamic / static library: re-exports the whole C ABI
//! surface of the `api` crate (symbols `init`, `idx_*`, `engine_*`,
//! `idx_free_*`, ...), so they end up in this crate's cdylib/staticlib.
//!
//! The default build (`cargo build`) produces only this library — no TUI.
//! The `cli` feature additionally builds the demo binary (`src/main.rs`).

pub use api::*;
