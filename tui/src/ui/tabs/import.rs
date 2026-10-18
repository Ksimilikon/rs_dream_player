//! IMPORT tab (`:import`): imports a packet file into the library. The TUI is
//! the frontend of the core's two-step import here: the core hands out the
//! files (songs and covers), the tab writes them into the SAVE TO directory
//! (default — the music directory) and passes the resulting paths to the
//! core. The default packet path is the directory the app was launched from.
//! Run — Shift+W.
//!
//! the same import can come straight from another device (Shift+R): RECEIVE
//! takes the connection like `:sync` (`listen`, `tcp <ip>`, `adb`,
//! `bt <MAC>`, ...), the other side sends from its EXPORT tab. FORCE applies;
//! a received song replaces a local file with the same name, other files go to
//! SAVE TO.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use api::cdc::FFICdcStatus;
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
};

use super::export::human_size;
use crate::ffi::cdc::{self, PreparedImport};
use crate::sync::{Mode, Msg, Target, Worker};
use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Form, FormField, Hint, MessageLog},
};

const F_PACKET: usize = 0;
const F_SAVE_TO: usize = 1;
const F_FORCE: usize = 2;
const F_RECEIVE: usize = 3;

pub struct ImportTab {
    form: Form,
    log: MessageLog,
    /// a running receive from another device.
    worker: Option<Worker>,
}

impl ImportTab {
    pub fn new(music_dir: &Path) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let form = Form::new(vec![
            FormField::text("PACKET", &cwd),
            FormField::text("SAVE TO", &music_dir.to_string_lossy()),
            FormField::toggle("FORCE", false),
            FormField::text("RECEIVE", "listen"),
        ]);
        let mut log = MessageLog::default();
        log.info("set the packet file and target dir, then Shift+W to import");
        log.info("or Shift+R to receive from a device (RECEIVE: listen | tcp <ip> | adb | bt <MAC>)");
        Self {
            form,
            log,
            worker: None,
        }
    }

    /// receives from another device that sends from its EXPORT tab.
    fn receive(&mut self) {
        if self.worker.as_ref().is_some_and(Worker::running) {
            return self.log.warn("a receive is already running");
        }
        let target = match Target::parse(self.form.text(F_RECEIVE).trim()) {
            Ok(t) => t,
            Err(e) => return self.log.error(e),
        };
        let save_to = PathBuf::from(self.form.text(F_SAVE_TO).trim());
        let force = self.form.toggle(F_FORCE);
        self.log.info(format!(
            "receiving into {}{}...",
            save_to.display(),
            if force { " (force)" } else { "" }
        ));
        self.worker = Some(Worker::start(target, Mode::Receive { force }, save_to));
    }

    /// the whole import; `true` — the library changed.
    fn run(&mut self) -> bool {
        let packet = PathBuf::from(self.form.text(F_PACKET).trim());
        let save_to = PathBuf::from(self.form.text(F_SAVE_TO).trim());
        let force = self.form.toggle(F_FORCE);

        if packet.is_dir() {
            self.log.error(format!("packet path is a directory: {}", packet.display()));
            return false;
        }
        let bytes = match fs::read(&packet) {
            Ok(b) => b,
            Err(e) => {
                self.log.error(format!("failed to read {}: {e}", packet.display()));
                return false;
            }
        };
        self.log.info(format!(
            "read {} from {}{}",
            human_size(bytes.len()),
            packet.display(),
            if force { " (force)" } else { "" }
        ));

        // step 1: the core checks the packet and the dates.
        let prepared = match PreparedImport::prepare(&bytes, force) {
            Ok(p) => p,
            Err(FFICdcStatus::Outdated) => {
                self.log.warn(cdc::describe(FFICdcStatus::Outdated));
                return false;
            }
            Err(st) => {
                self.log.error(format!("import rejected: {}", cdc::describe(st)));
                return false;
            }
        };
        drop(bytes);

        // step 2: we write the files ourselves.
        let files = prepared.files();
        let (songs, covers) = files.iter().fold((0, 0), |(s, c), f| match f.cover {
            true => (s, c + 1),
            false => (s + 1, c),
        });
        self.log.info(format!(
            "{songs} new song(s), {covers} cover(s) to write into {}",
            save_to.display()
        ));
        let mut written: Vec<PathBuf> = Vec::with_capacity(files.len());
        for f in &files {
            match write_unique(&save_to, &f.name, f.data) {
                Ok(path) => written.push(path),
                Err(e) => {
                    self.log.error(format!("failed to write {}: {e}", f.name));
                    remove_all(&written);
                    // `prepared` is cancelled in Drop.
                    return false;
                }
            }
        }
        drop(files);

        // step 3: the core saves the paths.
        let paths: Vec<Option<&Path>> = written.iter().map(|p| Some(p.as_path())).collect();
        match prepared.commit(&paths) {
            Ok(()) => {
                self.log.ok(format!("imported: {songs} song(s), {covers} cover(s)"));
                true
            }
            Err(st) => {
                remove_all(&written);
                match st {
                    FFICdcStatus::Outdated => self.log.warn(format!(
                        "{} (library changed during import)",
                        cdc::describe(st)
                    )),
                    _ => self.log.error(format!("import failed: {}", cdc::describe(st))),
                }
                false
            }
        }
    }
}

impl Tab for ImportTab {
    fn title(&self) -> String {
        "IMPORT".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        let [top, bottom] =
            Layout::vertical([Constraint::Length(6), Constraint::Min(4)]).areas(area);
        self.form.render(frame, top, "import", true);
        self.log.render(frame, bottom);
    }

    fn on_key(&mut self, key: KeyEvent, _model: &Model) -> TabEvent {
        if self.form.editing {
            self.form.edit_key(key);
            return TabEvent::None;
        }
        match key.code {
            KeyCode::Char('W') => {
                if self.run() {
                    return TabEvent::LibraryChanged;
                }
            }
            KeyCode::Char('R') => self.receive(),
            KeyCode::Char('c') => {
                if let Some(w) = &self.worker {
                    w.cancel();
                }
            }
            KeyCode::Char('j') | KeyCode::Down => self.form.down(),
            KeyCode::Char('k') | KeyCode::Up => self.form.up(),
            KeyCode::Enter | KeyCode::Char('i') => {
                self.form.activate();
            }
            _ => {}
        }
        TabEvent::None
    }

    fn hints(&self) -> Vec<Hint> {
        vec![("jk", "fld"), ("Ent", "edit"), ("W", "import"), ("R", "receive")]
    }

    fn poll(&mut self) -> TabEvent {
        let mut event = TabEvent::None;
        if let Some(w) = &self.worker {
            for msg in w.poll() {
                match msg {
                    Msg::Info(s) => self.log.info(s),
                    Msg::Ok(s) => self.log.ok(s),
                    Msg::Warn(s) => self.log.warn(s),
                    Msg::Error(s) => self.log.error(s),
                    Msg::LibraryChanged => event = TabEvent::LibraryChanged,
                    Msg::Finished { .. } => {}
                }
            }
        }
        event
    }

    fn wants_input(&self) -> bool {
        self.form.editing
    }
}

/// writes `data` into `dir/name` without overwriting existing files: if the
/// name is taken — `name (1).ext`, `name (2).ext`, ...
pub(crate) fn write_unique(dir: &Path, name: &str, data: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let (path, mut file) = create_unique(dir, name)?;
    if let Err(e) = file.write_all(data) {
        let _ = fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

fn create_unique(dir: &Path, name: &str) -> io::Result<(PathBuf, File)> {
    let base = Path::new(name);
    let stem = base.file_stem().map_or(name.into(), |s| s.to_string_lossy());
    let ext = base
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 0..10_000 {
        let candidate = match n {
            0 => name.to_owned(),
            _ => format!("{stem} ({n}){ext}"),
        };
        let path = dir.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::ErrorKind::AlreadyExists.into())
}

fn remove_all(paths: &[PathBuf]) {
    for p in paths {
        let _ = fs::remove_file(p);
    }
}
