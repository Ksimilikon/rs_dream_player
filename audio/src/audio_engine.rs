use std::{error::Error, io::Cursor, time::Duration};

use rodio::{
    Decoder, Player as RodioPlayer,
    stream::{DeviceSinkBuilder, MixerDeviceSink},
};

/// thin wrapper around a rodio output device + player, exposing the
/// playback controls described in the architecture doc: play/pause,
/// playstop, seek, speed and volume.
pub struct AudioEngine {
    _device: MixerDeviceSink,
    player: RodioPlayer,
}

impl AudioEngine {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let device = DeviceSinkBuilder::open_default_sink()?;
        let player = RodioPlayer::connect_new(device.mixer());
        Ok(Self {
            _device: device,
            player,
        })
    }

    /// stops whatever is currently playing and queues `data` for playback.
    /// arg: volume - volume from music
    pub fn load(&mut self, data: Vec<u8>, volume: f32) -> Result<(), Box<dyn Error>> {
        let decoder = Decoder::new(Cursor::new(data))?;

        self.player.stop();
        self.player.set_volume(volume);
        self.player.append(decoder);
        self.player.play();

        Ok(())
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

    pub fn set_volume(&mut self, volume: f32) {
        self.player.set_volume(volume);
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
