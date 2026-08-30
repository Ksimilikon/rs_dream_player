use std::{
    sync::{
        Arc,
        mpsc::{Receiver, RecvTimeoutError, Sender},
    },
    time::Duration,
};

use audio::AudioEngine;
use tui::Update;

use crate::orchestrator::manager::PlaylistManagerEvent;

/// как часто движок публикует текущую позицию воспроизведения в UI.
const POSITION_TICK: Duration = Duration::from_millis(250);

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
    tx_ui: Sender<Update>,
    master: f32,
) {
    std::thread::spawn(move || {
        handler_engine(tx_manager, tx_ui, master, rx);
    });
}

fn handler_engine(
    tx_manager: Arc<Sender<PlaylistManagerEvent>>,
    tx_ui: Sender<Update>,
    master: f32,
    rx: Receiver<EngineEvent>,
) {
    let res_engine = AudioEngine::new();
    let mut engine: AudioEngine;
    match res_engine {
        Ok(r) => engine = r,
        Err(err) => panic!("engine cant started::ERROR::{}", err),
    }
    engine.set_volume_master(master);

    // ждём команду не дольше POSITION_TICK: по таймауту публикуем позицию, чтобы
    // прогресс-бар в UI двигался во время проигрывания.
    loop {
        match rx.recv_timeout(POSITION_TICK) {
            Ok(e) => match e {
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
                EngineEvent::Seek(time) => match engine.seek(time) {
                    // сразу отражаем новую позицию, не дожидаясь тика.
                    Ok(()) => {
                        let _ = tx_ui.send(Update::Position(engine.get_pos().as_secs()));
                    }
                    // если формат/источник не поддерживает перемотку — не молчим.
                    Err(e) => {
                        let _ = tx_ui.send(Update::Error(format!("seek failed: {e}")));
                    }
                },
                EngineEvent::SetVolume(v) => engine.set_volume_track(v),
                EngineEvent::SetMaster(v) => engine.set_volume_master(v),
            },
            Err(RecvTimeoutError::Timeout) => {
                // публикуем позицию только когда есть что играть.
                if !engine.is_empty() {
                    let _ = tx_ui.send(Update::Position(engine.get_pos().as_secs()));
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}
