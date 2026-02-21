use std::{
    io::sink,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use rodio::{OutputStream, Sink};

use crate::audio::{self, audio_event::AudioEvent, song::metadata::Metadata};

use super::{playlist::Playlist, types::Volume};

pub struct Player {
    cur_playlist: Option<Playlist>,
    volume: Volume,
    sink: Sink,
    _stream: OutputStream,
    _dbus_tx: Option<tokio::sync::mpsc::Sender<Arc<Metadata>>>,
    _playungap_tx: std::sync::mpsc::Sender<AudioEvent>,
}
impl Player {
    pub fn new(dbus_tx: Option<tokio::sync::mpsc::Sender<Arc<Metadata>>>) -> Arc<Mutex<Self>> {
        let _stream = rodio::OutputStreamBuilder::open_default_stream().unwrap();
        let sink = Sink::connect_new(_stream.mixer());
        sink.set_volume(1.0);
        let (play_tx, play_rx) = std::sync::mpsc::channel::<AudioEvent>();

        let res = Arc::new(Mutex::new(Self {
            cur_playlist: None,
            volume: 0.5,
            sink,
            _stream,
            _dbus_tx: dbus_tx,
            _playungap_tx: play_tx,
        }));

        let self_clone = res.clone();
        std::thread::spawn(move || {
            while let Ok(e) = play_rx.recv() {
                match e {
                    AudioEvent::TrackEnd => self_clone.lock().unwrap().next_auto(),
                }
            }
        });

        res.clone()
    }
    pub fn set_playlist(&mut self, playlist: Playlist) {
        self.cur_playlist = Some(playlist);
    }
    pub fn play(&mut self) {
        // WARN:
        if let Some(playlist) = self.cur_playlist.as_mut() {
            playlist.play(&self.sink, self.volume);
            let tx = self._playungap_tx.clone();
            self.sink
                .append(rodio::source::EmptyCallback::new(Box::new(move || {
                    tx.send(AudioEvent::TrackEnd);
                })));
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
    pub fn next_auto(&mut self) {
        if let Some(playlist) = self.cur_playlist.as_mut() {
            playlist.next();
            self.play();
        }
    }
    pub fn next(&mut self) {
        if let Some(playlist) = self.cur_playlist.as_mut() {
            self.sink.stop();
            playlist.next();
            self.play();
        }
    }
    pub fn prev(&mut self) {
        if let Some(playlist) = self.cur_playlist.as_mut() {
            self.sink.stop();
            playlist.prev();
            self.play();
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
