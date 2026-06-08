use std::{sync::mpsc::Receiver, thread};

pub struct Player {
    is_playing: bool,
    cmd_rx: Receiver<PlayerCMD>,
    cmd_worker: thread::JoinHandle<()>,
}
enum PlayerCMD {
    PlayStop,
    Prev,
    Next,
    /// seconds
    Seek(i32),
    /// number track in playlist
    /// switch to selected track
    Switch(usize),
}
impl Player {
    pub fn new(cmd_rx: Receiver<PlayerCMD>) -> Self {
        let cmd_worker = thread::spawn(move || {});

        Self {
            is_playing: false,
            cmd_rx,
            cmd_worker,
        }
    }
    pub fn next() {}
    pub fn prev() {}
    pub fn play_stop() {}
    pub fn seek(seconds: i32) {}
}
