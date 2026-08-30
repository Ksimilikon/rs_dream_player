use audio_structs::{playlist::Playlist, track_virtual::TrackVirtual};

pub enum CoreEvent {
    /// save track in db
    DBTrackSave(TrackVirtual),
    /// load track by the id
    DBTrackLoad(u64),
    /// save playlist
    DBPlaylistSave(Playlist),
    /// load playlist by id
    DBPlaylistLoad(u64),
}
