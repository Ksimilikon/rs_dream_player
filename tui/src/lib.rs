//! TUI-фронтенд ядра плеера: демонстрирует крейт `api` (динамическую библиотеку)
//! целиком через его FFI. Вся работа с ядром идёт через [`ffi`]; интерфейс — в
//! [`ui`]. `main` лишь готовит каталоги и зовёт [`run`].

// Часть FFI-поверхности ядра обёрнута в `ffi/`, но ещё не вынесена в интерфейс
// (переключение устройств вывода, отдельные сеттеры/геттеры). Обёртки держим
// как демонстрацию API; allow снимет свои предупреждения о них.
#![allow(dead_code)]

use std::path::Path;

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
pub fn run(dirs: Dirs, master_volume: f32) -> std::io::Result<()> {
    // поднимаем ядро (нужна бд) и выставляем стартовую мастер-громкость.
    ffi::engine::init(dirs.cache, dirs.config, dirs.data, dirs.music, true);
    ffi::engine::set_master_volume(master_volume);
    // наполняем индекс каталогом музыки (idempotent).
    ffi::db::index_dir(None);

    let mut terminal = ratatui::init();
    let mut app = app::App::new();
    let res = app.run(&mut terminal);
    ratatui::restore();
    res
}
