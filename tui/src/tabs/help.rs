//! popup help window: every command and key with a short explanation and the
//! space where it is available. Scrollable with j/k.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::centered_rect;

/// help lines. Each is "key/command - what it does".
const HELP: &str = "\
GLOBAL (any tab)
  Shift+H / Shift+L  switch tabs (left / right)
  j / k              move cursor up / down
  Space              play / pause
  - / +              master volume
  [ / ]              current song volume
  :                  command mode
  :help  (or ?)      this help window
  q / Esc            quit the player

COMMANDS (after `:`)
  :q  :quit          quit
  :vol N             master volume N% (0..100)
  :svol N            song volume N%
  :pl <name>         load playlist by name
  :song <text>       select a song by title
  :<number>          select a song by number
  :new               create a playlist (editor)
  :edit <name>       edit a playlist by name
  :help              this help

PLAYLISTS TAB
  Enter              load / play selected playlist
  n                  create a new playlist
  e                  edit selected playlist

SONG TAB
  Enter              play the song under the cursor

SETTINGS TAB
  (shows the config)

PLAYLIST EDITOR
  type text          playlist name, then Enter
  h / l              switch panels (playlist / pool)
  j / k              move cursor
  Enter (pool)       add song to the playlist
  Enter (playlist)   remove song from the playlist
  /                  search the pool (Enter/Esc leaves search)
  digits + Enter     move a song to a position (in playlist)
  Ctrl+S             save (empty name = temporary, not stored in db)
  Esc                leave the editor

HELP
  j / k              scroll
  q / Esc            close this window";

/// number of help lines (for scroll clamping in the app).
pub fn help_lines() -> u16 {
    HELP.lines().count() as u16
}

/// draws the help window on top of the current screen, scrolled by `scroll`.
pub fn render_help(frame: &mut Frame, area: Rect, scroll: u16) {
    // fit content height (+2 for the border), but never exceed the screen.
    let h = help_lines().saturating_add(2);
    let popup = centered_rect(area, 68, h);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(HELP)
            .scroll((scroll, 0))
            .style(Style::new().fg(Color::White))
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .title(" HELP - j/k scroll, q/Esc close "),
            ),
        popup,
    );
}
