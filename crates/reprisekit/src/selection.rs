//! A shared multi-selection model: selected indices, focus and the Shift anchor, with no drawing.
//! [`listview`] and [`shellview`] translate clicks, keys and marquee drags into calls here.
//!
//! [`listview`]: crate::controls::listview
//! [`shellview`]: crate::controls::shellview

use crate::raster::Rect;
use std::collections::BTreeSet;

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

pub fn type_ahead<'a>(
    names: impl IntoIterator<Item = &'a str>,
    from: Option<usize>,
    ch: char,
) -> Option<usize> {
    let names: Vec<&str> = names.into_iter().collect();
    let n = names.len();
    if n == 0 {
        return None;
    }
    let ch = ch.to_lowercase().next()?;
    let start = from.map_or(0, |f| f + 1);
    (0..n).map(|k| (start + k) % n).find(|&k| {
        names[k]
            .chars()
            .next()
            .and_then(|c| c.to_lowercase().next())
            == Some(ch)
    })
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

    #[test]
    fn type_ahead_takes_the_next_match_after_from_and_wraps() {
        let rows = [
            "Write",
            "Other Programs:",
            "Image Viewer",
            "Notepad",
            "Draw",
            "Web Browser",
        ];
        let names = || rows.iter().copied();
        assert_eq!(type_ahead(names(), Some(1), 'w'), Some(5));
        assert_eq!(type_ahead(names(), Some(5), 'W'), Some(0));
        assert_eq!(type_ahead(names(), Some(3), 'n'), Some(3));
        assert_eq!(type_ahead(names(), Some(5), 'i'), Some(2));
        assert_eq!(type_ahead(names(), None, 'o'), Some(1));
        assert_eq!(type_ahead(names(), Some(0), 'z'), None);
        assert_eq!(type_ahead(std::iter::empty(), None, 'a'), None);
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
