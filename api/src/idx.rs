#[unsafe(no_mangle)]
pub extern "C" fn idx_list_playlists() {}

#[unsafe(no_mangle)]
pub extern "C" fn idx_list_track(id_playlist: u64) {}

#[unsafe(no_mangle)]
pub extern "C" fn idx_metadata_track(id_track: u64) {}
