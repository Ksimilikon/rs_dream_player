use std::sync::{Arc, Mutex};

use rodio::{OutputStream, Sink};

use crate::song::{audio_event::AudioEvent, metadata::Metadata};

use super::{playlist::Playlist, types::Volume};

pub struct Player {
    playlists: Vec<Playlist>,
    cur_playlist: usize,
    volume: Volume,
    sink: Sink,
    _stream: OutputStream,
    _dbus_tx: Option<tokio::sync::mpsc::Sender<Arc<Metadata>>>,
    _playungap_tx: std::sync::mpsc::Sender<AudioEvent>,
}
impl Player {
    pub fn new(
        dbus_tx: Option<tokio::sync::mpsc::Sender<Arc<Metadata>>>,
        playlists: Vec<Playlist>,
    ) -> Arc<Mutex<Self>> {
        let _stream = rodio::OutputStreamBuilder::open_default_stream().unwrap();
        let sink = Sink::connect_new(_stream.mixer());
        sink.set_volume(1.0);
        let (play_tx, play_rx) = std::sync::mpsc::channel::<AudioEvent>();

        let res = Arc::new(Mutex::new(Self {
            playlists,
            cur_playlist: 0,
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
    pub fn add_playlist(&mut self, playlist: Playlist) {
        self.playlists.push(playlist);
    }
    pub fn set_playlist(&mut self, playlist: usize) {
        if playlist > self.playlists.len() {
            self.cur_playlist = self.playlists.len() - 1;
        } else {
            self.cur_playlist = playlist;
        }
    }
    pub fn play(&mut self) {
        // WARN:
        if self.playlists.len() != 0 {
            let _ = self.playlists[self.cur_playlist].play(&self.sink, self.volume);
            let tx = self._playungap_tx.clone();
            self.sink
                .append(rodio::source::EmptyCallback::new(Box::new(move || {
                    tx.send(AudioEvent::TrackEnd);
                })));
        } else {
            println!("Player::play::not found playlists")
        }
        if let Some(tx) = self._dbus_tx.as_ref() {
            let _ = tx.blocking_send(self.playlists[self.cur_playlist].get_metadata());
        }
    }
    pub fn set_volume(&mut self, volume: Volume) {
        self.volume = volume;
        self.sink.set_volume(volume);
    }
    pub fn next_auto(&mut self) {
        self.playlists[self.cur_playlist].next();
        self.play();
    }
    pub fn next(&mut self) {
        self.sink.stop();
        self.playlists[self.cur_playlist].next();
        self.play();
    }
    pub fn prev(&mut self) {
        self.sink.stop();
        self.playlists[self.cur_playlist].prev();
        self.play();
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
    // WARN: error is ignored
    pub fn select_song(&mut self, id: u32) {
        self.sink.stop();
        let _ = self.playlists[self.cur_playlist].set_song(id);
        self.play();
    }
}
