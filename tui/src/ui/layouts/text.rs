//! a single-line input field with a caret: moving by characters and by
//! words, deleting left / right. Used both in the command line (`:`) and in
//! the editors (playlist name, metadata fields).

/// text field: the content in characters + the caret position (a character
/// index).
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

    /// caret position in characters (for drawing the cursor).
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

    /// delete the character left of the caret (Backspace).
    pub fn backspace(&mut self) {
        if self.caret > 0 {
            self.caret -= 1;
            self.chars.remove(self.caret);
        }
    }

    /// delete the character under the caret (Delete).
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

    /// to the start of the previous word (skip spaces, then the word).
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

    /// to the start of the next word (skip the word, then spaces).
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
