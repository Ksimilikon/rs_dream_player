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

pub use model::{CheckTarget, Control, PlaylistEntry, TrackInfo, Update};

use model::Model;
use tabs::{
    Action, COLOR_NAMES, EditorOutcome, EditorState, InvalidOutcome, InvalidPromptState, MetaEdit,
    MetaEditorOutcome, MetaEditorState, PlaylistsTab, SettingsTab, SongTab, Tab, help_lines,
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
    "Shift+H/L tabs | j/k move | Enter select | space play/pause | -/+ vol | [ and ] svol | n new | e edit | m meta | : cmd | ? help | q quit";

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
    /// показывать `status` красным (ошибки: удаление трека и т.п.).
    status_error: bool,
    /// активный редактор плейлиста (Some — модальный режим).
    editor: Option<EditorState>,
    /// активный редактор метаданных трека (Some — модальный режим).
    meta_editor: Option<MetaEditorState>,
    /// активное меню недействительного трека (Some — модальный режим).
    invalid_prompt: Option<InvalidPromptState>,
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
            status_error: false,
            editor: None,
            meta_editor: None,
            invalid_prompt: None,
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
            Update::TrackPatch(info) => self.apply_track_patch(info),
            Update::Error(msg) => self.set_error(msg),
            Update::Notice(msg) => self.set_warn(msg),
        }
    }

    /// применяет обновлённую инфу трека на месте: ко всем совпадениям по `id` в
    /// текущем списке и в предпросмотрах плейлистов (громкость не трогаем — она
    /// живёт своей жизнью в проигрывателе).
    fn apply_track_patch(&mut self, info: TrackInfo) {
        let patch = |t: &mut TrackInfo| {
            if t.id == info.id {
                let vol = t.volume;
                *t = info.clone();
                t.volume = vol;
            }
        };
        self.model.tracks.iter_mut().for_each(patch);
        for entry in &mut self.model.playlists {
            entry.tracks.iter_mut().for_each(patch);
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
        // редактор метаданных — тоже модальный.
        if self.meta_editor.is_some() {
            self.handle_meta_editor_key(key);
            return false;
        }
        // меню недействительного трека — модальное.
        if self.invalid_prompt.is_some() {
            self.handle_invalid_prompt_key(key);
            return false;
        }
        if self.command.is_some() {
            return self.handle_command_key(key);
        }
        self.status = None;
        self.status_warn = false;
        self.status_error = false;
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

    fn handle_meta_editor_key(&mut self, key: KeyEvent) {
        let Some(editor) = self.meta_editor.as_mut() else {
            return;
        };
        match editor.handle_key(key) {
            MetaEditorOutcome::None => {}
            MetaEditorOutcome::Cancel => self.meta_editor = None,
            MetaEditorOutcome::Save(edit) => self.finish_meta_editor(edit),
        }
    }

    fn handle_invalid_prompt_key(&mut self, key: KeyEvent) {
        let Some(prompt) = self.invalid_prompt.as_mut() else {
            return;
        };
        match prompt.handle_key(key) {
            InvalidOutcome::None => {}
            InvalidOutcome::Cancel => self.invalid_prompt = None,
            InvalidOutcome::SetPath { id, path } => {
                self.invalid_prompt = None;
                let _ = self.controls.send(Control::SetPath { id, path });
                self.set_warn("new path sent".to_string());
            }
            InvalidOutcome::Remove { id } => {
                self.invalid_prompt = None;
                let _ = self.controls.send(Control::RemoveTrack(id));
                self.set_warn("track removed from index".to_string());
            }
        }
    }

    /// открыть меню действий для недействительного трека по его id.
    fn open_invalid_prompt(&mut self, id: i64) {
        let title = self
            .model
            .tracks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        self.invalid_prompt = Some(InvalidPromptState::new(id, title));
    }

    /// закрывает редактор метаданных и рассылает изменения полей: пустое поле
    /// пропускается (значение не меняется). Списки разбираются по запятым.
    fn finish_meta_editor(&mut self, edit: MetaEdit) {
        self.meta_editor = None;
        let id = edit.id;
        if !edit.title.is_empty() {
            let _ = self.controls.send(Control::SetTitle {
                id,
                title: edit.title,
            });
        }
        if !edit.artists.is_empty() {
            let _ = self.controls.send(Control::SetArtists {
                id,
                artists: split_artists(&edit.artists),
            });
        }
        if !edit.album.is_empty() {
            let _ = self.controls.send(Control::SetAlbum {
                id,
                album: edit.album,
            });
        }
        if !edit.genres.is_empty() {
            let _ = self.controls.send(Control::SetGenres {
                id,
                genres: split_artists(&edit.genres),
            });
        }
        if !edit.filename.is_empty() {
            let _ = self.controls.send(Control::RenameFile {
                id,
                name: edit.filename,
            });
        }
        if !edit.cover.is_empty() {
            let _ = self.controls.send(Control::SetCover {
                id,
                path: edit.cover,
            });
        }
        if !edit.color.is_empty() {
            // валидация имени цвета до отправки.
            if COLOR_NAMES.contains(&edit.color.to_ascii_lowercase().as_str()) {
                let _ = self.controls.send(Control::SetColor {
                    id,
                    color: edit.color,
                });
            } else {
                self.set_error(format!("unknown color: {}", edit.color));
                return;
            }
        }
        if !edit.label.is_empty() {
            let _ = self.controls.send(Control::SetLabel {
                id,
                label: edit.label,
            });
        }
        self.set_warn("metadata edit sent".to_string());
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
            let _ = self.controls.send(Control::PlayTemp { ids, start: 0 });
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
        self.status_error = false;
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
                    let _ = self.controls.send(Control::LoadPlaylist {
                        name: arg.to_string(),
                        start: 0,
                    });
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
            // индексация каталога и проверка наличия файлов — глобальные команды.
            "scan" => {
                if arg.is_empty() {
                    self.status = Some("usage: :scan <directory>".into());
                } else {
                    let _ = self.controls.send(Control::Scan(arg.to_string()));
                    self.set_warn(format!("scanning: {arg}"));
                }
            }
            "check" => {
                if arg.is_empty() {
                    self.status = Some("usage: :check all | <playlist name>".into());
                } else if arg.eq_ignore_ascii_case("all") {
                    let _ = self.controls.send(Control::Check(CheckTarget::All));
                    self.set_warn("checking whole library...".into());
                } else {
                    let _ = self
                        .controls
                        .send(Control::Check(CheckTarget::Playlist(arg.to_string())));
                    self.set_warn(format!("checking playlist: {arg}"));
                }
            }
            // удалить из индекса все недействительные (красные) треки.
            "purge" => {
                let _ = self.controls.send(Control::PurgeInvalid);
                self.set_warn("purging invalid tracks...".into());
            }
            // редактирование метаданных — только на вкладке SONG (индекс 1).
            "title" | "artist" | "artists" | "album" | "genres" | "color" | "label"
            | "filename" | "cover" | "covertag" | "setpath" => {
                self.exec_song_command(head, arg);
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

    /// команды правки метаданных, доступные только на вкладке SONG (индекс 1).
    /// Действуют на трек под курсором списка (текущий индекс модели).
    fn exec_song_command(&mut self, head: &str, arg: &str) {
        if self.current_tab != 1 {
            self.status = Some("available only on the SONG tab".into());
            return;
        }
        let Some(track) = self.model.tracks.get(self.model.current) else {
            self.status = Some("no track selected".into());
            return;
        };
        if track.id < 0 {
            self.status = Some("this track is not in the library".into());
            return;
        }
        let id = track.id;
        // album/genres/color/label очищаются пустым аргументом; остальным нужен
        // непустой аргумент.
        let clearable = matches!(head, "album" | "genres" | "color" | "label");
        if arg.is_empty() && !clearable {
            self.status = Some(format!("usage: :{head} <value>"));
            return;
        }
        // валидация имени цвета.
        if head == "color"
            && !arg.is_empty()
            && !COLOR_NAMES.contains(&arg.to_ascii_lowercase().as_str())
        {
            self.status = Some(format!("unknown color: {arg} (try: {})", COLOR_NAMES.join("/")));
            return;
        }
        let control = match head {
            "title" => Control::SetTitle {
                id,
                title: arg.to_string(),
            },
            "artist" | "artists" => Control::SetArtists {
                id,
                artists: split_artists(arg),
            },
            "album" => Control::SetAlbum {
                id,
                album: arg.to_string(),
            },
            "genres" => Control::SetGenres {
                id,
                genres: split_artists(arg),
            },
            "color" => Control::SetColor {
                id,
                color: arg.to_string(),
            },
            "label" => Control::SetLabel {
                id,
                label: arg.to_string(),
            },
            "filename" => Control::RenameFile {
                id,
                name: arg.to_string(),
            },
            "cover" => Control::SetCover {
                id,
                path: arg.to_string(),
            },
            "covertag" => Control::SetCoverTag {
                id,
                path: arg.to_string(),
            },
            "setpath" => Control::SetPath {
                id,
                path: arg.to_string(),
            },
            _ => return,
        };
        let _ = self.controls.send(control);
        self.set_warn(format!("{head} edit sent"));
    }

    fn dispatch(&mut self, action: Action) {
        let control = match action {
            Action::LoadPlaylist { name, start } => Control::LoadPlaylist { name, start },
            Action::LoadPool { start } => Control::LoadPool { start },
            Action::PlayTemp { ids, start } => Control::PlayTemp { ids, start },
            Action::SelectSong(i) => Control::Select(i),
            Action::NewPlaylist => {
                self.open_create();
                return;
            }
            Action::EditPlaylist(idx) => {
                self.open_edit(idx);
                return;
            }
            Action::EditMeta(idx) => {
                self.open_meta_editor(idx);
                return;
            }
            Action::InvalidAction(id) => {
                self.open_invalid_prompt(id);
                return;
            }
        };
        let _ = self.controls.send(control);
    }

    /// открыть редактор метаданных трека по индексу в текущем списке. Треки вне
    /// бд (id < 0) редактировать нельзя.
    fn open_meta_editor(&mut self, idx: usize) {
        let Some(track) = self.model.tracks.get(idx) else {
            return;
        };
        if track.id < 0 {
            self.set_warn("this track is not in the library (cannot edit)".to_string());
            return;
        }
        // имя файла в TUI не хранится: поле остаётся пустым (пустое = не менять).
        self.meta_editor = Some(MetaEditorState::new(
            track.id,
            track.title.clone(),
            track.artists.clone(),
            track.album.clone().unwrap_or_default(),
            track.genres.join(", "),
            track.cover.clone().unwrap_or_default(),
            track.color.clone().unwrap_or_default(),
            track.user_label.clone().unwrap_or_default(),
        ));
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
        self.status_error = false;
    }

    fn set_error(&mut self, msg: String) {
        self.status = Some(msg);
        self.status_error = true;
        self.status_warn = false;
    }

    fn draw(&mut self, frame: &mut Frame) {
        // редактор — на весь экран.
        if let Some(editor) = &self.editor {
            editor.render(frame, frame.area());
            return;
        }
        // редактор метаданных — тоже на весь экран.
        if let Some(editor) = &self.meta_editor {
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
                let st = if self.status_error {
                    Style::new().fg(Color::Red)
                } else if self.status_warn {
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
        // меню недействительного трека — поверх всего.
        if let Some(prompt) = &self.invalid_prompt {
            prompt.render(frame, frame.area());
        }
    }
}

/// разбивает список артистов по запятым, отбрасывая пустые части.
fn split_artists(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// парсит "80" → 0.8 (проценты 0..=100 в долю 0.0..=1.0).
fn parse_percent(s: &str) -> Option<f32> {
    s.trim()
        .parse::<f32>()
        .ok()
        .map(|n| (n / 100.0).clamp(0.0, 1.0))
}
