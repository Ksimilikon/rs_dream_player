//! показ обложек прямо в терминале через graphics-протоколы (sixel/kitty/iterm2)
//! с детекцией поддержки. Модуль оформлен директорией, чтобы дробить код по мере
//! роста (детекция протокола, извлечение изображений, и т.п.).
//!
//! Если терминал не поддерживает graphics-протокол, [`ImageManager::supported`]
//! возвращает `false`, и вкладка SONG не резервирует место под изображение.
//!
//! По протоколам: sixel — статичная растровая графика (один кадр), нативной
//! анимации GIF нет; показать «гифку» можно только покадрово перерисовывая
//! область самому. Kitty/iTerm2 умеют больше, но пока цель — sixel-совместимый
//! статичный показ.

mod protocol;
mod source;

use ratatui::{Frame, layout::Rect};
use ratatui_image::{StatefulImage, picker::Picker, protocol::StatefulProtocol};

/// менеджер показа обложек: держит детектированный протокол терминала и текущее
/// подготовленное изображение. Пересобирает изображение только при смене
/// источника (чтобы не декодировать каждый кадр).
pub struct ImageManager {
    /// `Some` — терминал поддерживает graphics-протокол; `None` — показ выключен.
    picker: Option<Picker>,
    /// ключ текущего источника (путь к обложке) — чтобы не грузить повторно.
    current_key: Option<String>,
    /// подготовленный к отрисовке протокол текущей обложки.
    protocol: Option<StatefulProtocol>,
}

impl ImageManager {
    /// детектирует протокол терминала запросом к stdio. Должен вызываться уже
    /// после инициализации терминала (в raw-режиме).
    pub fn detect() -> Self {
        Self {
            picker: protocol::detect_graphics_picker(),
            current_key: None,
            protocol: None,
        }
    }

    /// поддерживает ли терминал показ изображений (есть настоящий graphics-протокол).
    pub fn supported(&self) -> bool {
        self.picker.is_some()
    }

    /// задаёт источник обложки текущего трека: `cover` — путь к отдельному файлу
    /// обложки из индекса (приоритет), `audio` — путь к самому аудиофайлу (из его
    /// тегов достаём встроенную обложку, если файла-обложки нет). Пересобирает
    /// протокол только при смене источника; ошибки чтения/декодирования дают
    /// пустой показ.
    pub fn set_source(&mut self, cover: Option<&str>, audio: Option<&str>) {
        if self.picker.is_none() {
            return;
        }
        // ключ кэша учитывает оба источника, чтобы обновляться при смене трека.
        let key = format!("{}|{}", cover.unwrap_or(""), audio.unwrap_or(""));
        if self.current_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.current_key = Some(key);
        self.protocol = self.load(cover, audio);
    }

    /// есть ли готовое к показу изображение (протокол собран).
    pub fn has_image(&self) -> bool {
        self.protocol.is_some()
    }

    /// готовит изображение: сперва отдельный файл обложки, затем встроенная
    /// обложка из тегов аудиофайла.
    fn load(&self, cover: Option<&str>, audio: Option<&str>) -> Option<StatefulProtocol> {
        let picker = self.picker.as_ref()?;
        let img = cover
            .and_then(source::load_image)
            .or_else(|| audio.and_then(source::load_embedded))?;
        Some(picker.new_resize_protocol(img))
    }

    /// рисует текущую обложку в `area` (если протокол и изображение есть).
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        if let Some(protocol) = self.protocol.as_mut() {
            frame.render_stateful_widget(
                StatefulImage::<StatefulProtocol>::default(),
                area,
                protocol,
            );
        }
    }
}
