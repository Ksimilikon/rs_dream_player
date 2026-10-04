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
        let decoder = Decoder::new(Cursor::new(data))?;
        // запоминаем длительность до перемещения декодера в очередь.
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
        // best-effort: имя устройства по умолчанию (open_default_sink может
        // уйти на резервное устройство, тогда имя будет приблизительным).
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

    /// имена доступных устройств вывода (на хосте по умолчанию).
    pub fn output_devices() -> Vec<String> {
        match rodio::cpal::default_host().output_devices() {
            Ok(devices) => devices
                .filter_map(|d| d.description().ok().map(|desc| desc.name().to_string()))
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// имя активного устройства вывода, если известно.
    pub fn get_output_device(&self) -> Option<&str> {
        self.device_name.as_deref()
    }

    /// переключает вывод на устройство с именем `name` (из [`Self::output_devices`]).
    /// Текущий загруженный трек сбрасывается — воспроизведение нужно запустить
    /// заново; громкость (master/track) сохраняется.
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
        // трек сброшен вместе со старым плеером.
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
