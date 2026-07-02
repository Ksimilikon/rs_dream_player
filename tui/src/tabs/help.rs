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
  [  and  ]          current song volume
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
  :scan <dir>        index a directory into the library
  :check all         verify all files exist (missing are flagged invalid)
  :check <playlist>  verify a playlist's files (missing are flagged invalid)
  :purge             delete all invalid (red) tracks from the index
  :help              this help

PLAYLISTS TAB
  h / l              switch panel (playlists / songs)
  j / k              move cursor in the focused panel
  Enter (playlists)  load / play selected playlist
  Enter (songs)      play the playlist starting from that song
                     (invalid song -> set new path / delete)
  n                  create a new playlist
  e                  edit selected playlist

SONG TAB
  Enter              play the song under the cursor
                     (invalid song -> set new path / delete)
  m                  edit metadata of the song under the cursor
  metadata commands (SONG tab only, act on the current song):
  :title <text>      set the title (writes the file tag)
  :artist <a, b>     set artists, comma separated (writes the tag)
  :album <text>      set the album (writes the tag; empty clears)
  :genres <a, b>     set genres, comma separated (writes the tag; empty clears)
  :color <name>      set the color mark: red/pink/orange/green/blue/cyan/purple
                     (db only; empty clears)
  :label <text>      set the user text label (db only; empty clears)
  :filename <name>   rename the file on disk (keeps the extension)
  :cover <path>      copy image to the config dir, store its path (db, png/jpg/gif)
  :covertag <path>   embed the image into the file's tags
  :setpath <path>    give an invalid track a new file path

SONG FIELDS (shown in lists / SONG tab)
  color square       the color mark, drawn before the title
  [album]            album name, at the end of the line
  red line           an invalid track (file missing) - skipped in playback

SETTINGS TAB
  (shows the config)

METADATA EDITOR
  Up / Down (Tab)    move between fields
  type text          edit the focused field (empty field = keep old value)
  Ctrl+S             save (writes tags / renames file / stores cover / db marks)
  Esc                leave without saving

INVALID TRACK MENU
  type text          new file path
  Enter              set the new path (dedups if already indexed)
  Ctrl+D             delete the track from the index
  Esc                cancel

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
