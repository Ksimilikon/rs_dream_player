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
    let mut config = match config::config_file() {
        Some(p) => config::Config::load(&p).unwrap_or_default(),
        None => config::Config::default(),
    };

    // источник: --playlist (из папки, без бд) проигрывается сразу; обычный режим
    // индексирует каталог и предлагает выбрать плейлист из бд (без авто-старта).
    // `playlists` — все плейлисты бд с их треками (для левой панели и предпросмотра).
    let (initial, playlists, db) = if let Some(dir) = args.playlist {
        (Some(Playlist::from_dir(&dir).unwrap()), Vec::new(), None)
    } else {
        let db_path = config::db_file().expect("не удалось определить каталог данных");
        let storage = storage::Db::init(db_path).unwrap();

        // директория музыки: явный --path используем как есть, иначе системный
        // каталог по умолчанию (с проверкой существования).
        let music = match args.path {
            Some(path) => Some(path),
            None => ensure_default_music_dir(),
        };
        print_dirs(music.as_deref());

        // индексируем каталог (наполняем бд); сам пул не запускаем.
        if let Some(path) = &music {
            let _ = storage.index_dir(path);
        }
        // первый пункт — виртуальный плейлист со всем пулом песен.
        let pool = storage
            .pool_playlist()
            .unwrap_or_else(|_| Playlist::from_tracks(Vec::new()));
        let mut entries = vec![tui::PlaylistEntry {
            name: "ALL SONGS".to_string(),
            tracks: track_infos(&pool),
            pool: true,
        }];
        entries.extend(storage.list_playlists().unwrap_or_default().iter().map(|p| {
            tui::PlaylistEntry {
                name: p.get_name().unwrap_or_else(|| "---".to_string()),
                tracks: track_infos(p),
                pool: false,
            }
        }));
        (None, entries, Some(storage))
    };

    // стартовое состояние «играющего» плейлиста — только для --playlist
    let (playlist_name, tracks) = match &initial {
        Some(p) => (p.get_name().unwrap_or_else(|| "---".to_string()), track_infos(p)),
        None => ("---".to_string(), Vec::new()),
    };

    // оркестратор играет в фоне, главный поток занимает интерфейс
    let master = config.master_volume;
    let (updates, controls) = Orchestrator::run(db, initial, master);

    // мост: команды TUI -> управление оркестратором. Мастер-громкость
    // дополнительно сохраняем в конфиг (песенная сохраняется в бд внутри менеджера).
    let (tx_ctl, rx_ctl) = std::sync::mpsc::channel::<tui::Control>();
    let config_path = config::config_file();
    std::thread::spawn(move || {
        while let Ok(c) = rx_ctl.recv() {
            match c {
                tui::Control::Next => controls.next(),
                tui::Control::Prev => controls.prev(),
                tui::Control::PlayPause => controls.play_pause(),
                tui::Control::Select(i) => controls.select(i),
                tui::Control::LoadPlaylist(name) => controls.load_playlist(name),
                tui::Control::LoadPool => controls.load_pool(),
                tui::Control::SongVolume(v) => controls.set_song_volume(v),
                tui::Control::MasterVolume(v) => {
                    controls.set_master_volume(v);
                    config.master_volume = v;
                    if let Some(path) = &config_path {
                        let _ = config.save(path);
                    }
                }
            }
        }
    });

    let view = tui::View {
        playlist_name,
        tracks,
        playlists,
        master_volume: master,
    };
    if let Err(e) = tui::run(view, updates, tx_ctl) {
        eprintln!("tui: {e}");
    }
}

/// собирает краткую инфу о треках плейлиста для TUI.
fn track_infos(playlist: &Playlist) -> Vec<tui::TrackInfo> {
    playlist
        .tracks()
        .iter()
        .map(|t| {
            let (title, artists) = match t.get_metadata() {
                Ok(m) => (m.title.clone(), m.artist.join(", ")),
                Err(_) => ("Unknown".to_string(), String::new()),
            };
            tui::TrackInfo {
                title,
                artists,
                volume: t.volume,
            }
        })
        .collect()
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

