use crate::NAME;
#[cfg(target_os = "linux")]
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "linux")]
use crate::{
    audio::player::{self, Player},
    traits::player_agent_os::PlayerAgentOS,
};

pub struct DbusInteraface {
    player: Arc<Mutex<Player>>,
}
pub struct Dbus {}

#[cfg(target_os = "linux")]
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl DbusInteraface {
    async fn play(&self) {
        let mut player = self.player.lock().unwrap();
        player.play();
    }

    async fn set_volume(&self, val: f32) {
        let mut player = self.player.lock().unwrap();
        player.set_volume(val);
    }
    async fn next(&self) {
        let mut player = self.player.lock().unwrap();
        player.next();
    }
    async fn prev(&self) {
        let mut player = self.player.lock().unwrap();
        {
            player.prev();
        }
    }
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        "Playing"
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
    #[zbus(property)]
    async fn send_title(&self) -> String {
        "ttttttt".to_string()
    }
}

#[cfg(target_os = "linux")]
impl Dbus {
    pub fn start_server(player: Arc<Mutex<Player>>, rx: mpsc::Receiver<String>) {
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            runtime.block_on(async move {
                use zbus::blocking::connection;

                let conn = connection::Builder::session()
                    .unwrap()
                    .name(NAME)
                    .serve
                while let Ok(title) = rx.recv() {}
                // tokio::task::spawn_blocking(move || while let Ok(title) = rx.recv() {});
                std::future::pending::<()>().await;
            })
        });
    }
}
