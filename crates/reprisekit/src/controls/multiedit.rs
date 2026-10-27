use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    self, Axis, BarHit, ScrollHit, ScrollParts, axis_lit, scrollbar_parts,
};
use crate::controls::scrollbar::{Press, ThumbDrag};
use crate::look::Paint;
use crate::look::widgets::{draw_edit_frame, draw_hscrollbar, draw_scroll_corner, draw_scrollbar};
use crate::pointer::Pointer;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::{
    Caret, Doc, Edit, EditKey, Op, Spot, Surrounding, TextSpot, Undo, index_at, words,
};
use accesskit::{NodeId, Role};
use std::cell::RefCell;

/// A place in the text: line index and byte offset on that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Spot for Pos {
    fn after(self, s: &str) -> Pos {
        match s.rfind('\n') {
            None => Pos {
                line: self.line,
                col: self.col + s.len(),
            },
            Some(i) => Pos {
                line: self.line + s.bytes().filter(|&b| b == b'\n').count(),
                col: s.len() - i - 1,
            },
        }
    }
}

/// How a search runs: a Match case check and an Up/Down group. Nothing wraps around.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindOpts {
    pub match_case: bool,
    pub down: bool,
}

/// Byte length of `needle` if it matches at byte `i` of `hay`; compared per char because case
/// folding can change byte lengths.
fn match_at(hay: &str, i: usize, needle: &str, match_case: bool) -> Option<usize> {
    let mut h = hay[i..].chars();
    let mut used = 0;
    for nc in needle.chars() {
        let hc = h.next()?;
        let same = if match_case {
            hc == nc
        } else {
            hc.to_lowercase().eq(nc.to_lowercase())
        };
        if !same {
            return None;
        }
        used += hc.len_utf8();
    }
    Some(used)
}

/// The first match at or after `from` when `o.down`, else the last one before it. `None` is a miss.
pub fn find(lines: &[String], query: &str, from: Pos, o: &FindOpts) -> Option<(Pos, Pos)> {
    if query.is_empty() || lines.is_empty() {
        return None;
    }
    let start_line = from.line.min(lines.len() - 1);
    if o.down {
        for (line, s) in lines.iter().enumerate().skip(start_line) {
            let lo = if line == from.line {
                from.col.min(s.len())
            } else {
                0
            };
            for col in lo..=s.len() {
                if !s.is_char_boundary(col) {
                    continue;
                }
                if let Some(len) = match_at(s, col, query, o.match_case) {
                    return Some((
                        Pos { line, col },
                        Pos {
                            line,
                            col: col + len,
                        },
                    ));
                }
            }
        }
    } else {
        for (line, s) in lines[..=start_line].iter().enumerate().rev() {
            let hi = if line == from.line {
                from.col.min(s.len())
            } else {
                s.len() + 1
            };
            let mut best = None;
            for col in 0..hi.min(s.len() + 1) {
                if !s.is_char_boundary(col) {
                    continue;
                }
                if let Some(len) = match_at(s, col, query, o.match_case) {
                    best = Some((
                        Pos { line, col },
                        Pos {
                            line,
                            col: col + len,
                        },
                    ));
                }
            }
            if best.is_some() {
                return best;
            }
        }
    }
    None
}

/// Replace every match in the whole document and report how many; `o.down` is ignored.
/// Each line is scanned once, left to right, so a replacement containing the query terminates.
pub fn replace_all(lines: &mut [String], query: &str, with: &str, o: &FindOpts) -> usize {
    let mut n = 0;
    for line in lines.iter_mut() {
        if let Some((out, k)) = replace_in(line, query, with, o) {
            *line = out;
            n += k;
        }
    }
    n
}

/// `line` with every match replaced and the match count; `None` when nothing matches.
fn replace_in(line: &str, query: &str, with: &str, o: &FindOpts) -> Option<(String, usize)> {
    if query.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(line.len());
    let (mut i, mut n) = (0, 0);
    while i < line.len() {
        if let Some(used) = match_at(line, i, query, o.match_case) {
            out.push_str(with);
            i += used;
            n += 1;
        } else {
            let ch = line[i..]
                .chars()
                .next()
                .expect("i is a char boundary in the line");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    (n > 0).then_some((out, n))
}

/// The 1-based character column of byte offset `col`, for the status bar.
/// A tab counts as one character: `ab` + Tab + `c` is Col 5.
pub fn char_col(line: &str, col: usize) -> usize {
    let mut end = col.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].chars().count() + 1
}

/// A key the multi-line edit takes: the single-line keys, plus the vertical moves and the line
/// break.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MultiKey {
    Edit(EditKey),
    Up { select: bool },
    Down { select: bool },
    PageUp { select: bool, rows: usize },
    PageDown { select: bool, rows: usize },
    DocHome { select: bool },
    DocEnd { select: bool },
    Enter,
    Tab,
}

/// The text with its caret and anchor; `caret == anchor` means nothing is selected.
/// Invariant: `lines` holds at least one line, and both positions sit on a char boundary of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiEdit {
    lines: Vec<String>,
    caret: Pos,
    anchor: Pos,
    /// The x the caret aims for on vertical moves (px).
    pub goal_x: Option<i32>,
    undo: Undo<Pos>,
    /// Bumped by every change to `lines`, so `key` reports a change without comparing documents.
    edits: u64,
    /// Per-line widths for `widest_px`; an edit forgets only the lines it touched.
    widths: RefCell<Widths>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Widths {
    key: (u32, usize),
    px: Vec<Option<i32>>,
    widest: Option<i32>,
}

impl Doc<Pos> for MultiEdit {
    fn put(&mut self, at: Pos, s: &str) {
        self.put_at(at, s);
    }

    fn cut(&mut self, at: Pos, s: &str) {
        self.cut_range(at, at.after(s));
    }
}

impl Default for MultiEdit {
    fn default() -> Self {
        MultiEdit::new("")
    }
}

impl MultiEdit {
    /// `text` split on line breaks (`\r\n` and `\n`), the caret at the start.
    pub fn new(text: &str) -> MultiEdit {
        let lines: Vec<String> = text
            .replace("\r\n", "\n")
            .split('\n')
            .map(str::to_string)
            .collect();
        MultiEdit {
            lines,
            caret: Pos::default(),
            anchor: Pos::default(),
            goal_x: None,
            undo: Undo::default(),
            edits: 0,
            widths: RefCell::default(),
        }
    }

    /// Replace every match ([`replace_all`]) as one undo step, returning the count.
    pub fn replace_all(&mut self, query: &str, with: &str, o: &FindOpts) -> usize {
        let any = FindOpts { down: true, ..*o };
        if find(&self.lines, query, Pos::default(), &any).is_none() {
            return 0;
        }
        self.checkpoint(Edit::None);
        let mut n = 0;
        for line in 0..self.lines.len() {
            let Some((new, k)) = replace_in(&self.lines[line], query, with, o) else {
                continue;
            };
            n += k;
            let at = Pos { line, col: 0 };
            let old = std::mem::replace(&mut self.lines[line], new);
            self.undo.record(Op::Delete { at, text: old });
            self.undo.record(Op::Insert {
                at,
                text: self.lines[line].clone(),
            });
        }
        self.replaced_lines();
        self.caret = self.clamp(self.caret);
        self.anchor = self.caret;
        self.goal_x = None;
        n
    }

    /// The lines, without their breaks; never empty.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Where the caret is; the selection runs from here to the anchor.
    pub fn caret(&self) -> Pos {
        self.caret
    }

    /// What an input method gets of this edit filling `field`, its caret drawn at `caret`: the
    /// caret's line, the anchor on it only when the selection starts and ends there.
    pub fn spot(&self, field: Rect, caret: Rect) -> TextSpot {
        let line = &self.lines[self.caret.line];
        let anchor = if self.anchor.line == self.caret.line {
            self.anchor.col
        } else {
            self.caret.col
        };
        TextSpot {
            field,
            caret,
            surrounding: Some(Surrounding::around(line, self.caret.col, anchor)),
            secret: false,
            multiline: true,
            can_undo: self.can_undo(),
            can_redo: self.can_redo(),
            selected: self.has_selection(),
            digits: false,
        }
    }

    /// The whole text, lines joined by `\n`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
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

    /// The selected range, ordered.
    pub fn selection(&self) -> (Pos, Pos) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        if a == b {
            return String::new();
        }
        if a.line == b.line {
            return self.lines[a.line][a.col..b.col].to_string();
        }
        let mut out = self.lines[a.line][a.col..].to_string();
        for l in &self.lines[a.line + 1..b.line] {
            out.push('\n');
            out.push_str(l);
        }
        out.push('\n');
        out.push_str(&self.lines[b.line][..b.col]);
        out
    }

    pub fn select_all(&mut self) {
        self.anchor = Pos::default();
        self.caret = self.end();
        self.goal_x = None;
    }

    fn end(&self) -> Pos {
        let line = self.lines.len() - 1;
        Pos {
            line,
            col: self.lines[line].len(),
        }
    }

    /// Every line may have changed: a change to report, and no width to trust.
    fn replaced_lines(&mut self) {
        self.edits += 1;
        *self.widths.get_mut() = Widths::default();
    }

    fn forget_widths(&mut self, at: usize, old: usize, new: usize) {
        let w = self.widths.get_mut();
        if at + old <= w.px.len() {
            w.px.splice(at..at + old, std::iter::repeat_n(None, new));
        }
        w.widest = None;
    }

    fn checkpoint(&mut self, kind: Edit) {
        self.undo.checkpoint(kind, self.caret, self.anchor);
    }

    /// Remove `a..b` without logging it.
    fn cut_range(&mut self, a: Pos, b: Pos) {
        self.edits += 1;
        let tail = self.lines[b.line][b.col..].to_string();
        self.lines[a.line].truncate(a.col);
        self.lines[a.line].push_str(&tail);
        self.lines.drain(a.line + 1..=b.line);
        self.forget_widths(a.line, b.line - a.line + 1, 1);
    }

    /// Put `s` (`\n` line breaks) in at `p` without logging it, returning where it ends.
    fn put_at(&mut self, p: Pos, s: &str) -> Pos {
        self.edits += 1;
        let Pos { line, col } = p;
        let tail = self.lines[line].split_off(col);
        let mut parts = s.split('\n').map(str::to_string);
        let first = parts.next().unwrap_or_default();
        self.lines[line].push_str(&first);
        let mut at = line;
        for part in parts {
            at += 1;
            self.lines.insert(at, part);
        }
        let end = self.lines[at].len();
        self.lines[at].push_str(&tail);
        self.forget_widths(line, 1, at - line + 1);
        Pos { line: at, col: end }
    }

    fn delete_selection(&mut self) {
        let (a, b) = self.selection();
        if a == b {
            return;
        }
        let text = self.selected_text();
        self.undo.record(Op::Delete { at: a, text });
        self.cut_range(a, b);
        self.caret = a;
        self.anchor = a;
    }

    fn insert(&mut self, s: &str) {
        self.delete_selection();
        if s.is_empty() {
            return;
        }
        let normalised = s.replace("\r\n", "\n");
        let at = self.caret;
        let end = self.put_at(at, &normalised);
        self.undo.record(Op::Insert {
            at,
            text: normalised,
        });
        self.caret = end;
        self.anchor = end;
    }

    /// Ctrl+Z or Ctrl+Y: replay the undo log on the lines.
    fn step_undo(&mut self, redo: bool) {
        let mut undo = std::mem::take(&mut self.undo);
        let to = if redo {
            undo.redo(self).map(|c| (c, c))
        } else {
            undo.undo(self)
        };
        self.undo = undo;
        if let Some((caret, anchor)) = to {
            self.caret = self.clamp(caret);
            self.anchor = self.clamp(anchor);
        }
    }

    /// One scalar back, a line break counting as one.
    fn prev(&self, p: Pos) -> Pos {
        self.back(p, words::prev_char)
    }

    fn next(&self, p: Pos) -> Pos {
        self.on(p, words::next_char)
    }

    /// One grapheme cluster back, a line break counting as one.
    fn prev_cluster(&self, p: Pos) -> Pos {
        self.back(p, words::prev_cluster)
    }

    fn next_cluster(&self, p: Pos) -> Pos {
        self.on(p, words::next_cluster)
    }

    fn back(&self, p: Pos, step: fn(&str, usize) -> usize) -> Pos {
        if p.col > 0 {
            let col = step(&self.lines[p.line], p.col);
            Pos { line: p.line, col }
        } else if p.line > 0 {
            Pos {
                line: p.line - 1,
                col: self.lines[p.line - 1].len(),
            }
        } else {
            p
        }
    }

    fn on(&self, p: Pos, step: fn(&str, usize) -> usize) -> Pos {
        let line = &self.lines[p.line];
        if p.col < line.len() {
            let col = step(line, p.col);
            Pos { line: p.line, col }
        } else if p.line + 1 < self.lines.len() {
            Pos {
                line: p.line + 1,
                col: 0,
            }
        } else {
            p
        }
    }

    fn word_left(&self, p: Pos) -> Pos {
        if p.col == 0 {
            return self.prev(p);
        }
        Pos {
            line: p.line,
            col: words::word_left(&self.lines[p.line], p.col),
        }
    }

    fn word_right(&self, p: Pos) -> Pos {
        let line = &self.lines[p.line];
        if p.col >= line.len() {
            return self.next(p);
        }
        Pos {
            line: p.line,
            col: words::word_right(line, p.col),
        }
    }

    fn move_to(&mut self, to: Pos, select: bool) {
        self.caret = to;
        if !select {
            self.anchor = to;
        }
    }

    /// Put the caret at `p` (a click), extending from the anchor when `extend`.
    pub fn place(&mut self, p: Pos, extend: bool) -> bool {
        let before = (self.caret, self.anchor);
        let p = self.clamp(p);
        self.move_to(p, extend);
        self.goal_x = None;
        self.undo.last = Edit::None;
        (self.caret, self.anchor) != before
    }

    fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        let text = &self.lines[line];
        let mut col = p.col.min(text.len());
        while !text.is_char_boundary(col) {
            col -= 1;
        }
        Pos { line, col }
    }

    /// Select the word at `p` (a double click), with its trailing spaces.
    pub fn select_word_at(&mut self, p: Pos) {
        let p = self.clamp(p);
        let span = words::word_span_at(&self.lines[p.line], p.col);
        self.anchor = Pos {
            line: p.line,
            col: span.start,
        };
        self.caret = Pos {
            line: p.line,
            col: span.end,
        };
        self.undo.last = Edit::None;
    }

    /// Paste `text` over the selection (all its lines).
    pub fn paste(&mut self, text: &str) -> bool {
        let clean = printable(text);
        if clean.is_empty() {
            return false;
        }
        self.checkpoint(Edit::None);
        self.insert(&clean);
        self.goal_x = None;
        true
    }

    /// Insert dropped `text` at `p`, leaving the selection alone and selecting what landed.
    pub fn drop_text(&mut self, p: Pos, text: &str) -> bool {
        let clean = printable(text);
        if clean.is_empty() {
            return false;
        }
        self.checkpoint(Edit::None);
        let p = self.clamp(p);
        (self.caret, self.anchor) = (p, p);
        self.insert(&clean);
        self.anchor = p;
        self.goal_x = None;
        true
    }

    /// `p` moved `n` bytes, a line break counting as one byte; a count ending inside a char takes
    /// it whole.
    fn walk(&self, mut p: Pos, mut n: usize, fwd: bool) -> Pos {
        while n > 0 {
            let q = if fwd { self.next(p) } else { self.prev(p) };
            if q == p {
                break;
            }
            let bytes = if q.line == p.line {
                q.col.abs_diff(p.col)
            } else {
                1
            };
            n = n.saturating_sub(bytes);
            p = q;
        }
        p
    }

    fn delete_surrounding(&mut self, before: usize, after: usize) {
        let (a, b) = self.selection();
        let (from, to) = (self.walk(a, before, false), self.walk(b, after, true));
        if (from, to) == (a, b) {
            return;
        }
        let caret_first = self.caret == a;
        self.checkpoint(Edit::None);
        (self.anchor, self.caret) = (b, to);
        self.delete_selection();
        (self.anchor, self.caret) = (from, a);
        self.delete_selection();
        let end = if b.line == a.line {
            Pos {
                line: from.line,
                col: from.col + b.col - a.col,
            }
        } else {
            Pos {
                line: b.line - (a.line - from.line),
                col: b.col,
            }
        };
        (self.caret, self.anchor) = if caret_first {
            (from, end)
        } else {
            (end, from)
        };
    }

    fn vertical(&mut self, line: usize, select: bool, text: &TextRenderer, m: &Metrics) {
        let goal = self
            .goal_x
            .unwrap_or_else(|| x_of(text, m, self, self.caret));
        let line = line.min(self.lines.len() - 1);
        let col = col_at(text, m, self, line, goal);
        self.move_to(Pos { line, col }, select);
        self.goal_x = Some(goal);
    }

    fn apply_edit(&mut self, k: EditKey) {
        match k {
            EditKey::Insert(s) => {
                let single = s.chars().count() == 1;
                if single && !self.has_selection() {
                    self.checkpoint(Edit::Typing);
                } else {
                    self.checkpoint(Edit::None);
                }
                self.insert(&s);
                if single {
                    self.undo.last = Edit::Typing;
                }
            }
            EditKey::Backspace { word } => {
                if self.has_selection() {
                    self.checkpoint(Edit::None);
                    self.delete_selection();
                } else if self.caret != Pos::default() {
                    self.checkpoint(Edit::Deleting);
                    let from = if word {
                        self.word_left(self.caret)
                    } else {
                        self.prev(self.caret)
                    };
                    self.anchor = from;
                    self.delete_selection();
                }
            }
            EditKey::Delete { word } => {
                if self.has_selection() {
                    self.checkpoint(Edit::None);
                    self.delete_selection();
                } else if self.caret != self.end() {
                    self.checkpoint(Edit::Deleting);
                    let to = if word {
                        self.word_right(self.caret)
                    } else {
                        self.next_cluster(self.caret)
                    };
                    self.anchor = to;
                    self.delete_selection();
                }
            }
            EditKey::Left { select, word } => {
                let to = if word {
                    self.word_left(self.caret)
                } else if self.has_selection() && !select {
                    self.selection().0
                } else {
                    self.prev_cluster(self.caret)
                };
                self.move_to(to, select);
            }
            EditKey::Right { select, word } => {
                let to = if word {
                    self.word_right(self.caret)
                } else if self.has_selection() && !select {
                    self.selection().1
                } else {
                    self.next_cluster(self.caret)
                };
                self.move_to(to, select);
            }
            EditKey::Home { select } => {
                let to = Pos {
                    line: self.caret.line,
                    col: 0,
                };
                self.move_to(to, select);
            }
            EditKey::End { select } => {
                let to = Pos {
                    line: self.caret.line,
                    col: self.lines[self.caret.line].len(),
                };
                self.move_to(to, select);
            }
            EditKey::SelectAll => self.select_all(),
            EditKey::Cut => {
                if self.has_selection() {
                    self.checkpoint(Edit::None);
                    self.delete_selection();
                }
            }
            EditKey::Copy | EditKey::Paste => {}
            EditKey::Undo => self.step_undo(false),
            EditKey::Redo => self.step_undo(true),
            EditKey::DeleteSurrounding { before, after } => {
                self.delete_surrounding(before, after);
            }
        }
    }

    /// [`MultiEdit::key`] for an editing key alone, for hosts without a text renderer (a form's
    /// memo).
    pub fn edit_key(&mut self, k: EditKey) -> bool {
        let before = (self.edits, self.caret, self.anchor);
        self.apply_edit(k);
        self.goal_x = None;
        if self.edits == before.0 {
            self.undo.last = Edit::None;
        }
        (self.edits, self.caret, self.anchor) != before
    }

    pub fn step_lines(&mut self, down: bool, n: usize) -> bool {
        let line = if down {
            (self.caret.line + n).min(self.lines.len() - 1)
        } else {
            self.caret.line.saturating_sub(n)
        };
        let chars = self.lines[self.caret.line][..self.caret.col]
            .chars()
            .count();
        let target = &self.lines[line];
        let col = target
            .char_indices()
            .nth(chars)
            .map_or(target.len(), |(i, _)| i);
        let to = Pos { line, col };
        let changed = to != self.caret || self.has_selection();
        self.move_to(to, false);
        self.goal_x = None;
        changed
    }

    pub fn key(&mut self, key: MultiKey, text: &TextRenderer, m: &Metrics) -> bool {
        let before = (self.edits, self.caret, self.anchor, self.goal_x);
        let mut keep_goal = false;
        match key {
            MultiKey::Edit(k) => self.apply_edit(k),
            MultiKey::Up { select } => {
                keep_goal = true;
                let line = self.caret.line.saturating_sub(1);
                self.vertical(line, select, text, m);
            }
            MultiKey::Down { select } => {
                keep_goal = true;
                let line = self.caret.line + 1;
                self.vertical(line, select, text, m);
            }
            MultiKey::PageUp { select, rows } => {
                keep_goal = true;
                let line = self.caret.line.saturating_sub(rows.max(1));
                self.vertical(line, select, text, m);
            }
            MultiKey::PageDown { select, rows } => {
                keep_goal = true;
                let line = self.caret.line + rows.max(1);
                self.vertical(line, select, text, m);
            }
            MultiKey::DocHome { select } => self.move_to(Pos::default(), select),
            MultiKey::DocEnd { select } => {
                let to = self.end();
                self.move_to(to, select);
            }
            MultiKey::Enter => {
                self.checkpoint(Edit::None);
                self.insert("\n");
            }
            MultiKey::Tab => {
                self.checkpoint(Edit::None);
                self.insert("\t");
            }
        }
        if !keep_goal {
            self.goal_x = None;
        }
        if self.edits == before.0 {
            self.undo.last = Edit::None;
        }
        (self.edits, self.caret, self.anchor, self.goal_x) != before
    }
}

/// A zero line pitch or tab width is read as 1. `font_px` is device px at the layout's scale; only
/// the logical sizes (pitch, bars, padding) scale.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub font_px: u32,
    pub line_h: i32,
    pub tab_cols: usize,
    /// Always reserve both scrollbars (a bar whose text fits draws disabled); off, a bar shows only
    /// on overflow.
    pub bars_always: bool,
    /// Always reserve the vertical scrollbar alone, as a dialog's comment box does; the horizontal
    /// one shows on overflow.
    pub vbar_always: bool,
    /// The scrollbars' thickness, logical px: the look's
    /// [`ControlMetrics::scroll`](crate::controls::metrics::ControlMetrics::scroll).
    pub bar: i32,
    /// The scrollbars' shortest thumb, logical px: the look's
    /// [`ControlMetrics::thumb_min`](crate::controls::metrics::ControlMetrics::thumb_min).
    pub thumb_min: i32,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            font_px: 13,
            line_h: 13,
            tab_cols: 8,
            bars_always: true,
            vbar_always: false,
            bar: crate::controls::metrics::ControlMetrics::CLASSIC.scroll,
            thumb_min: crate::controls::metrics::ControlMetrics::CLASSIC.thumb_min,
        }
    }
}

impl Metrics {
    /// Take the look's scrollbar thickness and shortest thumb from `cm`.
    pub fn follow_look(&mut self, cm: &crate::controls::metrics::ControlMetrics) {
        self.bar = cm.scroll;
        self.thumb_min = cm.thumb_min;
    }
}

fn expand(line: &str, tab_cols: usize) -> String {
    let tab_cols = tab_cols.max(1);
    let mut out = String::with_capacity(line.len());
    let mut col = 0;
    for ch in line.chars() {
        if ch == '\t' {
            let n = tab_cols - col % tab_cols;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

fn expanded_col(line: &str, col: usize, tab_cols: usize) -> usize {
    let tab_cols = tab_cols.max(1);
    let (mut e, mut vcol) = (0, 0);
    for ch in line[..col.min(line.len())].chars() {
        if ch == '\t' {
            let n = tab_cols - vcol % tab_cols;
            e += n;
            vcol += n;
        } else {
            e += ch.len_utf8();
            vcol += 1;
        }
    }
    e
}

fn raw_col(line: &str, ecol: usize, tab_cols: usize) -> usize {
    let tab_cols = tab_cols.max(1);
    let mut e = 0;
    let mut vcol = 0;
    for (i, ch) in line.char_indices() {
        if e >= ecol {
            return i;
        }
        let n = if ch == '\t' {
            tab_cols - vcol % tab_cols
        } else {
            1
        };
        let width = if ch == '\t' { n } else { ch.len_utf8() };
        if ecol < e + width {
            return if ecol - e < e + width - ecol {
                i
            } else {
                i + ch.len_utf8()
            };
        }
        e += width;
        vcol += n;
    }
    line.len()
}

/// The scroll state of a view onto a `MultiEdit`: the first line shown and the horizontal offset in
/// px.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct View {
    pub top: usize,
    pub left: i32,
    /// The part of either scrollbar the pointer rests on or holds, which the host tracks and `draw`
    /// lights.
    pub bars: Pointer<(Axis, ScrollHit)>,
}

/// The x of `p` in px from the text's origin.
pub fn x_of(text: &TextRenderer, m: &Metrics, edit: &MultiEdit, p: Pos) -> i32 {
    let line = &edit.lines[p.line.min(edit.lines.len() - 1)];
    let e = expand(line, m.tab_cols);
    let ecol = expanded_col(line, p.col, m.tab_cols);
    text.measure(&e[..ecol.min(e.len())], false, m.font_px)
}

/// The column on `line` nearest `x` px from the text's origin.
pub fn col_at(text: &TextRenderer, m: &Metrics, edit: &MultiEdit, line: usize, x: i32) -> usize {
    let raw = &edit.lines[line.min(edit.lines.len() - 1)];
    let e = expand(raw, m.tab_cols);
    raw_col(raw, index_at(text, &e, x, m.font_px), m.tab_cols)
}

/// The widest line's width in px at `m.font_px`.
pub fn widest_px(text: &TextRenderer, m: &Metrics, edit: &MultiEdit) -> i32 {
    let mut w = edit.widths.borrow_mut();
    let key = (m.font_px, m.tab_cols);
    if w.key != key || w.px.len() != edit.lines.len() {
        *w = Widths {
            key,
            px: vec![None; edit.lines.len()],
            widest: None,
        };
    }
    if let Some(widest) = w.widest {
        return widest;
    }
    let widest =
        w.px.iter_mut()
            .zip(&edit.lines)
            .map(|(px, line)| {
                *px.get_or_insert_with(|| text.measure(&expand(line, m.tab_cols), false, m.font_px))
            })
            .max()
            .unwrap_or(0);
    w.widest = Some(widest);
    widest
}

/// The layout of a multi-line edit filling `r`: the sunken edge, the white box, where the text
/// starts and which scrollbars show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    /// The white box inside the 2 px edge.
    pub inner: Rect,
    /// The rows of text that fit.
    pub rows: usize,
    pub vscroll: Option<Rect>,
    pub hscroll: Option<Rect>,
}

/// Lay the control out in `r` at scale `s` for `edit`'s size: a vertical scrollbar when the lines
/// outgrow the box and a horizontal one when the widest line does (both, always, with
/// `m.bars_always`).
pub fn layout(text: &TextRenderer, m: &Metrics, edit: &MultiEdit, r: Rect, s: i32) -> Layout {
    let inner = r.inset(2 * s);
    let line_h = m.line_h.max(1) * s;
    let widest = widest_px(text, m, edit);
    let mut rows = (inner.height / line_h).max(1) as usize;
    let mut box_w = inner.width;
    let mut box_h = inner.height;
    let sb = m.bar * s;
    let mut vscroll = m.bars_always || m.vbar_always || edit.lines.len() > rows;
    let mut hscroll = m.bars_always || widest + 2 * s > box_w;
    if vscroll {
        box_w -= sb;
        hscroll = hscroll || widest + 2 * s > box_w;
    }
    if hscroll {
        box_h -= sb;
        rows = (box_h / line_h).max(1) as usize;
        if !vscroll && edit.lines.len() > rows {
            vscroll = true;
            box_w -= sb;
        }
    }
    let inner_box = Rect::new(inner.x, inner.y, box_w, box_h);
    Layout {
        frame: r,
        inner: inner_box,
        rows,
        vscroll: vscroll.then(|| Rect::new(inner.x + box_w, inner.y, sb, box_h)),
        hscroll: hscroll.then(|| Rect::new(inner.x, inner.y + box_h, box_w, sb)),
    }
}

impl Layout {
    /// The box where the two scrollbars meet, when both show: the host draws
    /// `look::widgets::draw_scroll_corner` there with its grip while its window can be resized
    /// (plain face when maximized).
    pub fn corner(&self) -> Option<Rect> {
        match (self.vscroll, self.hscroll) {
            (Some(v), Some(h)) => Some(Rect::new(v.x, h.y, v.width, h.height)),
            _ => None,
        }
    }
}

/// Which of the control's scrollbars a point is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollBar {
    Vertical,
    Horizontal,
}

impl ScrollBar {
    /// Which of the view's bars this is.
    pub fn axis(self) -> Axis {
        match self {
            ScrollBar::Vertical => Axis::Vertical,
            ScrollBar::Horizontal => Axis::Horizontal,
        }
    }
}

fn vextent(edit: &MultiEdit, lay: &Layout) -> (usize, usize) {
    (lay.rows, edit.lines.len())
}

fn hextent(
    text: &TextRenderer,
    m: &Metrics,
    edit: &MultiEdit,
    lay: &Layout,
    s: i32,
) -> (usize, usize) {
    let total = widest_px(text, m, edit) + 2 * s;
    (lay.inner.width.max(1) as usize, total.max(1) as usize)
}

/// The parts of one bar in logical coordinates, if the layout has it.
pub fn bar_parts(
    text: &TextRenderer,
    m: &Metrics,
    edit: &MultiEdit,
    lay: &Layout,
    view: View,
    bar: ScrollBar,
) -> Option<ScrollParts> {
    match bar {
        ScrollBar::Vertical => {
            let (visible, total) = vextent(edit, lay);
            Some(scrollbar_parts(
                lay.vscroll?,
                1,
                view.top,
                visible,
                total,
                false,
                m.thumb_min,
            ))
        }
        ScrollBar::Horizontal => {
            let (visible, total) = hextent(text, m, edit, lay, 1);
            let pos = view.left.max(0) as usize;
            Some(scrollbar_parts(
                lay.hscroll?,
                1,
                pos,
                visible,
                total,
                true,
                m.thumb_min,
            ))
        }
    }
}

/// The scrollbar part under (`x`, `y`) and which bar; `None` off the bars and on the bare track of
/// a bar with no thumb.
pub fn bar_hit(
    text: &TextRenderer,
    m: &Metrics,
    edit: &MultiEdit,
    lay: &Layout,
    view: View,
    x: i32,
    y: i32,
) -> Option<(ScrollBar, BarHit)> {
    for bar in [ScrollBar::Vertical, ScrollBar::Horizontal] {
        let horizontal = bar == ScrollBar::Horizontal;
        let rect = if horizontal { lay.hscroll } else { lay.vscroll };
        if !rect.is_some_and(|r| r.contains(x, y)) {
            continue;
        }
        let p = bar_parts(text, m, edit, lay, view, bar)?;
        return match widgets::bar_hit(&p, x, y, horizontal) {
            BarHit::Track => None,
            hit => Some((bar, hit)),
        };
    }
    None
}

/// The view a thumb held in `thumb` scrolls to with the pointer at (`x`, `y`): a line for the
/// vertical bar, px for the horizontal.
pub fn drag_bar(
    text: &TextRenderer,
    m: &Metrics,
    edit: &MultiEdit,
    lay: &Layout,
    view: View,
    thumb: &ThumbDrag,
    at: (i32, i32),
) -> Option<View> {
    let bar = if thumb.grab(Axis::Vertical).is_some() {
        ScrollBar::Vertical
    } else {
        ScrollBar::Horizontal
    };
    bar_span(text, m, edit, lay, view, bar)?.drag(view, thumb, at)
}

/// One bar as a held thumb runs along it: track and thumb length, and the lines (vertical) or px
/// (horizontal) in view and in all. None of it changes while the thumb is held, so a host without a
/// `TextRenderer` at hand keeps the press's span and drags with [`BarSpan::drag`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarSpan {
    bar: ScrollBar,
    parts: ScrollParts,
    visible: usize,
    total: usize,
}

/// `bar`'s span as `lay` and `view` lay it out at 1x, if the layout has the bar.
pub fn bar_span(
    text: &TextRenderer,
    m: &Metrics,
    edit: &MultiEdit,
    lay: &Layout,
    view: View,
    bar: ScrollBar,
) -> Option<BarSpan> {
    let parts = bar_parts(text, m, edit, lay, view, bar)?;
    let (visible, total) = match bar {
        ScrollBar::Vertical => vextent(edit, lay),
        ScrollBar::Horizontal => hextent(text, m, edit, lay, 1),
    };
    Some(BarSpan {
        bar,
        parts,
        visible,
        total,
    })
}

impl BarSpan {
    pub fn axis(&self) -> Axis {
        self.bar.axis()
    }

    /// The view this bar's thumb, held in `thumb`, scrolls `view` to with the pointer at (`x`,
    /// `y`).
    pub fn drag(&self, view: View, thumb: &ThumbDrag, at: (i32, i32)) -> Option<View> {
        let pos = thumb.pos(self.axis(), &self.parts, at, self.visible, self.total)?;
        let mut v = view;
        match self.bar {
            ScrollBar::Vertical => v.top = pos,
            ScrollBar::Horizontal => v.left = pos as i32,
        }
        Some(v)
    }
}

impl View {
    /// The view a press on part `hit` of `bar` scrolls to, at 1x: a line (a character cell
    /// sideways) per arrow and a page per press on the track, held to the text's ends.
    pub fn press_bar(
        self,
        text: &TextRenderer,
        m: &Metrics,
        edit: &MultiEdit,
        lay: &Layout,
        bar: ScrollBar,
        hit: BarHit,
    ) -> View {
        let (line, page) = match bar {
            ScrollBar::Vertical => (1, lay.rows as i32),
            ScrollBar::Horizontal => (text.measure("M", false, m.font_px).max(1), lay.inner.width),
        };
        let by = Press::of(hit).by(line as i64, page as i64) as i32;
        let mut v = self;
        match bar {
            ScrollBar::Vertical => {
                v.scroll_lines(by as i64, edit, lay.rows);
            }
            ScrollBar::Horizontal => {
                let max = (widest_px(text, m, edit) + 2 - lay.inner.width).max(0);
                v.left = (v.left + by).clamp(0, max);
            }
        }
        v
    }

    /// Scroll `by` lines (down positive), held so the last line stays at the bottom of a box `rows`
    /// lines tall; the caret stays put, as under the wheel.
    pub fn scroll_lines(&mut self, by: i64, edit: &MultiEdit, rows: usize) -> bool {
        let max = edit.lines.len().saturating_sub(rows) as i64;
        let to = (self.top as i64 + by).clamp(0, max) as usize;
        let moved = to != self.top;
        self.top = to;
        moved
    }

    /// Scroll so the caret is inside the box.
    pub fn follow(
        &mut self,
        text: &TextRenderer,
        m: &Metrics,
        edit: &MultiEdit,
        lay: &Layout,
        s: i32,
    ) {
        let caret = edit.caret;
        if caret.line < self.top {
            self.top = caret.line;
        } else if caret.line >= self.top + lay.rows {
            self.top = caret.line + 1 - lay.rows;
        }
        let x = x_of(text, m, edit, caret);
        let w = lay.inner.width - 2 * s;
        if x < self.left {
            self.left = x;
        } else if x >= self.left + w {
            self.left = x - w + s;
        }
    }
}

/// The box the text draws in: 1 px (at scale `s`) inside the white box, unclamped.
fn text_clip(inner: Rect, s: i32) -> Rect {
    Rect::new(
        inner.x + s,
        inner.y + s,
        inner.width - 2 * s,
        inner.height - 2 * s,
    )
}

fn printable(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// The text position under (`x`, `y`), for a click: in the space `lay` was laid out in at scale
/// `s`, with `m.font_px` the size at that scale.
#[expect(clippy::too_many_arguments)]
pub fn pos_at(
    text: &TextRenderer,
    m: &Metrics,
    edit: &MultiEdit,
    lay: &Layout,
    view: View,
    x: i32,
    y: i32,
    s: i32,
) -> Pos {
    let row = ((y - lay.inner.y - s) / (m.line_h.max(1) * s)).max(0) as usize;
    let line = (view.top + row).min(edit.lines.len() - 1);
    let tx = x - lay.inner.x - s + view.left;
    Pos {
        line,
        col: col_at(text, m, edit, line, tx),
    }
}

/// The view one step on for a drag resting at (`x`, `y`), None when it stays: a line up in the top
/// row, a line down in the bottom row (and the slack), a line height sideways in a band at the left
/// or right side, never past the text's ends. The host calls it on a tick while the drag rests.
#[expect(clippy::too_many_arguments)]
pub fn drag_scroll(
    text: &TextRenderer,
    m: &Metrics,
    edit: &MultiEdit,
    lay: &Layout,
    view: View,
    x: i32,
    y: i32,
    s: i32,
) -> Option<View> {
    let i = lay.inner;
    if !i.contains(x, y) {
        return None;
    }
    let line_h = m.line_h.max(1) * s;
    let mut v = view;
    let (rows, lines) = vextent(edit, lay);
    if y < i.y + s + line_h {
        v.top = v.top.saturating_sub(1);
    } else if y >= i.y + s + (rows as i32 - 1) * line_h && v.top + rows < lines {
        v.top += 1;
    }
    let (visible, total) = hextent(text, m, edit, lay, s);
    let max_left = total.saturating_sub(visible) as i32;
    if x < i.x + line_h {
        v.left = (v.left - line_h).max(0);
    } else if x >= i.right() - line_h && v.left < max_left {
        v.left = (v.left + line_h).min(max_left);
    }
    (v != view).then_some(v)
}

/// What one [`select_drag`] call did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectStep {
    pub changed: bool,
    /// The pointer rests where a tick scrolls; the host keeps ticking while it holds.
    pub scrolling: bool,
}

/// A selection drag held at (`x`, `y`), inside the edit or past any side of it. The point is taken
/// at its nearest spot in the full rows; on a `tick` the view first steps as [`drag_scroll`]
/// decides there, then the selection extends to the position under it.
#[expect(clippy::too_many_arguments)]
pub fn select_drag(
    text: &TextRenderer,
    m: &Metrics,
    edit: &mut MultiEdit,
    lay: &Layout,
    view: &mut View,
    x: i32,
    y: i32,
    s: i32,
    tick: bool,
) -> SelectStep {
    let i = lay.inner;
    let full = i.y + s + lay.rows.max(1) as i32 * m.line_h.max(1) * s - 1;
    let (x, y) = (
        x.clamp(i.x, i.right() - 1),
        y.clamp(i.y, full.min(i.bottom() - 1)),
    );
    let before = *view;
    if tick && let Some(v) = drag_scroll(text, m, edit, lay, *view, x, y, s) {
        *view = v;
    }
    let pos = pos_at(text, m, edit, lay, *view, x, y, s);
    let placed = edit.place(pos, true);
    view.follow(text, m, edit, lay, s);
    SelectStep {
        changed: placed || *view != before,
        scrolling: drag_scroll(text, m, edit, lay, *view, x, y, s).is_some(),
    }
}

/// Draw the control: edge, box, lines from `view.top`, selection on `hilight`, the caret (1 px bar)
/// in the blink's shown half, and the scrollbars the layout calls for. On a blink-only frame the
/// caret bar is the damage (an empty rect when it is scrolled out of view).
#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    m: &Metrics,
    edit: &MultiEdit,
    view: View,
    lay: &Layout,
    caret: Caret,
) {
    let Paint { cs, s, .. } = *p;
    // With a preedit nothing is drawn selected; text-input-v3 removes the real selection.
    let preedit = c.preedit().filter(|_| caret.focused()).cloned();
    c.fill_rect(lay.frame, cs.button_face);
    draw_edit_frame(c, p, lay.frame, cs.window);
    let inner = lay.inner;
    c.fill_rect(inner, cs.window);
    let line_h = m.line_h * s;
    let (a, b) = match &preedit {
        Some(_) => (edit.caret, edit.caret),
        None => edit.selection(),
    };
    let x0 = inner.x + s - view.left;
    let clip = text_clip(inner, s);
    let mut caret_bar = None;
    for row in 0..lay.rows {
        let li = view.top + row;
        let Some(line) = edit.lines.get(li) else {
            break;
        };
        let y = inner.y + s + row as i32 * line_h;
        let composed = preedit
            .as_ref()
            .filter(|_| li == edit.caret.line)
            .map(|p| p.splice(line, edit.caret.col));
        let line = composed.as_ref().map_or(line.as_str(), |(t, ..)| t);
        let shown = expand(line, m.tab_cols);
        let width = text.measure(&shown, false, m.font_px);
        let r = Rect::new(x0, y, width.max(1), line_h);
        if a != b && li >= a.line && li <= b.line {
            let sa = if li == a.line {
                expanded_col(line, a.col, m.tab_cols)
            } else {
                0
            };
            let sb = if li == b.line {
                expanded_col(line, b.col, m.tab_cols)
            } else {
                shown.len()
            };
            let xa = x0 + text.measure(&shown[..sa], false, m.font_px);
            let mut xb = x0 + text.measure(&shown[..sb], false, m.font_px);
            if li < b.line {
                xb += text.measure(" ", false, m.font_px);
            }
            let run = Rect::new(xa, y, (xb - xa).max(0), line_h);
            c.fill_rect(run.intersect(&clip), cs.hilight);
            text.draw_clipped(
                c,
                r,
                clip,
                &shown,
                false,
                cs.button_text,
                Align::Left,
                m.font_px,
            );
            text.draw_clipped(
                c,
                run,
                clip,
                &shown[sa..sb],
                false,
                cs.hilight_text,
                Align::Left,
                m.font_px,
            );
        } else {
            text.draw_clipped(
                c,
                r,
                clip,
                &shown,
                false,
                cs.button_text,
                Align::Left,
                m.font_px,
            );
        }
        let col_x = |col: usize| {
            let e = expanded_col(line, col, m.tab_cols);
            x0 + text.measure(&shown[..e.min(shown.len())], false, m.font_px)
        };
        if let Some((_, run, _)) = &composed {
            let base = text
                .baseline(r, false, m.font_px)
                .unwrap_or(y + line_h - 3 * s);
            let (xa, xb) = (col_x(run.start), col_x(run.end));
            let ul = Rect::new(xa, base + s, xb - xa, s);
            c.fill_rect(ul.intersect(&clip), cs.button_text);
        }
        if li == edit.caret.line {
            let (col, bar_on) = match (&composed, &preedit) {
                (Some((_, _, at)), Some(p)) => (*at, p.cursor.is_some()),
                _ => (edit.caret.col, true),
            };
            let cx = col_x(col);
            if caret.shown() && bar_on && cx >= clip.x && cx < clip.right() {
                c.fill_rect(Rect::new(cx, y, s, line_h), cs.button_text);
            }
            caret_bar = Some(Rect::new(cx.clamp(clip.x, clip.right() - s), y, s, line_h));
        }
    }
    if caret.focused() {
        // A caret scrolled out of the box reports the nearest row, so the candidates stay by it.
        let bar = caret_bar.unwrap_or_else(|| {
            let y = if edit.caret.line < view.top {
                clip.y
            } else {
                clip.bottom() - line_h
            };
            let cx = x0 + x_of(text, m, edit, edit.caret);
            Rect::new(cx.clamp(clip.x, clip.right() - s), y, s, line_h)
        });
        c.report_spot(edit.spot(lay.frame, bar));
        if c.blink_only() {
            c.report_damage(caret_bar.unwrap_or_default());
        }
    }
    if let Some(sb) = lay.vscroll {
        let (visible, total) = vextent(edit, lay);
        let lit = axis_lit(view.bars.lit(), Axis::Vertical);
        draw_scrollbar(c, p, sb, view.top, visible, total, lit);
    }
    if let Some(hb) = lay.hscroll {
        let (visible, total) = hextent(text, m, edit, lay, s);
        let lit = axis_lit(view.bars.lit(), Axis::Horizontal);
        draw_hscrollbar(c, p, hb, view.left.max(0) as usize, visible, total, lit);
    }
    if let Some(corner) = lay.corner() {
        draw_scroll_corner(c, p, corner, false);
    }
}

/// Where a drag over the control would drop its text: a bar the caret's size at `at`, over what
/// `draw` drew.
#[expect(clippy::too_many_arguments)]
pub fn draw_drop_point(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    m: &Metrics,
    edit: &MultiEdit,
    view: View,
    lay: &Layout,
    at: Pos,
) {
    let s = p.s;
    let Some(row) = at.line.checked_sub(view.top).filter(|r| *r < lay.rows) else {
        return;
    };
    let inner = lay.inner;
    let clip = text_clip(inner, s);
    let line_h = m.line_h * s;
    let x = inner.x + s - view.left + x_of(text, m, edit, at);
    let y = inner.y + s + row as i32 * line_h;
    c.fill_rect(
        Rect::new(x, y, s, line_h).intersect(&clip),
        p.cs.button_text,
    );
}

/// A document past this many bytes gives its node only the text in view as its value.
const FULL_VALUE_MAX: usize = 1 << 20;

/// The edit's `access` part: a multi-line text input, one text run per line in view and for the
/// caret's and the anchor's lines (each but the document's last ending in its line break) at the
/// rect the line is drawn in, with the caret, the selection and the scroll position in px.
#[allow(clippy::too_many_arguments)]
pub fn access(
    edit: &MultiEdit,
    view: View,
    lay: &Layout,
    m: &Metrics,
    text: &TextRenderer,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let count = edit.lines.len();
    let shown = view.top.min(count)..(view.top + lay.rows).min(count);
    let mut n = access::node(Role::MultilineTextInput, lay.frame);
    access::focusable(&mut n, true);
    let line_h = f64::from(m.line_h);
    n.set_scroll_y(view.top as f64 * line_h);
    n.set_scroll_y_min(0.0);
    n.set_scroll_y_max(count.saturating_sub(lay.rows) as f64 * line_h);
    let bytes = edit.lines.iter().map(|l| l.len() + 1).sum::<usize>();
    n.set_value(if bytes <= FULL_VALUE_MAX {
        edit.text()
    } else {
        edit.lines[shown.clone()].join("\n")
    });
    let id = tree.push(parent, key, n);
    let x0 = lay.inner.x + 1 - view.left;
    let last = count.saturating_sub(1);
    let wanted = |i: usize| shown.contains(&i) || i == edit.caret.line || i == edit.anchor.line;
    let end = shown.end.max(edit.caret.line + 1).max(edit.anchor.line + 1);
    let mut starts = Vec::with_capacity(end.min(count));
    let mut lines = Vec::with_capacity(shown.len() + 2);
    let mut at = 0;
    for (i, line) in edit.lines.iter().enumerate().take(end) {
        starts.push(at);
        if wanted(i) {
            let y = lay.inner.y + 1 + (i as i32 - view.top as i32) * m.line_h;
            let w = x_of(
                text,
                m,
                edit,
                Pos {
                    line: i,
                    col: line.len(),
                },
            );
            lines.push(access::Line {
                text: line,
                start: at,
                rect: Rect::new(x0, y, w.max(1), m.line_h),
                hard_break: i < last,
            });
        }
        at += line.len() + 1;
    }
    let offset = |p: Pos| starts.get(p.line).map_or(0, |s| s + p.col);
    access::text_runs(
        tree,
        id,
        &lines,
        text,
        m.font_px,
        offset(edit.anchor),
        offset(edit.caret),
    );
    id
}

#[cfg(test)]
mod tests;
