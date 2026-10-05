//! демонстрационный фронтенд ядра: готовит каталоги, выставляет стартовую
//! громкость и передаёт управление TUI, который работает с ядром только через
//! его FFI (крейт `api`).

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Parser;

mod config;

#[derive(clap::Parser, Debug)]
#[command(version, about = "TUI demo frontend for the music player core")]
struct Args {
    /// каталог с музыкой для индексации (по умолчанию — системный `~/Music`).
    #[arg(short, long, value_name = "Dir")]
    path: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();

    // каталог приложения (конфиг/бд/кэш — в одном месте для демо).
    let app_dir = config::config_dir().unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&app_dir);

    // каталог музыки: явный --path берём как есть; иначе — системный по
    // умолчанию, при его отсутствии предлагаем создать.
    let music = match args.path {
        Some(path) => path,
        None => resolve_default_music_dir(),
    };

    // показываем основные пути до запуска интерфейса.
    print_dirs(&app_dir, &music);

    // мастер-громкость теперь хранит и применяет само ядро (его config.toml).
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

/// выводит основные пути приложения.
fn print_dirs(app_dir: &Path, music: &Path) {
    println!("config/data/cache dir: {}", app_dir.display());
    println!(
        "database file:         {}",
        app_dir.join(storage::DB_FILE_NAME).display()
    );
    println!("music dir:             {}", music.display());
}

/// системный каталог музыки по умолчанию. Если его нет на диске — объясняет
/// варианты и предлагает создать. Путь возвращается в любом случае: при отказе
/// он просто не существует, и индексация его пропустит.
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

/// задаёт вопрос и читает ответ из stdin. По умолчанию (пустой ввод / ошибка) —
/// «нет»; «да» только при `y`/`Y`.
fn prompt_yes_no(question: &str) -> bool {
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim(), "y" | "Y")
}
