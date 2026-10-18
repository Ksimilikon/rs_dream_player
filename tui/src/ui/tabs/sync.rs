//! SYNC tab (`:sync ...`): library sync with another device and the list of
//! paired devices. The sync itself runs in a worker thread (see
//! [`crate::sync`]); the tab shows its log. Commands start / cancel it, keys
//! manage the paired list: `r` rename (the name is for viewing only), `D`
//! forget, `c` cancel the running sync.

use std::path::PathBuf;

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::Paragraph,
};

use crate::ffi::{self, con::Paired};
use crate::sync::{Mode, Msg, Target, Worker};
use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Hint, ListLine, ListView, MessageLog, TextField},
};

pub struct SyncTab {
    music_dir: PathBuf,
    worker: Option<Worker>,
    log: MessageLog,
    list: ListView,
    paired: Vec<Paired>,
    /// fingerprint of this device's key.
    own: String,
    /// the new name of the selected paired device being typed.
    rename: Option<TextField>,
}

impl SyncTab {
    pub fn new(music_dir: PathBuf) -> Self {
        let own = match ffi::con::pub_key() {
            Ok(key) => ffi::con::fingerprint(&key),
            Err(st) => format!("unavailable: {}", ffi::con::describe(st)),
        };
        let mut log = MessageLog::default();
        log.info(":sync listen | tcp <ip> | adb [listen] | bt listen | bt <MAC> — see :help");
        let mut tab = Self {
            music_dir,
            worker: None,
            log,
            list: ListView::default(),
            paired: Vec::new(),
            own,
            rename: None,
        };
        tab.reload();
        tab
    }

    fn reload(&mut self) {
        self.paired = ffi::con::paired();
        self.list.set_len(self.paired.len());
    }

    fn running(&self) -> bool {
        self.worker.as_ref().is_some_and(Worker::running)
    }

    fn selected(&self) -> Option<&Paired> {
        self.paired.get(self.list.selected())
    }

    fn rename_key(&mut self, key: KeyEvent) {
        let Some(buf) = self.rename.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.rename = None,
            KeyCode::Enter => {
                let name = buf.text().trim().to_owned();
                self.rename = None;
                if let Some(id) = self.selected().map(|p| p.id) {
                    match ffi::con::rename(id, &name) {
                        Ok(()) => self.log.ok(format!("#{id} renamed to {name}")),
                        Err(st) => self
                            .log
                            .error(format!("rename: {}", ffi::con::describe(st))),
                    }
                    self.reload();
                }
            }
            KeyCode::Backspace => buf.backspace(),
            KeyCode::Delete => buf.delete(),
            KeyCode::Left => buf.left(),
            KeyCode::Right => buf.right(),
            KeyCode::Home => buf.home(),
            KeyCode::End => buf.end(),
            KeyCode::Char(c) => buf.insert(c),
            _ => {}
        }
    }
}

impl Tab for SyncTab {
    fn title(&self) -> String {
        "SYNC".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        let list_h = (self.paired.len() as u16 + 2).clamp(3, 10);
        let [header, list, log] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(list_h),
            Constraint::Min(4),
        ])
        .areas(area);

        let state = match self.running() {
            true => "running (c: cancel)",
            false => "idle",
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    "LIBRARY SYNC",
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ),
                Line::from(format!("this device: {}   sync: {state}", self.own)),
            ]),
            header,
        );

        let lines: Vec<ListLine> = self
            .paired
            .iter()
            .map(|p| {
                ListLine::new(format!(
                    "#{:<4} {:<24} {}",
                    p.id,
                    p.name,
                    ffi::con::fingerprint(&p.key)
                ))
            })
            .collect();
        self.list.render(frame, list, "paired devices", lines, true);
        self.log.render(frame, log);
    }

    fn on_key(&mut self, key: KeyEvent, _model: &Model) -> TabEvent {
        if self.rename.is_some() {
            self.rename_key(key);
            return TabEvent::None;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.list.down(),
            KeyCode::Char('k') | KeyCode::Up => self.list.up(),
            KeyCode::Char('r') => {
                if let Some(p) = self.selected() {
                    self.rename = Some(TextField::new(&p.name));
                }
            }
            KeyCode::Char('D') => {
                if let Some(id) = self.selected().map(|p| p.id) {
                    match ffi::con::forget(id) {
                        Ok(()) => self.log.ok(format!("#{id} forgotten")),
                        Err(st) => self
                            .log
                            .error(format!("forget: {}", ffi::con::describe(st))),
                    }
                    self.reload();
                }
            }
            KeyCode::Char('c') => {
                if let Some(w) = &self.worker {
                    w.cancel();
                }
            }
            _ => {}
        }
        TabEvent::None
    }

    fn hints(&self) -> Vec<Hint> {
        match self.rename {
            Some(_) => vec![("Ent", "save"), ("Esc", "cancel")],
            None => vec![
                ("jk", "nav"),
                ("r", "rename"),
                ("D", "forget"),
                ("c", "cancel"),
            ],
        }
    }

    fn wants_input(&self) -> bool {
        self.rename.is_some()
    }

    fn prompt(&self) -> Option<(String, usize)> {
        const LABEL: &str = "name: ";
        self.rename
            .as_ref()
            .map(|b| (format!("{LABEL}{}", b.text()), LABEL.len() + b.caret()))
    }

    fn command(&mut self, args: &str) -> Result<(), String> {
        match args.trim() {
            "" => Ok(()),
            "cancel" => match &self.worker {
                Some(w) if w.running() => {
                    w.cancel();
                    Ok(())
                }
                _ => Err("no sync is running".into()),
            },
            args => {
                if self.running() {
                    return Err("a sync is already running (:sync cancel)".into());
                }
                let target = Target::parse(args)?;
                self.worker = Some(Worker::start(target, Mode::Sync, self.music_dir.clone()));
                Ok(())
            }
        }
    }

    fn poll(&mut self) -> TabEvent {
        let Some(worker) = &self.worker else {
            return TabEvent::None;
        };
        let mut event = TabEvent::None;
        let mut finished = false;
        for msg in worker.poll() {
            match msg {
                Msg::Info(s) => self.log.info(s),
                Msg::Ok(s) => self.log.ok(s),
                Msg::Warn(s) => self.log.warn(s),
                Msg::Error(s) => self.log.error(s),
                Msg::LibraryChanged => event = TabEvent::LibraryChanged,
                Msg::Finished { .. } => finished = true,
            }
        }
        if finished {
            // a new device may have been paired.
            self.reload();
        }
        event
    }
}
