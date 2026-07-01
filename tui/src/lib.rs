use std::{
    sync::mpsc::{Receiver, Sender},
    time::Duration,
};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Paragraph, Tabs},
};

mod model;
mod tabs;

pub use model::{Control, PlaylistEntry, TrackInfo, Update};

use model::Model;
use tabs::{
    Action, EditorOutcome, EditorState, PlaylistsTab, SettingsTab, SongTab, Tab, help_lines,
    render_help,
};

/// исходные данные для старта интерфейса.
pub struct View {
    pub playlist_name: String,
    pub tracks: Vec<TrackInfo>,
    pub playlists: Vec<PlaylistEntry>,
    pub master_volume: f32,
    /// текст конфига для вкладки настроек (готовый к показу).
    pub config_text: String,
}

/// сколько вкладок доступно в навигации (playlists, song, settings).
const VISIBLE_TABS: usize = 3;

/// шаг изменения громкости клавишами.
const VOL_STEP: f32 = 0.05;

const HINTS: &str =
    "Shift+H/L tabs | j/k move | Enter select | space play/pause | -/+ vol | [/] svol | n new | e edit | : cmd | ? help | q quit";

/// запускает TUI: захватывает текущий поток до выхода пользователя.
pub fn run(
    view: View,
    updates: Receiver<Update>,
    controls: Sender<Control>,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let mut app = App::new(view, updates, controls);
    let res = app.run(&mut terminal);
    ratatui::restore();
    res
}

struct App {
    model: Model,
    tabs: Vec<Box<dyn Tab>>,
    current_tab: usize,
    /// буфер командной строки (Some — активен режим `:`).
    command: Option<String>,
    /// разовое сообщение (напр. итог сохранения плейлиста).
    status: Option<String>,
    /// показывать `status` жёлтым (предупреждения/итоги).
    status_warn: bool,
    /// активный редактор плейлиста (Some — модальный режим).
    editor: Option<EditorState>,
    /// показано ли окно справки.
    help: bool,
    /// вертикальная прокрутка окна справки.
    help_scroll: u16,
    /// сессионный временный плейлист (не в бд), показывается 2-м в списке.
    temp_entry: Option<PlaylistEntry>,
    updates: Receiver<Update>,
    controls: Sender<Control>,
}

impl App {
    fn new(view: View, updates: Receiver<Update>, controls: Sender<Control>) -> Self {
        let model = Model {
            playlists: view.playlists,
            playlist_name: view.playlist_name,
            tracks: view.tracks,
            current: 0,
            master_vol: view.master_volume,
            config_text: view.config_text,
        };
        Self {
            model,
            tabs: vec![
                Box::new(PlaylistsTab::default()),
                Box::new(SongTab::default()),
                Box::new(SettingsTab),
            ],
            current_tab: 0,
            command: None,
            status: None,
            status_warn: false,
            editor: None,
            help: false,
            help_scroll: 0,
            temp_entry: None,
            updates,
            controls,
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            // обновления состояния от оркестратора
            while let Ok(u) = self.updates.try_recv() {
                self.apply_update(u);
            }

            terminal.draw(|frame| self.draw(frame))?;

            if event::poll(Duration::from_millis(200))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && self.handle_key(key)
            {
                return Ok(());
            }
        }
    }

    fn apply_update(&mut self, update: Update) {
        match update {
            Update::NowPlaying(i) => self.model.current = i,
            Update::Playlist { name, tracks } => {
                self.model.playlist_name = name;
                self.model.tracks = tracks;
                self.model.current = 0;
                self.current_tab = 1; // авто-переход на вкладку песни
            }
            Update::Playlists(playlists) => {
                self.model.playlists = playlists;
                self.refresh_temp_slot();
            }
        }
    }

    /// обрабатывает клавишу; возвращает `true`, если нужно выйти.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        // окно справки перехватывает ввод: q/Esc закрывают его (не плеер),
        // j/k прокручивают содержимое.
        if self.help {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => self.help = false,
                KeyCode::Char('j') => {
                    let max = help_lines().saturating_sub(1);
                    self.help_scroll = (self.help_scroll + 1).min(max);
                }
                KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                _ => {}
            }
            return false;
        }
        // редактор — модальный режим, забирает весь ввод.
        if self.editor.is_some() {
            self.handle_editor_key(key);
            return false;
        }
        if self.command.is_some() {
            return self.handle_command_key(key);
        }
        self.status = None;
        self.status_warn = false;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char('?') => self.open_help(),
            KeyCode::Char(':') => self.command = Some(String::new()),
            // переключение вкладок — Shift+H / Shift+L (заглавные).
            KeyCode::Char('H') => {
                self.current_tab = self.current_tab.saturating_sub(1);
            }
            KeyCode::Char('L') => {
                if self.current_tab + 1 < VISIBLE_TABS {
                    self.current_tab += 1;
                }
            }
            KeyCode::Char(' ') => {
                let _ = self.controls.send(Control::PlayPause);
            }
            KeyCode::Char('-') => self.add_master(-VOL_STEP),
            KeyCode::Char('+') | KeyCode::Char('=') => self.add_master(VOL_STEP),
            KeyCode::Char('[') => self.add_song(-VOL_STEP),
            KeyCode::Char(']') => self.add_song(VOL_STEP),
            _ => {
                if let Some(action) = self.tabs[self.current_tab].on_key(key, &self.model) {
                    self.dispatch(action);
                }
            }
        }
        false
    }

    fn handle_editor_key(&mut self, key: KeyEvent) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match editor.handle_key(key) {
            EditorOutcome::None => {}
            EditorOutcome::Cancel => self.editor = None,
            EditorOutcome::Save { name, ids } => self.finish_editor(name, ids),
        }
    }

    /// завершает редактор: пустое имя ⇒ временный плейлист (играет, не в бд),
    /// иначе ⇒ запись в бд. В обоих случаях редактор закрывается.
    fn finish_editor(&mut self, name: String, ids: Vec<i64>) {
        self.editor = None;
        self.current_tab = 0;
        if name.is_empty() {
            let tracks = self.tracks_by_ids(&ids);
            let count = tracks.len();
            self.temp_entry = Some(PlaylistEntry {
                name: "(unnamed)".to_string(),
                tracks,
                pool: false,
                temp: true,
            });
            self.refresh_temp_slot();
            let _ = self.controls.send(Control::PlayTemp { ids });
            self.set_warn(format!(
                "temporary playlist ({count} tracks) - playing, not saved to db"
            ));
        } else {
            let _ = self.controls.send(Control::SavePlaylist {
                name: name.clone(),
                ids,
            });
            // обновлённый список плейлистов придёт через Update::Playlists.
            self.set_warn(format!("playlist saved as: {name}"));
        }
    }

    /// собирает треки по id из пула (для превью временного плейлиста).
    fn tracks_by_ids(&self, ids: &[i64]) -> Vec<TrackInfo> {
        let pool = self.pool_tracks();
        ids.iter()
            .filter_map(|id| pool.iter().find(|t| t.id == *id).cloned())
            .collect()
    }

    /// треки общего пула («ALL SONGS») — источник для редактора.
    fn pool_tracks(&self) -> Vec<TrackInfo> {
        self.model
            .playlists
            .iter()
            .find(|e| e.pool)
            .map(|e| e.tracks.clone())
            .unwrap_or_default()
    }

    /// пересобирает позицию временного плейлиста: всегда 2-й (после пула).
    fn refresh_temp_slot(&mut self) {
        self.model.playlists.retain(|e| !e.temp);
        if let Some(entry) = &self.temp_entry {
            let idx = 1.min(self.model.playlists.len());
            self.model.playlists.insert(idx, entry.clone());
        }
    }

    fn handle_command_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => self.command = None,
            KeyCode::Enter => {
                let cmd = self.command.take().unwrap_or_default();
                return self.exec_command(&cmd);
            }
            KeyCode::Backspace => {
                if let Some(buf) = self.command.as_mut() {
                    buf.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Some(buf) = self.command.as_mut() {
                    buf.push(c);
                }
            }
            _ => {}
        }
        false
    }

    /// выполняет команду (без ведущего `:`); `true` — выход.
    fn exec_command(&mut self, cmd: &str) -> bool {
        self.status = None;
        self.status_warn = false;
        let cmd = cmd.trim();
        let (head, arg) = match cmd.split_once(char::is_whitespace) {
            Some((h, a)) => (h, a.trim()),
            None => (cmd, ""),
        };
        match head {
            "q" | "quit" => return true,
            "help" => self.open_help(),
            "new" => self.open_create(),
            "edit" => {
                if arg.is_empty() {
                    self.status = Some("usage: :edit <playlist name>".into());
                } else {
                    match self
                        .model
                        .playlists
                        .iter()
                        .position(|e| !e.pool && e.name.eq_ignore_ascii_case(arg))
                    {
                        Some(i) => self.open_edit(i),
                        None => self.status = Some(format!("playlist not found: {arg}")),
                    }
                }
            }
            "vol" => {
                if let Some(v) = parse_percent(arg) {
                    self.set_master(v);
                }
            }
            "svol" => {
                if let Some(v) = parse_percent(arg) {
                    self.set_song(v);
                }
            }
            "pl" => {
                if !arg.is_empty() {
                    let _ = self.controls.send(Control::LoadPlaylist(arg.to_string()));
                }
            }
            "song" => {
                let needle = arg.to_lowercase();
                if let Some(i) = self
                    .model
                    .tracks
                    .iter()
                    .position(|t| t.title.to_lowercase().contains(&needle))
                {
                    let _ = self.controls.send(Control::Select(i));
                }
            }
            other => match other.parse::<usize>() {
                Ok(n) if n >= 1 && n <= self.model.tracks.len() => {
                    let _ = self.controls.send(Control::Select(n - 1));
                }
                _ => self.status = Some(format!("unknown command: {other}")),
            },
        }
        false
    }

    fn dispatch(&mut self, action: Action) {
        let control = match action {
            Action::LoadPlaylist(name) => Control::LoadPlaylist(name),
            Action::LoadPool => Control::LoadPool,
            Action::PlayTemp(ids) => Control::PlayTemp { ids },
            Action::SelectSong(i) => Control::Select(i),
            Action::NewPlaylist => {
                self.open_create();
                return;
            }
            Action::EditPlaylist(idx) => {
                self.open_edit(idx);
                return;
            }
        };
        let _ = self.controls.send(control);
    }

    /// открыть окно справки с прокруткой от начала.
    fn open_help(&mut self) {
        self.help = true;
        self.help_scroll = 0;
    }

    /// открыть редактор для нового плейлиста.
    fn open_create(&mut self) {
        self.editor = Some(EditorState::create(self.pool_tracks()));
    }

    /// открыть редактор для правки плейлиста под индексом (кроме пула).
    fn open_edit(&mut self, idx: usize) {
        let pool = self.pool_tracks();
        if let Some(entry) = self.model.playlists.get(idx)
            && !entry.pool
        {
            self.editor = Some(EditorState::edit(
                entry.name.clone(),
                entry.tracks.clone(),
                pool,
            ));
        }
    }

    fn add_master(&mut self, delta: f32) {
        self.set_master(self.model.master_vol + delta);
    }
    fn set_master(&mut self, v: f32) {
        self.model.master_vol = v.clamp(0.0, 1.0);
        let _ = self.controls.send(Control::MasterVolume(self.model.master_vol));
    }
    fn add_song(&mut self, delta: f32) {
        self.set_song(self.model.song_vol() + delta);
    }
    fn set_song(&mut self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        if let Some(t) = self.model.tracks.get_mut(self.model.current) {
            t.volume = v;
        }
        let _ = self.controls.send(Control::SongVolume(v));
    }

    fn set_warn(&mut self, msg: String) {
        self.status = Some(msg);
        self.status_warn = true;
    }

    fn draw(&mut self, frame: &mut Frame) {
        // редактор — на весь экран.
        if let Some(editor) = &self.editor {
            editor.render(frame, frame.area());
            return;
        }

        let [tabbar, content, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .areas(frame.area());

        // бар вкладок: подсказки H/L по краям
        let [h_hint, bar, l_hint] = Layout::horizontal([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .areas(tabbar);
        frame.render_widget(Paragraph::new("H<"), h_hint);
        let titles: Vec<String> = self
            .tabs
            .iter()
            .take(VISIBLE_TABS)
            .map(|t| format!(" {} ", t.title()))
            .collect();
        frame.render_widget(Tabs::new(titles).select(self.current_tab), bar);
        frame.render_widget(Paragraph::new(">L"), l_hint);

        // контент активной вкладки
        self.tabs[self.current_tab].render(frame, content, &self.model);

        // нижний блок (независим от вкладок)
        let [sep, status, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(bottom);
        frame.render_widget(Paragraph::new("-".repeat(sep.width as usize)), sep);

        let now = self
            .model
            .tracks
            .get(self.model.current)
            .map(|t| format!("TITLE: {} | ARTISTS: {}", t.title, t.artists))
            .unwrap_or_else(|| "-".into());
        frame.render_widget(
            Paragraph::new(format!(
                "> {}    vol {}%   svol {}%",
                now,
                (self.model.master_vol * 100.0).round() as u32,
                (self.model.song_vol() * 100.0).round() as u32,
            )),
            status,
        );

        let (line, style) = match (&self.command, &self.status) {
            (Some(buf), _) => (format!(":{buf}"), Style::default()),
            (None, Some(s)) => {
                let st = if self.status_warn {
                    Style::new().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                (s.clone(), st)
            }
            (None, None) => (HINTS.to_string(), Style::default()),
        };
        frame.render_widget(Paragraph::new(line).style(style), hints);

        if self.help {
            render_help(frame, frame.area(), self.help_scroll);
        }
    }
}

/// парсит "80" → 0.8 (проценты 0..=100 в долю 0.0..=1.0).
fn parse_percent(s: &str) -> Option<f32> {
    s.trim()
        .parse::<f32>()
        .ok()
        .map(|n| (n / 100.0).clamp(0.0, 1.0))
}
