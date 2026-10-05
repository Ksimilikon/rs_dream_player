//! TUI-фронтенд ядра плеера: демонстрирует крейт `api` (динамическую библиотеку)
//! целиком через его FFI. Вся работа с ядром идёт через [`ffi`]; интерфейс — в
//! [`ui`]. `main` лишь готовит каталоги и зовёт [`run`].

// Часть FFI-поверхности ядра обёрнута в `ffi/`, но ещё не вынесена в интерфейс
// (переключение устройств вывода, отдельные сеттеры/геттеры). Обёртки держим
// как демонстрацию API; allow снимет свои предупреждения о них.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

mod app;
mod ffi;
mod images;
mod integration;
mod ui;

/// каталоги, нужные ядру для инициализации (передаются в FFI `init`).
pub struct Dirs<'a> {
    pub cache: &'a Path,
    pub config: &'a Path,
    pub data: &'a Path,
    pub music: &'a Path,
}

/// инициализирует ядро, индексирует каталог музыки и запускает интерфейс.
/// Захватывает текущий поток до выхода пользователя.
pub fn run(dirs: Dirs) -> std::io::Result<()> {
    // поднимаем ядро (нужна бд). Мастер-громкость ядро само применяет из своего
    // сохранённого конфига — здесь её не трогаем.
    ffi::engine::init(dirs.cache, dirs.config, dirs.data, dirs.music, true);
    // наполняем индекс каталогом музыки (idempotent).
    ffi::db::index_dir(None);

    let mut terminal = ratatui::init();
    // паника не должна «уносить» консоль: возвращаем терминал в норму и пишем
    // причину в лог-файл (консоль под Windows может закрыться — лог останется).
    install_panic_hook(dirs.config.join("tui_panic.log"));
    let mut app = app::App::new();
    let res = app.run(&mut terminal);
    ratatui::restore();
    res
}

/// оборачивает текущий обработчик паники: сначала восстанавливает терминал
/// (raw/alt-экран), затем дописывает сообщение+бэктрейс в `log_path`, затем
/// вызывает прежний обработчик.
fn install_panic_hook(log_path: PathBuf) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // вернуть терминал в нормальный режим до любого вывода.
        ratatui::restore();
        let bt = std::backtrace::Backtrace::force_capture();
        let msg = format!("{info}\n\nbacktrace:\n{bt}\n");
        let _ = std::fs::write(&log_path, &msg);
        prev(info);
    }));
}
