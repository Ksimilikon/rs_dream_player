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
#[zbus::interface(name = "org.dream_player.music_player")]
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
                use crate::NAME;

                let mpris = mpris_server::Player::builder(NAME)
                    .can_go_next(true)
                    .can_pause(true)
                    .can_go_previous(true)
                    .build()
                    .await
                    .unwrap();

                let metadata = mpris_server::Metadata::builder().title("test data").build();

                mpris.set_metadata(metadata).await.unwrap();
                mpris.run().await;
                std::future::pending::<()>().await;
            })
        });
    }
}
