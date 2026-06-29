use std::sync::{Arc, mpsc::channel};

use crate::orchestrator::manager::PlaylistManagerEvent;

pub mod engine;
pub mod errors;
pub mod manager;

pub struct Orchestrator {}
impl Orchestrator {
    pub fn run() {
        let (tx_manager, rx_manager) = channel::<PlaylistManagerEvent>();
        let (tx_engine, rx_engine) = channel::<engine::EngineEvent>();
        let arc_tx_manager = Arc::new(tx_manager);
        let arc_tx_engine = Arc::new(tx_engine);

        manager::spawn(rx_manager, arc_tx_engine);
        engine::spawn(rx_engine, arc_tx_manager);
    }
}
