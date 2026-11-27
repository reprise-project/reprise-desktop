//! A shared multi-selection model: selected indices, focus and the Shift anchor, with no drawing.
//! [`listview`] and [`shellview`] translate clicks, keys and marquee drags into calls here.
//!
//! [`listview`]: crate::controls::listview
//! [`shellview`]: crate::controls::shellview

use crate::raster::Rect;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub set: BTreeSet<usize>,
    pub focus: Option<usize>,
    pub anchor: Option<usize>,
}

impl Selection {
    /// A click on index `i`: plain replaces the set with `{i}`; Ctrl
    /// toggles `i` in place, leaving the anchor where it was (or setting
    /// it to `i` if there wasn't one); Shift selects the anchor..=i range,
    /// added to the existing set instead of replacing it when Ctrl is
    /// also held. Focus always ends on `i`.
    pub fn click(&mut self, i: usize, mods: Mods) {
        if mods.shift {
            let Some(anchor) = self.anchor else {
                self.set = BTreeSet::from([i]);
                self.focus = Some(i);
                self.anchor = Some(i);
                return;
            };
            let range = range_between(anchor, i);
            if mods.ctrl {
                self.set.extend(range);
            } else {
                self.set = range.collect();
            }
            self.focus = Some(i);
        } else if mods.ctrl {
            if !self.set.remove(&i) {
                self.set.insert(i);
            }
            self.focus = Some(i);
            if self.anchor.is_none() {
                self.anchor = Some(i);
            }
        } else {
            self.set = BTreeSet::from([i]);
            self.focus = Some(i);
            self.anchor = Some(i);
        }
    }

    pub fn set_only(&mut self, i: Option<usize>) {
        match i {
            Some(i) => self.click(i, Mods::default()),
            None => self.clear(),
        }
    }

    pub fn select_all(&mut self, n: usize) {
        self.set = (0..n).collect();
    }

    pub fn invert(&mut self, n: usize) {
        self.set = (0..n).filter(|i| !self.set.contains(i)).collect();
    }

    pub fn clear(&mut self) {
        self.set.clear();
        self.focus = None;
        self.anchor = None;
    }

    pub fn is(&self, i: usize) -> bool {
        self.set.contains(&i)
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    pub fn single(&self) -> Option<usize> {
        let mut it = self.set.iter();
        let only = *it.next()?;
        it.next().is_none().then_some(only)
    }

    pub fn move_focus(&mut self, to: usize, mods: Mods) {
        if mods.shift {
            let anchor = self.anchor.unwrap_or(to);
            self.anchor = Some(anchor);
            self.set = range_between(anchor, to).collect();
            self.focus = Some(to);
        } else if mods.ctrl {
            self.focus = Some(to);
        } else {
            self.click(to, Mods::default());
        }
    }

    pub fn marquee(&mut self, cells: &[Rect], band: Rect, base: &BTreeSet<usize>, ctrl: bool) {
        let intersecting: BTreeSet<usize> = cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| !cell.intersect(&band).is_empty())
            .map(|(i, _)| i)
            .collect();
        self.focus = intersecting.iter().next_back().copied();
        self.set = if ctrl {
            base.symmetric_difference(&intersecting).copied().collect()
        } else {
            intersecting
        };
    }

    pub fn remove_index(&mut self, i: usize) {
        self.set = self
            .set
            .iter()
            .filter(|&&j| j != i)
            .map(|&j| if j > i { j - 1 } else { j })
            .collect();
        self.focus = shift_index(self.focus, i);
        self.anchor = shift_index(self.anchor, i);
    }

    pub fn reorder(&mut self, order: &[usize]) {
        let mut new_of = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            new_of[old] = new;
        }
        let map = |i: usize| new_of.get(i).copied();
        self.set = self.set.iter().filter_map(|&i| map(i)).collect();
        self.focus = self.focus.and_then(map);
        self.anchor = self.anchor.and_then(map);
    }
}

fn shift_index(index: Option<usize>, removed: usize) -> Option<usize> {
    match index {
        Some(j) if j == removed => None,
        Some(j) if j > removed => Some(j - 1),
        other => other,
    }
}
/// Where a highlight on old row `at` goes among the `new` rows `takes` accepts: its own key, else
/// the next old row still listed, else the one before.
pub fn follow<K: PartialEq>(
    old: &[K],
    new: &[K],
    at: usize,
    takes: impl Fn(usize) -> bool,
) -> Option<usize> {
    let find = |i: usize| (0..new.len()).find(|&j| new[j] == old[i] && takes(j));
    let at = at.min(old.len().checked_sub(1)?);
    find(at)
        .or_else(|| (at + 1..old.len()).find_map(find))
        .or_else(|| (0..at).rev().find_map(find))
}

pub fn scroll_into_view(top: &mut usize, i: usize, visible: usize) {
    let visible = visible.max(1);
    if i < *top {
        *top = i;
    } else if i >= *top + visible {
        *top = i + 1 - visible;
    }
}

/// `n` is 0. A `cur` of `None` sits before the first row, so Down from
/// nothing lands on row 0.
pub fn step_index(cur: Option<usize>, delta: i64, n: usize) -> Option<usize> {
    let last = n.checked_sub(1)? as i64;
    let from = cur.map_or(-1, |i| i as i64);
    Some((from + delta).clamp(0, last) as usize)
}

/// How long after a letter typed on a list the next one still adds to the prefix: a word typed
/// at any pace a reader keeps up stays one search, and a pause to read the list starts a new one.
pub const TYPE_AHEAD_GAP: Duration = Duration::from_secs(1);

/// The letters typed on a list in a row, each within [`TYPE_AHEAD_GAP`] of the last.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeAhead {
    typed: String,
    last: Option<Instant>,
}

impl TypeAhead {
    /// [`TypeAhead::find_at`] for a letter typed now.
    pub fn find<S: AsRef<str>>(
        &mut self,
        names: impl IntoIterator<Item = S>,
        from: Option<usize>,
        ch: char,
    ) -> Option<usize> {
        self.find_at(names, from, ch, Instant::now())
    }

    /// The row of `names` for `ch` typed at `now` with row `from` focused, matched without case.
    /// One letter, or one letter typed again and again, steps to the next row after `from` that
    /// starts with it, wrapping; a longer prefix takes the first row from `from` on that starts
    /// with it, so `from` stays while it still matches.
    pub fn find_at<S: AsRef<str>>(
        &mut self,
        names: impl IntoIterator<Item = S>,
        from: Option<usize>,
        ch: char,
        now: Instant,
    ) -> Option<usize> {
        if self
            .last
            .is_none_or(|t| now.saturating_duration_since(t) > TYPE_AHEAD_GAP)
        {
            self.typed.clear();
        }
        self.last = Some(now);
        self.typed.extend(ch.to_lowercase());
        let names: Vec<S> = names.into_iter().collect();
        let n = names.len();
        let first = self.typed.chars().next()?;
        let (prefix, start) = match self.typed.chars().all(|c| c == first) {
            true => (&self.typed[..first.len_utf8()], from.map_or(0, |f| f + 1)),
            false => (self.typed.as_str(), from.unwrap_or(0)),
        };
        (0..n)
            .map(|k| (start + k) % n)
            .find(|&k| starts_with_lower(names[k].as_ref(), prefix))
    }
}

/// Whether a list takes `c` for type-ahead: any printing character but a space, which works the
/// focused row.
pub fn types_ahead(c: char) -> bool {
    !c.is_control() && !c.is_whitespace()
}

fn starts_with_lower(name: &str, prefix: &str) -> bool {
    let mut name = name.chars().flat_map(char::to_lowercase);
    prefix.chars().all(|c| name.next() == Some(c))
}

fn range_between(a: usize, b: usize) -> std::ops::RangeInclusive<usize> {
    a.min(b)..=a.max(b)
}

/// The rectangle a marquee drag from `from` to `to` (in either direction)
/// spans: it runs from the lower point up to but not including the
/// higher one, so a drag that has not moved on an axis has no width there.
pub fn band(from: (i32, i32), to: (i32, i32)) -> Rect {
    let x = from.0.min(to.0);
    let y = from.1.min(to.1);
    Rect::new(x, y, (from.0 - to.0).abs(), (from.1 - to.1).abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_into_view_moves_top_the_least_it_can() {
        let mut top = 5;
        scroll_into_view(&mut top, 7, 3);
        assert_eq!(top, 5, "already in view");
        scroll_into_view(&mut top, 2, 3);
        assert_eq!(top, 2, "above: lands on the row");
        scroll_into_view(&mut top, 9, 3);
        assert_eq!(top, 7, "below: the row is the last visible");
        scroll_into_view(&mut top, 8, 0);
        assert_eq!(top, 8, "a zero-row box counts as one");
    }

    #[test]
    fn step_index_clamps_and_starts_before_the_first_row() {
        assert_eq!(step_index(None, 1, 4), Some(0));
        assert_eq!(step_index(Some(3), 1, 4), Some(3));
        assert_eq!(step_index(Some(1), -5, 4), Some(0));
        assert_eq!(step_index(Some(1), i64::MAX / 2, 4), Some(3));
    }

    #[test]
    fn step_index_on_no_rows_is_none() {
        assert_eq!(step_index(None, 1, 0), None);
        assert_eq!(step_index(Some(0), -1, 0), None);
    }

    const ROWS: [&str; 7] = [
        "Write",
        "Other Programs:",
        "Image Viewer",
        "Notepad",
        "Draw",
        "Web Browser",
        "Notes",
    ];

    #[test]
    fn one_letter_steps_to_the_next_row_starting_with_it_and_wraps() {
        let t0 = Instant::now();
        let mut at = 0;
        let mut find = |from, ch| {
            at += 2;
            let now = t0 + Duration::from_secs(at);
            TypeAhead::default().find_at(ROWS, from, ch, now)
        };
        assert_eq!(find(Some(1), 'w'), Some(5));
        assert_eq!(find(Some(5), 'W'), Some(0));
        assert_eq!(find(Some(3), 'n'), Some(6));
        assert_eq!(find(Some(6), 'n'), Some(3));
        assert_eq!(find(None, 'o'), Some(1));
        assert_eq!(find(Some(0), 'z'), None);
        let none: [&str; 0] = [];
        assert_eq!(TypeAhead::default().find(none, None, 'a'), None);
    }

    #[test]
    fn letters_typed_together_build_a_prefix() {
        let t0 = Instant::now();
        let ms = |m| t0 + Duration::from_millis(m);
        let mut t = TypeAhead::default();
        assert_eq!(t.find_at(ROWS, Some(0), 'n', ms(0)), Some(3));
        assert_eq!(
            t.find_at(ROWS, Some(3), 'o', ms(400)),
            Some(3),
            "\"no\" stays on Notepad"
        );
        assert_eq!(t.find_at(ROWS, Some(3), 't', ms(800)), Some(3));
        assert_eq!(t.find_at(ROWS, Some(3), 'E', ms(1700)), Some(3));
        assert_eq!(
            t.find_at(ROWS, Some(3), 's', ms(2500)),
            Some(6),
            "\"notes\""
        );
        assert_eq!(
            t.find_at(ROWS, Some(6), 'x', ms(3000)),
            None,
            "\"notesx\" matches nothing"
        );
        assert_eq!(
            t.find_at(ROWS, Some(6), 'd', ms(4100)),
            Some(4),
            "after a pause over a second, \"d\" starts afresh"
        );
    }

    #[test]
    fn one_letter_typed_again_and_again_cycles() {
        let t0 = Instant::now();
        let ms = |m| t0 + Duration::from_millis(m);
        let mut t = TypeAhead::default();
        assert_eq!(t.find_at(ROWS, None, 'n', ms(0)), Some(3));
        assert_eq!(t.find_at(ROWS, Some(3), 'n', ms(200)), Some(6));
        assert_eq!(t.find_at(ROWS, Some(6), 'n', ms(400)), Some(3));
    }

    #[test]
    fn plain_click_replaces_the_set() {
        let mut s = Selection::default();
        s.click(3, Mods::default());
        s.click(5, Mods::default());
        assert_eq!(s.set, BTreeSet::from([5]));
        assert_eq!(s.focus, Some(5));
        assert_eq!(s.anchor, Some(5));
    }

    #[test]
    fn ctrl_click_toggles_and_keeps_anchor() {
        let mut s = Selection::default();
        s.click(2, Mods::default());
        s.click(
            7,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        assert_eq!(s.set, BTreeSet::from([2, 7]));
        assert_eq!(s.anchor, Some(2));
        assert_eq!(s.focus, Some(7));

        s.click(
            7,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        assert_eq!(s.set, BTreeSet::from([2]));
        assert_eq!(s.anchor, Some(2));
    }

    #[test]
    fn shift_click_selects_the_range_from_anchor() {
        let mut s = Selection::default();
        s.click(2, Mods::default());
        s.click(
            5,
            Mods {
                ctrl: false,
                shift: true,
            },
        );
        assert_eq!(s.set, BTreeSet::from([2, 3, 4, 5]));
        assert_eq!(s.anchor, Some(2));
        assert_eq!(s.focus, Some(5));

        let mut r = Selection::default();
        r.click(5, Mods::default());
        r.click(
            2,
            Mods {
                ctrl: false,
                shift: true,
            },
        );
        assert_eq!(r.set, BTreeSet::from([2, 3, 4, 5]));
    }

    #[test]
    fn ctrl_shift_click_adds_the_range_to_the_existing_set() {
        let mut s = Selection::default();
        s.click(0, Mods::default());
        s.click(
            2,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        s.anchor = Some(5);
        s.click(
            7,
            Mods {
                ctrl: true,
                shift: true,
            },
        );
        assert_eq!(s.set, BTreeSet::from([0, 2, 5, 6, 7]));
    }

    #[test]
    fn shift_click_without_an_anchor_is_a_plain_click() {
        let mut s = Selection::default();
        s.click(
            4,
            Mods {
                ctrl: false,
                shift: true,
            },
        );
        assert_eq!(s.set, BTreeSet::from([4]));
        assert_eq!(s.anchor, Some(4));
        assert_eq!(s.focus, Some(4));
    }

    #[test]
    fn set_only_and_clear() {
        let mut s = Selection::default();
        s.set_only(Some(3));
        assert_eq!(s.set, BTreeSet::from([3]));
        s.set_only(None);
        assert!(s.is_empty());
        assert_eq!(s.focus, None);
        assert_eq!(s.anchor, None);
    }

    #[test]
    fn select_all_and_invert() {
        let mut s = Selection::default();
        s.select_all(4);
        assert_eq!(s.set, BTreeSet::from([0, 1, 2, 3]));
        s.click(1, Mods::default());
        s.invert(4);
        assert_eq!(s.set, BTreeSet::from([0, 2, 3]));
    }

    #[test]
    fn single_is_some_only_for_exactly_one() {
        let mut s = Selection::default();
        assert_eq!(s.single(), None);
        s.click(3, Mods::default());
        assert_eq!(s.single(), Some(3));
        s.click(
            4,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        assert_eq!(s.single(), None);
    }

    #[test]
    fn move_focus_plain_shift_and_ctrl() {
        let mut s = Selection::default();
        s.click(2, Mods::default());

        s.move_focus(
            5,
            Mods {
                ctrl: false,
                shift: true,
            },
        );
        assert_eq!(s.set, BTreeSet::from([2, 3, 4, 5]));
        assert_eq!(s.anchor, Some(2));
        assert_eq!(s.focus, Some(5));

        s.move_focus(
            8,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        assert_eq!(s.set, BTreeSet::from([2, 3, 4, 5]), "ctrl only moves focus");
        assert_eq!(s.focus, Some(8));

        s.move_focus(1, Mods::default());
        assert_eq!(s.set, BTreeSet::from([1]));
        assert_eq!(s.anchor, Some(1));
        assert_eq!(s.focus, Some(1));
    }

    #[test]
    fn marquee_over_two_cells_with_and_without_ctrl() {
        let cells = vec![
            Rect::new(0, 0, 10, 10),
            Rect::new(0, 10, 10, 10),
            Rect::new(0, 20, 10, 10),
        ];
        let band = Rect::new(0, 10, 10, 20); // covers cells 1 and 2
        let base = BTreeSet::from([0]);

        let mut s = Selection::default();
        s.marquee(&cells, band, &base, true);
        assert_eq!(s.set, BTreeSet::from([0, 1, 2]));
        assert_eq!(s.focus, Some(2));

        let mut s = Selection::default();
        s.marquee(&cells, band, &base, false);
        assert_eq!(s.set, BTreeSet::from([1, 2]));
    }

    #[test]
    fn remove_index_shifts_greater_indices_down() {
        let mut s = Selection {
            set: BTreeSet::from([0, 1, 3]),
            focus: Some(3),
            anchor: Some(1),
        };
        s.remove_index(1);
        assert_eq!(s.set, BTreeSet::from([0, 2]));
        assert_eq!(s.focus, Some(2));
        assert_eq!(s.anchor, None);
    }

    #[test]
    fn band_spans_the_points_exclusive_of_the_higher() {
        assert_eq!(band((10, 10), (3, 4)), Rect::new(3, 4, 7, 6));
        assert_eq!(band((3, 4), (10, 10)), Rect::new(3, 4, 7, 6));
    }

    #[test]
    fn follow_keeps_the_entry_else_takes_the_next_then_the_one_before() {
        let old = ["a", "b", "c"];
        assert_eq!(follow(&old, &["x", "a", "b", "c"], 1, |_| true), Some(2));
        assert_eq!(follow(&old, &["a", "c"], 1, |_| true), Some(1));
        assert_eq!(follow(&old, &["a", "b"], 2, |_| true), Some(1));
        assert_eq!(follow(&old, &["a", "c"], 1, |j| j == 0), Some(0));
        assert_eq!(follow(&old, &["x"], 0, |_| true), None);
    }
}
