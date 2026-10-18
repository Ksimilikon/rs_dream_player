use std::{error::Error, io::Cursor, time::Duration};

use rodio::{
    Decoder, Player as RodioPlayer, Source,
    cpal::traits::{DeviceTrait, HostTrait},
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
    /// name of the active output device (from the default host), if known.
    device_name: Option<String>,
    /// total duration of the currently loaded track (`0` if unknown / nothing
    /// loaded). Some formats don't report it, so treat `0` as "unknown".
    duration: Duration,
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
        // the byte length makes the stream seekable: without it symphonia
        // reads forward only and seeking backwards fails (`ForwardOnly`).
        let len = data.len() as u64;
        let decoder = Decoder::builder()
            .with_data(Cursor::new(data))
            .with_byte_len(len)
            .with_seekable(true)
            .build()?;
        // remember the duration before the decoder moves into the queue.
        self.duration = decoder.total_duration().unwrap_or_default();

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
        // best-effort: the default device name (open_default_sink may fall
        // back to another device, then the name is approximate).
        let device_name = rodio::cpal::default_host()
            .default_output_device()
            .and_then(|d| d.description().ok())
            .map(|desc| desc.name().to_string());
        Ok(Self {
            _device: device,
            player,
            master_volume: 1.0,
            track_volume: 1.0,
            device_name,
            duration: Duration::ZERO,
        })
    }

    /// total duration of the loaded track (`Duration::ZERO` if unknown).
    pub fn get_duration(&self) -> Duration {
        self.duration
    }

    /// names of the available output devices (on the default host).
    pub fn output_devices() -> Vec<String> {
        match rodio::cpal::default_host().output_devices() {
            Ok(devices) => devices
                .filter_map(|d| d.description().ok().map(|desc| desc.name().to_string()))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// name of the active output device, if known.
    pub fn get_output_device(&self) -> Option<&str> {
        self.device_name.as_deref()
    }

    /// switches output to the device named `name` (from
    /// [`Self::output_devices`]). The currently loaded track is dropped —
    /// playback has to be started again; the volume (master/track) is kept.
    pub fn set_output_device(&mut self, name: &str) -> Result<(), Box<dyn Error>> {
        let device = rodio::cpal::default_host()
            .output_devices()?
            .find(|d| {
                d.description()
                    .map(|desc| desc.name() == name)
                    .unwrap_or(false)
            })
            .ok_or_else(|| format!("output device not found: {name}"))?;

        let sink = DeviceSinkBuilder::from_device(device)?.open_stream()?;
        let player = RodioPlayer::connect_new(sink.mixer());
        player.set_volume(self.track_volume * self.master_volume);

        self._device = sink;
        self.player = player;
        self.device_name = Some(name.to_string());
        // the track is dropped together with the old player.
        self.duration = Duration::ZERO;
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
        self.duration = Duration::ZERO;
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
