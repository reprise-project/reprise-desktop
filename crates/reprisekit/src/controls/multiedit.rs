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
use crate::textedit::{Caret, Edit, EditKey, Surrounding, TextSpot, Undo, index_at, words};
use accesskit::{NodeId, Role};
use std::cell::RefCell;

/// A place in the text: line index and byte offset on that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
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
    if query.is_empty() {
        return 0;
    }
    let mut n = 0;
    for line in lines.iter_mut() {
        let mut out = String::with_capacity(line.len());
        let mut i = 0;
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
        *line = out;
    }
    n
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
    undo: Undo<Vec<String>, Pos>,
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
        let n = replace_all(&mut self.lines, query, with, o);
        self.edits += 1;
        *self.widths.get_mut() = Widths::default();
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
        }
    }

    /// The whole text, lines joined by `\n`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
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

    fn forget_widths(&mut self, at: usize, old: usize, new: usize) {
        let w = self.widths.get_mut();
        if at + old <= w.px.len() {
            w.px.splice(at..at + old, std::iter::repeat_n(None, new));
        }
        w.widest = None;
    }

    fn checkpoint(&mut self, kind: Edit) {
        self.undo
            .checkpoint(kind, &self.lines, self.caret, self.anchor);
    }

    fn delete_selection(&mut self) {
        let (a, b) = self.selection();
        if a == b {
            return;
        }
        self.edits += 1;
        let tail = self.lines[b.line][b.col..].to_string();
        self.lines[a.line].truncate(a.col);
        self.lines[a.line].push_str(&tail);
        self.lines.drain(a.line + 1..=b.line);
        self.forget_widths(a.line, b.line - a.line + 1, 1);
        self.caret = a;
        self.anchor = a;
    }

    fn insert(&mut self, s: &str) {
        self.delete_selection();
        if s.is_empty() {
            return;
        }
        self.edits += 1;
        let Pos { line, col } = self.caret;
        let tail = self.lines[line].split_off(col);
        let normalised = s.replace("\r\n", "\n");
        let mut parts = normalised.split('\n').map(str::to_string);
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
        self.caret = Pos { line: at, col: end };
        self.anchor = self.caret;
    }

    fn prev(&self, p: Pos) -> Pos {
        if p.col > 0 {
            let col = words::prev_char(&self.lines[p.line], p.col);
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

    fn next(&self, p: Pos) -> Pos {
        let line = &self.lines[p.line];
        if p.col < line.len() {
            let col = words::next_char(line, p.col);
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
                        self.next(self.caret)
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
                    self.prev(self.caret)
                };
                self.move_to(to, select);
            }
            EditKey::Right { select, word } => {
                let to = if word {
                    self.word_right(self.caret)
                } else if self.has_selection() && !select {
                    self.selection().1
                } else {
                    self.next(self.caret)
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
            EditKey::Undo => {
                if self
                    .undo
                    .toggle(&mut self.lines, &mut self.caret, &mut self.anchor)
                {
                    self.edits += 1;
                    *self.widths.get_mut() = Widths::default();
                    self.caret = self.clamp(self.caret);
                    self.anchor = self.clamp(self.anchor);
                }
            }
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
/// in the blink's shown half, and the scrollbars the layout calls for.
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
    let clip = Rect::new(
        inner.x + s,
        inner.y + s,
        inner.width - 2 * s,
        inner.height - 2 * s,
    );
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
    let clip = Rect::new(
        inner.x + s,
        inner.y + s,
        inner.width - 2 * s,
        inner.height - 2 * s,
    );
    let line_h = m.line_h * s;
    let x = inner.x + s - view.left + x_of(text, m, edit, at);
    let y = inner.y + s + row as i32 * line_h;
    c.fill_rect(
        Rect::new(x, y, s, line_h).intersect(&clip),
        p.cs.button_text,
    );
}

/// The edit's `access` part: a multi-line text input, one text run per line (each but the last
/// ending in its line break) at the rect the line is drawn in, with the caret and the selection.
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
    let mut n = access::node(Role::MultilineTextInput, lay.frame);
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let x0 = lay.inner.x + 1 - view.left;
    let mut starts = Vec::with_capacity(edit.lines.len());
    let mut at = 0;
    let last = edit.lines.len().saturating_sub(1);
    let lines: Vec<access::Line> = edit
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            starts.push(at);
            let start = at;
            at += line.len() + 1;
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
            access::Line {
                text: line,
                start,
                rect: Rect::new(x0, y, w.max(1), m.line_h),
                hard_break: i < last,
            }
        })
        .collect();
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
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    fn e(text: &str) -> MultiEdit {
        MultiEdit::new(text)
    }
    fn k(edit: &mut MultiEdit, key: MultiKey) -> bool {
        let text = TextRenderer::load(&FontConfig::default());
        edit.key(key, &text, &Metrics::default())
    }

    #[test]
    fn changes_are_counted_and_the_widest_line_follows_them() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics::default();
        let mut t = e("ab\nc");
        assert!(
            !k(
                &mut t,
                MultiKey::Edit(EditKey::Left {
                    select: false,
                    word: false
                })
            ),
            "at the start already"
        );
        let w0 = widest_px(&text, &m, &t);
        assert_eq!(
            w0,
            widest_px(&text, &m, &t),
            "the same document measures the same"
        );
        assert!(k(
            &mut t,
            MultiKey::Edit(EditKey::Insert("wider line".into()))
        ));
        let w1 = widest_px(&text, &m, &t);
        assert!(w1 > w0);
        assert!(k(&mut t, MultiKey::Edit(EditKey::Undo)));
        assert_eq!(widest_px(&text, &m, &t), w0);
        assert_eq!(
            t.replace_all(
                "c",
                "cccccccccccccc",
                &FindOpts {
                    match_case: true,
                    down: true
                }
            ),
            1
        );
        assert!(widest_px(&text, &m, &t) > w0);
    }
    fn p(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }

    #[test]
    fn dropped_text_lands_at_the_drop_point_selected() {
        let mut t = e("one two\nthree");
        t.place(p(1, 0), false);
        t.place(p(1, 5), true);
        assert!(t.drop_text(p(0, 4), "big\r\nred "));
        assert_eq!(t.lines, ["one big", "red two", "three"]);
        assert_eq!(t.selected_text(), "big\nred ", "the drop is selected");
        assert_eq!(t.caret(), p(1, 4), "the caret sits at its end");
    }

    #[test]
    fn one_undo_takes_a_drop_back_and_restores_the_selection() {
        let mut t = e("one two\nthree");
        t.place(p(1, 0), false);
        t.place(p(1, 5), true);
        t.drop_text(p(0, 4), "big ");
        assert!(k(&mut t, MultiKey::Edit(EditKey::Undo)));
        assert_eq!(t.lines, ["one two", "three"]);
        assert_eq!(t.selected_text(), "three");
    }

    #[test]
    fn a_drop_of_nothing_printable_changes_nothing() {
        let mut t = e("ab");
        assert!(!t.drop_text(p(0, 1), "\r\u{7}"));
        assert_eq!(t.lines, ["ab"]);
    }

    #[test]
    fn delete_surrounding_crosses_lines_and_keeps_the_selection() {
        let mut t = e("ab\ncd\nef");
        t.place(p(1, 1), false);
        t.place(p(2, 1), true);
        assert!(k(
            &mut t,
            MultiKey::Edit(EditKey::DeleteSurrounding {
                before: 2,
                after: 1
            })
        ));
        assert_eq!(t.lines, ["abd", "e"]);
        assert_eq!(t.selected_text(), "d\ne");
        assert_eq!(t.caret(), p(1, 1), "the caret stays at the selection's end");
        assert!(!k(
            &mut t,
            MultiKey::Edit(EditKey::DeleteSurrounding {
                before: 0,
                after: 0
            })
        ));
    }

    #[test]
    fn a_preedit_draws_in_the_caret_line_and_the_spot_is_that_line() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let m = Metrics {
            bars_always: false,
            ..Default::default()
        };
        let mut t = e("top\nab");
        t.place(p(1, 1), false);
        let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
        let mut c = Canvas::new(200, 60);
        c.set_preedit(crate::textedit::Preedit::new("xyz".into(), -1, -1));
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &m,
            &t,
            View::default(),
            &lay,
            Caret::Hidden,
        );
        let x0 = lay.inner.x + 1 + text.measure("a", false, m.font_px);
        let x1 = lay.inner.x + 1 + text.measure("axyz", false, m.font_px);
        let row = Rect::new(lay.inner.x + 1, lay.inner.y + 1 + m.line_h, 10, m.line_h);
        let ul = text.baseline(row, false, m.font_px).unwrap() + 1;
        let ink = theme.colors.button_text.argb();
        assert!((x0..x1).all(|x| c.get(x, ul) == ink));
        let spot = c.take_spot().expect("focused");
        assert!(spot.multiline);
        assert_eq!(spot.caret.x, x1, "a hidden preedit cursor sits at its end");
        let sur = spot.surrounding.unwrap();
        assert_eq!((sur.text.as_str(), sur.cursor), ("ab", 1));
    }

    #[test]
    fn a_preedit_range_draws_as_a_caret_at_its_end_over_no_selection() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let m = Metrics {
            bars_always: false,
            ..Default::default()
        };
        let mut t = e("top\nab");
        t.place(p(0, 1), false);
        t.place(p(1, 1), true);
        let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
        let draw_with = |pre| {
            let mut c = Canvas::new(200, 60);
            c.set_preedit(pre);
            draw(
                &mut c,
                &text,
                &classic_paint(&theme.colors, 1),
                &m,
                &t,
                View::default(),
                &lay,
                Caret::Shown,
            );
            c
        };
        let mut range = draw_with(crate::textedit::Preedit::new("xyz".into(), 1, 3));
        let mut at_end = draw_with(crate::textedit::Preedit::new("xyz".into(), 3, 3));
        let hl = theme.colors.hilight.argb();
        assert!(range.pixels.iter().all(|&p| p != hl), "nothing on hilight");
        assert_eq!(range.pixels, at_end.pixels);
        let x = lay.inner.x + 1 + text.measure("axyz", false, m.font_px);
        let spot = range.take_spot().expect("focused");
        assert_eq!(spot.caret.x, x, "the caret ends the range");
        assert_eq!(Some(spot), at_end.take_spot());
    }

    #[test]
    fn the_drop_point_draws_a_bar_at_its_position() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let m = Metrics::default();
        let t = e("top\nab");
        let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
        let paint = classic_paint(&theme.colors, 1);
        let mut c = Canvas::new(200, 60);
        draw(
            &mut c,
            &text,
            &paint,
            &m,
            &t,
            View::default(),
            &lay,
            Caret::Hidden,
        );
        let before = c.clone();
        draw_drop_point(
            &mut c,
            &text,
            &paint,
            &m,
            &t,
            View::default(),
            &lay,
            p(1, 1),
        );
        let x = lay.inner.x + 1 + text.measure("a", false, m.font_px);
        let y = lay.inner.y + 1 + m.line_h;
        let ink = theme.colors.button_text.argb();
        assert!((y..y + m.line_h).all(|y| c.get(x, y) == ink), "the bar");
        assert_eq!(c.get(x + 1, y), before.get(x + 1, y), "1 px wide");
        assert_eq!(c.get(x, y - 1), before.get(x, y - 1), "one line tall");

        let scrolled = View {
            top: 1,
            ..View::default()
        };
        let mut c = Canvas::new(200, 60);
        draw(&mut c, &text, &paint, &m, &t, scrolled, &lay, Caret::Hidden);
        let before = c.clone();
        draw_drop_point(&mut c, &text, &paint, &m, &t, scrolled, &lay, p(0, 1));
        assert!(c == before, "line 0 is above the box");
    }

    #[test]
    fn an_edit_remeasures_only_the_lines_it_touched() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics::default();
        let unmeasured =
            |t: &MultiEdit| t.widths.borrow().px.iter().filter(|w| w.is_none()).count();
        let mut t = e("one\ntwo\nthree\nfour");
        widest_px(&text, &m, &t);
        assert_eq!(unmeasured(&t), 0);
        t.place(p(1, 3), false);
        k(&mut t, MultiKey::Edit(EditKey::Insert("x".into())));
        assert_eq!(unmeasured(&t), 1, "typing touches its own line");
        k(&mut t, MultiKey::Enter);
        assert_eq!(unmeasured(&t), 2, "Enter splits it in two");
        assert_eq!(
            widest_px(&text, &m, &t),
            widest_px(&text, &m, &e(&t.text()))
        );
        t.place(p(0, 1), false);
        t.place(p(3, 2), true);
        k(&mut t, MultiKey::Edit(EditKey::Delete { word: false }));
        assert_eq!(t.lines, ["oree", "four"]);
        assert_eq!(unmeasured(&t), 1, "a deletion across lines leaves one");
        assert_eq!(
            widest_px(&text, &m, &t),
            widest_px(&text, &m, &e(&t.text()))
        );
        t.paste("a much longer pasted line\nb");
        assert_eq!(
            widest_px(&text, &m, &t),
            widest_px(&text, &m, &e(&t.text()))
        );
    }

    #[test]
    fn splits_joins_and_edits_lines() {
        let mut t = e("ab\ncd");
        assert_eq!(t.lines, ["ab", "cd"]);
        assert_eq!(t.text(), "ab\ncd");
        k(
            &mut t,
            MultiKey::Edit(EditKey::Right {
                select: false,
                word: false,
            }),
        );
        k(&mut t, MultiKey::Enter);
        assert_eq!(t.lines, ["a", "b", "cd"]);
        assert_eq!(t.caret, p(1, 0));
        k(&mut t, MultiKey::Edit(EditKey::Backspace { word: false }));
        assert_eq!(t.lines, ["ab", "cd"]);
        k(&mut t, MultiKey::Edit(EditKey::End { select: false }));
        k(&mut t, MultiKey::Edit(EditKey::Delete { word: false }));
        assert_eq!(t.lines, ["abcd"]);
        t.paste("x\ny");
        assert_eq!(t.lines, ["abx", "ycd"]);
        assert_eq!(t.caret, p(1, 1));
    }

    #[test]
    fn selection_spans_lines_and_typing_replaces_it() {
        let mut t = e("one\ntwo\nthree");
        t.place(p(0, 1), false);
        t.place(p(2, 2), true);
        assert_eq!(t.selected_text(), "ne\ntwo\nth");
        k(&mut t, MultiKey::Edit(EditKey::Insert("X".into())));
        assert_eq!(t.lines, ["oXree"]);
        k(&mut t, MultiKey::Edit(EditKey::Undo));
        assert_eq!(t.lines, ["one", "two", "three"]);
        assert_eq!(t.selected_text(), "ne\ntwo\nth");
        t.select_all();
        assert_eq!(t.selected_text(), "one\ntwo\nthree");
        k(&mut t, MultiKey::Edit(EditKey::Cut));
        assert_eq!(t.lines, [""]);
    }

    #[test]
    fn vertical_moves_keep_the_goal_column() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics::default();
        let mut t = e("longer line\nab\nlonger line");
        t.place(p(0, 8), false);
        let goal = x_of(&text, &m, &t, p(0, 8));
        t.key(MultiKey::Down { select: false }, &text, &m);
        assert_eq!(t.caret, p(1, 2), "a short line clamps");
        assert_eq!(t.goal_x, Some(goal));
        t.key(MultiKey::Down { select: false }, &text, &m);
        assert_eq!(t.caret, p(2, 8), "the goal column comes back");
        k(&mut t, MultiKey::DocHome { select: true });
        assert_eq!(t.selection(), (p(0, 0), p(2, 8)));
        assert_eq!(t.goal_x, None);
        k(&mut t, MultiKey::DocEnd { select: false });
        assert_eq!(t.caret, p(2, 11));
    }

    #[test]
    fn words_wrap_across_lines_and_double_click_selects_them() {
        let mut t = e("ab cd\nef");
        k(&mut t, MultiKey::DocEnd { select: false });
        k(
            &mut t,
            MultiKey::Edit(EditKey::Left {
                select: false,
                word: true,
            }),
        );
        assert_eq!(t.caret, p(1, 0));
        k(
            &mut t,
            MultiKey::Edit(EditKey::Left {
                select: false,
                word: true,
            }),
        );
        assert_eq!(
            t.caret,
            p(0, 5),
            "at a line start, up to the previous line's end"
        );
        k(
            &mut t,
            MultiKey::Edit(EditKey::Left {
                select: false,
                word: true,
            }),
        );
        assert_eq!(t.caret, p(0, 3));
        t.select_word_at(p(0, 1));
        assert_eq!(t.selected_text(), "ab ");
        k(&mut t, MultiKey::Edit(EditKey::Backspace { word: true }));
        assert_eq!(t.lines, ["cd", "ef"]);
    }

    #[test]
    fn tabs_expand_for_measuring_and_map_back() {
        assert_eq!(expand("a\tb", 8), "a       b");
        assert_eq!(expanded_col("a\tb", 2, 8), 8);
        assert_eq!(
            expanded_col("é\tb", 3, 8),
            expand("é\t", 8).len(),
            "a 2-byte char is one column before the tab"
        );
        assert_eq!(raw_col("a\tb", 8, 8), 2);
        assert_eq!(raw_col("a\tb", 4, 8), 1, "inside the tab lands on it");
        assert_eq!(raw_col("ab", 9, 8), 2);
    }

    #[test]
    fn zero_metrics_do_not_divide_by_zero() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics {
            line_h: 0,
            tab_cols: 0,
            ..Metrics::default()
        };
        let t = e("a\tb\nc");
        assert_eq!(expand("a\tb", 0), "a b");
        assert_eq!(raw_col("a\tb", 2, 0), 2);
        let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 60), 1);
        assert_eq!(
            pos_at(&text, &m, &t, &lay, View::default(), 3, 4, 1).line,
            1
        );
    }

    #[test]
    fn a_17_px_bar_takes_17_px_off_the_text_box_each_way() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics {
            bar: 17,
            ..Metrics::default()
        };
        let l = layout(&text, &m, &e("a"), Rect::new(0, 0, 200, 60), 1);
        assert_eq!(l.inner, Rect::new(2, 2, 196 - 17, 56 - 17));
        assert_eq!(l.vscroll, Some(Rect::new(2 + 196 - 17, 2, 17, 56 - 17)));
        assert_eq!(l.hscroll, Some(Rect::new(2, 2 + 56 - 17, 196 - 17, 17)));
        assert_eq!(
            l.corner(),
            Some(Rect::new(2 + 196 - 17, 2 + 56 - 17, 17, 17))
        );
        let l = layout(&text, &m, &e("a"), Rect::new(0, 0, 400, 120), 2);
        assert_eq!(
            l.corner(),
            Some(Rect::new(4 + 392 - 34, 4 + 112 - 34, 34, 34))
        );
    }

    #[test]
    fn layout_adds_scrollbars_when_the_text_outgrows_the_box() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics {
            bars_always: false,
            ..Metrics::default()
        };
        let r = Rect::new(0, 0, 200, 60);
        let short = e("a\nb");
        let l = layout(&text, &m, &short, r, 1);
        assert_eq!(l.inner, r.inset(2));
        assert_eq!(l.rows, 4);
        assert!(l.vscroll.is_none() && l.hscroll.is_none());
        let l = layout(&text, &Metrics::default(), &short, r, 1);
        assert_eq!(l.vscroll, Some(Rect::new(2 + 196 - 16, 2, 16, 56 - 16)));
        assert_eq!(l.hscroll, Some(Rect::new(2, 2 + 56 - 16, 196 - 16, 16)));
        assert_eq!(
            l.corner(),
            Some(Rect::new(2 + 196 - 16, 2 + 56 - 16, 16, 16))
        );
        assert_eq!(l.rows, 3);
        let tall = e("a\nb\nc\nd\ne\nf");
        let l = layout(&text, &m, &tall, r, 1);
        assert_eq!(l.vscroll, Some(Rect::new(2 + 196 - 16, 2, 16, 56)));
        assert_eq!(l.inner.width, 196 - 16);
        let wide = e(&"x".repeat(80));
        let l = layout(&text, &m, &wide, r, 1);
        assert!(l.hscroll.is_some());
        assert_eq!(l.rows, (56 - 16) / 13);
    }

    #[test]
    fn scrollbar_presses_map_to_parts_and_a_thumb_drag_scrolls() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics::default();
        let mut src = "x".repeat(200);
        for _ in 0..39 {
            src.push_str("\na");
        }
        let t = e(&src);
        let r = Rect::new(0, 0, 200, 150);
        let l = layout(&text, &m, &t, r, 1);
        let vb = l.vscroll.unwrap();
        let hb = l.hscroll.unwrap();
        let view = View::default();
        let hit =
            |x, y| bar_hit(&text, &m, &t, &l, view, x, y).and_then(|(b, h)| Some((b, h.part()?)));
        assert_eq!(
            hit(vb.x + 8, vb.y + 8),
            Some((ScrollBar::Vertical, ScrollHit::Back))
        );
        assert_eq!(
            hit(vb.x + 8, vb.bottom() - 8),
            Some((ScrollBar::Vertical, ScrollHit::Forward))
        );
        let vp = bar_parts(&text, &m, &t, &l, view, ScrollBar::Vertical).unwrap();
        let thumb = vp.thumb.unwrap();
        assert_eq!(thumb.y, vp.track.y, "at the top the thumb leads the track");
        assert_eq!(
            hit(vb.x + 8, thumb.bottom() + 1),
            Some((ScrollBar::Vertical, ScrollHit::PageForward))
        );
        assert_eq!(
            hit(hb.x + 8, hb.y + 8),
            Some((ScrollBar::Horizontal, ScrollHit::Back))
        );
        assert_eq!(
            hit(hb.right() - 8, hb.y + 8),
            Some((ScrollBar::Horizontal, ScrollHit::Forward))
        );
        assert_eq!(hit(100, 30), None);
        let (bar, h) = bar_hit(&text, &m, &t, &l, view, vb.x + 8, thumb.y + 2).unwrap();
        assert_eq!((bar, h), (ScrollBar::Vertical, BarHit::Thumb(2)));
        let mut held = ThumbDrag::default();
        held.press(bar.axis(), h);
        let at = |y| drag_bar(&text, &m, &t, &l, view, &held, (vb.x + 8, y)).unwrap();
        assert_eq!(at(vp.track.bottom()).top, 40 - l.rows);
        assert_eq!(at(thumb.y + 2).top, 0);
        let mid = vp.track.y + (vp.track.height - thumb.height) / 2 + 2;
        assert_eq!(at(mid).top, (40 - l.rows) / 2);
        let hp = bar_parts(&text, &m, &t, &l, view, ScrollBar::Horizontal).unwrap();
        let total = widest_px(&text, &m, &t) + 2;
        held.press(Axis::Horizontal, BarHit::Thumb(0));
        let end = drag_bar(&text, &m, &t, &l, view, &held, (hp.track.right(), 0)).unwrap();
        assert_eq!(end.left, total - l.inner.width);
    }

    #[test]
    fn view_follows_the_caret_and_clicks_land() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let m = Metrics::default();
        let mut t = e("a\nb\nc\nd\ne\nf\ng");
        let r = Rect::new(0, 0, 200, 60);
        let lay = layout(&text, &m, &t, r, 1);
        let mut v = View::default();
        k(&mut t, MultiKey::DocEnd { select: false });
        v.follow(&text, &m, &t, &lay, 1);
        assert_eq!(v.top, 7 - lay.rows);
        let q = pos_at(&text, &m, &t, &lay, v, 4, lay.inner.y + 1 + 13 + 3, 1);
        assert_eq!(q, p(v.top + 1, 0));
        // Draws without panicking, caret on the last row.
        let mut c = Canvas::new(200, 60);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &m,
            &t,
            v,
            &lay,
            Caret::Shown,
        );
        assert_eq!(c.get(0, 0), theme.colors.button_shadow.argb());
        assert_eq!(c.get(3, 3), theme.colors.window.argb());
    }

    #[test]
    fn a_drag_resting_in_an_edge_row_or_band_scrolls_one_step() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics::default();
        let long = "x".repeat(80);
        let t = e(&format!("{long}\n{}", "l\n".repeat(49)));
        let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 80), 1);
        let i = lay.inner;
        let (mid_x, mid_y) = (i.x + i.width / 2, i.y + 1 + 13 + 6);
        let last_row = i.y + 1 + (lay.rows as i32 - 1) * 13;
        let step = |v: View, x, y| drag_scroll(&text, &m, &t, &lay, v, x, y, 1);
        let at = |top, left| View {
            top,
            left,
            ..View::default()
        };
        assert_eq!(step(at(5, 0), mid_x, mid_y), None);
        let bar = lay.vscroll.unwrap();
        assert_eq!(step(at(5, 0), bar.x + 2, last_row + 2), None);
        assert_eq!(step(at(5, 0), mid_x, i.y), Some(at(4, 0)));
        assert_eq!(step(at(5, 0), mid_x, i.y + 13), Some(at(4, 0)));
        assert_eq!(step(at(5, 0), mid_x, last_row), Some(at(6, 0)));
        assert_eq!(step(at(5, 0), mid_x, i.bottom() - 1), Some(at(6, 0)));
        assert_eq!(step(at(0, 0), mid_x, i.y), None);
        assert_eq!(
            step(at(t.lines().len() - lay.rows, 0), mid_x, last_row),
            None
        );
        assert_eq!(step(at(5, 0), i.right() - 1, mid_y), Some(at(5, 13)));
        assert_eq!(step(at(5, 13), i.x, mid_y), Some(at(5, 0)));
        assert_eq!(step(at(5, 5), i.x + 12, mid_y), Some(at(5, 0)));
        assert_eq!(step(at(5, 0), i.x, mid_y), None);
        let max = (widest_px(&text, &m, &t) + 2) - i.width;
        assert_eq!(step(at(5, max - 3), i.right() - 1, mid_y), Some(at(5, max)));
        assert_eq!(step(at(5, max), i.right() - 1, mid_y), None);
        assert_eq!(step(at(5, 0), i.right() - 1, i.y), Some(at(4, 13)));
    }

    #[test]
    fn a_selection_drag_past_an_edge_scrolls_on_ticks_and_extends_to_the_edge_row() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = Metrics::default();
        let mut t = e(&"line\n".repeat(50));
        let lay = layout(&text, &m, &t, Rect::new(0, 0, 200, 80), 1);
        let i = lay.inner;
        let rows = lay.rows;
        let mut v = View::default();
        let drag = |t: &mut MultiEdit, v: &mut View, x, y, tick| {
            select_drag(&text, &m, t, &lay, v, x, y, 1, tick)
        };
        t.place(Pos { line: 1, col: 2 }, false);
        let below = i.bottom() + 40;
        let r = drag(&mut t, &mut v, i.x + 20, below, false);
        assert!(r.changed && r.scrolling);
        assert_eq!(v.top, 0, "motion past the edge does not scroll");
        assert_eq!(
            t.caret().line,
            rows - 1,
            "the caret goes to the last full row"
        );
        for n in 1..=3 {
            let r = drag(&mut t, &mut v, i.x + 20, below, true);
            assert!(r.changed && r.scrolling);
            assert_eq!(v.top, n);
            assert_eq!(t.caret().line, n + rows - 1);
        }
        assert_eq!(t.selection(), (Pos { line: 1, col: 2 }, t.caret()));
        let r = drag(&mut t, &mut v, i.x + 20, i.y - 30, true);
        assert_eq!((v.top, t.caret().line), (2, 2), "above the edit scrolls up");
        assert!(r.scrolling);
        let r = drag(&mut t, &mut v, i.x + 20, i.y + 1 + 13 + 6, true);
        assert!(!r.scrolling, "the middle rests still");
        assert_eq!(v.top, 2);

        let mut t = e(&format!("{}\nb", "x".repeat(80)));
        let mut v = View::default();
        let r = drag(&mut t, &mut v, i.right() + 50, i.y + 2, true);
        assert!(
            r.scrolling && v.left > 0,
            "past the right side scrolls right"
        );
        let left = v.left;
        let r = drag(&mut t, &mut v, i.x - 50, i.y + 2, true);
        assert!(
            r.changed && v.left < left,
            "past the left side scrolls back"
        );
    }

    #[test]
    fn a_scrolled_view_shifts_every_line_left() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let m = Metrics::default();
        let t = e("ab\nabcdefghijklmnopqrstuvwxyz abcdefghijklmnopqrstuvwxyz");
        let r = Rect::new(0, 0, 120, 60);
        let lay = layout(&text, &m, &t, r, 1);
        let mut flat = Canvas::new(400, 60);
        let wide = layout(&text, &m, &t, Rect::new(0, 0, 400, 60), 1);
        draw(
            &mut flat,
            &text,
            &classic_paint(&theme.colors, 1),
            &m,
            &t,
            View::default(),
            &wide,
            Caret::Unfocused,
        );
        let v = View {
            left: 40,
            ..View::default()
        };
        let mut c = Canvas::new(120, 60);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &m,
            &t,
            v,
            &lay,
            Caret::Unfocused,
        );
        let x0 = lay.inner.x + 1;
        for y in lay.inner.y + 1..lay.inner.y + 2 * m.line_h {
            for x in x0..lay.inner.right() - 1 {
                assert_eq!(c.get(x, y), flat.get(x + 40, y), "at {x},{y}");
            }
        }
    }

    #[test]
    fn find_returns_none_for_an_empty_query() {
        let lines = vec!["anything".to_string()];
        let o = FindOpts {
            match_case: false,
            down: true,
        };
        assert_eq!(find(&lines, "", Pos::default(), &o), None);
    }

    #[test]
    fn find_walks_down_from_the_caret() {
        let lines = vec!["hello world".to_string(), "hello again".to_string()];
        let o = FindOpts {
            match_case: true,
            down: true,
        };
        assert_eq!(
            find(&lines, "hello", Pos::default(), &o),
            Some((Pos { line: 0, col: 0 }, Pos { line: 0, col: 5 }))
        );
        assert_eq!(
            find(&lines, "hello", Pos { line: 0, col: 1 }, &o),
            Some((Pos { line: 1, col: 0 }, Pos { line: 1, col: 5 }))
        );
    }

    #[test]
    fn find_up_finds_the_match_before_the_caret() {
        let lines = vec!["hello hello".to_string()];
        let o = FindOpts {
            match_case: true,
            down: false,
        };
        assert_eq!(
            find(&lines, "hello", Pos { line: 0, col: 11 }, &o),
            Some((Pos { line: 0, col: 6 }, Pos { line: 0, col: 11 }))
        );
        assert_eq!(
            find(&lines, "hello", Pos { line: 0, col: 6 }, &o),
            Some((Pos { line: 0, col: 0 }, Pos { line: 0, col: 5 }))
        );
    }

    #[test]
    fn find_does_not_wrap() {
        let lines = vec!["alpha".to_string(), "beta".to_string()];
        let down = FindOpts {
            match_case: true,
            down: true,
        };
        assert_eq!(find(&lines, "alpha", Pos { line: 1, col: 4 }, &down), None);
        let up = FindOpts {
            match_case: true,
            down: false,
        };
        assert_eq!(find(&lines, "beta", Pos { line: 0, col: 0 }, &up), None);
    }

    #[test]
    fn find_ignores_case_only_when_asked() {
        let lines = vec!["Hello".to_string()];
        let sensitive = FindOpts {
            match_case: true,
            down: true,
        };
        let insensitive = FindOpts {
            match_case: false,
            down: true,
        };
        assert_eq!(find(&lines, "hello", Pos::default(), &sensitive), None);
        assert_eq!(
            find(&lines, "hello", Pos::default(), &insensitive),
            Some((Pos { line: 0, col: 0 }, Pos { line: 0, col: 5 }))
        );
    }

    #[test]
    fn find_returns_byte_offsets_on_a_multibyte_line() {
        let lines = vec!["héllo wörld".to_string()];
        let o = FindOpts {
            match_case: false,
            down: true,
        };
        let (s, e) = find(&lines, "WÖRLD", Pos::default(), &o).expect("the match is there");
        assert_eq!(&lines[0][s.col..e.col], "wörld");
    }

    /// Replace All rewrites the whole document and must not run over its own output: replacing "a"
    /// with "aa" terminates and reports 3.
    #[test]
    fn replace_all_does_not_rerun_over_its_own_output() {
        let mut lines = vec!["a b a".to_string(), "a".to_string()];
        let o = FindOpts {
            match_case: true,
            down: true,
        };
        assert_eq!(replace_all(&mut lines, "a", "aa", &o), 3);
        assert_eq!(lines, vec!["aa b aa".to_string(), "aa".to_string()]);
    }

    #[test]
    fn replace_all_leaves_a_valid_caret_and_one_undo_step() {
        let mut t = e("cat cat cat");
        k(&mut t, MultiKey::Edit(EditKey::Insert("x".into())));
        t.place(p(0, 5), false);
        t.place(p(0, 8), true);
        assert_eq!(t.selected_text(), "cat");
        let o = FindOpts {
            match_case: true,
            down: true,
        };
        assert_eq!(t.replace_all("cat", "c", &o), 3);
        assert_eq!(t.text(), "xc c c");
        assert_eq!(t.caret, p(0, 6));
        assert!(!t.has_selection());
        assert_eq!(t.selected_text(), "");
        k(&mut t, MultiKey::Edit(EditKey::Undo));
        assert_eq!(
            t.text(),
            "xcat cat cat",
            "Ctrl+Z undoes the Replace All alone"
        );
        assert_eq!(t.replace_all("zzz", "y", &o), 0);
        k(&mut t, MultiKey::Edit(EditKey::Undo));
        assert_eq!(
            t.text(),
            "xc c c",
            "a Replace All that matched nothing keeps the undo state"
        );
    }

    #[test]
    fn char_col_counts_characters_not_bytes() {
        const COL_AFTER_AB_TAB_C: usize = 5;
        let line = "ab\tc";
        assert_eq!(char_col(line, line.len()), COL_AFTER_AB_TAB_C);
        let wide = "héllo";
        assert_eq!(char_col(wide, wide.len()), 6);
        assert_eq!(char_col(wide, 0), 1);
    }

    #[test]
    fn lines_are_runs_and_the_caret_lands_on_its_line() {
        use crate::access::{ROOT, Tree, accesskit::Role};
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let m = Metrics::default();
        let mut edit = MultiEdit::new("hello\nworld");
        edit.place(Pos { line: 1, col: 2 }, false);
        let lay = layout(&text, &m, &edit, Rect::new(0, 20, 300, 200), 1);
        let mut tree = Tree::window("T", (300, 220));
        let id = access(
            &edit,
            View::default(),
            &lay,
            &m,
            &text,
            &mut tree,
            ROOT,
            "edit",
        );
        let node = tree.get(id).unwrap();
        assert_eq!(node.role(), Role::MultilineTextInput);
        let runs = node.children().to_vec();
        let values: Vec<_> = runs.iter().map(|r| tree.get(*r).unwrap().value()).collect();
        assert_eq!(values, [Some("hello\n"), Some("world")]);
        let second = tree.get(runs[1]).unwrap().bounds().unwrap();
        assert_eq!(second.y0, f64::from(lay.inner.y + 1 + m.line_h));
        let sel = node.text_selection().unwrap();
        assert_eq!((sel.focus.node, sel.focus.character_index), (runs[1], 2));
    }

    #[test]
    fn a_blue_text_boxs_lit_bar_part_draws_hot() {
        use crate::classic::widgets::{Axis, ScrollHit};
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let mut m = Metrics::default();
        m.follow_look(&theme.look.controls());
        let t = e(&"line\n".repeat(60));
        let r = Rect::new(0, 0, 200, 120);
        let lay = layout(&text, &m, &t, r, 1);
        let sb = lay.vscroll.expect("60 lines overflow the box");
        let mut view = View::default();
        let up = bar_hit(&text, &m, &t, &lay, view, sb.x + 8, sb.y + 2)
            .and_then(|(b, h)| Some((b.axis(), h.part()?)));
        assert_eq!(up, Some((Axis::Vertical, ScrollHit::Back)));
        view.bars.motion(up);
        let mut c = Canvas::new(200, 120);
        draw(
            &mut c,
            &text,
            &theme.paint(),
            &m,
            &t,
            view,
            &lay,
            Caret::Unfocused,
        );
        assert_eq!(
            c.get(sb.x + 8, sb.y + 2),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Hot,
                8,
                2
            ),
            "the hot up arrow"
        );
    }
}
