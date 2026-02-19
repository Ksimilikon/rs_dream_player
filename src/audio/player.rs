use std::{sync::mpsc, time::Duration};

use rodio::{OutputStream, Sink};

use crate::audio::metadata::Metadata;

use super::{playlist::Playlist, types::Volume};

pub struct Player {
    cur_playlist: Option<Playlist>,
    volume: Volume,
    sink: Sink,
    _stream: OutputStream,
    _dbus_tx: Option<tokio::sync::mpsc::Sender<Metadata>>,
}
impl Player {
    pub fn new(dbus_tx: Option<tokio::sync::mpsc::Sender<Metadata>>) -> Self {
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
        // WARN: unwrap panic
        self.cur_playlist
            .as_mut()
            .unwrap()
            .play(&self.sink, self.volume)
            .unwrap();
        if let Some(tx) = self._dbus_tx.as_ref() {
            let _ = tx.blocking_send(Metadata {
                title: "ava".to_string(),
                artist: vec!["momo".to_string()],
                cover_art: None,
            });
        }
    }
    pub fn set_volume(&mut self, volume: Volume) {
        self.volume = volume;
        self.sink.set_volume(volume);
    }
    pub fn next(&mut self) {
        if let Some(playlist) = self.cur_playlist.as_mut() {
            playlist.next();
        }
    }
    pub fn prev(&mut self) {
        if let Some(playlist) = self.cur_playlist.as_mut() {
            playlist.prev();
        }
    }
}
