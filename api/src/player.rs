use std::sync::{Arc, Mutex, OnceLock};

// TODO: Player must contain list playilsts

#[unsafe(no_mangle)]
pub extern "C" fn get_playlist() {}
#[unsafe(no_mangle)]
pub extern "C" fn get_playlists() {}
#[unsafe(no_mangle)]
pub extern "C" fn select_song(id: u32) {}
#[unsafe(no_mangle)]
pub extern "C" fn next() {}
#[unsafe(no_mangle)]
pub extern "C" fn prev() {}
#[unsafe(no_mangle)]
pub extern "C" fn playstop() {}
