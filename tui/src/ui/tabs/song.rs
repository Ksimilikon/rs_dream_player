//! вкладка SONG: появляется с началом проигрывания плейлиста. Слева — список
//! треков играющего плейлиста (играющий выделен зелёным), справа — все
//! метаданные трека под селектором и обложка под ними. Команды: правка
//! метаданных (E), правка плейлиста (P), перестановка (Shift+J/K или ввод
//! номера позиции цифрами + Enter — всё временно, без записи в бд),
//! удаление (x — временно, D — ещё и из плейлиста в бд).

use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::images::ImageManager;
use crate::ui::{
    Model, Tab, TabEvent,
    layouts::{Hint, ListLine, ListView, target_index, two_pane},
};

pub struct SongTab {
    list: ListView,
    images: ImageManager,
    /// набираемый номер позиции для перемещения трека (цифры, Enter — применить).
    move_buf: String,
}

impl SongTab {
    pub fn new(images: ImageManager) -> Self {
        Self {
            list: ListView::default(),
            images,
            move_buf: String::new(),
        }
    }

    fn meta_lines(t: &crate::ffi::TrackMeta) -> Vec<Line<'static>> {
        let field = |k: &str, v: String| {
            Line::from(vec![
                Span::styled(format!("{k:<12}"), ratatui::style::Style::default().fg(ratatui::style::Color::DarkGray)),
                Span::raw(v),
            ])
        };
        vec![
            field("TITLE", t.title.clone()),
            field("ARTISTS", t.artists_line()),
            field("ALBUM", t.album.clone().unwrap_or_else(|| "-".into())),
            field("GENRES", if t.genres.is_empty() { "-".into() } else { t.genres.join(", ") }),
            field("PATH", t.path.clone().unwrap_or_else(|| "-".into())),
            field("DURATION", format!("{}s", t.duration_s)),
            field("BITRATE", format!("{} kbps", t.bitrate)),
            field("SAMPLERATE", format!("{} Hz", t.sample_rate)),
            field("LISTENS", t.listen_count.to_string()),
            field("COVER", t.cover_art.clone().unwrap_or_else(|| "-".into())),
        ]
    }
}

impl Tab for SongTab {
    fn title(&self) -> String {
        "SONG".to_string()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect, model: &Model) {
        let Some(playing) = &model.playing else {
            frame.render_widget(
                Paragraph::new("no playlist playing")
                    .block(Block::default().borders(Borders::ALL).title(" SONG ")),
                area,
            );
            return;
        };

        let (left, right) = two_pane(area, 45);

        // левый список: треки играющего плейлиста, играющий — зелёным.
        let lines: Vec<ListLine> = playing
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                ListLine::new(format!("{:>3}. {} — {}", i + 1, t.title, t.artists_line()))
                    .playing(i == playing.current)
            })
            .collect();
        // заголовок левого списка показывает набираемый номер перемещения.
        let left_title = if self.move_buf.is_empty() {
            playing.name.clone()
        } else {
            format!("{}  move→{}", playing.name, self.move_buf)
        };
        self.list.render(frame, left, &left_title, lines, true);

        // правая панель: метаданные сверху, обложка снизу.
        let [meta_area, cover_area] =
            Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(right);

        let sel = self.list.selected();
        if let Some(track) = playing.tracks.get(sel) {
            frame.render_widget(
                Paragraph::new(Self::meta_lines(track))
                    .block(Block::default().borders(Borders::ALL).title(" METADATA ")),
                meta_area,
            );
            // обложка: отдельный файл из индекса либо встроенная в аудиофайл.
            let cover_block = Block::default().borders(Borders::ALL).title(" COVER ");
            let inner = cover_block.inner(cover_area);
            frame.render_widget(cover_block, cover_area);
            // ВАЖНО: рендер картинки (ratatui-image) в почти нулевую площадь
            // паникует (особенно на Windows при ресайзе) — рисуем только если
            // области хватает.
            if inner.width >= 2 && inner.height >= 2 {
                self.images
                    .set_source(track.cover_art.as_deref(), track.path.as_deref());
                if self.images.supported() && self.images.has_image() {
                    self.images.render(frame, inner);
                } else {
                    frame.render_widget(Paragraph::new("(no cover / unsupported terminal)"), inner);
                }
            }
        }
    }

    fn on_key(&mut self, key: KeyEvent, model: &Model) -> TabEvent {
        let len = model.playing.as_ref().map(|p| p.tracks.len()).unwrap_or(0);
        let sel = self.list.selected();
        // набор номера позиции: цифры копим, Enter применяет (перемещение
        // временное, в бд не пишется).
        if let KeyCode::Char(c) = key.code
            && c.is_ascii_digit()
        {
            if self.move_buf.len() < 6 {
                self.move_buf.push(c);
            }
            return TabEvent::None;
        }
        match key.code {
            KeyCode::Enter if !self.move_buf.is_empty() => {
                let buf = std::mem::take(&mut self.move_buf);
                match target_index(&buf, len) {
                    Some(to) => {
                        self.list.select(to);
                        TabEvent::MovePlaying { from: sel, to }
                    }
                    None => TabEvent::None,
                }
            }
            KeyCode::Esc => {
                self.move_buf.clear();
                TabEvent::None
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_buf.clear();
                self.list.down();
                TabEvent::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_buf.clear();
                self.list.up();
                TabEvent::None
            }
            KeyCode::Enter => TabEvent::SelectPlaying(sel),
            KeyCode::Char('E') => TabEvent::EditMeta(sel),
            KeyCode::Char('P') => TabEvent::EditPlayingPlaylist,
            // перестановка (временно): Shift+J вниз, Shift+K вверх.
            KeyCode::Char('J') if sel + 1 < len => {
                self.list.select(sel + 1);
                TabEvent::MovePlaying { from: sel, to: sel + 1 }
            }
            KeyCode::Char('K') if sel > 0 => {
                self.list.select(sel - 1);
                TabEvent::MovePlaying { from: sel, to: sel - 1 }
            }
            KeyCode::Char('x') => TabEvent::RemovePlaying { index: sel, from_db: false },
            KeyCode::Char('D') => TabEvent::RemovePlaying { index: sel, from_db: true },
            _ => TabEvent::None,
        }
    }

    fn hints(&self) -> Vec<Hint> {
        vec![
            ("jk", "nav"),
            ("Ent", "play"),
            ("E", "meta"),
            ("P", "pl"),
            ("JK", "move"),
            ("0-9+Ent", "to#"),
            ("x/D", "del"),
        ]
    }

    fn on_tracks_changed(&mut self) {
        self.move_buf.clear();
        self.list.select(0);
    }
}
