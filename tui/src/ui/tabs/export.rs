//! EXPORT tab (`:export`): packs the library / selected playlists / selected
//! tracks into a packet file. The form on the left (path, what, flags), the
//! selection list on the right (for playlists / tracks), the message log at
//! the bottom. The default path is the directory the app was launched from;
//! if the path is a directory, the file is created in it as
//! `dream_player_<unix-time>.dplr`. An existing file is never overwritten.
//! Run — Shift+W.
//!
//! the same selection and flags can be sent straight to another device
//! instead (Shift+S): SEND TO takes the connection like `:sync` (`listen`,
//! `tcp <ip>`, `adb`, `bt <MAC>`, ...); the other side receives on its IMPORT
//! tab.

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
use crate::sync::{Mode, Msg, Selection, Target, Worker};
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
const F_SEND_TO: usize = 5;

/// a selection list item: a playlist — by name (its identifier), a track — by id.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Item {
    Playlist(String),
    Track(i64),
}

pub struct ExportTab {
    form: Form,
    list: ListView,
    /// focus is on the selection list (otherwise on the form).
    in_list: bool,
    /// marked items (playlists or tracks — depending on the mode).
    checked: HashSet<Item>,
    log: MessageLog,
    /// a running send to another device.
    worker: Option<Worker>,
    music_dir: PathBuf,
}

impl ExportTab {
    pub fn new(music_dir: PathBuf) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let form = Form::new(vec![
            FormField::text("PATH", &cwd),
            FormField::choice("EXPORT", WHAT),
            FormField::toggle("AUDIO", true),
            FormField::toggle("FILE NAMES", true),
            FormField::toggle("COVERS", true),
            FormField::text("SEND TO", "listen"),
        ]);
        let mut log = MessageLog::default();
        log.info("set the path and options, then Shift+W to export");
        log.info("or Shift+S to send to a device (SEND TO: listen | tcp <ip> | adb | bt <MAC>)");
        Self {
            form,
            list: ListView::default(),
            in_list: false,
            checked: HashSet::new(),
            log,
            worker: None,
            music_dir,
        }
    }

    fn what(&self) -> usize {
        self.form.choice(F_WHAT)
    }

    /// (item, label) pairs for the selection list of the current mode.
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

    /// the mode changed: the selection is reset.
    fn on_what_changed(&mut self) {
        self.checked.clear();
        self.list = ListView::default();
    }

    /// the final file path: a directory -> a file named by the time.
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

    /// the marked selection and the flags; `None` — nothing marked (logged).
    fn picked(&mut self, model: &Model) -> Option<(Selection, Flags)> {
        // marked items — in list order.
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
        if self.what() != 0 && picked.is_empty() {
            self.log.warn("nothing selected: mark items in the list with Enter");
            return None;
        }
        let what = match self.what() {
            0 => Selection::Library,
            1 => Selection::Playlists(names),
            _ => Selection::Tracks(ids),
        };
        let flags = Flags {
            audio: self.form.toggle(F_AUDIO),
            names: self.form.toggle(F_NAMES),
            // song and playlist covers: tracks have their covers too.
            covers: self.form.toggle(F_COVERS),
        };
        Some((what, flags))
    }

    /// "the whole library" / "3 playlists" + the flags, for the log.
    fn describe(what: &Selection, flags: Flags) -> String {
        let what = match what {
            Selection::Library => "the whole library".to_string(),
            Selection::Playlists(n) => format!("{} playlists", n.len()),
            Selection::Tracks(n) => format!("{} tracks", n.len()),
        };
        format!(
            "{what} (audio: {}, names: {}, covers: {})",
            flags.audio, flags.names, flags.covers
        )
    }

    /// sends the selection to another device.
    fn send(&mut self, model: &Model) {
        if self.worker.as_ref().is_some_and(Worker::running) {
            return self.log.warn("a send is already running");
        }
        let target = match Target::parse(self.form.text(F_SEND_TO).trim()) {
            Ok(t) => t,
            Err(e) => return self.log.error(e),
        };
        let Some((what, flags)) = self.picked(model) else {
            return;
        };
        self.log.info(format!("sending {}...", Self::describe(&what, flags)));
        let mode = Mode::Send { what, flags };
        self.worker = Some(Worker::start(target, mode, self.music_dir.clone()));
    }

    fn run(&mut self, model: &Model) {
        let path = match self.target() {
            Ok(p) => p,
            Err(e) => return self.log.error(e),
        };
        let Some((what, flags)) = self.picked(model) else {
            return;
        };
        self.log.info(format!("packing {}...", Self::describe(&what, flags)));
        let export = match &what {
            Selection::Library => Export::Library,
            Selection::Playlists(names) => Export::Playlists(names),
            Selection::Tracks(ids) => Export::Tracks(ids),
        };
        let bytes = match cdc::pack(export, flags) {
            Ok(b) => b,
            Err(st) => return self.log.error(format!("export failed: {}", cdc::describe(st))),
        };
        // don't overwrite an existing file.
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
            Layout::vertical([Constraint::Min(8), Constraint::Length(8)]).areas(area);
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
            KeyCode::Char('S') => self.send(model),
            KeyCode::Char('c') => {
                if let Some(w) = &self.worker {
                    w.cancel();
                }
            }
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
                // mark all / unmark all.
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
            true => vec![
                ("jk", "nav"),
                ("Ent", "mark"),
                ("a", "all"),
                ("h", "form"),
                ("W", "export"),
                ("S", "send"),
            ],
            false => vec![("jk", "fld"), ("Ent", "edit"), ("l", "list"), ("W", "export"), ("S", "send")],
        }
    }

    fn poll(&mut self) -> TabEvent {
        if let Some(w) = &self.worker {
            for msg in w.poll() {
                match msg {
                    Msg::Info(s) => self.log.info(s),
                    Msg::Ok(s) => self.log.ok(s),
                    Msg::Warn(s) => self.log.warn(s),
                    Msg::Error(s) => self.log.error(s),
                    Msg::LibraryChanged | Msg::Finished { .. } => {}
                }
            }
        }
        TabEvent::None
    }

    fn wants_input(&self) -> bool {
        self.form.editing
    }
}

/// size in human-readable form.
pub(super) fn human_size(n: usize) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    match n {
        n if n < 1024 => format!("{n} B"),
        n if (n as f64) < MB => format!("{:.1} KiB", n as f64 / 1024.0),
        n => format!("{:.1} MiB", n as f64 / MB),
    }
}
