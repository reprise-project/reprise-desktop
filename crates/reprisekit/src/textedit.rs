//! A single-line edit field's state and keys (text, caret, selection anchor), plus what edits share with an input method: [`Caret`], [`Preedit`], [`TextSpot`].

use zeroize::Zeroize;

/// A secret field's reservation, so typing a password does not outgrow its buffer.
const SECRET_CAPACITY: usize = 256;

/// Most text a single-line edit holds, in bytes; a paste is cut on a char boundary to fit.
pub const MAX_LEN: usize = 32_767;

/// Text with a caret and a selection anchor, byte offsets on char boundaries; `caret == anchor` means nothing selected. A `secret` edit holds a password: no undo snapshot, no buffer freed without zeroizing, zeroized on drop, `Debug` redacted.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct TextEdit {
    pub text: String,
    caret: usize,
    anchor: usize,
    undo: Undo<String, usize>,
    secret: bool,
}

impl std::fmt::Debug for TextEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("TextEdit");
        if self.secret {
            d.field("text", &"<redacted>").field("undo", &"<redacted>");
        } else {
            d.field("text", &self.text).field("undo", &self.undo);
        }
        d.field("caret", &self.caret)
            .field("anchor", &self.anchor)
            .field("secret", &self.secret)
            .finish()
    }
}

impl Drop for TextEdit {
    fn drop(&mut self) {
        if self.secret {
            self.text.zeroize();
        }
    }
}

/// What the last edit was, so a run of typing (or deleting) is one undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Edit {
    #[default]
    None,
    Typing,
    Deleting,
}

/// Single-level undo shared by the single- and multi-line edits: the state before the last run of edits; Ctrl+Z again puts it back. `T` is the text, `P` a caret position.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct Undo<T, P> {
    saved: Option<(T, P, P)>,
    pub last: Edit,
}

impl<T: Clone, P: Copy> Undo<T, P> {
    pub fn checkpoint(&mut self, kind: Edit, text: &T, caret: P, anchor: P) {
        if kind == Edit::None || kind != self.last {
            self.saved = Some((text.clone(), caret, anchor));
        }
        self.last = kind;
    }

    /// Ctrl+Z: swap the saved state with the current one; false when nothing to undo. The caller clamps the positions it gets back.
    pub fn toggle(&mut self, text: &mut T, caret: &mut P, anchor: &mut P) -> bool {
        let Some((t, c, a)) = self.saved.take() else {
            return false;
        };
        self.saved = Some((std::mem::replace(text, t), *caret, *anchor));
        *caret = c;
        *anchor = a;
        self.last = Edit::None;
        true
    }
}

pub(crate) mod words {
    pub fn prev_char(text: &str, i: usize) -> usize {
        text[..i].char_indices().last().map_or(0, |(j, _)| j)
    }

    pub fn next_char(text: &str, i: usize) -> usize {
        text[i..].chars().next().map_or(i, |c| i + c.len_utf8())
    }

    pub fn word_left(text: &str, i: usize) -> usize {
        let mut j = i;
        while j > 0 && text[..j].ends_with(' ') {
            j -= 1;
        }
        while j > 0 && !text[..j].ends_with(' ') {
            j = prev_char(text, j);
        }
        j
    }

    pub fn word_right(text: &str, i: usize) -> usize {
        let mut j = i;
        while j < text.len() && !text[j..].starts_with(' ') {
            j = next_char(text, j);
        }
        while j < text.len() && text[j..].starts_with(' ') {
            j += 1;
        }
        j
    }

    pub fn word_span_at(text: &str, index: usize) -> std::ops::Range<usize> {
        let index = index.min(text.len());
        let bytes = text.as_bytes();
        let space = |i: usize| bytes.get(i) == Some(&b' ');
        let (mut a, mut b) = (index, index);
        if space(index) || (index == bytes.len() && index > 0 && space(index - 1)) {
            while a > 0 && space(a - 1) {
                a -= 1;
            }
            while b < bytes.len() && space(b) {
                b += 1;
            }
        } else {
            while a > 0 && !space(a - 1) {
                a -= 1;
            }
            while b < bytes.len() && !space(b) {
                b += 1;
            }
            while b < bytes.len() && space(b) {
                b += 1;
            }
        }
        a..b
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditKey {
    Insert(String),
    Backspace {
        word: bool,
    },
    Delete {
        word: bool,
    },
    Left {
        select: bool,
        word: bool,
    },
    Right {
        select: bool,
        word: bool,
    },
    Home {
        select: bool,
    },
    End {
        select: bool,
    },
    SelectAll,
    /// Ctrl+C / X / V. The model only deletes on Cut; the view moves text through the seat's clipboard.
    Copy,
    Cut,
    Paste,
    Undo,
    /// An input method's delete: `before` bytes just before the selection and `after` just after it (text-input-v3 counts bytes).
    DeleteSurrounding {
        before: usize,
        after: usize,
    },
}

/// Whether an edit has keyboard focus and, if so, which half of the blink its caret is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Caret {
    #[default]
    Unfocused,
    Hidden,
    Shown,
}

impl Caret {
    pub fn new(focused: bool, shown: bool) -> Caret {
        match (focused, shown) {
            (false, _) => Caret::Unfocused,
            (true, false) => Caret::Hidden,
            (true, true) => Caret::Shown,
        }
    }

    pub fn focused(self) -> bool {
        self != Caret::Unfocused
    }

    pub fn shown(self) -> bool {
        self == Caret::Shown
    }
}

/// Text an input method is composing: drawn underlined at the focused edit's caret, not part of the text until committed. `cursor` is the byte range the input method marks (`None` hides the caret); a wider range is drawn selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    pub cursor: Option<std::ops::Range<usize>>,
}

impl Preedit {
    /// A preedit of `text` marking `begin..end` bytes as text-input-v3 sends it. A `begin` that is negative, past the end or inside a char hides the caret; a bad `end` leaves a plain caret at `begin`.
    pub fn new(text: String, begin: i32, end: i32) -> Option<Preedit> {
        if text.is_empty() {
            return None;
        }
        let at = |i: i32| {
            usize::try_from(i)
                .ok()
                .filter(|&c| text.is_char_boundary(c))
        };
        let cursor = at(begin).map(|b| b..at(end).filter(|&e| e >= b).unwrap_or(b));
        Some(Preedit { text, cursor })
    }

    /// `text` with the preedit put in at byte `at`: the string to show, the preedit's byte range, and the range it marks.
    pub fn splice(
        &self,
        text: &str,
        at: usize,
    ) -> (String, std::ops::Range<usize>, std::ops::Range<usize>) {
        let at = text.floor_char_boundary(at);
        let mut shown = String::with_capacity(text.len() + self.text.len());
        shown.push_str(&text[..at]);
        shown.push_str(&self.text);
        shown.push_str(&text[at..]);
        let run = at..at + self.text.len();
        let end = self.text.len();
        let mark = self.cursor.clone().unwrap_or(end..end);
        (shown, run, at + mark.start..at + mark.end)
    }

    /// `own` with the preedit put in at its caret, for a single-line painter: the edit to draw, the preedit's byte range, and whether the caret bar shows.
    pub fn compose(&self, own: &TextEdit) -> (TextEdit, std::ops::Range<usize>, bool) {
        let (t, run, mark) = self.splice(&own.text, own.caret());
        let mut e = TextEdit::new(t, false);
        e.set_selection(mark.start, mark.end);
        (e, run, self.cursor.is_some())
    }
}

/// Most surrounding text an input method takes, in bytes: what fits in one Wayland message.
pub const SURROUNDING_MAX: usize = 4000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surrounding {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

impl Surrounding {
    /// The part of `text` around `cursor` that fits in [`SURROUNDING_MAX`]: the whole selection when it fits, else the cursor alone.
    pub fn around(text: &str, cursor: usize, anchor: usize) -> Surrounding {
        let cursor = text.floor_char_boundary(cursor);
        let anchor = text.floor_char_boundary(anchor);
        let (lo, hi) = (cursor.min(anchor), cursor.max(anchor));
        let anchor = if hi - lo <= SURROUNDING_MAX {
            anchor
        } else {
            cursor
        };
        let (lo, hi) = (cursor.min(anchor), cursor.max(anchor));
        // The room past the selection goes half to each side; a side short of text gives the rest to the other.
        let spare = SURROUNDING_MAX - (hi - lo);
        let right = (text.len() - hi).min(spare - (spare / 2).min(lo));
        let left = spare - right;
        let start = text.ceil_char_boundary(lo.saturating_sub(left));
        let end = text.floor_char_boundary(hi + right);
        Surrounding {
            text: text[start..end].to_string(),
            cursor: cursor - start,
            anchor: anchor - start,
        }
    }
}

/// What an input method needs from the focused edit, reported as the control draws (see [`crate::raster::Canvas::report_spot`]). Rects are in the canvas' logical px.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSpot {
    pub field: crate::raster::Rect,
    pub caret: crate::raster::Rect,
    /// `None` for a secret field, which never hands its text out.
    pub surrounding: Option<Surrounding>,
    pub secret: bool,
    pub multiline: bool,
}

impl TextSpot {
    pub fn offset(mut self, dx: i32, dy: i32) -> TextSpot {
        self.field.x += dx;
        self.field.y += dy;
        self.caret.x += dx;
        self.caret.y += dy;
        self
    }
}

impl TextEdit {
    /// `text` with the caret at its end; all selected when `select`.
    pub fn new(text: impl Into<String>, select: bool) -> TextEdit {
        let text = text.into();
        let end = text.len();
        TextEdit {
            anchor: if select { 0 } else { end },
            caret: end,
            text,
            undo: Undo::default(),
            secret: false,
        }
    }

    pub fn secret() -> TextEdit {
        TextEdit {
            text: String::with_capacity(SECRET_CAPACITY),
            caret: 0,
            anchor: 0,
            undo: Undo::default(),
            secret: true,
        }
    }

    pub fn is_secret(&self) -> bool {
        self.secret
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn set_selection(&mut self, anchor: usize, caret: usize) {
        self.anchor = self.text.floor_char_boundary(anchor);
        self.caret = self.text.floor_char_boundary(caret);
    }

    fn checkpoint(&mut self, kind: Edit) {
        if !self.secret {
            self.undo
                .checkpoint(kind, &self.text, self.caret, self.anchor);
        }
    }

    pub fn set_selected(&mut self, text: impl Into<String>) {
        let secret = self.secret;
        *self = TextEdit::new(text, true);
        self.secret = secret;
    }

    /// Zeroize the text and undo snapshot in place and leave the field empty. The buffer is kept, so a secret field's reservation outlives a retry.
    pub fn wipe(&mut self) {
        self.text.zeroize();
        if let Some((saved, ..)) = self.undo.saved.as_mut() {
            saved.zeroize();
        }
        self.undo.saved = None;
        self.undo.last = Edit::None;
        self.caret = 0;
        self.anchor = 0;
    }

    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn all_selected(&self) -> bool {
        !self.text.is_empty() && self.selection() == (0..self.text.len())
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    pub fn key(&mut self, key: EditKey) -> bool {
        let (caret, anchor) = (self.caret, self.anchor);
        // Set by every arm that changes the text. Nothing here clones the edit, so a secret field's text is never copied.
        let mut edited = false;
        match key {
            EditKey::Insert(mut s) => {
                let sel = self.selection();
                let room = MAX_LEN.saturating_sub(self.text.len() - sel.len());
                s.truncate(s.floor_char_boundary(room));
                if !s.is_empty() || !sel.is_empty() {
                    // A paste or replacing insert is its own undo step, but typed chars run together with the typing after them.
                    let mut chars = s.chars();
                    let single = chars.next().is_some() && chars.next().is_none();
                    if single && sel.is_empty() {
                        self.checkpoint(Edit::Typing);
                    } else {
                        self.checkpoint(Edit::None);
                    }
                    self.splice(sel.clone(), &s);
                    self.caret = sel.start + s.len();
                    self.anchor = self.caret;
                    if single {
                        self.undo.last = Edit::Typing;
                    }
                    edited = true;
                }
                if self.secret {
                    s.zeroize();
                }
            }
            EditKey::Backspace { word } => {
                if self.has_selection() {
                    self.checkpoint(Edit::None);
                    self.delete_selection();
                    edited = true;
                } else if self.caret > 0 {
                    self.checkpoint(Edit::Deleting);
                    let from = if word {
                        words::word_left(&self.text, self.caret)
                    } else {
                        words::prev_char(&self.text, self.caret)
                    };
                    self.splice(from..self.caret, "");
                    self.caret = from;
                    self.anchor = from;
                    edited = true;
                }
            }
            EditKey::Delete { word } => {
                if self.has_selection() {
                    self.checkpoint(Edit::None);
                    self.delete_selection();
                    edited = true;
                } else if self.caret < self.text.len() {
                    self.checkpoint(Edit::Deleting);
                    let to = if word {
                        words::word_right(&self.text, self.caret)
                    } else {
                        words::next_char(&self.text, self.caret)
                    };
                    self.splice(self.caret..to, "");
                    self.anchor = self.caret;
                    edited = true;
                }
            }
            EditKey::Left { select, word } => {
                let to = if word {
                    words::word_left(&self.text, self.caret)
                } else if self.has_selection() && !select {
                    self.selection().start
                } else {
                    words::prev_char(&self.text, self.caret)
                };
                self.move_to(to, select);
            }
            EditKey::Right { select, word } => {
                let to = if word {
                    words::word_right(&self.text, self.caret)
                } else if self.has_selection() && !select {
                    self.selection().end
                } else {
                    words::next_char(&self.text, self.caret)
                };
                self.move_to(to, select);
            }
            EditKey::Home { select } => self.move_to(0, select),
            EditKey::End { select } => self.move_to(self.text.len(), select),
            EditKey::SelectAll => self.select_all(),
            // A secret's text never reaches the clipboard, so a cut would only lose it: it does nothing, as Copy does.
            EditKey::Cut => {
                if self.has_selection() && !self.secret {
                    self.checkpoint(Edit::None);
                    self.delete_selection();
                    edited = true;
                }
            }
            EditKey::Copy | EditKey::Paste => {}
            EditKey::Undo => {
                if self
                    .undo
                    .toggle(&mut self.text, &mut self.caret, &mut self.anchor)
                {
                    self.caret = self.caret.min(self.text.len());
                    self.anchor = self.anchor.min(self.text.len());
                    edited = true;
                }
            }
            EditKey::DeleteSurrounding { before, after } => {
                let sel = self.selection();
                let from = self
                    .text
                    .floor_char_boundary(sel.start.saturating_sub(before));
                let to = self
                    .text
                    .ceil_char_boundary(sel.end.saturating_add(after).min(self.text.len()));
                if (from, to) != (sel.start, sel.end) {
                    self.checkpoint(Edit::None);
                    self.splice(sel.end..to, "");
                    self.splice(from..sel.start, "");
                    self.caret -= sel.start - from;
                    self.anchor -= sel.start - from;
                    edited = true;
                }
            }
        }
        if !edited {
            self.undo.last = Edit::None;
        }
        edited || (self.caret, self.anchor) != (caret, anchor)
    }

    pub fn select_word_at(&mut self, index: usize) {
        let span = words::word_span_at(&self.text, index);
        self.anchor = span.start;
        self.caret = span.end;
        self.undo.last = Edit::None;
    }

    pub fn spot(&self, field: crate::raster::Rect, caret: crate::raster::Rect) -> TextSpot {
        TextSpot {
            field,
            caret,
            surrounding: (!self.secret)
                .then(|| Surrounding::around(&self.text, self.caret, self.anchor)),
            secret: self.secret,
            multiline: false,
        }
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    pub fn place(&mut self, index: usize, extend: bool) -> bool {
        let (caret, anchor) = (self.caret, self.anchor);
        let index = self.text.floor_char_boundary(index);
        self.move_to(index, extend);
        (self.caret, self.anchor) != (caret, anchor)
    }

    /// Paste: the first line of clipboard text that fits in `MAX_LEN`, replacing the selection. Control chars and chars in `banned` are dropped. A password goes to PAM a line at a time, so a stray line break must never reach it.
    pub fn paste(&mut self, text: &str, banned: &[char]) -> bool {
        let first = text.split(['\r', '\n']).next().unwrap_or("");
        let first = &first[..first.floor_char_boundary(MAX_LEN)];
        // Sized up front: a collect would grow through smaller buffers, each freed with a prefix of the line in it.
        let mut line = String::with_capacity(first.len());
        line.extend(
            first
                .chars()
                .filter(|c| !c.is_control() && !banned.contains(c)),
        );
        if line.is_empty() {
            return false;
        }
        self.key(EditKey::Insert(line))
    }

    /// `replace_range`, except that a secret field outgrowing its buffer moves to a bigger one by hand and zeroizes the old.
    fn splice(&mut self, range: std::ops::Range<usize>, s: &str) {
        let need = self.text.len() - range.len() + s.len();
        if self.secret && need > self.text.capacity() {
            let mut grown = String::with_capacity(need.max(2 * self.text.capacity()));
            grown.push_str(&self.text);
            std::mem::replace(&mut self.text, grown).zeroize();
        }
        self.text.replace_range(range, s);
    }

    fn delete_selection(&mut self) {
        let sel = self.selection();
        self.splice(sel.clone(), "");
        self.caret = sel.start;
        self.anchor = sel.start;
    }

    fn move_to(&mut self, to: usize, select: bool) {
        self.caret = to;
        if !select {
            self.anchor = to;
        }
    }
}

/// X keysym values the edit keys are recognised by; hosts pass whatever their keyboard stack reports.
pub mod keysym {
    pub const SPACE: u32 = 0x0020;
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const DELETE: u32 = 0xffff;
    pub const KP_DELETE: u32 = 0xff9f;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const HOME: u32 = 0xff50;
    pub const END: u32 = 0xff57;
    pub const KP_LEFT: u32 = 0xff96;
    pub const KP_UP: u32 = 0xff97;
    pub const KP_RIGHT: u32 = 0xff98;
    pub const KP_DOWN: u32 = 0xff99;
    pub const KP_PAGE_UP: u32 = 0xff9a;
    pub const KP_PAGE_DOWN: u32 = 0xff9b;
    pub const KP_HOME: u32 = 0xff95;
    pub const KP_END: u32 = 0xff9c;
    pub const F2: u32 = 0xffbf;
    pub const F3: u32 = 0xffc0;
    pub const F4: u32 = 0xffc1;
    pub const F5: u32 = 0xffc2;
    pub const F10: u32 = 0xffc7;
    pub const MENU: u32 = 0xff67;
    pub const ALT_L: u32 = 0xffe9;
    pub const ALT_R: u32 = 0xffea;
    /// A seat sends Shift+Tab as this keysym rather than Tab with Shift.
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
}

/// The editing key a press stands for, modifiers folded in. Ctrl+A / C / X / V / Z are commands, Shift extends, Ctrl steps words, other printable input is an insert. None for keys an edit has no use for.
pub fn key_for(sym: u32, ctrl: bool, shift: bool, utf8: Option<&str>) -> Option<EditKey> {
    use keysym::*;
    let (select, word) = (shift, ctrl);
    let lower = if (0x41..=0x5a).contains(&sym) {
        sym + 0x20
    } else {
        sym
    };
    let special = match sym {
        BACKSPACE => Some(EditKey::Backspace { word }),
        DELETE | KP_DELETE => Some(EditKey::Delete { word }),
        LEFT | KP_LEFT => Some(EditKey::Left { select, word }),
        RIGHT | KP_RIGHT => Some(EditKey::Right { select, word }),
        HOME | KP_HOME => Some(EditKey::Home { select }),
        END | KP_END => Some(EditKey::End { select }),
        _ if ctrl => match lower {
            0x61 => Some(EditKey::SelectAll),
            0x63 => Some(EditKey::Copy),
            0x78 => Some(EditKey::Cut),
            0x76 => Some(EditKey::Paste),
            0x7a => Some(EditKey::Undo),
            _ => None,
        },
        _ => None,
    };
    if special.is_some() || ctrl {
        return special;
    }
    let s = utf8?;
    if s.is_empty() || s.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(EditKey::Insert(s.to_string()))
}

/// Byte index of the leftmost visible character so the caret stays inside a field `inner_w` px wide. Scrolls by whole characters; each walk stops once past `inner_w`, so the cost follows what is on screen.
pub fn edit_first_visible(
    text: &crate::text::TextRenderer,
    edit: &TextEdit,
    inner_w: i32,
    px: u32,
    first: usize,
) -> usize {
    let s = edit.text.as_str();
    let adv = |ch| text.advance(ch, false, px);
    let mut first = first.min(s.len());
    if !s.is_char_boundary(first) {
        first = 0;
    }
    if edit.caret() < first {
        first = edit.caret();
    }
    let mut used = 0;
    let mut start = edit.caret();
    for (i, ch) in s[first..edit.caret()].char_indices().rev() {
        used += adv(ch);
        if used > inner_w {
            break;
        }
        start = first + i;
    }
    first = start;
    let mut tail = 0;
    for ch in s[first..].chars() {
        tail += adv(ch);
        if tail > inner_w {
            return first;
        }
    }
    for (i, ch) in s[..first].char_indices().rev() {
        tail += adv(ch);
        if tail > inner_w {
            break;
        }
        first = i;
    }
    first
}

/// Byte index in `s` nearest to `x` px: the char boundary whose prefix advance is closest, so a click on a glyph's right half lands after it; ties go earlier. The walk stops at the first prefix reaching `x`.
pub fn index_at(text: &crate::text::TextRenderer, s: &str, x: i32, px: u32) -> usize {
    let mut best = (0usize, x.abs());
    let mut pen = 0;
    for (i, ch) in s.char_indices() {
        if pen >= x {
            break;
        }
        pen += text.advance(ch, false, px);
        let d = (pen - x).abs();
        if d < best.1 {
            best = (i + ch.len_utf8(), d);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_step_over_words_and_spaces() {
        let t = "ab  cd é";
        assert_eq!(words::word_left(t, 6), 4);
        assert_eq!(words::word_left(t, 4), 0);
        assert_eq!(words::word_left(t, 0), 0);
        assert_eq!(words::word_right(t, 0), 4);
        assert_eq!(words::word_right(t, 4), 7);
        assert_eq!(words::word_right(t, 7), t.len());
        assert_eq!(words::prev_char(t, t.len()), 7, "é is two bytes");
        assert_eq!(words::next_char(t, 7), t.len());
        assert_eq!(words::word_span_at(t, 1), 0..4, "word plus its spaces");
        assert_eq!(words::word_span_at(t, 3), 2..4, "a run of spaces");
        assert_eq!(words::word_span_at(t, 99), 7..t.len(), "clamped");
    }

    fn e(text: &str) -> TextEdit {
        TextEdit::new(text, false)
    }

    #[test]
    fn new_puts_the_caret_at_the_end_and_can_select_all() {
        let plain = e("abc");
        assert_eq!((plain.caret, plain.anchor), (3, 3));
        assert!(!plain.has_selection());
        let sel = TextEdit::new("abc", true);
        assert_eq!(sel.selection(), 0..3);
        assert!(sel.all_selected());
        assert!(!TextEdit::new("", true).all_selected());
    }

    #[test]
    fn typing_replaces_the_selection_and_backspace_clears_it() {
        let mut t = TextEdit::new("notepad", true);
        assert!(t.key(EditKey::Insert("x".into())));
        assert_eq!((t.text.as_str(), t.caret, t.anchor), ("x", 1, 1));
        let mut t = TextEdit::new("notepad", true);
        t.key(EditKey::Backspace { word: false });
        assert_eq!(t.text, "");
        assert!(!t.key(EditKey::Backspace { word: false }));
        assert!(!t.key(EditKey::Delete { word: false }));
        assert!(!t.key(EditKey::Left {
            select: false,
            word: false
        }));
    }

    #[test]
    fn arrows_move_shift_extends_and_a_plain_arrow_collapses() {
        let mut t = e("abc");
        t.key(EditKey::Left {
            select: true,
            word: false,
        });
        t.key(EditKey::Left {
            select: true,
            word: false,
        });
        assert_eq!((t.caret, t.anchor), (1, 3));
        assert_eq!(t.selection(), 1..3);
        t.key(EditKey::Insert("X".into()));
        assert_eq!(t.text, "aX");
        t.key(EditKey::Home { select: true });
        assert_eq!(t.selection(), 0..2);
        t.key(EditKey::End { select: false });
        assert!(!t.has_selection());
        assert_eq!(t.caret, 2);
        t.key(EditKey::SelectAll);
        t.key(EditKey::Left {
            select: false,
            word: false,
        });
        assert_eq!((t.caret, t.anchor), (0, 0));
        t.key(EditKey::SelectAll);
        t.key(EditKey::Right {
            select: false,
            word: false,
        });
        assert_eq!((t.caret, t.anchor), (2, 2));
    }

    #[test]
    fn insert_and_delete_in_the_middle() {
        let mut t = e("ac");
        t.key(EditKey::Left {
            select: false,
            word: false,
        });
        t.key(EditKey::Insert("b".into()));
        assert_eq!((t.text.as_str(), t.caret), ("abc", 2));
        t.key(EditKey::Delete { word: false });
        assert_eq!((t.text.as_str(), t.caret), ("ab", 2));
        t.key(EditKey::Home { select: false });
        t.key(EditKey::Delete { word: false });
        assert_eq!((t.text.as_str(), t.caret), ("b", 0));
    }

    #[test]
    fn ctrl_moves_and_deletes_by_word() {
        let mut t = e("foo  bar baz");
        t.key(EditKey::Left {
            select: false,
            word: true,
        });
        assert_eq!(t.caret, 9);
        t.key(EditKey::Left {
            select: true,
            word: true,
        });
        assert_eq!(t.selection(), 5..9);
        t.key(EditKey::Home { select: false });
        t.key(EditKey::Right {
            select: false,
            word: true,
        });
        assert_eq!(t.caret, 5);
        t.key(EditKey::Delete { word: true });
        assert_eq!(t.text, "foo  baz");
        t.key(EditKey::End { select: false });
        t.key(EditKey::Backspace { word: true });
        assert_eq!(t.text, "foo  ");
        t.key(EditKey::Backspace { word: true });
        assert_eq!(t.text, "");
    }

    #[test]
    fn a_secret_ignores_cut() {
        let mut t = TextEdit::secret();
        t.paste("hunter2", &[]);
        t.key(EditKey::SelectAll);
        assert!(!t.key(EditKey::Cut));
        assert_eq!(t.text.as_str(), "hunter2", "the text stays");
    }

    #[test]
    fn place_paste_and_cut() {
        let mut t = e("hello world");
        assert!(t.place(5, false));
        assert_eq!((t.caret, t.anchor), (5, 5));
        assert!(t.place(0, true));
        assert_eq!(t.selection(), 0..5);
        assert_eq!(t.selected_text(), "hello");
        assert!(t.key(EditKey::Cut));
        assert_eq!(t.text, " world");
        assert!(!t.key(EditKey::Copy));
        assert!(t.paste("a/b\tc\nsecond", &['/']));
        assert_eq!(t.text, "abc world");
        assert!(!t.paste("\n", &[]));
        t.place(99, false);
        assert_eq!(t.caret, 9);
    }

    #[test]
    fn a_paste_stops_at_the_first_line_break_and_drops_control_chars() {
        let pasted = |s: &str| {
            let mut t = TextEdit::secret();
            t.paste(s, &[]);
            t.text.clone()
        };
        assert_eq!(pasted("pw\n"), "pw");
        assert_eq!(pasted("pw\r\n"), "pw");
        assert_eq!(pasted("a\nb"), "a");
        assert_eq!(pasted("a\rb"), "a");
        assert_eq!(pasted("a\tb\x1b"), "ab");
    }

    #[test]
    fn a_paste_and_typing_stop_at_max_len_on_a_char_boundary() {
        let mut t = e("");
        assert!(t.paste(&"é".repeat(MAX_LEN), &[]));
        assert_eq!(t.text.len(), MAX_LEN - 1, "é is two bytes, MAX_LEN is odd");
        assert!(t.key(EditKey::Insert("a".into())));
        assert_eq!(t.text.len(), MAX_LEN);
        assert!(!t.key(EditKey::Insert("b".into())));
        assert!(!t.paste("more", &[]));
        assert_eq!(t.text.len(), MAX_LEN);
        t.key(EditKey::Left {
            select: true,
            word: false,
        });
        assert!(t.key(EditKey::Insert("c".into())));
        assert!(t.text.ends_with('c'));
    }

    #[test]
    fn debug_redacts_a_secret_field() {
        let mut t = TextEdit::secret();
        t.paste("hunter2", &[]);
        assert!(!format!("{t:?}").contains("hunter2"));
        assert!(format!("{:?}", e("notepad")).contains("notepad"));
    }

    #[test]
    fn place_inside_a_char_floors_to_its_start() {
        let mut t = e(" é");
        t.place(2, false);
        assert_eq!((t.caret, t.anchor), (1, 1));
        t.place(3, true);
        assert_eq!(t.selected_text(), "é");
    }

    #[test]
    fn set_selection_clamps_and_floors_to_char_boundaries() {
        let mut t = e(" é");
        t.set_selection(2, 99);
        assert_eq!((t.anchor(), t.caret()), (1, 3));
        assert_eq!(t.selected_text(), "é");
    }

    #[test]
    fn undo_takes_back_a_run_of_typing_and_toggles() {
        let mut t = TextEdit::new("notepad", true);
        t.key(EditKey::Insert("c".into()));
        t.key(EditKey::Insert("a".into()));
        t.key(EditKey::Insert("l".into()));
        assert_eq!(t.text, "cal");
        assert!(t.key(EditKey::Undo));
        assert_eq!(t.text, "notepad");
        assert!(t.all_selected());
        assert!(t.key(EditKey::Undo));
        assert_eq!(t.text, "cal");
        t.key(EditKey::Home { select: false });
        t.key(EditKey::End { select: false });
        t.key(EditKey::Insert("c".into()));
        t.key(EditKey::Undo);
        assert_eq!(t.text, "cal");
        t.key(EditKey::Backspace { word: false });
        t.key(EditKey::Backspace { word: false });
        assert_eq!(t.text, "c");
        t.key(EditKey::Undo);
        assert_eq!(t.text, "cal");
        t.paste("xyz", &[]);
        assert_eq!(t.text, "calxyz");
        t.key(EditKey::Undo);
        assert_eq!(t.text, "cal");
        assert!(!e("").key(EditKey::Undo));
    }

    #[test]
    fn wipe_clears_the_field_and_its_undo_snapshot() {
        let mut t = e("secret");
        // Insert then delete populates `undo.saved` with a copy of the
        // text as it was before the deleting run started.
        t.key(EditKey::Insert("!".into()));
        t.key(EditKey::Backspace { word: false });
        t.wipe();
        assert_eq!(t.text, "");
        assert_eq!(t.caret, 0);
        assert_eq!(t.anchor, 0);
        assert!(!t.key(EditKey::Undo));
        assert_eq!(t.text, "");
    }

    #[test]
    fn double_click_selects_the_word_and_its_trailing_spaces() {
        let mut t = e("echo  one two");
        t.select_word_at(1);
        assert_eq!(t.selected_text(), "echo  ");
        t.select_word_at(4);
        assert_eq!(t.selected_text(), "  ");
        t.select_word_at(7);
        assert_eq!(t.selected_text(), "one ");
        t.select_word_at(13);
        assert_eq!(t.selected_text(), "two");
        let mut t = e("");
        t.select_word_at(0);
        assert!(!t.has_selection());
    }

    #[test]
    fn key_for_folds_the_modifiers_in() {
        use keysym::*;
        assert_eq!(
            key_for(0x61, true, false, Some("\u{1}")),
            Some(EditKey::SelectAll)
        );
        assert_eq!(key_for(0x41, true, true, None), Some(EditKey::SelectAll));
        assert_eq!(key_for(0x7a, true, false, None), Some(EditKey::Undo));
        assert_eq!(
            key_for(LEFT, true, true, None),
            Some(EditKey::Left {
                select: true,
                word: true
            })
        );
        assert_eq!(
            key_for(KP_END, false, false, None),
            Some(EditKey::End { select: false })
        );
        assert_eq!(
            key_for(0x61, false, false, Some("a")),
            Some(EditKey::Insert("a".into()))
        );
        assert_eq!(key_for(RETURN, false, false, Some("\r")), None);
        assert_eq!(
            key_for(0x62, true, false, Some("b")),
            None,
            "Ctrl+B is nothing"
        );
        assert_eq!(key_for(F10, false, false, None), None);
    }

    #[test]
    fn index_at_picks_the_nearest_boundary() {
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let px = crate::theme::Theme::default().fonts.size_px;
        let s = "New Text";
        assert_eq!(index_at(&text, s, -5, px), 0);
        assert_eq!(index_at(&text, s, 1000, px), s.len());
        let w3 = text.measure("New", false, px);
        assert_eq!(index_at(&text, s, w3, px), 3);
        assert_eq!(index_at(&text, s, w3 + 1, px), 3);
    }

    /// Reference for `index_at`: measure every prefix and keep the first closest.
    fn index_by_remeasure(text: &crate::text::TextRenderer, s: &str, x: i32, px: u32) -> usize {
        let mut best = (0usize, i32::MAX);
        for i in s.char_indices().map(|(i, _)| i).chain([s.len()]) {
            let d = (text.measure(&s[..i], false, px) - x).abs();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    }

    #[test]
    fn index_at_matches_remeasuring_every_prefix() {
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let px = crate::theme::Theme::default().fonts.size_px;
        for s in [
            "",
            "abc",
            "C:\\Program Files\\Reprise\\notepad.exe /a /b",
            "iiiiiiiWWWWWWWiiiiiiiWWWWWWW",
            "áéíóú日本語テキストáéíóú\u{301}x\u{10FFFF}ÿ",
        ] {
            for x in -3..=text.measure(s, false, px) + 3 {
                assert_eq!(
                    index_at(&text, s, x, px),
                    index_by_remeasure(&text, s, x, px),
                    "{s:?} at x {x}"
                );
            }
        }
    }

    #[test]
    fn delete_surrounding_takes_bytes_around_the_selection_and_keeps_it() {
        let mut t = e("ni hao ma");
        t.set_selection(3, 6);
        assert!(t.key(EditKey::DeleteSurrounding {
            before: 3,
            after: 1
        }));
        assert_eq!(t.text, "haoma");
        assert_eq!(t.selected_text(), "hao");
        let mut t = e("ab");
        assert!(t.key(EditKey::DeleteSurrounding {
            before: 9,
            after: 9
        }));
        assert_eq!((t.text.as_str(), t.caret()), ("", 0));
        assert!(!t.key(EditKey::DeleteSurrounding {
            before: 1,
            after: 0
        }));
        let mut t = e("a你");
        assert!(t.key(EditKey::DeleteSurrounding {
            before: 1,
            after: 0
        }));
        assert_eq!(t.text, "a");
    }

    #[test]
    fn a_preedit_splices_in_at_the_caret_with_its_own_cursor() {
        let p = Preedit::new("nihao".into(), 2, 2).expect("not empty");
        assert_eq!(p.splice("ab", 1), ("anihaob".into(), 1..6, 3..3));
        let hidden = Preedit::new("你好".into(), -1, -1).expect("not empty");
        assert_eq!(hidden.cursor, None);
        let end = "你好".len();
        assert_eq!(hidden.splice("", 0).2, end..end, "at its end");
        assert_eq!(Preedit::new("你".into(), 1, 1).unwrap().cursor, None);
        assert_eq!(Preedit::new(String::new(), 0, 0), None);
    }

    #[test]
    fn a_preedit_range_is_selected_with_the_caret_at_its_end() {
        let p = Preedit::new("nihao".into(), 0, 5).expect("not empty");
        assert_eq!(p.cursor, Some(0..5));
        assert_eq!(p.splice("ab", 1).2, 1..6);
        let mut own = TextEdit::new("ab", false);
        own.place(1, false);
        let (e, run, bar) = p.compose(&own);
        assert_eq!((e.text.as_str(), run, bar), ("anihaob", 1..6, true));
        assert_eq!((e.selection(), e.caret()), (1..6, 6));
        let back = Preedit::new("nihao".into(), 3, 1).expect("not empty");
        assert_eq!(back.cursor, Some(3..3));
        let split = Preedit::new("你好".into(), 0, 1).expect("not empty");
        assert_eq!(split.cursor, Some(0..0));
    }

    #[test]
    fn surrounding_text_fits_the_message_around_the_caret() {
        let s = Surrounding::around("hello", 2, 4);
        assert_eq!((s.text.as_str(), s.cursor, s.anchor), ("hello", 2, 4));
        let long = "é".repeat(5000);
        let s = Surrounding::around(&long, 5000, 5000);
        assert!(s.text.len() <= SURROUNDING_MAX);
        assert!(s.text.len() >= SURROUNDING_MAX - 2);
        assert_eq!(&s.text[..s.cursor], &long[5000 - s.cursor..5000]);
        let s = Surrounding::around(&long, 10, 10);
        assert_eq!(s.cursor, 10);
        assert!(s.text.len() >= SURROUNDING_MAX - 2);
        let a = "a".repeat(9000);
        let s = Surrounding::around(&a, 8000, 10);
        assert_eq!(s.cursor, s.anchor);
        assert_eq!(s.text.len(), SURROUNDING_MAX);
    }

    #[test]
    fn a_secret_field_hands_no_text_to_an_input_method() {
        let r = crate::raster::Rect::new(0, 0, 10, 10);
        let mut t = TextEdit::secret();
        t.paste("hunter2", &[]);
        let spot = t.spot(r, r);
        assert!(spot.secret);
        assert_eq!(spot.surrounding, None);
        let spot = e("abc").spot(r, r);
        assert_eq!(spot.surrounding.map(|s| s.cursor), Some(3));
    }

    #[test]
    fn steps_whole_utf8_chars() {
        let mut t = e("aé");
        t.key(EditKey::Left {
            select: false,
            word: false,
        });
        assert_eq!(t.caret, 1);
        t.key(EditKey::Right {
            select: true,
            word: false,
        });
        assert_eq!(t.selection(), 1..3);
        t.key(EditKey::Backspace { word: false });
        assert_eq!(t.text, "a");
    }
}
