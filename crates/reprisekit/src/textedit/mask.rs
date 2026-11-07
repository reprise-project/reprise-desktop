use super::TextEdit;

/// What a masked field shows for each char of its value.
pub const BULLET: &str = "\u{25cf}";

impl TextEdit {
    /// The bullets a masked field shows, with caret and anchor mapped by char index. Secret
    /// itself, so the field shows no preedit and reports no text to an input method.
    pub fn masked(&self) -> TextEdit {
        let at = |i: usize| self.text[..i].chars().count() * BULLET.len();
        let mut m = TextEdit::secret();
        m.set_cue(self.cue());
        m.text.push_str(&BULLET.repeat(self.text.chars().count()));
        m.set_selection(at(self.anchor()), at(self.caret()));
        m
    }
}
