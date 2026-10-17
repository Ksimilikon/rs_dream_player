//! TUI frontend of the player core: demonstrates the `api` crate (a dynamic
//! library) entirely through its FFI. All work with the core goes through
//! [`ffi`]; the interface lives in [`ui`]. `main` only prepares directories
//! and calls [`run`].

// Part of the core FFI surface is wrapped in `ffi/` but not yet exposed in
// the interface (output device switching, some setters / getters). The
// wrappers are kept as an API demo; the allow silences their warnings.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

mod app;
mod ffi;
mod images;
mod integration;
mod ui;

/// directories the core needs for initialization (passed to FFI `init`).
pub struct Dirs<'a> {
    pub cache: &'a Path,
    pub config: &'a Path,
    pub data: &'a Path,
    pub music: &'a Path,
}

/// initializes the core, indexes the music directory and starts the
/// interface. Takes over the current thread until the user quits.
pub fn run(dirs: Dirs) -> std::io::Result<()> {
    // a panic must not "take down" the console: restore the terminal and write
    // the cause to a log file (the console may close on Windows — the log stays).
    // Installed BEFORE the first core call: the core hook then wraps this one
    // and keeps panics the core caught itself (returned as a `Panic` status)
    // away from it, so they don't tear down the interface.
    install_panic_hook(dirs.config.join("tui_panic.log"));

    // bring up the core (a db is needed). The core applies the master volume
    // from its own saved config — don't touch it here.
    ffi::engine::init(dirs.cache, dirs.config, dirs.data, dirs.music, true)
        .map_err(|st| std::io::Error::other(format!("core init: {}", ffi::engine::describe_init(st))))?;
    // fill the index from the music directory (idempotent).
    ffi::db::index_dir(None);

    let mut terminal = ratatui::init();
    let mut app = app::App::new(dirs.music.to_path_buf());
    let res = app.run(&mut terminal);
    ratatui::restore();
    res
}

/// wraps the current panic handler: first restores the terminal (raw / alt
/// screen), then appends the message + backtrace to `log_path`, then calls the
/// previous handler.
fn install_panic_hook(log_path: PathBuf) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // return the terminal to normal mode before any output.
        ratatui::restore();
        let bt = std::backtrace::Backtrace::force_capture();
        let msg = format!("{info}\n\nbacktrace:\n{bt}\n");
        let _ = std::fs::write(&log_path, &msg);
        prev(info);
    }));
}
