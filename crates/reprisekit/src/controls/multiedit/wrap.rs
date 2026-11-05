//! Soft wrapping: the rows a [`MultiEdit`]'s lines break into at a width.

use super::{Metrics, MultiEdit, Pos};
use crate::text::TextRenderer;
use crate::textedit::words;
use unicode_segmentation::UnicodeSegmentation;

/// One drawn row of text: bytes `start..end` of line `line`, the whole line unless wrapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    /// The line's last row: a line break or the text's end follows it.
    pub last: bool,
}

impl Row {
    /// Where End and a click past the text leave the caret: before a soft-wrapped row's last
    /// cluster, as its end is where the next row starts.
    pub(super) fn end_col(self, line: &str) -> usize {
        if self.last {
            self.end
        } else {
            words::prev_cluster(line, self.end).max(self.start)
        }
    }
}

/// Where `line`'s rows after the first start at `width` px: before a word crossing the edge, or
/// inside one wider than a row. Spaces ending a row hang past it; tabs stop from the row's start.
pub fn breaks(text: &TextRenderer, m: &Metrics, line: &str, width: i32) -> Vec<usize> {
    let tab_cols = m.tab_cols.max(1);
    let space = text.advance(' ', false, m.font_px);
    let mut out = Vec::new();
    let (mut row, mut x, mut col) = (0, 0, 0);
    // The word being read: its first byte, and the row's x and column there.
    let mut word: Option<(usize, i32, usize)> = None;
    for (i, g) in line.grapheme_indices(true) {
        let (adv, cols) = if g == "\t" {
            let n = tab_cols - col % tab_cols;
            (n as i32 * space, n)
        } else {
            let adv = g.chars().map(|c| text.advance(c, false, m.font_px)).sum();
            (adv, g.chars().count())
        };
        if g.starts_with(char::is_whitespace) {
            word = None;
        } else {
            let (w, wx, wcol) = *word.get_or_insert((i, x, col));
            if x + adv > width && i > row {
                if w > row {
                    out.push(w);
                    (row, x, col) = (w, x - wx, col - wcol);
                }
                if x + adv > width && i > row {
                    out.push(i);
                    (row, x, col) = (i, 0, 0);
                }
                word = Some((row, 0, 0));
            }
        }
        x += adv;
        col += cols;
    }
    out
}

/// Each line's breaks at one width, with the row each line starts on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    /// Font px, tab columns and width.
    key: Option<(u32, usize, i32)>,
    breaks: Vec<Option<Vec<usize>>>,
    /// Per line, its first row's index, and one more entry holding the row count. `None` until
    /// every line's breaks are known.
    first: Option<Vec<usize>>,
}

/// The rows of the last two widths asked for (a layout whose scrollbar shows only on overflow
/// tries two); an edit forgets only the lines it touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Wraps {
    /// The width the last wrapped layout used, which Up, Down, Home and End move by.
    pub(super) laid: Option<i32>,
    slots: [Slot; 2],
    recent: usize,
}

impl Wraps {
    /// Lines `at..at + old` became `new` lines.
    pub(super) fn forget(&mut self, at: usize, old: usize, new: usize) {
        for s in &mut self.slots {
            if at + old <= s.breaks.len() {
                s.breaks
                    .splice(at..at + old, std::iter::repeat_n(None, new));
                s.first = None;
            } else {
                *s = Slot::default();
            }
        }
    }

    /// Every line may have changed.
    pub(super) fn clear(&mut self) {
        self.slots = Default::default();
    }
}

impl MultiEdit {
    /// The width keys move by: the last layout's, while `m.wrap` is on.
    pub(super) fn wrap_width(&self, m: &Metrics) -> Option<i32> {
        m.wrap.then(|| self.wraps.borrow().laid).flatten()
    }

    fn with_rows<R>(
        &self,
        text: &TextRenderer,
        m: &Metrics,
        width: i32,
        f: impl FnOnce(&[usize], &[Option<Vec<usize>>]) -> R,
    ) -> R {
        let mut w = self.wraps.borrow_mut();
        let key = Some((m.font_px, m.tab_cols, width));
        let n = self.lines.len();
        let found = (w.slots.iter()).position(|s| s.key == key && s.breaks.len() == n);
        let i = found.unwrap_or_else(|| {
            let i = 1 - w.recent;
            w.slots[i] = Slot {
                key,
                breaks: vec![None; n],
                first: None,
            };
            i
        });
        w.recent = i;
        let slot = &mut w.slots[i];
        let first = slot.first.get_or_insert_with(|| {
            let mut first = Vec::with_capacity(n + 1);
            let mut at = 0;
            for (b, line) in slot.breaks.iter_mut().zip(&self.lines) {
                first.push(at);
                at += 1 + b.get_or_insert_with(|| breaks(text, m, line, width)).len();
            }
            first.push(at);
            first
        });
        f(first, &slot.breaks)
    }

    /// The rows the text takes wrapped at `wrap` px, or a row per line.
    pub(super) fn row_count(&self, text: &TextRenderer, m: &Metrics, wrap: Option<i32>) -> usize {
        match wrap {
            None => self.lines.len(),
            Some(w) => self.with_rows(text, m, w, |first, _| first[first.len() - 1]),
        }
    }

    /// The row `p` is drawn on; a position at a soft break starts the next row.
    pub(super) fn row_of(
        &self,
        text: &TextRenderer,
        m: &Metrics,
        wrap: Option<i32>,
        p: Pos,
    ) -> usize {
        let p = self.clamp(p);
        match wrap {
            None => p.line,
            Some(w) => self.with_rows(text, m, w, |first, breaks| {
                let b = breaks[p.line].as_deref().unwrap_or_default();
                first[p.line] + b.partition_point(|&b| b <= p.col)
            }),
        }
    }

    /// Row `i`, or the last row past the end.
    pub(super) fn row(&self, text: &TextRenderer, m: &Metrics, wrap: Option<i32>, i: usize) -> Row {
        let whole = |line: usize| Row {
            line,
            start: 0,
            end: self.lines[line].len(),
            last: true,
        };
        let Some(w) = wrap else {
            return whole(i.min(self.lines.len() - 1));
        };
        self.with_rows(text, m, w, |first, breaks| {
            let line = (first.partition_point(|&f| f <= i) - 1).min(self.lines.len() - 1);
            let b = breaks[line].as_deref().unwrap_or_default();
            let k = (i - first[line]).min(b.len());
            Row {
                line,
                start: if k == 0 { 0 } else { b[k - 1] },
                end: b.get(k).copied().unwrap_or(self.lines[line].len()),
                last: k == b.len(),
            }
        })
    }
}
