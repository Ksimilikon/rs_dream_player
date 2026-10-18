//! HELP tab (`:help`): controls reference. Closed with Shift+X (handled by
//! the app as a transient tab). j/k — scroll.

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::ui::{HELP_RED, Model, Tab, TabEvent, layouts::Hint};

const HELP: &[(&str, &str)] = &[
    ("", "GLOBAL"),
    ("Shift+H / Shift+L", "switch tabs"),
    ("space", "play / pause"),
    ("< / >", "previous / next track"),
    ("- / +", "master volume down / up"),
    ("[ / ]", "track volume down / up"),
    (": ", "command line (vim-style)"),
    ("Ctrl+L", "redraw screen"),
    ("Shift+X", "close current tab (any except PLAYLISTS; SONG stops playback)"),
    ("q", "quit"),
    ("", ""),
    ("", "NAVIGATION (inside a tab)"),
    ("h / l", "move focus between panes"),
    ("j / k", "move selection down / up"),
    ("Enter", "activate (play / confirm)"),
    ("/", "search list (playlist editor): type, Enter keep filter, Esc clear"),
    ("Up / Down", "move selection while typing a search"),
    ("", ""),
    ("", "PLAYLISTS tab"),
    ("Shift+N", "new playlist (opens editor)"),
    ("Shift+E", "edit selected playlist (opens editor)"),
    ("Shift+D", "delete selected playlist (from db; asks y/N)"),
    ("", ""),
    ("", "SONG tab (while playing)"),
    ("Shift+E", "edit metadata of selected track (opens editor)"),
    ("Shift+P", "edit the playing playlist (opens editor)"),
    ("Shift+J / Shift+K", "move track down / up (temporary)"),
    ("digits then Enter", "move selected track to position N (temporary)"),
    ("x", "remove track from queue (temporary)"),
    ("Shift+D", "remove track from playlist (saved to db; asks y/N)"),
    ("", "  in ALL SONGS: delete the song from the device — file + library (asks y/N)"),
    ("", ""),
    ("", "PLAYLIST EDITOR (NEW_PLAYLIST / MOD_<name>)"),
    ("Tab / h / l", "switch name / playlist / pool"),
    ("Enter", "name: edit · pool: add · playlist: remove"),
    ("digits then Enter", "move selected track to position N (in playlist pane)"),
    ("S / R / T", "sort field / direction / hide-added"),
    ("/", "search focused list · pool: Enter adds · playlist: Enter jumps to it"),
    ("Shift+W", "SAVE the playlist (to db)"),
    ("Esc", "cancel / leave field"),
    ("", ""),
    ("", "METADATA EDITOR (MOD_SONG_...)"),
    ("j / k", "select field"),
    ("Enter / i", "edit field"),
    ("Shift+W", "SAVE the track metadata (to db)"),
    ("Esc", "cancel / leave field"),
    ("", ""),
    ("", "EXPORT tab (:export)"),
    ("j / k", "select field (form) / move (list)"),
    ("Enter / i", "PATH: edit · EXPORT: library/playlists/tracks · flags: toggle"),
    ("PATH", "file or directory (default: launch dir); dir -> dream_player_<time>.dplr"),
    ("l / h / Tab", "focus selection list / form (playlists & tracks modes)"),
    ("Enter (list)", "mark / unmark item"),
    ("a", "mark all / unmark all"),
    ("Shift+W", "EXPORT (an existing file is never overwritten)"),
    ("SEND TO", "connection for Shift+S: listen | tcp <ip> | usb | adb [listen] | bt listen | bt <MAC>"),
    ("Shift+S", "SEND the same selection and flags to a device (it receives on IMPORT)"),
    ("", ""),
    ("", "IMPORT tab (:import)"),
    ("j / k", "select field"),
    ("Enter / i", "PACKET / SAVE TO: edit · FORCE: toggle"),
    ("PACKET", "packet file to import (default: launch dir)"),
    ("SAVE TO", "where new songs and covers are written (default: music dir)"),
    ("FORCE", "apply even if the packet is older than local playlists"),
    ("Shift+W", "IMPORT (outdated packets are rejected without FORCE)"),
    ("RECEIVE", "connection for Shift+R: listen | tcp <ip> | usb | adb [listen] | bt listen | bt <MAC>"),
    ("Shift+R", "RECEIVE from a device that sends from EXPORT (FORCE and SAVE TO apply)"),
    ("c", "cancel a running send / receive (EXPORT / IMPORT)"),
    ("", ""),
    ("", "ORPHANS tab (:orphans) — songs in no playlist, with their paths"),
    ("f", "show: not marked / kept / all"),
    ("i", "keep the song (the collector stops listing it) / list it again"),
    ("D / A", "delete the selected / every shown song from the device (y/N)"),
    ("r", "reload the list"),
    ("", ""),
    ("", "SYNC tab (:sync ...)"),
    ("", "both devices get every playlist and the songs in playlists;"),
    ("", "the listening side gives its library first, then takes the other one"),
    (":sync listen [port]", "wait for a device on the local network (default port 47800)"),
    (":sync tcp <ip[:port]>", "connect to a device that listens"),
    (":sync usb [VID:PID]", "USB without adb / debug mode: the phone becomes an accessory (Dream Player › Devices › USB cable)"),
    ("", "Linux: allow USB access once (udev), e.g. /etc/udev/rules.d/51-dream-player.rules:"),
    ("", "  SUBSYSTEM==\"usb\", ATTR{idVendor}==\"18d1\", MODE=\"0666\" + the phone's own vendor id"),
    (":sync adb [port]", "USB: adb forward + connect to the phone that listens"),
    (":sync adb listen [port]", "USB: adb reverse + wait for the phone"),
    (":sync bt listen", "wait for a Bluetooth device (Linux; pair in the OS first)"),
    (":sync bt <MAC>", "connect to a Bluetooth device (Linux)"),
    (":sync cancel / c", "cancel the running sync"),
    ("r / D", "rename (name for viewing only) / forget the selected paired device"),
    ("", "a received song replaces a local file with the same name"),
    ("", ""),
    ("", "COMMANDS (:)"),
    (":help", "open this help"),
    (":settings", "output device selection (also :device)"),
    (":export", "open the EXPORT tab (pack library / playlists / tracks)"),
    (":import", "open the IMPORT tab (import a packet file)"),
    (":sync", "open the SYNC tab (library sync with another device)"),
    (":orphans / :gc", "open the ORPHANS tab (songs in no playlist)"),
    (":q", "quit"),
    (":seek <sec>", "seek current track"),
    (":vol <pct>", "set master volume"),
    (":svol <pct>", "set track volume"),
    (":scan <dir>", "index a directory"),
];

#[derive(Default)]
pub struct HelpTab {
    scroll: u16,
}

impl Tab for HelpTab {
    fn title(&self) -> String {
        "HELP".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, _model: &Model) {
        let mut lines: Vec<Line> = Vec::with_capacity(HELP.len() + 1);
        lines.push(Line::from(Span::styled(
            "press Shift+X to close this help",
            Style::default().fg(HELP_RED).add_modifier(Modifier::BOLD),
        )));
        for (key, desc) in HELP {
            if key.is_empty() && !desc.is_empty() {
                // section header.
                lines.push(Line::from(Span::styled(
                    (*desc).to_string(),
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )));
            } else if key.is_empty() {
                lines.push(Line::from(""));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {key:<20}"),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::raw((*desc).to_string()),
                ]));
            }
        }
        let para = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" HELP "))
            .scroll((self.scroll, 0));
        frame.render_widget(para, area);
    }

    fn on_key(&mut self, key: KeyEvent, _model: &Model) -> TabEvent {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.scroll = self.scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            _ => {}
        }
        TabEvent::None
    }

    fn hints(&self) -> Vec<Hint> {
        vec![("j/k", "scroll")]
    }
}
