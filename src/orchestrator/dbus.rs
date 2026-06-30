use std::sync::{
    Arc,
    mpsc::{Receiver, Sender},
};

use dbus::{DBus, DBusData, DBusEvent};

use crate::orchestrator::{engine::EngineEvent, manager::PlaylistManagerEvent};

/// поднимает dbus-слой: поток с MPRIS-соединением и роутер, который
/// переводит команды DE во внутренние каналы плеера.
pub fn spawn(
    tx_cmd: Sender<DBusEvent>,
    rx_cmd: Receiver<DBusEvent>,
    rx_data: Receiver<DBusData>,
    tx_manager: Arc<Sender<PlaylistManagerEvent>>,
    tx_engine: Arc<Sender<EngineEvent>>,
) {
    // поток с MPRIS-соединением — как остальные воркеры
    let dbus = DBus::new(tx_cmd, rx_data);
    std::thread::spawn(move || {
        if let Err(err) = dbus.run() {
            println!("ERROR::orchestrator::dbus::run::{}", err);
        }
    });

    // роутер: команды от DE -> внутренние каналы плеера
    std::thread::spawn(move || {
        handler_router(rx_cmd, tx_manager, tx_engine);
    });
}

fn handler_router(
    rx_cmd: Receiver<DBusEvent>,
    tx_manager: Arc<Sender<PlaylistManagerEvent>>,
    tx_engine: Arc<Sender<EngineEvent>>,
) {
    while let Ok(cmd) = rx_cmd.recv() {
        match cmd {
            DBusEvent::Next => {
                let _ = tx_manager.send(PlaylistManagerEvent::Next);
            }
            DBusEvent::Prev => {
                let _ = tx_manager.send(PlaylistManagerEvent::Prev);
            }
            DBusEvent::PlayPause | DBusEvent::Play | DBusEvent::Pause | DBusEvent::Stop => {
                let _ = tx_engine.send(EngineEvent::PlayPause);
            }
        }
    }
}
