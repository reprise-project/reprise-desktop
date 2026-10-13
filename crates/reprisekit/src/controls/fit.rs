//! Room for measured text in a form laid out in fixed px: a label wider than its rect takes
//! the space to its right, and when that is not enough the column of controls right of it
//! moves over. Widths only change where the text does not fit, and a label that cannot get
//! all the room it needs leaves the layout as it was.

use crate::raster::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Left-aligned text `need` px wide.
    Text { need: i32 },
    /// A control that can give up width at its left edge (an edit, a combo, a list, a
    /// trackbar), whose value wants `need` px of width (0 for no value of its own).
    Stretch { need: i32 },
    /// A control of fixed width: it moves with its column.
    Fixed,
    /// A push button: it holds its place, and a column it is in does not move.
    Anchored,
    /// A group box. The items inside it reach at most its right edge less the pad.
    Group,
    /// Drawn on or under other items (a group title, a break in a line, a rule): in no row.
    Apart,
}

/// The px kept before the next control on a row, at 100% text.
pub(crate) const GAP: i32 = 4;
/// The px kept inside a group box's right edge, at 100% text.
pub(crate) const PAD: i32 = 6;

/// Where item `i`'s right edge may run to in `rects`: the next control on its row, or its
/// group box's right edge less `pad`.
pub(crate) fn limit(rects: &[Rect], kinds: &[Kind], i: usize, pad: i32) -> i32 {
    let f = Fit {
        out: rects.to_vec(),
        kinds,
        gap: 0,
        pad,
    };
    f.limit(i)
}

impl Kind {
    fn in_rows(self) -> bool {
        !matches!(self, Kind::Group | Kind::Apart)
    }
}

/// The rects `rects` take with every text and value given the width it needs where there is
/// room: `gap` px is kept before the next control on a row, and `pad` px inside a group box's
/// right edge.
pub(crate) fn fit(rects: &[Rect], kinds: &[Kind], gap: i32, pad: i32) -> Vec<Rect> {
    let mut f = Fit {
        out: rects.to_vec(),
        kinds,
        gap,
        pad,
    };
    for (i, &kind) in kinds.iter().enumerate() {
        if let Kind::Text { need } = kind
            && need > f.out[i].width
        {
            f.text(i, need);
        }
    }
    for (i, &kind) in kinds.iter().enumerate() {
        if let Kind::Stretch { need } = kind
            && need > f.out[i].width
        {
            let room = f.limit(i) - f.out[i].x;
            f.out[i].width = need.min(room).max(f.out[i].width);
        }
    }
    f.out
}

struct Fit<'a> {
    out: Vec<Rect>,
    kinds: &'a [Kind],
    gap: i32,
    pad: i32,
}

impl Fit<'_> {
    /// Gives text item `i` the `need` px it takes, or leaves the layout as it was when that
    /// room cannot be made: a label cut a few px later reads no better.
    fn text(&mut self, i: usize, need: i32) {
        let before = self.out.clone();
        if let Some(j) = self.next(i) {
            let short = need - (self.out[j].x - self.gap - self.out[i].x);
            if short > 0 {
                self.shift_column(j, short);
            }
        }
        if self.limit(i) - self.out[i].x >= need {
            self.out[i].width = need;
        } else {
            self.out = before;
        }
    }

    /// Moves the column item `j` starts `d` px right, unless that moves a push button or
    /// pushes a control that keeps its width past its bound.
    fn shift_column(&mut self, j: usize, d: i32) {
        let before = self.out.clone();
        let bounds: Vec<i32> = (0..before.len()).map(|k| self.bound(k)).collect();
        self.shift_column_by(j, d);
        let blocked = (0..self.out.len()).any(|k| {
            self.out[k] != before[k]
                && match self.kinds[k] {
                    Kind::Anchored => true,
                    Kind::Stretch { .. } => false,
                    _ => self.out[k].right() > bounds[k],
                }
        });
        if blocked {
            self.out = before;
        }
    }

    /// Where item `i`'s right edge may run to: the next control on its row less the gap,
    /// or its bound.
    fn limit(&self, i: usize) -> i32 {
        match self.next(i) {
            Some(j) => self.out[j].x - self.gap,
            None => self.bound(i),
        }
    }

    fn row(&self, a: usize, b: usize) -> bool {
        let (p, q) = (self.out[a], self.out[b]);
        a != b
            && self.kinds[a].in_rows()
            && self.kinds[b].in_rows()
            && p.y < q.bottom()
            && q.y < p.bottom()
    }

    /// The nearest control starting right of item `i`'s left edge on its row.
    fn next(&self, i: usize) -> Option<usize> {
        (0..self.out.len())
            .filter(|&j| self.row(i, j) && self.out[j].x > self.out[i].x)
            .min_by_key(|&j| self.out[j].x)
    }

    /// The innermost group box holding item `i`.
    fn group(&self, i: usize) -> Option<usize> {
        let r = self.out[i];
        (0..self.out.len())
            .filter(|&g| g != i && self.kinds[g] == Kind::Group)
            .filter(|&g| {
                let b = self.out[g];
                b.x <= r.x && b.y <= r.y && r.right() <= b.right() && r.bottom() <= b.bottom()
            })
            .min_by_key(|&g| self.out[g].width as i64 * self.out[g].height as i64)
    }

    /// How far right item `i` may reach: its group box's right edge less the pad, or outside
    /// any group the rightmost edge in the form.
    fn bound(&self, i: usize) -> i32 {
        match self.group(i) {
            Some(g) => self.out[g].right() - self.pad,
            None => (0..self.out.len())
                .filter(|&k| self.kinds[k] != Kind::Apart)
                .map(|k| self.out[k].right())
                .max()
                .unwrap_or(0),
        }
    }

    /// Moves every control left-aligned with item `j` in its group `d` px right.
    fn shift_column_by(&mut self, j: usize, d: i32) {
        let (x, group) = (self.out[j].x, self.group(j));
        let column: Vec<usize> = (0..self.out.len())
            .filter(|&k| self.kinds[k].in_rows() && self.out[k].x == x && self.group(k) == group)
            .collect();
        let mut moved = vec![false; self.out.len()];
        for k in column {
            self.shift(k, d, &mut moved);
        }
    }

    /// Moves item `k`'s left edge `d` px right. A stretching control keeps its right edge
    /// (a combo then widens again where its value needs it); any other moves whole, and what
    /// follows it on its row moves with it.
    fn shift(&mut self, k: usize, d: i32, moved: &mut [bool]) {
        if moved[k] {
            return;
        }
        moved[k] = true;
        self.out[k].x += d;
        if matches!(self.kinds[k], Kind::Stretch { .. }) {
            self.out[k].width = (self.out[k].width - d).max(0);
            return;
        }
        let right = self.out[k].right() - d;
        let follower = (0..self.out.len())
            .filter(|&f| !moved[f] && self.row(k, f) && self.out[f].x >= right - 1)
            .min_by_key(|&f| self.out[f].x);
        if let Some(f) = follower {
            self.shift(f, d, moved);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect::new(x, y, w, h)
    }

    #[test]
    fn a_layout_whose_text_fits_is_unchanged() {
        let rects = [r(0, 0, 200, 100), r(12, 20, 80, 13), r(92, 16, 100, 21)];
        let kinds = [
            Kind::Group,
            Kind::Text { need: 80 },
            Kind::Stretch { need: 90 },
        ];
        assert_eq!(fit(&rects, &kinds, GAP, PAD), rects.to_vec());
    }

    #[test]
    fn a_label_widens_into_the_gap_before_its_field() {
        let rects = [r(12, 20, 70, 13), r(90, 16, 100, 21)];
        let kinds = [Kind::Text { need: 74 }, Kind::Stretch { need: 0 }];
        let out = fit(&rects, &kinds, GAP, PAD);
        assert_eq!(out[0], r(12, 20, 74, 13));
        assert_eq!(out[1], rects[1], "the field keeps its place");
    }

    #[test]
    fn a_wide_label_moves_its_column_and_the_fields_keep_their_right_edge() {
        // Two rows in a group: label + combo to the group's content edge, then label + combo.
        let rects = [
            r(0, 0, 200, 80),
            r(12, 20, 80, 13),
            r(92, 16, 96, 21),
            r(12, 46, 80, 13),
            r(92, 42, 96, 21),
        ];
        let kinds = [
            Kind::Group,
            Kind::Text { need: 70 },
            Kind::Stretch { need: 0 },
            Kind::Text { need: 90 },
            Kind::Stretch { need: 0 },
        ];
        let out = fit(&rects, &kinds, GAP, PAD);
        assert_eq!(out[3], r(12, 46, 90, 13));
        assert_eq!(out[4], r(106, 42, 82, 21), "14 px over, right edge kept");
        assert_eq!(out[2], r(106, 16, 82, 21), "the column moves as one");
        assert_eq!(out[1], rects[1]);
    }

    #[test]
    fn a_spinner_moves_with_the_units_after_it() {
        let rects = [
            r(0, 0, 300, 80),
            r(12, 20, 100, 13),
            r(116, 16, 50, 20),
            r(170, 20, 30, 13),
        ];
        let kinds = [
            Kind::Group,
            Kind::Text { need: 110 },
            Kind::Fixed,
            Kind::Text { need: 12 },
        ];
        let out = fit(&rects, &kinds, GAP, PAD);
        assert_eq!(out[1].width, 110);
        assert_eq!(out[2], r(126, 16, 50, 20));
        assert_eq!(out[3], r(180, 20, 30, 13));
    }

    #[test]
    fn a_label_alone_on_its_row_widens_up_to_the_group_pad() {
        let rects = [r(0, 0, 200, 80), r(12, 20, 160, 13)];
        let widened = fit(&rects, &[Kind::Group, Kind::Text { need: 170 }], GAP, PAD);
        assert_eq!(widened[1].width, 170);
        let kinds = [Kind::Group, Kind::Text { need: 190 }];
        assert_eq!(
            fit(&rects, &kinds, GAP, PAD),
            rects.to_vec(),
            "past 6 px inside the box's right edge, nothing moves"
        );
    }

    #[test]
    fn a_combo_widens_to_its_widest_value_where_there_is_room() {
        let rects = [
            r(0, 0, 200, 80),
            r(12, 16, 100, 21),
            r(12, 46, 100, 21),
            r(150, 42, 30, 20),
        ];
        let kinds = [
            Kind::Group,
            Kind::Stretch { need: 130 },
            Kind::Stretch { need: 160 },
            Kind::Fixed,
        ];
        let out = fit(&rects, &kinds, GAP, PAD);
        assert_eq!(out[1].width, 130);
        assert_eq!(out[2].width, 134, "up to the gap before the next control");
    }

    #[test]
    fn a_control_at_the_form_edge_holds_its_column() {
        let rects = [r(10, 20, 100, 13), r(114, 16, 75, 23)];
        let kinds = [Kind::Text { need: 130 }, Kind::Fixed];
        let out = fit(&rects, &kinds, GAP, PAD);
        assert_eq!(out[1], rects[1], "the control keeps the form's right edge");
        assert_eq!(out[0].width, 100);
    }

    #[test]
    fn a_push_button_holds_its_place() {
        let rects = [r(0, 0, 300, 80), r(10, 20, 100, 13), r(114, 16, 75, 23)];
        let kinds = [Kind::Group, Kind::Text { need: 130 }, Kind::Anchored];
        let out = fit(&rects, &kinds, GAP, PAD);
        assert_eq!(out[2], rects[2]);
        assert_eq!(out[1].width, 100);
    }

    #[test]
    fn items_on_or_under_a_row_stay_put() {
        let rects = [
            r(0, 0, 200, 80),
            r(10, -5, 40, 13),
            r(12, 20, 80, 13),
            r(84, 16, 96, 21),
        ];
        let kinds = [
            Kind::Group,
            Kind::Apart,
            Kind::Text { need: 90 },
            Kind::Stretch { need: 0 },
        ];
        let out = fit(&rects, &kinds, GAP, PAD);
        assert_eq!(out[1], rects[1]);
        assert_eq!(out[0], rects[0]);
        assert_eq!(out[3], r(106, 16, 74, 21));
    }
}
