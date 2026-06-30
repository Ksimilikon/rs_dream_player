use std::sync::{Arc, mpsc::channel};

use ::dbus::{DBusData, DBusEvent};
use audio_structs::playlist::Playlist;

use crate::orchestrator::manager::PlaylistManagerEvent;

pub mod dbus;
pub mod engine;
pub mod errors;
pub mod manager;

pub struct Orchestrator {}
impl Orchestrator {
    pub fn run(playlist_default: Playlist) {
        let (tx_manager, rx_manager) = channel::<PlaylistManagerEvent>();
        let (tx_engine, rx_engine) = channel::<engine::EngineEvent>();

        let (tx_cmd, rx_cmd) = channel::<DBusEvent>();
        let (tx_data, rx_data) = channel::<DBusData>();

        let arc_tx_manager = Arc::new(tx_manager);
        let arc_tx_engine = Arc::new(tx_engine);

        dbus::spawn(
            tx_cmd,
            rx_cmd,
            rx_data,
            arc_tx_manager.clone(),
            arc_tx_engine.clone(),
        );
        manager::spawn(rx_manager, arc_tx_engine, tx_data);
        engine::spawn(rx_engine, arc_tx_manager.clone());

        // костыль: сразу ставим пул на проигрывание — обработчик Playlist
        // загрузит первый трек и стартует воспроизведение.
        let _ = arc_tx_manager.send(PlaylistManagerEvent::Playlist(playlist_default));
    }
}
