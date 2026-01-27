use std::time::Duration;

use rodio::{OutputStream, Sink};

use super::{playlist::Playlist, types::Volume};

pub struct Player {
    cur_playlist: Option<Playlist>,
    volume: Volume,
    position: Duration,
    preset: Vec<String>,
    sink: Sink,
    _stream: OutputStream,
}
impl Player {
    pub fn new() -> Self {
        let _stream = rodio::OutputStreamBuilder::open_default_stream().unwrap();
        let sink = Sink::connect_new(&_stream.mixer());
        sink.set_volume(0.5);
        Self {
            cur_playlist: None,
            volume: 0.5,
            position: Duration::new(0, 0),
            preset: vec!["default".to_string()],
            sink,
            _stream,
        }
    }
    pub fn set_playlist(&mut self, playlist: Playlist) {
        self.cur_playlist = Some(playlist);
    }
    pub fn play(&mut self) {
        self.cur_playlist
            .as_mut()
            .unwrap()
            .play(&self.sink, self.volume)
            .unwrap();
    }
}
// impl PlayerAgentOS for PlayerState {
//     fn send_meta_data(&self) -> Result<(), Box<dyn std::error::Error>> {
//
//     }
// }
