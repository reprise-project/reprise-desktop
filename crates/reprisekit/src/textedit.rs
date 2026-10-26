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
    undo: Undo<usize>,
    secret: bool,
    cue: String,
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
            .field("cue", &self.cue)
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

/// A text position an undo log can step over inserted text with.
pub(crate) trait Spot: Copy + Eq {
    /// Where `s` ends when it starts here.
    fn after(self, s: &str) -> Self;
}

impl Spot for usize {
    fn after(self, s: &str) -> usize {
        self + s.len()
    }
}

/// The text an undo log replays its ops on.
pub(crate) trait Doc<P> {
    fn put(&mut self, at: P, s: &str);
    /// Remove `s`, which starts at `at`.
    fn cut(&mut self, at: P, s: &str);
}

impl Doc<usize> for String {
    fn put(&mut self, at: usize, s: &str) {
        self.insert_str(at, s);
    }

    fn cut(&mut self, at: usize, s: &str) {
        self.replace_range(at..at + s.len(), "");
    }
}

/// One change to the text, holding only the text it put in or took out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op<P> {
    Insert { at: P, text: String },
    Delete { at: P, text: String },
}

impl<P> Op<P> {
    fn text(&self) -> &str {
        match self {
            Op::Insert { text, .. } | Op::Delete { text, .. } => text,
        }
    }
}

/// One undo step: its ops in order, and the caret and anchor before the first.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Step<P> {
    ops: Vec<Op<P>>,
    caret: P,
    anchor: P,
}

impl<P> Step<P> {
    fn bytes(&self) -> usize {
        self.ops.iter().map(|op| op.text().len()).sum()
    }
}

/// Most undo steps kept; the oldest go first.
const UNDO_STEPS: usize = 1000;
/// Most text the undo steps hold, in bytes. The newest step stays whatever its size.
const UNDO_BYTES: usize = 8 << 20;

/// Undo and redo for the single- and multi-line edits: a log of the [`Op`]s each step made, so a
/// step holds the text it changed and never a copy of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Undo<P> {
    done: std::collections::VecDeque<Step<P>>,
    undone: Vec<Step<P>>,
    /// Text held by `done`, in bytes.
    bytes: usize,
    /// The caret and anchor a fresh step starts from, set by a checkpoint that begins one.
    fresh: Option<(P, P)>,
    pub last: Edit,
}

impl<P> Default for Undo<P> {
    fn default() -> Self {
        Undo {
            done: Default::default(),
            undone: Vec::new(),
            bytes: 0,
            fresh: None,
            last: Edit::None,
        }
    }
}

impl<P: Spot> Undo<P> {
    /// Before an edit of `kind`: anything but a continued run of typing or deleting starts a
    /// new step, which opens with the next recorded op.
    pub fn checkpoint(&mut self, kind: Edit, caret: P, anchor: P) {
        if kind == Edit::None || kind != self.last || self.done.is_empty() {
            self.fresh = Some((caret, anchor));
        }
        self.last = kind;
    }

    /// Log an op the edit just made; a new edit drops whatever was undone.
    pub fn record(&mut self, op: Op<P>) {
        if op.text().is_empty() {
            return;
        }
        self.undone.clear();
        self.bytes += op.text().len();
        let step = match (self.fresh.take(), self.done.back_mut()) {
            (None, Some(step)) => step,
            (fresh, _) => {
                let (caret, anchor) = fresh.unwrap_or(match &op {
                    Op::Insert { at, .. } | Op::Delete { at, .. } => (*at, *at),
                });
                self.done.push_back(Step {
                    ops: Vec::new(),
                    caret,
                    anchor,
                });
                self.done.back_mut().expect("just pushed")
            }
        };
        match (step.ops.last_mut(), op) {
            (Some(Op::Insert { at, text }), Op::Insert { at: next, text: t })
                if at.after(text) == next =>
            {
                text.push_str(&t);
            }
            (Some(Op::Delete { at, text }), Op::Delete { at: next, text: t })
                if next.after(&t) == *at =>
            {
                text.insert_str(0, &t);
                *at = next;
            }
            (Some(Op::Delete { at, text }), Op::Delete { at: next, text: t }) if next == *at => {
                text.push_str(&t);
            }
            (_, op) => step.ops.push(op),
        }
        while self.done.len() > 1 && (self.done.len() > UNDO_STEPS || self.bytes > UNDO_BYTES) {
            let old = self.done.pop_front().expect("more than one step");
            self.bytes -= old.bytes();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// Ctrl+Z: take the last step back out of `doc`, returning the caret and anchor it started
    /// from; `None` with nothing to undo.
    pub fn undo(&mut self, doc: &mut impl Doc<P>) -> Option<(P, P)> {
        let step = self.done.pop_back()?;
        self.bytes -= step.bytes();
        for op in step.ops.iter().rev() {
            match op {
                Op::Insert { at, text } => doc.cut(*at, text),
                Op::Delete { at, text } => doc.put(*at, text),
            }
        }
        let back = (step.caret, step.anchor);
        self.undone.push(step);
        self.fresh = None;
        self.last = Edit::None;
        Some(back)
    }

    /// Ctrl+Y: make the last undone step again, returning where the caret ends up.
    pub fn redo(&mut self, doc: &mut impl Doc<P>) -> Option<P> {
        let step = self.undone.pop()?;
        let mut caret = step.caret;
        for op in &step.ops {
            caret = match op {
                Op::Insert { at, text } => {
                    doc.put(*at, text);
                    at.after(text)
                }
                Op::Delete { at, text } => {
                    doc.cut(*at, text);
                    *at
                }
            };
        }
        self.bytes += step.bytes();
        self.done.push_back(step);
        self.fresh = None;
        self.last = Edit::None;
        Some(caret)
    }

    /// Forget every step, zeroizing the text they held.
    pub fn clear(&mut self) {
        for step in self.done.iter_mut().chain(self.undone.iter_mut()) {
            for op in &mut step.ops {
                match op {
                    Op::Insert { text, .. } | Op::Delete { text, .. } => text.zeroize(),
                }
            }
        }
        *self = Undo::default();
    }

    #[cfg(test)]
    pub fn held(&self) -> (usize, usize) {
        (self.done.iter().map(|s| s.ops.len()).sum(), self.bytes)
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
    /// Ctrl+Y or Ctrl+Shift+Z: make the last undone step again.
    Redo,
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

/// Text an input method is composing: drawn underlined at the focused edit's caret, not part of the text until committed. `cursor` is the caret's byte offset in it (`None` hides the caret).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    pub cursor: Option<usize>,
}

impl Preedit {
    /// A preedit of `text` with the caret at `end`, from text-input-v3's `begin..end`: a range is not drawn, only its end. A `begin` that is negative, past the end or inside a char hides the caret; a bad `end` puts it at `begin`.
    pub fn new(text: String, begin: i32, end: i32) -> Option<Preedit> {
        if text.is_empty() {
            return None;
        }
        let at = |i: i32| {
            usize::try_from(i)
                .ok()
                .filter(|&c| text.is_char_boundary(c))
        };
        let cursor = at(begin).map(|b| at(end).filter(|&e| e >= b).unwrap_or(b));
        Some(Preedit { text, cursor })
    }

    /// `text` with the preedit put in at byte `at`: the string to show, the preedit's byte range, and the caret's offset in the string.
    pub fn splice(&self, text: &str, at: usize) -> (String, std::ops::Range<usize>, usize) {
        let at = text.floor_char_boundary(at);
        let mut shown = String::with_capacity(text.len() + self.text.len());
        shown.push_str(&text[..at]);
        shown.push_str(&self.text);
        shown.push_str(&text[at..]);
        let run = at..at + self.text.len();
        (shown, run, at + self.cursor.unwrap_or(self.text.len()))
    }

    /// `own` with the preedit put in at its caret, for a single-line painter: the edit to draw (nothing selected), the preedit's byte range, and whether the caret bar shows.
    pub fn compose(&self, own: &TextEdit) -> (TextEdit, std::ops::Range<usize>, bool) {
        let (t, run, caret) = self.splice(&own.text, own.caret());
        let mut e = TextEdit::new(t, false);
        e.set_selection(caret, caret);
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
    /// The edit has a step to undo, for its edit menu's Undo.
    pub can_undo: bool,
    /// The edit has an undone step to make again, for its edit menu's Redo.
    pub can_redo: bool,
    pub selected: bool,
    /// A digits-only field (a spin box): no input method turns on over it, so its keys type
    /// the digits themselves.
    pub digits: bool,
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
            cue: String::new(),
        }
    }

    /// A rename box holding `name`: all of it selected when `whole`, else its stem as
    /// `Path::file_stem` has it (a leading dot is part of the stem: `.bashrc` has no extension).
    pub fn rename(name: &str, whole: bool) -> TextEdit {
        let mut ed = TextEdit::new(name, true);
        if whole {
            return ed;
        }
        let stem = std::path::Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name);
        if !stem.is_empty() && stem.len() < name.len() {
            ed.set_selection(0, stem.len());
        }
        ed
    }

    pub fn secret() -> TextEdit {
        TextEdit {
            text: String::with_capacity(SECRET_CAPACITY),
            caret: 0,
            anchor: 0,
            undo: Undo::default(),
            secret: true,
            cue: String::new(),
        }
    }

    pub fn is_secret(&self) -> bool {
        self.secret
    }

    /// Greyed text the field shows while it is empty; it is never part of the value.
    pub fn cue(&self) -> &str {
        &self.cue
    }

    pub fn set_cue(&mut self, cue: impl Into<String>) {
        self.cue = cue.into();
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
            self.undo.checkpoint(kind, self.caret, self.anchor);
        }
    }

    pub fn set_selected(&mut self, text: impl Into<String>) {
        let secret = self.secret;
        let cue = std::mem::take(&mut self.cue);
        *self = TextEdit::new(text, true);
        self.secret = secret;
        self.cue = cue;
    }

    /// Zeroize the text and undo log in place and leave the field empty. The buffer is kept, so a secret field's reservation outlives a retry.
    pub fn wipe(&mut self) {
        self.text.zeroize();
        self.undo.clear();
        self.caret = 0;
        self.anchor = 0;
    }

    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// A right press at byte `index`: inside the selection it stays for the edit menu to act
    /// on, anywhere else the caret moves there.
    pub fn press_for_menu(&mut self, index: usize) -> bool {
        let sel = self.selection();
        if self.has_selection() && (sel.start..=sel.end).contains(&index) {
            return false;
        }
        self.place(index, false)
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
                    self.replace(sel.clone(), &s);
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
                    self.replace(from..self.caret, "");
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
                    self.replace(self.caret..to, "");
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
                if let Some((caret, anchor)) = self.undo.undo(&mut self.text) {
                    self.caret = self.text.floor_char_boundary(caret);
                    self.anchor = self.text.floor_char_boundary(anchor);
                    edited = true;
                }
            }
            EditKey::Redo => {
                if let Some(caret) = self.undo.redo(&mut self.text) {
                    self.caret = self.text.floor_char_boundary(caret);
                    self.anchor = self.caret;
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
                    self.replace(sel.end..to, "");
                    self.replace(from..sel.start, "");
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
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            selected: self.has_selection(),
            digits: false,
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

    /// A drop, taken as [`TextEdit::paste`] takes it, put in at byte `at` and selected; the selection there was is let go.
    /// Paths get a space on each side that touches a non-space char, which stays unselected.
    pub fn drop_text(&mut self, at: usize, line: &DropLine) -> bool {
        let at = self.text.floor_char_boundary(at);
        let text = line.text();
        let (lead, trail) = match line {
            DropLine::Paths(p) if !p.is_empty() => {
                let apart = |c: Option<char>| c.is_some_and(|c| !c.is_whitespace());
                (
                    apart(self.text[..at].chars().next_back()),
                    apart(self.text[at..].chars().next()),
                )
            }
            _ => (false, false),
        };
        let (caret, anchor) = (self.caret, self.anchor);
        (self.caret, self.anchor) = (at, at);
        let padded;
        let text = match (lead, trail) {
            (false, false) => text,
            _ => {
                padded = format!(
                    "{}{text}{}",
                    if lead { " " } else { "" },
                    if trail { " " } else { "" }
                );
                padded.as_str()
            }
        };
        if !self.paste(text, &[]) {
            (self.caret, self.anchor) = (caret, anchor);
            return false;
        }
        self.anchor = (at + usize::from(lead)).min(self.caret);
        if trail && self.caret > self.anchor && self.text[..self.caret].ends_with(' ') {
            self.caret -= 1;
        }
        true
    }

    /// [`TextEdit::splice`], logged for undo unless the field is secret.
    fn replace(&mut self, range: std::ops::Range<usize>, s: &str) {
        if !self.secret {
            let at = range.start;
            self.undo.record(Op::Delete {
                at,
                text: self.text[range.clone()].to_string(),
            });
            self.undo.record(Op::Insert {
                at,
                text: s.to_string(),
            });
        }
        self.splice(range, s);
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
        self.replace(sel.clone(), "");
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

/// The editing key a press stands for, modifiers folded in. Ctrl+A / C / X / V / Y / Z (Shift+Z redoes) are commands, Shift extends, Ctrl steps words, other printable input is an insert. None for keys an edit has no use for.
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
            0x79 => Some(EditKey::Redo),
            0x7a if shift => Some(EditKey::Redo),
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

/// Byte index of the leftmost visible character so the caret stays inside a field `inner_w` px wide: [`first_visible_at`] the caret.
pub fn edit_first_visible(
    text: &crate::text::TextRenderer,
    edit: &TextEdit,
    inner_w: i32,
    px: u32,
    first: usize,
) -> usize {
    first_visible_at(text, &edit.text, edit.caret(), inner_w, px, first)
}

/// Byte index of the leftmost visible character so byte `at` of `s` stays inside a field `inner_w` px wide. Scrolls by whole characters; each walk stops once past `inner_w`, so the cost follows what is on screen.
pub fn first_visible_at(
    text: &crate::text::TextRenderer,
    s: &str,
    at: usize,
    inner_w: i32,
    px: u32,
    first: usize,
) -> usize {
    let adv = |ch| text.advance(ch, false, px);
    let at = s.floor_char_boundary(at);
    let mut first = first.min(s.len());
    if !s.is_char_boundary(first) {
        first = 0;
    }
    if at < first {
        first = at;
    }
    let mut used = 0;
    let mut start = at;
    for (i, ch) in s[first..at].char_indices().rev() {
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

/// The scroll offset one character on for a drag resting `x` px into a field `inner_w` px wide that shows `s` from byte `first`, None when it stays: back a character in the `band` px at the left (or left of the field) while text is hidden there, on one in the band at the right while text runs past it. The host calls it on a tick while the drag rests.
pub fn drag_scroll(
    text: &crate::text::TextRenderer,
    s: &str,
    first: usize,
    inner_w: i32,
    band: i32,
    x: i32,
    px: u32,
) -> Option<usize> {
    let first = first.min(s.len());
    if x < band {
        return (first > 0).then(|| words::prev_char(s, first));
    }
    if x >= inner_w - band && text.measure(&s[first..], false, px) > inner_w {
        return Some(words::next_char(s, first));
    }
    None
}

/// What a drag drops on a field: text as it came, or a line of paths ([`paths_line`]) kept
/// apart from the text around the drop point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropLine {
    Text(String),
    Paths(String),
}

impl DropLine {
    pub fn text(&self) -> &str {
        match self {
            DropLine::Text(t) | DropLine::Paths(t) => t,
        }
    }
}

/// Dropped paths as one line for a field: space-separated, a path holding a space or a character
/// a shell reads specially in single quotes.
pub fn paths_line<P: AsRef<std::path::Path>>(paths: &[P]) -> String {
    let plain = |c: char| c.is_alphanumeric() || "/._-+,:@%=~".contains(c);
    paths
        .iter()
        .map(|p| {
            let p = p.as_ref().to_string_lossy();
            if !p.is_empty() && p.chars().all(plain) {
                p.into_owned()
            } else {
                format!("'{}'", p.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    fn a_menu_press_keeps_the_selection_under_it_and_moves_the_caret_off_it() {
        let mut e = TextEdit::new("alpha beta", false);
        e.set_selection(6, 10);
        assert!(!e.press_for_menu(8));
        assert!(!e.press_for_menu(10), "its edge is inside");
        assert_eq!(e.selection(), 6..10);
        assert!(e.press_for_menu(2));
        assert_eq!((e.caret(), e.has_selection()), (2, false));
    }

    #[test]
    fn a_spot_says_whether_there_is_a_selection_and_an_undo() {
        let r = crate::raster::Rect::new(0, 0, 10, 10);
        let mut e = TextEdit::new("ab", false);
        let s = e.spot(r, r);
        assert!(!s.can_undo && !s.selected);
        e.key(EditKey::Backspace { word: false });
        e.key(EditKey::Left {
            select: true,
            word: false,
        });
        let s = e.spot(r, r);
        assert!(s.can_undo && s.selected);
    }

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

    #[test]
    fn a_rename_selects_the_stem_unless_whole() {
        assert_eq!(TextEdit::rename("notes.txt", false).selection(), 0..5);
        assert!(TextEdit::rename("Reports.d", true).all_selected());
        assert!(TextEdit::rename("README", false).all_selected());
        assert!(TextEdit::rename(".bashrc", false).all_selected());
        assert_eq!(
            TextEdit::rename("a.tar.gz", false).selection(),
            0.."a.tar".len()
        );
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
    fn undo_takes_back_a_run_of_typing_and_redo_makes_it_again() {
        let mut t = TextEdit::new("notepad", true);
        t.key(EditKey::Insert("c".into()));
        t.key(EditKey::Insert("a".into()));
        t.key(EditKey::Insert("l".into()));
        assert_eq!(t.text, "cal");
        assert!(t.key(EditKey::Undo));
        assert_eq!(t.text, "notepad");
        assert!(t.all_selected());
        assert!(!t.key(EditKey::Undo), "one step back is all there is");
        assert!(t.can_redo());
        assert!(t.key(EditKey::Redo));
        assert_eq!((t.text.as_str(), t.caret, t.anchor), ("cal", 3, 3));
        assert!(!t.key(EditKey::Redo));
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
    fn wipe_clears_the_field_and_its_undo_log() {
        let mut t = e("secret");
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
    fn undo_and_redo_walk_steps_one_at_a_time() {
        let mut t = e("");
        for c in ["a", "b"] {
            t.key(EditKey::Insert(c.into()));
        }
        t.paste("XY", &[]);
        t.key(EditKey::Backspace { word: false });
        t.key(EditKey::Backspace { word: false });
        assert_eq!(t.text, "ab");
        assert_eq!(t.undo.held().0, 3, "typing, the paste and the deleting run");
        t.key(EditKey::Undo);
        assert_eq!((t.text.as_str(), t.caret), ("abXY", 4));
        t.key(EditKey::Undo);
        assert_eq!((t.text.as_str(), t.caret), ("ab", 2));
        t.key(EditKey::Undo);
        assert_eq!(t.text, "");
        t.key(EditKey::Redo);
        t.key(EditKey::Redo);
        assert_eq!((t.text.as_str(), t.caret), ("abXY", 4));
        t.key(EditKey::Insert("z".into()));
        assert!(!t.can_redo(), "a new edit drops the undone steps");
        assert!(!t.key(EditKey::Redo));
    }

    #[test]
    fn a_secret_field_logs_nothing_to_undo() {
        let mut t = TextEdit::secret();
        t.paste("hunter2", &[]);
        t.key(EditKey::Backspace { word: false });
        assert_eq!(t.undo.held(), (0, 0));
        assert!(!t.key(EditKey::Undo));
    }

    #[test]
    fn the_undo_log_drops_its_oldest_steps_past_its_bound() {
        let mut u = Undo::<usize>::default();
        let mut doc = String::new();
        for i in 0..UNDO_STEPS + 5 {
            u.checkpoint(Edit::None, i, i);
            doc.push('x');
            u.record(Op::Insert {
                at: i,
                text: "x".into(),
            });
        }
        assert_eq!(u.held(), (UNDO_STEPS, UNDO_STEPS));
        while u.undo(&mut doc).is_some() {}
        assert_eq!(doc, "xxxxx", "the five oldest steps are gone");

        let mut u = Undo::<usize>::default();
        let big = "y".repeat(UNDO_BYTES / 2 + 1);
        for at in [0, big.len()] {
            u.checkpoint(Edit::None, at, at);
            u.record(Op::Insert {
                at,
                text: big.clone(),
            });
        }
        assert_eq!(
            u.held(),
            (1, big.len()),
            "over the byte bound the newest step stays"
        );
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
        assert_eq!(key_for(0x79, true, false, None), Some(EditKey::Redo));
        assert_eq!(key_for(0x5a, true, true, None), Some(EditKey::Redo));
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
        assert_eq!(p.splice("ab", 1), ("anihaob".into(), 1..6, 3));
        let hidden = Preedit::new("你好".into(), -1, -1).expect("not empty");
        assert_eq!(hidden.cursor, None);
        let end = "你好".len();
        assert_eq!(hidden.splice("", 0).2, end, "at its end");
        assert_eq!(Preedit::new("你".into(), 1, 1).unwrap().cursor, None);
        assert_eq!(Preedit::new(String::new(), 0, 0), None);
    }

    #[test]
    fn a_preedit_range_puts_the_caret_at_its_end_and_selects_nothing() {
        let p = Preedit::new("nihao".into(), 0, 5).expect("not empty");
        assert_eq!(p.cursor, Some(5));
        assert_eq!(p.splice("ab", 1).2, 6);
        let mut own = TextEdit::new("ab", false);
        own.place(1, false);
        let (e, run, bar) = p.compose(&own);
        assert_eq!((e.text.as_str(), run, bar), ("anihaob", 1..6, true));
        assert_eq!((e.selection(), e.caret()), (6..6, 6));
        let mid = Preedit::new("nihao".into(), 1, 3).expect("not empty");
        assert_eq!(mid.cursor, Some(3));
        let back = Preedit::new("nihao".into(), 3, 1).expect("not empty");
        assert_eq!(back.cursor, Some(3));
        let split = Preedit::new("你好".into(), 0, 1).expect("not empty");
        assert_eq!(split.cursor, Some(0));
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

    #[test]
    fn a_drag_resting_in_an_edge_band_scrolls_a_character_while_text_is_hidden_there() {
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let px = crate::theme::Theme::default().fonts.size_px;
        let s = "C:/a long path/that runs/past the field";
        let w = text.measure(&s[..10], false, px);
        let step = |first, x| drag_scroll(&text, s, first, w, 8, x, px);
        assert_eq!(step(0, w / 2), None, "the middle rests still");
        assert_eq!(step(0, 2), None, "nothing hidden at the left");
        assert_eq!(step(0, w - 2), Some(1), "the right band steps on");
        assert_eq!(step(3, -4), Some(2), "left of the field counts as the band");
        let end = first_visible_at(&text, s, s.len(), w, px, 0);
        assert_eq!(step(end, w - 2), None, "the end is in view");
        assert_eq!(drag_scroll(&text, "ab", 0, w, 8, w - 2, px), None);
        let e = "aé";
        assert_eq!(
            drag_scroll(&text, e, e.len(), 0, 8, 0, px),
            Some(1),
            "back a whole char"
        );
    }

    #[test]
    fn first_visible_at_keeps_any_index_in_view() {
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let px = crate::theme::Theme::default().fonts.size_px;
        let s = "0123456789abcdefghij";
        let w = text.measure("01234", false, px);
        assert_eq!(
            first_visible_at(&text, s, 2, w, px, 10),
            2,
            "an index left of the view pulls it back"
        );
        let first = first_visible_at(&text, s, 15, w, px, 0);
        assert!(text.measure(&s[first..15], false, px) <= w && first > 0);
        assert_eq!(
            first_visible_at(&text, s, 12, w, px, 10),
            10,
            "an index in view leaves it"
        );
        let caret = e("0123456789abcdefghij");
        assert_eq!(
            edit_first_visible(&text, &caret, w, px, 0),
            first_visible_at(&text, s, s.len(), w, px, 0),
            "the caret's view"
        );
    }

    #[test]
    fn dropped_text_lands_at_the_drop_point_selected() {
        let mut t = TextEdit::new("hello world", true);
        assert!(t.drop_text(5, &DropLine::Text(" big\nnext line".into())));
        assert_eq!(t.text, "hello big world", "the first line, as a paste");
        assert_eq!(t.selected_text(), " big");
        assert!(t.key(EditKey::Undo));
        assert_eq!(t.text, "hello world");
        let mut t = TextEdit::new("ab", false);
        assert!(!t.drop_text(1, &DropLine::Text("\u{7}".into())));
        assert_eq!(
            (t.text.as_str(), t.caret, t.anchor),
            ("ab", 2, 2),
            "nothing to drop moves nothing"
        );
    }

    fn drop_paths(text: &str, at: usize) -> (String, String) {
        let mut t = TextEdit::new(text, false);
        assert!(t.drop_text(at, &DropLine::Paths("'/tmp/a b'".into())));
        (t.text.clone(), t.selected_text().to_string())
    }

    #[test]
    fn dropped_paths_get_a_space_from_the_text_they_touch() {
        let sel = "'/tmp/a b'".to_string();
        assert_eq!(
            drop_paths("cat", 3),
            ("cat '/tmp/a b'".into(), sel.clone()),
            "at the end"
        );
        assert_eq!(
            drop_paths("cat", 0),
            ("'/tmp/a b' cat".into(), sel.clone()),
            "at the start"
        );
        assert_eq!(
            drop_paths("ab", 1),
            ("a '/tmp/a b' b".into(), sel.clone()),
            "mid-word"
        );
        assert_eq!(
            drop_paths("", 0),
            (sel.clone(), sel.clone()),
            "an empty field"
        );
        assert_eq!(
            drop_paths("a  b", 2),
            ("a '/tmp/a b' b".into(), sel),
            "between spaces"
        );
    }

    #[test]
    fn dropped_text_gets_no_space() {
        let mut t = TextEdit::new("cat", false);
        assert!(t.drop_text(3, &DropLine::Text("x".into())));
        assert_eq!(t.text, "catx");
        assert_eq!(t.selected_text(), "x");
    }

    #[test]
    fn dropped_paths_join_with_spaces_and_odd_ones_are_quoted() {
        assert_eq!(paths_line(&["/usr/bin/foot"]), "/usr/bin/foot");
        assert_eq!(
            paths_line(&["/home/u/My Files/a.txt", "/tmp/b.txt"]),
            "'/home/u/My Files/a.txt' /tmp/b.txt"
        );
        assert_eq!(paths_line(&["/tmp/it's"]), "'/tmp/it'\\''s'");
        assert_eq!(paths_line(&["/tmp/$HOME;x"]), "'/tmp/$HOME;x'");
        assert_eq!(paths_line::<&str>(&[]), "");
    }
}
