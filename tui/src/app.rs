//! приложение TUI: состояние, цикл событий, оркестрация воспроизведения поверх
//! FFI ядра (следующий/предыдущий/автоплей), управление вкладками, командная
//! строка и общий каркас кадра (бар вкладок + контент + нижняя панель).

use std::time::Duration;

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Paragraph, Tabs},
};

use crate::ffi::{self, TrackMeta};
use crate::images::ImageManager;
use crate::integration::{Media, MediaCommand, MediaMeta};
use crate::ui::{
    MetaEdit, Model, Playing, PlaylistView, Tab, TabEvent,
    layouts::{Hint, TextField, hints_line, progress_bar},
    tabs::{HelpTab, PlaylistEditor, PlaylistsTab, SettingsTab, SongEditor, SongTab},
};

const VOL_STEP: f32 = 0.05;
const MASTER_VOL_MAX: f32 = 1.0;
const SONG_VOL_MAX: f32 = 2.0;

/// глобальные подсказки (общие для всех вкладок) — максимально кратко.
const GLOBAL_HINTS: &[Hint] = &[
    ("H/L", "tab"),
    ("spc", "play"),
    ("</>", "trk"),
    ("-/+", "vol"),
    ("[/]", "svol"),
    (":", "cmd"),
    ("X", "cls"),
    ("q", "quit"),
];

#[derive(PartialEq, Eq)]
enum Status {
    Info,
    Warn,
    Error,
}

/// отложенная удаляющая операция, ждущая подтверждения (y/N).
enum Pending {
    DeletePlaylist(usize),
    RemovePlayingDb(usize),
}

pub struct App {
    model: Model,
    screens: Vec<Box<dyn Tab>>,
    current: usize,
    /// активная командная строка (`:`).
    command: Option<TextField>,
    status: Option<(String, Status)>,
    /// отложенное удаление: красный запрос подтверждения внизу (y/N).
    confirm: Option<(String, Pending)>,
    force_clear: bool,
    quit: bool,
    /// системная медиа-интеграция (MPRIS/SMTC) — команды и метаданные.
    media: Media,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            model: Model {
                playlists: Vec::new(),
                playing: None,
                master_vol: ffi::engine::master_volume(),
                pos: 0,
            },
            screens: vec![Box::new(PlaylistsTab::default())],
            current: 0,
            command: None,
            status: None,
            confirm: None,
            force_clear: false,
            quit: false,
            media: Media::start(),
        };
        app.reload_library();
        app
    }

    /// перечитывает библиотеку из индекса: пул «ALL SONGS» + плейлисты из бд.
    /// Треки плейлистов собираются из пула по id (один запрос пула).
    fn reload_library(&mut self) {
        let pool = ffi::db::get_tracks();
        let by_id: std::collections::HashMap<i64, TrackMeta> = pool
            .iter()
            .filter_map(|t| t.id.map(|id| (id, t.clone())))
            .collect();

        let mut playlists = vec![PlaylistView {
            name: "ALL SONGS".to_string(),
            tracks: pool,
            pool: true,
            id: None,
        }];
        for info in ffi::db::get_playlists() {
            let tracks = info
                .track_ids
                .iter()
                .filter_map(|id| by_id.get(id).cloned())
                .collect();
            playlists.push(PlaylistView {
                name: info.name.unwrap_or_else(|| "(unnamed)".to_string()),
                tracks,
                pool: false,
                id: None,
            });
        }
        self.model.playlists = playlists;
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        self.startup_resync(terminal)?;
        while !self.quit {
            // автоплей: ядро сообщило о конце трека -> следующий.
            if ffi::engine::take_track_ended() {
                self.next_track();
            }
            // команды системных медиа-клавиш (MPRIS/SMTC).
            while let Some(cmd) = self.media.poll() {
                self.handle_media(cmd);
            }
            self.model.pos = ffi::engine::position();

            terminal.draw(|frame| self.draw(frame))?;

            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
                    Event::Resize(_, _) => self.force_clear = true,
                    _ => {}
                }
            }
            if self.force_clear {
                // очистку не делаем фатальной: промежуточный ресайз может дать
                // временную ошибку, из-за которой не стоит падать/выходить.
                let _ = terminal.clear();
                self.force_clear = false;
            }
        }
        Ok(())
    }

    /// авто-фикс артефактов старта (как ручной ресайз): пересчёт буферов под
    /// фактический размер бэкенда и полная очистка.
    fn startup_resync(&self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        std::thread::sleep(Duration::from_millis(30));
        // трюк 1×1 + autoresize — обходной путь для части Unix-терминалов
        // (kitty/tty/zellij). На Windows он лишний и может рассинхронизировать
        // буфер бэкенда с окном консоли, роняя следующий ресайз, — поэтому
        // ограничен Unix.
        #[cfg(unix)]
        if let Ok(size) = terminal.size()
            && size.width > 0
            && size.height > 0
        {
            let _ = terminal.resize(Rect::new(0, 0, 1, 1));
            let _ = terminal.autoresize();
        }
        terminal.clear()
    }

    // ───────────────────────── playback ─────────────────────────

    /// запускает плейлист `index` из библиотеки с трека `start`.
    fn load_playlist(&mut self, index: usize, start: usize) {
        let Some(pl) = self.model.playlists.get(index) else {
            return;
        };
        if pl.tracks.is_empty() {
            self.set_status("playlist is empty", Status::Warn);
            return;
        }
        let start = start.min(pl.tracks.len() - 1);
        self.model.playing = Some(Playing {
            name: pl.name.clone(),
            tracks: pl.tracks.clone(),
            current: start,
        });
        self.ensure_song_tab();
        self.play_current();
    }

    /// вкладка SONG существует только во время проигрывания: создаём её (с
    /// детекцией graphics-протокола) и переключаемся на неё.
    fn ensure_song_tab(&mut self) {
        let idx = match self.screens.iter().position(|t| t.title() == "SONG") {
            Some(i) => i,
            None => {
                // SONG сразу после PLAYLISTS (индекс 1).
                let pos = 1.min(self.screens.len());
                self.screens
                    .insert(pos, Box::new(SongTab::new(ImageManager::detect())));
                pos
            }
        };
        self.current = idx;
    }

    /// играет трек под `playing.current` через FFI (по id или пути) и публикует
    /// его метаданные в системный транспорт.
    fn play_current(&mut self) {
        let Some(playing) = &self.model.playing else {
            return;
        };
        let Some(track) = playing.tracks.get(playing.current) else {
            return;
        };
        let ok = match (track.id, track.path.as_deref()) {
            (Some(id), _) => ffi::engine::play_track_id(id),
            (None, Some(path)) => ffi::engine::play_track_path(path),
            _ => false,
        };
        self.media.set_metadata(MediaMeta {
            title: track.title.clone(),
            artists: track.artists.clone(),
            art_path: track.cover_art.clone().map(std::path::PathBuf::from),
        });
        if !ok {
            self.set_status("failed to play track", Status::Error);
        }
        self.model.pos = 0;
    }

    /// переключает устройство вывода с авто-возобновлением: движок сбрасывает
    /// трек при смене устройства, поэтому запоминаем позицию/паузу и
    /// восстанавливаем текущий трек на новом устройстве.
    fn switch_device(&mut self, name: String) {
        let pos = ffi::engine::position();
        let paused = ffi::engine::is_paused();
        let was_playing = self.model.playing.is_some();
        if !ffi::engine::set_output_device(&name) {
            self.set_status(format!("failed to switch to: {name}"), Status::Error);
            return;
        }
        if was_playing {
            self.play_current();
            if pos > 0 {
                ffi::engine::seek(pos);
                self.model.pos = pos;
            }
            if paused {
                ffi::engine::pause();
            }
        }
        self.set_status(format!("output device: {name}"), Status::Info);
    }

    /// применяет команду системных медиа-клавиш.
    fn handle_media(&mut self, cmd: MediaCommand) {
        match cmd {
            MediaCommand::Next => self.next_track(),
            MediaCommand::Prev => self.prev_track(),
            MediaCommand::PlayPause => ffi::engine::play_pause(),
            MediaCommand::Play => ffi::engine::play(),
            MediaCommand::Pause => ffi::engine::pause(),
            MediaCommand::Stop => ffi::engine::stop(),
        }
    }

    fn next_track(&mut self) {
        if let Some(p) = &mut self.model.playing
            && !p.tracks.is_empty()
        {
            p.current = (p.current + 1) % p.tracks.len();
            self.play_current();
        }
    }

    fn prev_track(&mut self) {
        if let Some(p) = &mut self.model.playing
            && !p.tracks.is_empty()
        {
            p.current = if p.current == 0 {
                p.tracks.len() - 1
            } else {
                p.current - 1
            };
            self.play_current();
        }
    }

    fn select_playing(&mut self, i: usize) {
        if let Some(p) = &mut self.model.playing
            && i < p.tracks.len()
        {
            p.current = i;
            self.play_current();
        }
    }

    // ───────────────────────── volume ─────────────────────────

    fn add_master(&mut self, delta: f32) {
        let v = (self.model.master_vol + delta).clamp(0.0, MASTER_VOL_MAX);
        self.model.master_vol = v;
        ffi::engine::set_master_volume(v);
    }

    fn add_song(&mut self, delta: f32) {
        let v = (ffi::engine::track_volume() + delta).clamp(0.0, SONG_VOL_MAX);
        ffi::engine::set_track_volume(v);
        // сохранить персональную громкость трека в бд.
        if let Some(p) = &self.model.playing
            && let Some(t) = p.tracks.get(p.current)
            && let Some(id) = t.id
        {
            ffi::db::set_track_volume(id, v);
        }
    }

    // ───────────────────────── tabs ─────────────────────────

    /// добавляет (или переключает на уже открытую) транзиентную вкладку.
    fn push_transient(&mut self, tab: Box<dyn Tab>) {
        let title = tab.title();
        if let Some(i) = self.screens.iter().position(|t| t.title() == title) {
            self.current = i;
        } else {
            self.screens.push(tab);
            self.current = self.screens.len() - 1;
        }
    }

    /// закрывает текущую вкладку (Shift+X / отмена / подтверждение). PLAYLISTS
    /// не закрывается. Закрытие SONG останавливает проигрывание и закрывает
    /// плейлист — выделение играющего на PLAYLISTS пропадает само (playing=None).
    fn close_current(&mut self) {
        if !self.screens[self.current].closable() {
            return;
        }
        if self.screens[self.current].title() == "SONG" {
            ffi::engine::stop();
            self.model.playing = None;
        }
        self.screens.remove(self.current);
        self.current = self.current.min(self.screens.len().saturating_sub(1));
    }

    fn open_help(&mut self) {
        self.push_transient(Box::new(HelpTab::default()));
    }

    // ───────────────────────── events ─────────────────────────

    fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl+L / Ctrl+C — всегда.
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('l') => {
                    self.force_clear = true;
                    return;
                }
                KeyCode::Char('c') => {
                    self.quit = true;
                    return;
                }
                _ => {}
            }
        }
        // командная строка перехватывает ввод.
        if self.command.is_some() {
            self.handle_command_key(key);
            return;
        }
        // ждём подтверждения удаления: y/Y — выполнить, иначе — отмена.
        if self.confirm.is_some() {
            self.handle_confirm_key(key);
            return;
        }
        // вкладка активно вводит текст (поле редактора) — отдаём ей весь ввод,
        // чтобы буквы/пробел не срабатывали как глобальные команды.
        if self.screens[self.current].wants_input() {
            let ev = self.screens[self.current].on_key(key, &self.model);
            self.handle_tab_event(ev);
            return;
        }

        self.status = None;
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char(':') => self.command = Some(TextField::new("")),
            KeyCode::Char('X') => self.close_current(),
            KeyCode::Char('H') => self.current = self.current.saturating_sub(1),
            KeyCode::Char('L') => {
                if self.current + 1 < self.screens.len() {
                    self.current += 1;
                }
            }
            KeyCode::Char(' ') => ffi::engine::play_pause(),
            KeyCode::Char('>') => self.next_track(),
            KeyCode::Char('<') => self.prev_track(),
            KeyCode::Char('-') => self.add_master(-VOL_STEP),
            KeyCode::Char('+') | KeyCode::Char('=') => self.add_master(VOL_STEP),
            KeyCode::Char('[') => self.add_song(-VOL_STEP),
            KeyCode::Char(']') => self.add_song(VOL_STEP),
            _ => {
                let ev = self.screens[self.current].on_key(key, &self.model);
                self.handle_tab_event(ev);
            }
        }
    }

    fn handle_tab_event(&mut self, ev: TabEvent) {
        match ev {
            TabEvent::None => {}
            TabEvent::Close => self.close_current(),
            TabEvent::PlayPlaylist { index, start } => {
                self.load_playlist(index, start);
                for t in self.screens.iter_mut() {
                    t.on_tracks_changed();
                }
            }
            TabEvent::SelectPlaying(i) => self.select_playing(i),
            TabEvent::NewPlaylist => {
                let pool = self.model.pool().to_vec();
                self.push_transient(Box::new(PlaylistEditor::create(pool)));
            }
            TabEvent::EditPlaylist(index) => self.open_playlist_editor(index),
            TabEvent::EditPlayingPlaylist => {
                // правка играющего плейлиста: ищем его в библиотеке по имени.
                if let Some(name) = self.model.playing.as_ref().map(|p| p.name.clone())
                    && let Some(idx) = self.model.playlists.iter().position(|p| p.name == name)
                {
                    self.open_playlist_editor(idx);
                }
            }
            TabEvent::DeletePlaylist(index) => {
                // пул не удаляется — подтверждение не спрашиваем.
                match self.model.playlists.get(index) {
                    Some(pl) if !pl.pool => {
                        let msg = format!("delete playlist '{}'? (y/N)", pl.name);
                        self.confirm = Some((msg, Pending::DeletePlaylist(index)));
                    }
                    _ => self.set_status("cannot delete the pool playlist", Status::Warn),
                }
            }
            TabEvent::EditMeta(index) => self.open_meta_editor(index),
            TabEvent::MovePlaying { from, to } => self.move_playing(from, to),
            // временное удаление из очереди — без подтверждения (обратимо);
            // удаление из плейлиста в бд — с подтверждением.
            TabEvent::RemovePlaying { index, from_db: false } => self.remove_playing(index, false),
            TabEvent::RemovePlaying { index, from_db: true } => {
                self.confirm = Some((
                    "remove track from playlist (db)? (y/N)".to_string(),
                    Pending::RemovePlayingDb(index),
                ));
            }
            TabEvent::SavePlaylist { name, ids } => {
                if ffi::db::save_playlist(&name, &ids, None) {
                    self.close_current();
                    self.reload_library();
                    self.refresh_playing();
                    self.set_status(format!("playlist saved: {name}"), Status::Info);
                } else {
                    self.set_status("failed to save playlist", Status::Error);
                }
            }
            TabEvent::SaveMeta(edit) => {
                self.apply_meta(edit);
                self.close_current();
                self.reload_library();
                self.refresh_playing();
                self.set_status("metadata saved", Status::Info);
            }
            TabEvent::SetOutputDevice(name) => self.switch_device(name),
            TabEvent::Status(msg) => self.set_status(msg, Status::Warn),
        }
    }

    /// открывает редактор правки плейлиста `index` (кроме пула).
    fn open_playlist_editor(&mut self, index: usize) {
        let pool = self.model.pool().to_vec();
        let Some(pl) = self.model.playlists.get(index) else {
            return;
        };
        if pl.pool {
            self.set_status("cannot edit the pool playlist", Status::Warn);
            return;
        }
        let editor = PlaylistEditor::edit(pl.name.clone(), pl.tracks.clone(), pool);
        self.push_transient(Box::new(editor));
    }

    /// открывает редактор метаданных трека `index` играющего списка.
    fn open_meta_editor(&mut self, index: usize) {
        let Some(track) = self
            .model
            .playing
            .as_ref()
            .and_then(|p| p.tracks.get(index))
        else {
            return;
        };
        if track.id.is_none() {
            self.set_status("track is not in the library", Status::Warn);
            return;
        }
        let editor = SongEditor::new(track);
        self.push_transient(Box::new(editor));
    }

    /// применяет правки метаданных через FFI (поле `None` — не трогаем;
    /// album/genres пустые — очищаем).
    fn apply_meta(&self, e: MetaEdit) {
        ffi::db::set_track_meta(e.id, e.title.as_deref(), e.artists.as_deref());
        if let Some(a) = &e.album {
            ffi::db::set_track_album(e.id, (!a.is_empty()).then_some(a.as_str()));
        }
        if let Some(g) = &e.genres {
            ffi::db::set_track_genres(e.id, g);
        }
        if let Some(c) = &e.cover
            && !c.is_empty()
        {
            ffi::db::set_track_cover(e.id, Some(c));
        }
        if let Some(c) = &e.color {
            ffi::db::set_track_color(e.id, (!c.is_empty()).then_some(c.as_str()));
        }
        if let Some(l) = &e.label {
            ffi::db::set_track_label(e.id, (!l.is_empty()).then_some(l.as_str()));
        }
    }

    /// перестановка трека в играющем списке (временно): сохраняет играющий трек
    /// под `current`, воспроизведение не прерывает.
    fn move_playing(&mut self, from: usize, to: usize) {
        if let Some(p) = &mut self.model.playing {
            let len = p.tracks.len();
            if from >= len || to >= len {
                return;
            }
            let cur = p.tracks.get(p.current).map(ident);
            let t = p.tracks.remove(from);
            p.tracks.insert(to, t);
            if let Some(id) = cur
                && let Some(ni) = p.tracks.iter().position(|x| ident(x) == id)
            {
                p.current = ni;
            }
        }
    }

    /// убирает трек из играющего списка; `from_db` — ещё и из плейлиста в бд
    /// (только для реального плейлиста, не пула).
    fn remove_playing(&mut self, index: usize, from_db: bool) {
        let (name, ids) = {
            let Some(p) = self.model.playing.as_mut() else {
                return;
            };
            if index >= p.tracks.len() {
                return;
            }
            p.tracks.remove(index);
            if index < p.current {
                p.current = p.current.saturating_sub(1);
            }
            if p.current >= p.tracks.len() {
                p.current = p.tracks.len().saturating_sub(1);
            }
            (
                p.name.clone(),
                p.tracks.iter().filter_map(|t| t.id).collect::<Vec<i64>>(),
            )
        };
        if !from_db {
            return;
        }
        if self.model.playlists.iter().any(|x| x.name == name && !x.pool) {
            if ffi::db::save_playlist(&name, &ids, None) {
                self.reload_library();
                self.set_status("track removed from playlist (db)", Status::Info);
            } else {
                self.set_status("failed to update playlist", Status::Error);
            }
        } else {
            self.set_status("cannot modify the pool in db", Status::Warn);
        }
    }

    /// пересобирает треки играющего плейлиста из обновлённой библиотеки
    /// (по имени), сохраняя индекс по возможности.
    fn refresh_playing(&mut self) {
        let Some(name) = self.model.playing.as_ref().map(|p| p.name.clone()) else {
            return;
        };
        let tracks = self
            .model
            .playlists
            .iter()
            .find(|x| x.name == name)
            .map(|x| x.tracks.clone());
        if let (Some(p), Some(tracks)) = (self.model.playing.as_mut(), tracks) {
            p.tracks = tracks;
            if p.current >= p.tracks.len() {
                p.current = p.tracks.len().saturating_sub(1);
            }
        }
    }

    fn delete_playlist(&mut self, index: usize) {
        let Some(pl) = self.model.playlists.get(index) else {
            return;
        };
        if pl.pool {
            return;
        }
        let name = pl.name.clone();
        if ffi::db::remove_playlist(&name) {
            self.reload_library();
            self.set_status(format!("playlist deleted: {name}"), Status::Info);
        } else {
            self.set_status("failed to delete playlist", Status::Error);
        }
    }

    /// обрабатывает ответ на запрос подтверждения удаления.
    fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some((_, pending)) = self.confirm.take() {
                    self.exec_pending(pending);
                }
            }
            _ => {
                self.confirm = None;
                self.set_status("cancelled", Status::Warn);
            }
        }
    }

    /// выполняет подтверждённую удаляющую операцию.
    fn exec_pending(&mut self, pending: Pending) {
        match pending {
            Pending::DeletePlaylist(index) => self.delete_playlist(index),
            Pending::RemovePlayingDb(index) => self.remove_playing(index, true),
        }
    }

    fn handle_command_key(&mut self, key: KeyEvent) {
        let Some(buf) = self.command.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.command = None,
            KeyCode::Enter => {
                let cmd = buf.text();
                self.command = None;
                self.exec_command(&cmd);
            }
            KeyCode::Backspace => buf.backspace(),
            KeyCode::Delete => buf.delete(),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => buf.word_left(),
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => buf.word_right(),
            KeyCode::Left => buf.left(),
            KeyCode::Right => buf.right(),
            KeyCode::Home => buf.home(),
            KeyCode::End => buf.end(),
            KeyCode::Char(c) => buf.insert(c),
            _ => {}
        }
    }

    /// выполняет команду (без ведущего `:`).
    fn exec_command(&mut self, cmd: &str) {
        let cmd = cmd.trim();
        let (head, arg) = match cmd.split_once(char::is_whitespace) {
            Some((h, a)) => (h, a.trim()),
            None => (cmd, ""),
        };
        match head {
            "q" | "quit" => self.quit = true,
            "redraw" | "clear" => self.force_clear = true,
            "help" => self.open_help(),
            "settings" | "device" | "dev" => {
                self.push_transient(Box::new(SettingsTab::default()))
            }
            "seek" => match arg.parse::<u64>() {
                Ok(secs) => {
                    ffi::engine::seek(secs);
                    self.model.pos = secs;
                }
                Err(_) => self.set_status("usage: :seek <seconds>", Status::Warn),
            },
            "vol" => {
                if let Some(v) = parse_percent(arg, MASTER_VOL_MAX) {
                    self.model.master_vol = v;
                    ffi::engine::set_master_volume(v);
                }
            }
            "svol" => {
                if let Some(v) = parse_percent(arg, SONG_VOL_MAX) {
                    ffi::engine::set_track_volume(v);
                }
            }
            "scan" => {
                let dir = (!arg.is_empty()).then_some(arg);
                if ffi::db::index_dir(dir) {
                    self.reload_library();
                    self.set_status("library indexed", Status::Info);
                } else {
                    self.set_status("scan failed", Status::Error);
                }
            }
            "" => {}
            other => self.set_status(format!("unknown command: {other}"), Status::Warn),
        }
    }

    fn set_status(&mut self, msg: impl Into<String>, kind: Status) {
        self.status = Some((msg.into(), kind));
    }

    // ───────────────────────── render ─────────────────────────

    fn draw(&mut self, frame: &mut Frame) {
        // слишком маленькая область (сворачивание/промежуточный ресайз) — ничего
        // не рисуем, чтобы не делить площадь в ноль и не писать мимо буфера.
        let area = frame.area();
        if area.width < 4 || area.height < 6 {
            return;
        }
        let [tabbar, content, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(4),
        ])
        .areas(area);

        self.draw_tabbar(frame, tabbar);
        self.screens[self.current].render(frame, content, &self.model);
        self.draw_bottom(frame, bottom);
    }

    fn draw_tabbar(&self, frame: &mut Frame, area: Rect) {
        let [h, bar, l] = Layout::horizontal([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new("H<"), h);
        let titles: Vec<String> = self.screens.iter().map(|t| format!(" {} ", t.title())).collect();
        frame.render_widget(Tabs::new(titles).select(self.current), bar);
        frame.render_widget(Paragraph::new(">L"), l);
    }

    fn draw_bottom(&self, frame: &mut Frame, area: Rect) {
        let [sep, status, progress, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        frame.render_widget(Paragraph::new("-".repeat(sep.width as usize)), sep);

        // текущий трек + громкости.
        let now = self
            .model
            .playing
            .as_ref()
            .and_then(|p| p.tracks.get(p.current))
            .map(|t| format!("{} — {}", t.title, t.artists_line()))
            .unwrap_or_else(|| "nothing playing".to_string());
        frame.render_widget(
            Paragraph::new(format!(
                "> {now}    vol {}%  svol {}%",
                (self.model.master_vol * 100.0).round() as u32,
                (ffi::engine::track_volume() * 100.0).round() as u32,
            )),
            status,
        );

        // прогресс-бар (зелёный).
        let total = ffi::engine::duration();
        let bar = progress_bar(self.model.pos, total, progress.width as usize);
        frame.render_widget(
            Paragraph::new(bar).style(Style::new().fg(crate::ui::PLAYING_GREEN)),
            progress,
        );

        // низ: запрос подтверждения (красный) / командная строка / статус / подсказки.
        if let Some((msg, _)) = &self.confirm {
            frame.render_widget(
                Paragraph::new(msg.clone()).style(Style::new().fg(Color::Red)),
                hints,
            );
            return;
        }
        match (&self.command, &self.status) {
            (Some(buf), _) => {
                frame.render_widget(Paragraph::new(format!(":{}", buf.text())), hints);
                // курсор в позиции каретки (с учётом ведущего `:`).
                frame.set_cursor_position((hints.x + 1 + buf.caret() as u16, hints.y));
            }
            (None, Some((msg, kind))) => {
                let style = match kind {
                    Status::Error => Style::new().fg(Color::Red),
                    Status::Warn => Style::new().fg(Color::Yellow),
                    Status::Info => Style::default(),
                };
                frame.render_widget(Paragraph::new(msg.clone()).style(style), hints);
            }
            (None, None) => {
                let mut all = GLOBAL_HINTS.to_vec();
                all.extend(self.screens[self.current].hints());
                frame.render_widget(Paragraph::new(hints_line(&all)), hints);
            }
        }
    }
}

/// "80" -> 0.8, ограничено сверху `max`.
fn parse_percent(s: &str, max: f32) -> Option<f32> {
    s.trim().parse::<f32>().ok().map(|n| (n / 100.0).clamp(0.0, max))
}

/// устойчивая идентичность трека для поиска после перестановки.
fn ident(t: &TrackMeta) -> (Option<i64>, Option<String>, String) {
    (t.id, t.path.clone(), t.title.clone())
}
