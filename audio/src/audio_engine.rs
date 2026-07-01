use std::{error::Error, io::Cursor, time::Duration};

use rodio::{
    Decoder, Player as RodioPlayer,
    stream::{DeviceSinkBuilder, MixerDeviceSink},
};

use crate::source_callback::SourceCallback;

/// thin wrapper around a rodio output device + player, exposing the
/// playback controls described in the architecture doc: play/pause,
/// playstop, seek, speed and volume.
pub struct AudioEngine {
    _device: MixerDeviceSink,
    player: RodioPlayer,
    /// общая громкость (множитель ко всем трекам), 0.0..
    master: f32,
    /// громкость текущего трека (без учёта мастера).
    current_volume: f32,
}

impl AudioEngine {
    /// stops whatever is currently playing and queues `data` for playback.
    /// arg: volume - volume from music
    pub fn load<F>(
        &mut self,
        data: Vec<u8>,
        volume: f32,
        f: Option<F>,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnOnce() + Send + 'static,
    {
        let decoder = Decoder::new(Cursor::new(data))?;

        self.player.stop();
        self.current_volume = volume;
        self.player.set_volume(volume * self.master);
        match f {
            Some(cb) => self
                .player
                .append(SourceCallback::new(Box::new(decoder), cb)),
            None => self.player.append(decoder),
        }
        self.player.play();

        Ok(())
    }
}
impl AudioEngine {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let device = DeviceSinkBuilder::open_default_sink()?;
        let player = RodioPlayer::connect_new(device.mixer());
        Ok(Self {
            _device: device,
            player,
            master: 1.0,
            current_volume: 1.0,
        })
    }

    pub fn play(&mut self) {
        self.player.play();
    }

    pub fn pause(&mut self) {
        self.player.pause();
    }

    /// toggles between play and pause.
    pub fn play_pause(&mut self) {
        if self.player.is_paused() {
            self.player.play();
        } else {
            self.player.pause();
        }
    }

    /// stops playback and empties the queue.
    pub fn stop(&mut self) {
        self.player.stop();
    }

    pub fn seek(&mut self, pos: Duration) -> Result<(), Box<dyn Error>> {
        self.player.try_seek(pos)?;
        Ok(())
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.player.set_speed(speed);
    }

    /// volume for current track
    /// `volume * master`.
    pub fn set_volume(&mut self, volume: f32) {
        self.current_volume = volume;
        self.player.set_volume(volume * self.master);
    }

    /// general volume
    pub fn set_master(&mut self, master: f32) {
        self.master = master;
        self.player.set_volume(self.current_volume * self.master);
    }

    pub fn get_pos(&self) -> Duration {
        self.player.get_pos()
    }

    /// `true` once the loaded track has finished playing (nothing left in queue).
    pub fn is_empty(&self) -> bool {
        self.player.empty()
    }

    pub fn is_pause(&self) -> bool {
        self.player.is_paused()
    }
}
