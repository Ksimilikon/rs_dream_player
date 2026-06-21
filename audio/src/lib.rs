use std::{
    sync::mpsc::{Receiver, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

pub mod audio_engine;

pub use audio_engine::AudioEngine;

/// commands accepted by the audio thread (see [`spawn`]).
#[derive(Debug)]
pub enum PlayerCommand {
    /// stop whatever is playing and queue this track's raw bytes.
    Load(Vec<u8>, f32),
    Play,
    Pause,
    /// toggles between play and pause.
    PlayStop,
    Stop,
    Seek(Duration),
    SetSpeed(f32),
    SetVolume(f32),
    Shutdown,
}

/// events emitted by the audio thread (see [`spawn`]).
#[derive(Debug)]
pub enum PlayerEvent {
    /// the loaded track finished playing on its own.
    TrackEnded,
    /// periodic playback position update.
    Position(Duration),
    Error(String),
}

const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// runs the [`AudioEngine`] on its own thread: applies [`PlayerCommand`]s
/// from `cmd_rx` and reports [`PlayerEvent`]s (position ticks, track-ended)
/// on `event_tx` until [`PlayerCommand::Shutdown`] or the channel closes.
pub fn spawn(cmd_rx: Receiver<PlayerCommand>, event_tx: Sender<PlayerEvent>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut engine = match AudioEngine::new() {
            Ok(engine) => engine,
            Err(e) => {
                let _ = event_tx.send(PlayerEvent::Error(e.to_string()));
                return;
            }
        };

        let mut playing = false;

        loop {
            match cmd_rx.recv_timeout(POLL_INTERVAL) {
                Ok(cmd) => match cmd {
                    PlayerCommand::Load(data, volume) => match engine.load(data, volume) {
                        Ok(()) => playing = true,
                        Err(e) => {
                            playing = false;
                            let _ = event_tx.send(PlayerEvent::Error(e.to_string()));
                        }
                    },
                    PlayerCommand::Play => engine.play(),
                    PlayerCommand::Pause => engine.pause(),
                    PlayerCommand::PlayStop => engine.play_pause(),
                    PlayerCommand::Stop => {
                        engine.stop();
                        playing = false;
                    }
                    PlayerCommand::Seek(pos) => {
                        if let Err(e) = engine.seek(pos) {
                            let _ = event_tx.send(PlayerEvent::Error(e.to_string()));
                        }
                    }
                    PlayerCommand::SetSpeed(speed) => engine.set_speed(speed),
                    PlayerCommand::SetVolume(volume) => engine.set_volume(volume),
                    PlayerCommand::Shutdown => break,
                },
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }

            if playing {
                if engine.is_empty() {
                    playing = false;
                    let _ = event_tx.send(PlayerEvent::TrackEnded);
                } else {
                    let _ = event_tx.send(PlayerEvent::Position(engine.get_pos()));
                }
            }
        }
    })
}
