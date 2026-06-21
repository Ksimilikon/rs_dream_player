//! the bridge between the FFI surface (the `extern "C"` functions mods call) and
//! the running core. The core installs a command channel and keeps a small
//! state snapshot here; the `extern "C"` functions push commands onto the
//! channel and read the snapshot, so they never need to know about the core's
//! concrete types.

use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

/// playback commands a mod can issue. The core maps these onto its own internal
/// command type (see `crate::orchestrator::CoreCommand` in the binary).
#[derive(Debug, Clone)]
pub enum ApiCommand {
    PlayStop,
    Next,
    Prev,
    SelectTrack(u32),
    /// seek to an absolute position, in seconds.
    Seek(f32),
    /// set the master volume (everything), 0.0..=1.0.
    SetVolume(f32),
    /// set the volume of the currently selected track only, 0.0..=1.0.
    SetTrackVolume(f32),
}

/// what is currently playing, mirrored from core events for FFI queries.
#[derive(Debug, Default, Clone)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub duration_sec: u64,
}

/// read-only snapshot of core state, exposed to mods over FFI.
#[derive(Debug, Default, Clone)]
pub struct CoreSnapshot {
    pub now: NowPlaying,
    pub playlists: Vec<String>,
}

static COMMAND_TX: OnceLock<Mutex<Sender<ApiCommand>>> = OnceLock::new();
static STATE: OnceLock<Mutex<CoreSnapshot>> = OnceLock::new();

/// installs the command channel the FFI layer forwards mod commands onto.
/// Called once by the core at startup. Returns an error if called twice.
pub fn init(tx: Sender<ApiCommand>) -> Result<(), &'static str> {
    COMMAND_TX
        .set(Mutex::new(tx))
        .map_err(|_| "api bridge already initialised")?;
    let _ = STATE.set(Mutex::new(CoreSnapshot::default()));
    Ok(())
}

/// forwards a command to the core, dropping it if the bridge isn't initialised
/// or the core has shut down.
pub(crate) fn send(cmd: ApiCommand) {
    if let Some(tx) = COMMAND_TX.get() {
        if let Ok(guard) = tx.lock() {
            let _ = guard.send(cmd);
        }
    }
}

/// the core calls this to publish the currently playing track.
pub fn set_now_playing(now: NowPlaying) {
    if let Some(state) = STATE.get() {
        if let Ok(mut guard) = state.lock() {
            guard.now = now;
        }
    }
}

/// the core calls this to publish the list of available playlists.
pub fn set_playlists(names: Vec<String>) {
    if let Some(state) = STATE.get() {
        if let Ok(mut guard) = state.lock() {
            guard.playlists = names;
        }
    }
}

/// current snapshot for FFI queries.
pub(crate) fn snapshot() -> CoreSnapshot {
    STATE
        .get()
        .and_then(|s| s.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_default()
}
