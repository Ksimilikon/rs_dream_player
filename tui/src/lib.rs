use std::{
    sync::mpsc::{Receiver, Sender},
    time::Duration,
};

use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    widgets::{Paragraph, Tabs},
};

mod model;
mod tabs;

pub use model::{Control, PlaylistEntry, TrackInfo, Update};

use model::Model;
use tabs::{Action, PlaylistsTab, SettingsTab, SongTab, Tab};

/// исходные данные для старта интерфейса.
pub struct View {
    pub playlist_name: String,
    pub tracks: Vec<TrackInfo>,
    pub playlists: Vec<PlaylistEntry>,
    pub master_volume: f32,
}

/// сколько вкладок реально доступно (3-я, settings, пока скрыта).
const VISIBLE_TABS: usize = 2;

/// шаг изменения громкости клавишами.
const VOL_STEP: f32 = 0.05;

const HINTS: &str =
    "H/L tabs | J/K move | Enter select | space ⏯ | -/+ vol | [/] svol | : cmd | q quit";

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
    /// разовое сообщение (напр. вывод `:help`).
    status: Option<String>,
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
        }
    }

    /// обрабатывает клавишу; возвращает `true`, если нужно выйти.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.command.is_some() {
            return self.handle_command_key(key);
        }
        self.status = None;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char(':') => self.command = Some(String::new()),
            KeyCode::Char('h') | KeyCode::Char('H') => {
                self.current_tab = self.current_tab.saturating_sub(1);
            }
            KeyCode::Char('l') | KeyCode::Char('L') => {
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
        let cmd = cmd.trim();
        let (head, arg) = match cmd.split_once(char::is_whitespace) {
            Some((h, a)) => (h, a.trim()),
            None => (cmd, ""),
        };
        match head {
            "q" | "quit" => return true,
            "help" => {
                self.status =
                    Some("команды: :q  :vol N  :svol N  :pl <name>  :song <name>  :<номер>".into());
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
                _ => self.status = Some(format!("неизвестная команда: {other}")),
            },
        }
        false
    }

    fn dispatch(&mut self, action: Action) {
        let control = match action {
            Action::LoadPlaylist(name) => Control::LoadPlaylist(name),
            Action::LoadPool => Control::LoadPool,
            Action::SelectSong(i) => Control::Select(i),
        };
        let _ = self.controls.send(control);
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

    fn draw(&mut self, frame: &mut Frame) {
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
        frame.render_widget(Paragraph::new("H‹"), h_hint);
        let titles: Vec<String> = self
            .tabs
            .iter()
            .take(VISIBLE_TABS)
            .map(|t| format!(" {} ", t.title()))
            .collect();
        frame.render_widget(Tabs::new(titles).select(self.current_tab), bar);
        frame.render_widget(Paragraph::new("›L"), l_hint);

        // контент активной вкладки
        self.tabs[self.current_tab].render(frame, content, &self.model);

        // нижний блок (независим от вкладок)
        let [sep, status, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(bottom);
        frame.render_widget(Paragraph::new("─".repeat(sep.width as usize)), sep);

        let now = self
            .model
            .tracks
            .get(self.model.current)
            .map(|t| format!("{} — {}", t.title, t.artists))
            .unwrap_or_else(|| "—".into());
        frame.render_widget(
            Paragraph::new(format!(
                "▶ {}    vol {}%   svol {}%",
                now,
                (self.model.master_vol * 100.0).round() as u32,
                (self.model.song_vol() * 100.0).round() as u32,
            )),
            status,
        );

        let hint_line = match (&self.command, &self.status) {
            (Some(buf), _) => format!(":{buf}"),
            (None, Some(s)) => s.clone(),
            (None, None) => HINTS.to_string(),
        };
        frame.render_widget(Paragraph::new(hint_line), hints);
    }
}

/// парсит "80" → 0.8 (проценты 0..=100 в долю 0.0..=1.0).
fn parse_percent(s: &str) -> Option<f32> {
    s.trim()
        .parse::<f32>()
        .ok()
        .map(|n| (n / 100.0).clamp(0.0, 1.0))
}
