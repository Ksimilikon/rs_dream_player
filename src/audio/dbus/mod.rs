use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use player::player::Player;
use zbus::{
    interface,
    zvariant::{ObjectPath, Value},
};
pub mod android;
pub mod linux;
pub mod windows;

#[derive(Default)]
pub struct DbusData {
    pub title: String,
    pub artist: Vec<String>,
    pub cover_art: Option<Vec<u8>>,
}
pub struct DbusPlayer {
    pub data: Arc<Mutex<DbusData>>,
    player: Arc<Mutex<Player>>,
}
pub struct Dbus {}
