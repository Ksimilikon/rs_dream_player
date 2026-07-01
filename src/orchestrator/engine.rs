use std::{
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    time::Duration,
};

use audio::AudioEngine;

use crate::orchestrator::{engine, manager::PlaylistManagerEvent};

pub enum EngineEvent {
    /// сырые байты трека + его персональная громкость.
    Add(Vec<u8>, f32),
    PlayPause,
    Seek(Duration),
    /// громкость текущего трека (0.0..=1.0).
    SetVolume(f32),
    /// общая (мастер-) громкость (0.0..=1.0).
    SetMaster(f32),
}

pub fn spawn(
    rx: Receiver<EngineEvent>,
    tx_manager: Arc<Sender<PlaylistManagerEvent>>,
    master: f32,
) {
    let worker_engine = std::thread::spawn(move || {
        handler_engine(tx_manager, master, rx);
    });
}

fn handler_engine(
    tx_manager: Arc<Sender<PlaylistManagerEvent>>,
    master: f32,
    rx: Receiver<EngineEvent>,
) {
    let res_engine = AudioEngine::new();
    let mut engine: AudioEngine;
    match res_engine {
        Ok(r) => engine = r,
        Err(err) => panic!("engine cant started::ERROR::{}", err),
    }
    engine.set_master(master);

    while let Ok(e) = rx.recv() {
        match e {
            EngineEvent::PlayPause => engine.play_pause(),
            EngineEvent::Add(b, volume) => {
                let tx_clone = tx_manager.clone();
                let res = engine.load(
                    b,
                    volume,
                    Some(move || {
                        let _ = tx_clone.send(PlaylistManagerEvent::Next);
                    }),
                );
                if let Err(err) = res {
                    println!(
                        "ERROR::orchestrator::engine::handler_engine::load bytes::{}",
                        err
                    );
                }
            }
            EngineEvent::Seek(time) => {
                let _ = engine.seek(time);
            }
            EngineEvent::SetVolume(v) => engine.set_volume(v),
            EngineEvent::SetMaster(v) => engine.set_master(v),
        }
    }
}
