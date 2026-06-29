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
    Add(Vec<u8>),
    PlayPause,
    Seek(Duration),
}

pub fn spawn(rx: Receiver<EngineEvent>, tx_manager: Arc<Sender<PlaylistManagerEvent>>) {
    let worker_engine = std::thread::spawn(move || {
        handler_engine(tx_manager, rx);
    });
}

fn handler_engine(tx_manager: Arc<Sender<PlaylistManagerEvent>>, rx: Receiver<EngineEvent>) {
    let res_engine = AudioEngine::new();
    let mut engine: AudioEngine;
    match res_engine {
        Ok(r) => engine = r,
        Err(err) => panic!("engine cant started::ERROR::{}", err),
    }

    while let Ok(e) = rx.recv() {
        match e {
            EngineEvent::PlayPause => engine.play_pause(),
            EngineEvent::Add(b) => {
                let tx_clone = tx_manager.clone();
                let res = engine.load(
                    b,
                    1.,
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
        }
    }
}
