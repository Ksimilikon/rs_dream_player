use std::path::PathBuf;

use clap::Parser;

// use crate::audio::dbus::Dbus;

mod orchestrator;
mod playlist_manager;
mod storage;
mod traits;

pub mod music;
pub mod types;

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

    // --path
    if let Some(path) = args.path {}

    // --without_mods
    if !args.without_mods {
        let mut mod_manager = api::ModManager::new();
        let _ = mod_manager.load_mods("mods");
        println!("{:#?}", mod_manager);
    }
    // NOTE: only for test
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
}
