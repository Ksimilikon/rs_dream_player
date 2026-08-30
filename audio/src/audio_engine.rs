use std::{error::Error, io::Cursor, time::Duration};

use rodio::{
    Decoder, Player as RodioPlayer,
    source::SeekError,
    stream::{DeviceSinkBuilder, MixerDeviceSink},
};

use crate::source_callback::SourceCallback;

/// thin wrapper around a rodio output device + player, exposing the
/// playback controls described in the architecture doc: play/pause,
/// playstop, seek, speed and volume.
pub struct AudioEngine {
    _device: MixerDeviceSink,
    player: RodioPlayer,
    master_volume: f32,
    track_volume: f32,
}

impl AudioEngine {
    /// stop and clear any in player, and load new data
    pub fn load<F>(
        &mut self,
        data: Vec<u8>,
        volume_track: f32,
        f: Option<F>,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnOnce() + Send + 'static,
    {
        let decoder = Decoder::new(Cursor::new(data))?;

        self.player.stop();
        self.track_volume = volume_track;
        self.player.set_volume(volume_track * self.master_volume);
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
            master_volume: 1.0,
            track_volume: 1.0,
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

    pub fn seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.player.try_seek(pos)?;
        Ok(())
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.player.set_speed(speed);
    }

    /// volume for current track
    /// `volume * master`.
    pub fn set_volume_track(&mut self, volume_track: f32) {
        self.track_volume = volume_track;
        self.player.set_volume(volume_track * self.master_volume);
    }

    /// general volume
    pub fn set_volume_master(&mut self, volume: f32) {
        self.master_volume = volume;
        self.player
            .set_volume(self.track_volume * self.master_volume);
    }

    /// return position seek
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

    pub fn get_volume_master(&self) -> f32 {
        self.master_volume
    }

    pub fn get_volume_track(&self) -> f32 {
        self.track_volume
    }
}
