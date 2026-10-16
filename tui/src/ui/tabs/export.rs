//! вкладка EXPORT (`:export`): упаковка библиотеки / выбранных плейлистов /
//! выбранных треков в файл пакета. Слева форма (путь, что, флаги), справа —
//! список выбора (для плейлистов/треков), снизу — журнал сообщений.
//! Путь по умолчанию — каталог, из которого запущено приложение; если путь —
//! каталог, файл создаётся в нём с именем `dream_player_<unix-time>.dplr`.
//! Существующий файл не перезаписывается. Запуск — Shift+W.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
};

use crate::ffi::cdc::{self, Export, Flags};
use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Form, FormField, Hint, ListLine, ListView, MessageLog, two_pane},
};

const WHAT: &[&str] = &["library", "playlists", "tracks"];
const F_PATH: usize = 0;
const F_WHAT: usize = 1;
const F_AUDIO: usize = 2;
const F_NAMES: usize = 3;
const F_COVERS: usize = 4;

/// пункт списка выбора: плейлист — по имени (его идентификатор), трек — по id.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Item {
    Playlist(String),
    Track(i64),
}

pub struct ExportTab {
    form: Form,
    list: ListView,
    /// фокус на списке выбора (иначе — на форме).
    in_list: bool,
    /// отмеченные пункты (плейлисты или треки — по режиму).
    checked: HashSet<Item>,
    log: MessageLog,
}

impl ExportTab {
    pub fn new() -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let form = Form::new(vec![
            FormField::text("PATH", &cwd),
            FormField::choice("EXPORT", WHAT),
            FormField::toggle("AUDIO", true),
            FormField::toggle("FILE NAMES", true),
            FormField::toggle("COVERS", true),
        ]);
        let mut log = MessageLog::default();
        log.info("set the path and options, then Shift+W to export");
        Self {
            form,
            list: ListView::default(),
            in_list: false,
            checked: HashSet::new(),
            log,
        }
    }

    fn what(&self) -> usize {
        self.form.choice(F_WHAT)
    }

    /// (пункт, подпись) для списка выбора текущего режима.
    fn items(&self, model: &Model) -> Vec<(Item, String)> {
        match self.what() {
            1 => model
                .playlists
                .iter()
                .filter(|p| !p.pool)
                .map(|p| {
                    let label = format!("{} ({})", p.name, p.tracks.len());
                    (Item::Playlist(p.name.clone()), label)
                })
                .collect(),
            2 => model
                .pool()
                .iter()
                .filter_map(|t| {
                    let label = format!("{} — {}", t.title, t.artists_line());
                    Some((Item::Track(t.id?), label))
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// режим сменился: выбор сбрасывается, обложки без плейлистов недоступны.
    fn on_what_changed(&mut self) {
        self.checked.clear();
        self.list = ListView::default();
        self.form.fields[F_COVERS].disabled = self.what() == 2;
    }

    /// итоговый путь файла: каталог -> файл с именем по времени.
    fn target(&self) -> Result<PathBuf, String> {
        let raw = self.form.text(F_PATH);
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("path is empty".into());
        }
        let path = PathBuf::from(raw);
        if path.is_dir() {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            return Ok(path.join(format!("dream_player_{secs}.dplr")));
        }
        Ok(path)
    }

    fn run(&mut self, model: &Model) {
        let path = match self.target() {
            Ok(p) => p,
            Err(e) => return self.log.error(e),
        };
        // отмеченные — в порядке списка.
        let picked: Vec<Item> = self
            .items(model)
            .into_iter()
            .map(|(item, _)| item)
            .filter(|item| self.checked.contains(item))
            .collect();
        let names: Vec<String> = picked
            .iter()
            .filter_map(|i| match i {
                Item::Playlist(n) => Some(n.clone()),
                Item::Track(_) => None,
            })
            .collect();
        let ids: Vec<i64> = picked
            .iter()
            .filter_map(|i| match i {
                Item::Track(id) => Some(*id),
                Item::Playlist(_) => None,
            })
            .collect();
        let what = match self.what() {
            0 => Export::Library,
            1 => Export::Playlists(&names),
            _ => Export::Tracks(&ids),
        };
        if self.what() != 0 && picked.is_empty() {
            return self.log.warn("nothing selected: mark items in the list with Enter");
        }
        let flags = Flags {
            audio: self.form.toggle(F_AUDIO),
            names: self.form.toggle(F_NAMES),
            covers: self.form.toggle(F_COVERS),
        };
        self.log.info(format!(
            "packing {} (audio: {}, names: {}, covers: {})...",
            match self.what() {
                0 => "the whole library".to_string(),
                n => format!("{} {}", picked.len(), WHAT[n]),
            },
            flags.audio,
            flags.names,
            flags.covers
        ));
        let bytes = match cdc::pack(what, flags) {
            Ok(b) => b,
            Err(st) => return self.log.error(format!("export failed: {}", cdc::describe(st))),
        };
        // существующий файл не перезаписываем.
        let res = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut f| f.write_all(&bytes));
        match res {
            Ok(()) => self.log.ok(format!(
                "exported {} to {}",
                human_size(bytes.len()),
                path.display()
            )),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => self
                .log
                .error(format!("file already exists: {}", path.display())),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                self.log.error(format!("failed to write {}: {e}", path.display()))
            }
        }
    }
}

impl Tab for ExportTab {
    fn title(&self) -> String {
        "EXPORT".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model) {
        let [top, bottom] =
            Layout::vertical([Constraint::Min(7), Constraint::Length(8)]).areas(area);
        let (left, right) = two_pane(top, 50);
        self.form.render(frame, left, "export", !self.in_list);

        let items = self.items(model);
        let title = match self.what() {
            0 => "whole library — nothing to select".to_string(),
            n => format!("{} ({} selected)", WHAT[n], self.checked.len()),
        };
        let lines = items
            .iter()
            .map(|(item, label)| {
                let mark = if self.checked.contains(item) { "[x]" } else { "[ ]" };
                ListLine::new(format!("{mark} {label}"))
            })
            .collect();
        self.list.render(frame, right, &title, lines, self.in_list);
        self.log.render(frame, bottom);
    }

    fn on_key(&mut self, key: KeyEvent, model: &Model) -> TabEvent {
        if self.form.editing {
            self.form.edit_key(key);
            return TabEvent::None;
        }
        match key.code {
            KeyCode::Char('W') => self.run(model),
            KeyCode::Tab => self.in_list = !self.in_list && self.what() != 0,
            KeyCode::Char('l') | KeyCode::Right if self.what() != 0 => self.in_list = true,
            KeyCode::Char('h') | KeyCode::Left => self.in_list = false,
            KeyCode::Char('j') | KeyCode::Down if self.in_list => self.list.down(),
            KeyCode::Char('k') | KeyCode::Up if self.in_list => self.list.up(),
            KeyCode::Char('j') | KeyCode::Down => self.form.down(),
            KeyCode::Char('k') | KeyCode::Up => self.form.up(),
            KeyCode::Enter if self.in_list => {
                if let Some((item, _)) = self.items(model).get(self.list.selected())
                    && !self.checked.remove(item)
                {
                    self.checked.insert(item.clone());
                }
            }
            KeyCode::Char('a') if self.in_list => {
                // отметить все / снять все.
                let all: Vec<Item> = self.items(model).into_iter().map(|(i, _)| i).collect();
                if all.iter().all(|i| self.checked.contains(i)) {
                    self.checked.clear();
                } else {
                    self.checked.extend(all);
                }
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                let changed = self.form.activate();
                if changed && self.form.sel == F_WHAT {
                    self.on_what_changed();
                }
            }
            _ => {}
        }
        TabEvent::None
    }

    fn hints(&self) -> Vec<Hint> {
        match self.in_list {
            true => vec![("jk", "nav"), ("Ent", "mark"), ("a", "all"), ("h", "form"), ("W", "export")],
            false => vec![("jk", "fld"), ("Ent", "edit"), ("l", "list"), ("W", "export")],
        }
    }

    fn wants_input(&self) -> bool {
        self.form.editing
    }
}

/// размер в человекочитаемом виде.
pub(super) fn human_size(n: usize) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    match n {
        n if n < 1024 => format!("{n} B"),
        n if (n as f64) < MB => format!("{:.1} KiB", n as f64 / 1024.0),
        n => format!("{:.1} MiB", n as f64 / MB),
    }
}
