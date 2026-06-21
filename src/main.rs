use std::{path::PathBuf, sync::mpsc, thread, time::Duration};

use api::bridge::{ApiCommand, NowPlaying};
use audio_structs::playlist::Playlist;
use clap::Parser;

use crate::{
    orchestrator::{CoreCommand, CoreEvent, Orchestrator},
    storage::Storage,
};

mod orchestrator;
mod playlist_manager;
mod storage;
mod traits;

#[derive(clap::Parser, Debug)]
#[command(version, about = "cli for music player core")]
struct Args {
    #[arg(short, long, value_name = "Dir")]
    path: Option<PathBuf>,
    #[arg(short, long)]
    without_mods: bool,
}
fn main() {
    let args = Args::parse();

    // --without_mods
    if !args.without_mods {
        let mut mod_manager = api::ModManager::new();
        let _ = mod_manager.load_mods("mods");
        println!("{:#?}", mod_manager);
    }

    let storage = match Storage::new() {
        Ok(storage) => storage,
        Err(e) => {
            eprintln!("failed to open storage: {e}");
            return;
        }
    };
    // snapshot playlist names for FFI queries before the core takes ownership.
    let playlists = storage.list_playlists().unwrap_or_default();

    let orchestrator = Orchestrator::new(storage);

    // wire the FFI bridge: mods push ApiCommands, a forwarder maps them onto
    // CoreCommands and drives the live core.
    let (api_tx, api_rx) = mpsc::channel::<ApiCommand>();
    if let Err(e) = api::bridge::init(api_tx) {
        eprintln!("failed to init api bridge: {e}");
    }
    api::bridge::set_playlists(playlists);

    let core_sender = orchestrator.sender();
    thread::spawn(move || {
        for cmd in api_rx {
            let core_cmd = match cmd {
                ApiCommand::Next => CoreCommand::Next,
                ApiCommand::Prev => CoreCommand::Prev,
                ApiCommand::PlayStop => CoreCommand::PlayStop,
                ApiCommand::SelectTrack(i) => CoreCommand::SelectTrack(i as usize),
                ApiCommand::Seek(s) => CoreCommand::Seek(Duration::from_secs_f32(s)),
                ApiCommand::SetVolume(v) => CoreCommand::SetVolume(v),
                ApiCommand::SetTrackVolume(v) => CoreCommand::SetTrackVolume(v),
            };
            core_sender.send(core_cmd);
        }
    });

    // --path: load an anonymous playlist from a directory and start playing it
    if let Some(path) = &args.path {
        match Playlist::from_dir(path) {
            Ok(playlist) => orchestrator.send(CoreCommand::SetPlaylist(playlist)),
            Err(e) => eprintln!("failed to load playlist from {}: {e}", path.display()),
        }
    }

    println!(
        "commands: next | prev | playstop | seek <sec> | volume <0.0-1.0> | load <dir> | \
         index <dir> | find <name> | playlists | q"
    );

    let mut input = String::new();
    loop {
        while let Ok(event) = orchestrator.try_recv_event() {
            print_event(event);
        }

        input.clear();
        if std::io::stdin().read_line(&mut input).unwrap() == 0 {
            break;
        }
        let mut parts = input.trim().split_whitespace();
        match parts.next() {
            Some("next") => orchestrator.send(CoreCommand::Next),
            Some("prev") => orchestrator.send(CoreCommand::Prev),
            Some("playstop") => orchestrator.send(CoreCommand::PlayStop),
            Some("seek") => {
                if let Some(secs) = parts.next().and_then(|s| s.parse::<f32>().ok()) {
                    orchestrator.send(CoreCommand::Seek(Duration::from_secs_f32(secs)));
                }
            }
            Some("volume") => {
                if let Some(volume) = parts.next().and_then(|s| s.parse::<f32>().ok()) {
                    orchestrator.send(CoreCommand::SetVolume(volume));
                }
            }
            Some("load") => match parts.next().map(Playlist::from_dir) {
                Some(Ok(playlist)) => orchestrator.send(CoreCommand::SetPlaylist(playlist)),
                Some(Err(e)) => eprintln!("failed to load playlist: {e}"),
                None => eprintln!("usage: load <dir>"),
            },
            Some("index") => match parts.next() {
                Some(dir) => orchestrator.send(CoreCommand::IndexDir(PathBuf::from(dir))),
                None => eprintln!("usage: index <dir>"),
            },
            Some("find") => {
                let query = parts.collect::<Vec<_>>().join(" ");
                if query.is_empty() {
                    eprintln!("usage: find <name>");
                } else {
                    orchestrator.send(CoreCommand::FindTrack(query));
                }
            }
            Some("playlists") => orchestrator.send(CoreCommand::ListPlaylists),
            Some("q") | Some("quit") => break,
            Some(other) => eprintln!("unknown command: {other}"),
            None => {}
        }
    }

    orchestrator.shutdown();
}

fn print_event(event: CoreEvent) {
    match event {
        CoreEvent::TrackChanged {
            title,
            artist,
            duration_sec,
        } => {
            println!(
                "now playing: {title} - {} ({duration_sec}s)",
                artist.join(", ")
            );
            api::bridge::set_now_playing(NowPlaying {
                title,
                artist: artist.join(", "),
                duration_sec,
            });
        }
        CoreEvent::Position(pos) => println!("position: {:.1}s", pos.as_secs_f32()),
        CoreEvent::Error(e) => eprintln!("error: {e}"),
        CoreEvent::Info(msg) => println!("{msg}"),
        CoreEvent::Stopped => println!("stopped"),
    }
}
