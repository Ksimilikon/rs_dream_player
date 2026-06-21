//! `extern "C"` playback controls exported to mods. Each one forwards a command
//! to the running core through [`crate::bridge`].

use crate::bridge::{self, ApiCommand};

#[unsafe(no_mangle)]
pub extern "C" fn next() {
    bridge::send(ApiCommand::Next);
}

#[unsafe(no_mangle)]
pub extern "C" fn prev() {
    bridge::send(ApiCommand::Prev);
}

#[unsafe(no_mangle)]
pub extern "C" fn playstop() {
    bridge::send(ApiCommand::PlayStop);
}

#[unsafe(no_mangle)]
pub extern "C" fn select_track(id: u32) {
    bridge::send(ApiCommand::SelectTrack(id));
}

#[unsafe(no_mangle)]
pub extern "C" fn seek(seconds: f32) {
    bridge::send(ApiCommand::Seek(seconds));
}

/// master volume for the whole app, 0.0..=1.0.
#[unsafe(no_mangle)]
pub extern "C" fn set_volume(volume: f32) {
    bridge::send(ApiCommand::SetVolume(volume));
}

/// volume of the currently selected track only, 0.0..=1.0.
#[unsafe(no_mangle)]
pub extern "C" fn set_track_volume(volume: f32) {
    bridge::send(ApiCommand::SetTrackVolume(volume));
}
