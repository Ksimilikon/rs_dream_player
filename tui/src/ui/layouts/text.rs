//! однострочное поле ввода с кареткой: перемещение по символам и по словам,
//! удаление влево/вправо. Используется и в командной строке (`:`), и в
//! редакторах (название плейлиста, поля метаданных).

/// текстовое поле: содержимое в символах + позиция каретки (индекс символа).
#[derive(Default, Clone)]
pub struct TextField {
    chars: Vec<char>,
    caret: usize,
}

impl TextField {
    pub fn new(initial: &str) -> Self {
        let chars: Vec<char> = initial.chars().collect();
        let caret = chars.len();
        Self { chars, caret }
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// позиция каретки в символах (для отрисовки курсора).
    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn clear(&mut self) {
        self.chars.clear();
        self.caret = 0;
    }

    pub fn insert(&mut self, c: char) {
        self.chars.insert(self.caret, c);
        self.caret += 1;
    }

    /// удалить символ слева от каретки (Backspace).
    pub fn backspace(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
            self.chars.remove(self.caret);
        }
    }

    /// удалить символ под кареткой (Delete).
    pub fn delete(&mut self) {
        if self.caret < self.chars.len() {
            self.chars.remove(self.caret);
        }
    }

    pub fn left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.caret = (self.caret + 1).min(self.chars.len());
    }

    pub fn home(&mut self) {
        self.caret = 0;
    }

    pub fn end(&mut self) {
        self.caret = self.chars.len();
    }

    /// к началу предыдущего слова (пропустить пробелы, затем слово).
    pub fn word_left(&mut self) {
        let mut i = self.caret;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.caret = i;
    }

    /// к началу следующего слова (пропустить слово, затем пробелы).
    pub fn word_right(&mut self) {
        let n = self.chars.len();
        let mut i = self.caret;
        while i < n && !self.chars[i].is_whitespace() {
            i += 1;
        }
        while i < n && self.chars[i].is_whitespace() {
            i += 1;
        }
        self.caret = i;
    }
}
