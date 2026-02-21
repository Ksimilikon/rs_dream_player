use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use rodio::{OutputStream, Sink};

use crate::audio::song::metadata::Metadata;

use super::{playlist::Playlist, types::Volume};

pub struct Player {
    cur_playlist: Option<Playlist>,
    volume: Volume,
    sink: Sink,
    _stream: OutputStream,
    _dbus_tx: Option<tokio::sync::mpsc::Sender<Arc<Metadata>>>,
}
impl Player {
    pub fn new(dbus_tx: Option<tokio::sync::mpsc::Sender<Arc<Metadata>>>) -> Self {
        let _stream = rodio::OutputStreamBuilder::open_default_stream().unwrap();
        let sink = Sink::connect_new(_stream.mixer());
        sink.set_volume(1.0);
        Self {
            cur_playlist: None,
            volume: 0.5,
            sink,
            _stream,
            _dbus_tx: dbus_tx,
        }
    }
    pub fn set_playlist(&mut self, playlist: Playlist) {
        self.cur_playlist = Some(playlist);
    }
    pub fn play(&mut self) {
        // WARN:
        if let Some(playlist) = self.cur_playlist.as_mut() {
            playlist.play(&self.sink, self.volume);
        } else {
            println!("Player::play::not found playlist")
        }
        if let (Some(tx), Some(playlist)) = (self._dbus_tx.as_ref(), self.cur_playlist.as_ref()) {
            let _ = tx.blocking_send(playlist.get_metadata());
        }
    }
    pub fn set_volume(&mut self, volume: Volume) {
        self.volume = volume;
        self.sink.set_volume(volume);
    }
    pub fn next(&mut self) {
        if let Some(playlist) = self.cur_playlist.as_mut() {
            self.sink.stop();
            playlist.next();
            playlist.play(&self.sink, self.volume);
        }
    }
    pub fn prev(&mut self) {
        if let Some(playlist) = self.cur_playlist.as_mut() {
            self.sink.stop();
            playlist.prev();
            playlist.play(&self.sink, self.volume);
        }
    }
    pub fn play_pause(&mut self) {
        if self.sink.is_paused() {
            self.sink.play();
        } else {
            self.sink.pause();
        }
    }
    pub fn is_pause(&self) -> bool {
        self.sink.is_paused()
    }
}
