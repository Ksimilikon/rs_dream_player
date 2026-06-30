use std::path::PathBuf;

use audio_structs::playlist::Playlist;
use clap::Parser;

use crate::orchestrator::Orchestrator;

mod config;
mod orchestrator;
mod playlist_manager;
mod traits;

#[derive(clap::Parser, Debug)]
#[command(version, about = "cli for music player core")]
struct Args {
    /// дефолтная директория музыки для индексации. Без аргумента берётся
    /// системный каталог музыки (~/Music).
    #[arg(short, long, value_name = "Dir")]
    path: Option<PathBuf>,
    /// директория, из которой собрать плейлист и сразу проиграть его как
    /// дефолтный. Индексация в бд при этом не выполняется.
    #[arg(long, value_name = "Dir")]
    playlist: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();

    // конфиг приложения (на ПК = ~/.config/dream_player/config.toml)
    let _config = match config::config_file() {
        Some(p) => config::Config::load(&p).unwrap_or_default(),
        None => config::Config::default(),
    };

    // источник плейлиста: --playlist (без индексации) приоритетнее --path.
    let playlist = if let Some(dir) = args.playlist {
        // плейлист прямо из директории, в бд ничего не пишем.
        Playlist::from_dir(&dir).unwrap()
    } else {
        // дефолтный режим: всегда работаем через бд и отдаём её общий пул.
        let db_path = config::db_file().expect("не удалось определить каталог данных");
        let storage = storage::Db::init(db_path).unwrap();

        // директория музыки: явный --path используем как есть, иначе системный
        // каталог по умолчанию (с проверкой существования).
        let music = match args.path {
            Some(path) => Some(path),
            None => ensure_default_music_dir(),
        };
        print_dirs(music.as_deref());

        // есть директория — индексируем её; в любом случае берём пул из бд.
        match music {
            Some(path) => storage.index_dir(&path).unwrap(),
            None => storage.pool_playlist().unwrap(),
        }
    };

    print_playlist(&playlist);
    Orchestrator::run(playlist);

    let _ = std::io::stdin().read_line(&mut String::new());
}

/// каталоги приложения: настроек, конфигов (пока совпадает с настройками)
/// и музыки (может отсутствовать — тогда `—`).
fn print_dirs(music: Option<&std::path::Path>) {
    let fmt = |d: Option<PathBuf>| {
        d.map(|p| p.display().to_string())
            .unwrap_or_else(|| "—".into())
    };
    println!("settings dir: {}", fmt(config::settings_dir()));
    println!("config dir:   {}", fmt(config::config_dir()));
    println!("music dir:    {}", fmt(music.map(|p| p.to_path_buf())));
}

/// определяет системный каталог музыки по умолчанию. Если его нет на диске —
/// объясняет варианты и предлагает создать его. Возвращает каталог, только
/// если он существует или был создан по согласию пользователя.
fn ensure_default_music_dir() -> Option<PathBuf> {
    let dir = config::music_dir()?;
    if dir.is_dir() {
        return Some(dir);
    }

    println!("каталог музыки по умолчанию не найден: {}", dir.display());
    println!("варианты:");
    println!("  - создать этот каталог;");
    #[cfg(target_os = "linux")]
    println!("  - задать XDG_MUSIC_DIR (~/.config/user-dirs.dirs);");
    println!("  - указать каталог самому через аргумент --path <Dir>.");

    if !prompt_yes_no(&format!(
        "создать каталог по умолчанию ({})? [y/N]:",
        dir.display()
    )) {
        return None;
    }

    match std::fs::create_dir_all(&dir) {
        Ok(()) => Some(dir),
        Err(e) => {
            println!("не удалось создать каталог: {e}");
            None
        }
    }
}

/// задаёт вопрос и читает ответ из stdin. По умолчанию (пустой ввод / ошибка) —
/// «нет»; «да» только при `y`/`Y`.
fn prompt_yes_no(question: &str) -> bool {
    use std::io::Write;
    print!("{question} ");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim(), "y" | "Y")
}

/// компактный вывод плейлиста: номер, название, исполнители и громкость.
fn print_playlist(playlist: &Playlist) {
    println!("\nplaylist ({} tracks):", playlist.get_count());
    for (i, track) in playlist.tracks().iter().enumerate() {
        let (title, artists) = match track.get_metadata() {
            Ok(meta) => (meta.title.clone(), meta.artist.join(", ")),
            Err(_) => ("Unknown".to_string(), String::new()),
        };
        println!(
            "  {:>2}. {} — {} [vol {:.2}]",
            i + 1,
            title,
            artists,
            track.volume
        );
    }
}
