use player::player::Player;
use std::sync::{Arc, Mutex, OnceLock};

// TODO: Player must contain list playilsts
static PLAYER: OnceLock<Arc<Mutex<Player>>> = OnceLock::new();

pub fn init(player: Arc<Mutex<Player>>) {
    let _ = PLAYER.set(player);
}
pub extern "C" fn get_playlist() {}
pub extern "C" fn get_playlists() {}
pub extern "C" fn select_song() {}
pub extern "C" fn next() {
    PLAYER.get().unwrap().lock().unwrap().next();
}
pub extern "C" fn prev() {
    PLAYER.get().unwrap().lock().unwrap().prev();
}
pub extern "C" fn playstop() {
    PLAYER.get().unwrap().lock().unwrap().play_pause();
}
