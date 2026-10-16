//! demo frontend of the core: prepares directories, sets the startup volume
//! and hands control to the TUI, which works with the core only through its
//! FFI (the `api` crate).

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Parser;

mod config;

#[derive(clap::Parser, Debug)]
#[command(version, about = "TUI demo frontend for the music player core")]
struct Args {
    /// music directory to index (default — the system `~/Music`).
    #[arg(short, long, value_name = "Dir")]
    path: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();

    // app directory (config / db / cache — in one place for the demo).
    let app_dir = config::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&app_dir);

    // music directory: an explicit --path is taken as is; otherwise the system
    // default, offering to create it if it is missing.
    let music = match args.path {
        Some(path) => path,
        None => resolve_default_music_dir(),
    };

    // show the main paths before starting the interface.
    print_dirs(&app_dir, &music);

    // the master volume is now stored and applied by the core itself (its
    // config.toml).
    let dirs = tui::Dirs {
        cache: &app_dir,
        config: &app_dir,
        data: &app_dir,
        music: &music,
    };
    if let Err(e) = tui::run(dirs) {
        eprintln!("tui: {e}");
    }
}

/// prints the main app paths.
fn print_dirs(app_dir: &Path, music: &Path) {
    println!("config/data/cache dir: {}", app_dir.display());
    println!(
        "database file:         {}",
        app_dir.join(storage::DB_FILE_NAME).display()
    );
    println!("music dir:             {}", music.display());
}

/// the default system music directory. If it doesn't exist on disk, explains
/// the options and offers to create it. The path is returned in any case: if
/// declined, it simply doesn't exist and indexing skips it.
fn resolve_default_music_dir() -> PathBuf {
    let Some(dir) = config::music_dir() else {
        return PathBuf::from(".");
    };
    if dir.is_dir() {
        return dir;
    }

    println!("default music directory not found: {}", dir.display());
    println!("options:");
    println!("  - create this directory;");
    #[cfg(target_os = "linux")]
    println!("  - set XDG_MUSIC_DIR (~/.config/user-dirs.dirs);");
    println!("  - pass the directory yourself via --path <Dir>.");

    if prompt_yes_no(&format!(
        "create the default music directory ({})? [y/N]:",
        dir.display()
    )) {
        match std::fs::create_dir_all(&dir) {
            Ok(()) => println!("created: {}", dir.display()),
            Err(e) => println!("failed to create directory: {e}"),
        }
    }
    dir
}

/// asks a question and reads the answer from stdin. The default (empty input
/// / error) is "no"; "yes" only on `y`/`Y`.
fn prompt_yes_no(question: &str) -> bool {
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim(), "y" | "Y")
}
