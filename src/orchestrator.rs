use std::{
    error::Error,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use audio::{PlayerCommand, PlayerEvent};
use audio_structs::{playlist::Playlist, types::Volume};

use crate::{playlist_manager::PlaylistManager, storage::Storage};

/// commands the outside world can send to the [`Orchestrator`]'s core thread.
#[derive(Debug)]
pub enum CoreCommand {
    /// toggles between play and pause.
    PlayStop,
    Next,
    Prev,
    SelectTrack(usize),
    Seek(Duration),
    /// master volume for the whole app.
    SetVolume(Volume),
    /// volume of the currently selected track only.
    SetTrackVolume(Volume),
    SetSpeed(f32),
    /// replaces the active playlist and starts playing its first track.
    SetPlaylist(Playlist),
    /// scans a directory and persists it into the index as a named playlist
    /// (named after the directory).
    IndexDir(PathBuf),
    /// searches the index for tracks whose title matches the substring.
    FindTrack(String),
    /// reports the names of all stored playlists.
    ListPlaylists,
    Shutdown,
}

/// events the core thread reports back to the outside world.
#[derive(Debug)]
pub enum CoreEvent {
    TrackChanged {
        title: String,
        artist: Vec<String>,
        duration_sec: u64,
    },
    Position(Duration),
    Error(String),
    /// textual feedback for storage / index operations.
    Info(String),
    /// the playlist is empty, nothing to play.
    Stopped,
}

/// messages handled by the core thread's single select loop: either a
/// [`CoreCommand`] from the outside, or a [`PlayerEvent`] forwarded from the
/// audio thread.
enum Internal {
    Cmd(CoreCommand),
    Audio(PlayerEvent),
}

/// wires together the audio module, [`PlaylistManager`] and [`Storage`],
/// each running on its own thread and communicating over `mpsc` channels.
pub struct Orchestrator {
    internal_tx: Sender<Internal>,
    event_rx: Receiver<CoreEvent>,
    handles: Vec<JoinHandle<()>>,
}

impl Orchestrator {
    /// spawns the audio thread, a forwarder thread (merges audio events into
    /// the core's internal channel) and the core thread (owns
    /// [`PlaylistManager`] + `storage`).
    pub fn new(storage: Storage) -> Self {
        let (audio_cmd_tx, audio_cmd_rx) = mpsc::channel::<PlayerCommand>();
        let (audio_event_tx, audio_event_rx) = mpsc::channel::<PlayerEvent>();
        let audio_handle = audio::spawn(audio_cmd_rx, audio_event_tx);

        let (internal_tx, internal_rx) = mpsc::channel::<Internal>();

        let forwarder_tx = internal_tx.clone();
        let forwarder_handle = thread::spawn(move || {
            while let Ok(event) = audio_event_rx.recv() {
                if forwarder_tx.send(Internal::Audio(event)).is_err() {
                    break;
                }
            }
        });

        let (event_tx, event_rx) = mpsc::channel::<CoreEvent>();
        let core_handle = thread::spawn(move || {
            core_loop(internal_rx, audio_cmd_tx, event_tx, storage);
        });

        Self {
            internal_tx,
            event_rx,
            handles: vec![audio_handle, forwarder_handle, core_handle],
        }
    }

    pub fn send(&self, cmd: CoreCommand) {
        let _ = self.internal_tx.send(Internal::Cmd(cmd));
    }

    /// a cheap, cloneable handle for sending commands from other threads (e.g.
    /// the FFI forwarder). Stays valid as long as the core thread is running.
    pub fn sender(&self) -> CoreSender {
        CoreSender(self.internal_tx.clone())
    }

    pub fn try_recv_event(&self) -> Result<CoreEvent, mpsc::TryRecvError> {
        self.event_rx.try_recv()
    }

    /// asks the core thread to shut down and waits for every thread to exit.
    pub fn shutdown(mut self) {
        let _ = self.internal_tx.send(Internal::Cmd(CoreCommand::Shutdown));
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

/// a cloneable command sender that other threads can use to drive the core
/// without owning the [`Orchestrator`].
#[derive(Clone)]
pub struct CoreSender(Sender<Internal>);

impl CoreSender {
    pub fn send(&self, cmd: CoreCommand) {
        let _ = self.0.send(Internal::Cmd(cmd));
    }
}

fn core_loop(
    internal_rx: Receiver<Internal>,
    audio_cmd_tx: Sender<PlayerCommand>,
    event_tx: Sender<CoreEvent>,
    mut storage: Storage,
) {
    let mut manager = PlaylistManager::new();

    for internal in internal_rx {
        match internal {
            Internal::Cmd(CoreCommand::PlayStop) => {
                let _ = audio_cmd_tx.send(PlayerCommand::PlayStop);
            }
            Internal::Cmd(CoreCommand::Next) => match manager.next() {
                Ok(_) => load_current_track(&manager, &audio_cmd_tx, &event_tx),
                Err(e) => report_error(&event_tx, &*e),
            },
            Internal::Cmd(CoreCommand::Prev) => match manager.prev() {
                Ok(_) => load_current_track(&manager, &audio_cmd_tx, &event_tx),
                Err(e) => report_error(&event_tx, &*e),
            },
            Internal::Cmd(CoreCommand::SelectTrack(number)) => {
                match manager.select_track(number) {
                    Ok(_) => load_current_track(&manager, &audio_cmd_tx, &event_tx),
                    Err(e) => report_error(&event_tx, &*e),
                }
            }
            Internal::Cmd(CoreCommand::Seek(pos)) => {
                let _ = audio_cmd_tx.send(PlayerCommand::Seek(pos));
            }
            Internal::Cmd(CoreCommand::SetVolume(volume)) => {
                storage.config_mut().volume = volume;
                // effective gain = master * current track volume.
                let effective = volume * manager.current_volume();
                let _ = audio_cmd_tx.send(PlayerCommand::SetVolume(effective));
            }
            Internal::Cmd(CoreCommand::SetTrackVolume(volume)) => {
                manager.set_current_volume(volume);
                let master = storage.config_mut().volume;
                let _ = audio_cmd_tx.send(PlayerCommand::SetVolume(master * volume));
            }
            Internal::Cmd(CoreCommand::SetSpeed(speed)) => {
                let _ = audio_cmd_tx.send(PlayerCommand::SetSpeed(speed));
            }
            Internal::Cmd(CoreCommand::SetPlaylist(playlist)) => {
                match manager.set_playlist(playlist) {
                    Ok(()) => load_current_track(&manager, &audio_cmd_tx, &event_tx),
                    Err(e) => report_error(&event_tx, &*e),
                }
            }
            Internal::Cmd(CoreCommand::IndexDir(dir)) => match Playlist::from_dir(&dir) {
                Ok(mut playlist) => {
                    let name = dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "playlist".into());
                    playlist.set_name(name.clone());
                    match storage.save_playlist(&playlist) {
                        Ok(_) => {
                            let _ = event_tx.send(CoreEvent::Info(format!(
                                "indexed {} track(s) into playlist '{name}'",
                                playlist.get_count()
                            )));
                            if let Ok(names) = storage.list_playlists() {
                                api::bridge::set_playlists(names);
                            }
                        }
                        Err(e) => report_error(&event_tx, &*e),
                    }
                }
                Err(e) => report_error(&event_tx, &*e),
            },
            Internal::Cmd(CoreCommand::FindTrack(query)) => {
                match storage.find_track(Some(query), None, None, None) {
                    Ok(tracks) => {
                        let mut msg = format!("found {} track(s):", tracks.len());
                        for track in &tracks {
                            if let Ok(meta) = track.get_metadata() {
                                msg.push_str(&format!(
                                    "\n  {} - {}",
                                    meta.title,
                                    meta.artist.join(", ")
                                ));
                            }
                        }
                        let _ = event_tx.send(CoreEvent::Info(msg));
                    }
                    Err(e) => report_error(&event_tx, &*e),
                }
            }
            Internal::Cmd(CoreCommand::ListPlaylists) => match storage.list_playlists() {
                Ok(names) => {
                    api::bridge::set_playlists(names.clone());
                    let _ = event_tx.send(CoreEvent::Info(format!("playlists: {}", names.join(", "))));
                }
                Err(e) => report_error(&event_tx, &*e),
            },
            Internal::Cmd(CoreCommand::Shutdown) => {
                let _ = audio_cmd_tx.send(PlayerCommand::Shutdown);
                if let Err(e) = storage.save_config() {
                    report_error(&event_tx, &*e);
                }
                break;
            }
            Internal::Audio(PlayerEvent::TrackEnded) => match manager.next() {
                Ok(_) => load_current_track(&manager, &audio_cmd_tx, &event_tx),
                Err(e) => report_error(&event_tx, &*e),
            },
            Internal::Audio(PlayerEvent::Position(pos)) => {
                let _ = event_tx.send(CoreEvent::Position(pos));
            }
            Internal::Audio(PlayerEvent::Error(msg)) => {
                let _ = event_tx.send(CoreEvent::Error(msg));
            }
        }
    }
}

fn report_error(event_tx: &Sender<CoreEvent>, err: &dyn Error) {
    let _ = event_tx.send(CoreEvent::Error(err.to_string()));
}

/// loads the playlist manager's current track into the audio thread and
/// announces it, or reports [`CoreEvent::Stopped`] if there's nothing to play.
fn load_current_track(
    manager: &PlaylistManager,
    audio_cmd_tx: &Sender<PlayerCommand>,
    event_tx: &Sender<CoreEvent>,
) {
    let Some(track) = manager.get_track() else {
        let _ = event_tx.send(CoreEvent::Stopped);
        return;
    };

    let raw = match track.get_track() {
        Ok(t) => t.get_copy_data(),
        Err(e) => return report_error(event_tx, &e),
    };
    let metadata = match track.get_metadata() {
        Ok(m) => m,
        Err(e) => return report_error(event_tx, &e),
    };

    let _ = audio_cmd_tx.send(PlayerCommand::Load(raw, track.volume));

    let duration_sec = metadata.params.as_ref().map(|p| p.duration_sec).unwrap_or(0);
    let _ = event_tx.send(CoreEvent::TrackChanged {
        title: metadata.title.clone(),
        artist: metadata.artist.clone(),
        duration_sec,
    });
}
