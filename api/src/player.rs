use std::time::Duration;

use audio_structs::track_virtual::TrackVirtual;

use super::GLOBAL_STATE;

#[unsafe(no_mangle)]
pub extern "C" fn engine_add_track(id_track: i64) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let mut track: Option<TrackVirtual> = None;
    if let Some(db) = guard.get_db()
        && let Ok(tracks) = db.find_track(None, None, Some(id_track), None)
        && !tracks.is_empty()
    {
        track = Some(tracks.into_iter().next().unwrap());
    }
    if let Some(t) = track.as_mut() {
        let _ = t.load_track();
        let _ = guard
            .get_engine_mut()
            .load(t.take_track().unwrap(), t.volume, None::<fn()>);
    }
}
// pub extern "C" fn engine_add_track_bytes()
#[unsafe(no_mangle)]
pub extern "C" fn engine_play_pause() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().play_pause();
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_play() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().play();
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_pause() {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().pause();
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_seek(seek_sec: u64) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let _ = guard.get_engine_mut().seek(Duration::from_secs(seek_sec));
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_seek() -> u64 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    let duration = guard.get_engine().get_pos();
    duration.as_secs()
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_volume_track(volume: f32) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().set_volume_track(volume);
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_set_volume_master(volume: f32) {
    let mut guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine_mut().set_volume_master(volume);
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_volume_track() -> f32 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().get_volume_track()
}
#[unsafe(no_mangle)]
pub extern "C" fn engine_get_volume_master() -> f32 {
    let guard = GLOBAL_STATE.get().unwrap().lock().unwrap();
    guard.get_engine().get_volume_master()
}
