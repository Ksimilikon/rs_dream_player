use player::player::Player;
use std::sync::{Arc, Mutex, OnceLock};

// TODO: Player must contain list playilsts
static PLAYER: OnceLock<Arc<Mutex<Player>>> = OnceLock::new();

pub fn init(player: Arc<Mutex<Player>>) {
    let _ = PLAYER.set(player);
}

#[unsafe(no_mangle)]
pub extern "C" fn get_playlist() {}
#[unsafe(no_mangle)]
pub extern "C" fn get_playlists() {}
#[unsafe(no_mangle)]
pub extern "C" fn select_song(id: u32) {
    PLAYER.get().unwrap().lock().unwrap().select_song(id);
}
#[unsafe(no_mangle)]
pub extern "C" fn next() {
    PLAYER.get().unwrap().lock().unwrap().next();
}
#[unsafe(no_mangle)]
pub extern "C" fn prev() {
    PLAYER.get().unwrap().lock().unwrap().prev();
}
#[unsafe(no_mangle)]
pub extern "C" fn playstop() {
    PLAYER.get().unwrap().lock().unwrap().play_pause();
}
