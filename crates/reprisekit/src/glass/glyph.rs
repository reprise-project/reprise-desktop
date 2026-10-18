//! The glyphs Glass draws from shapes, each a [`Mark`]: pieces (rects, polygons, lines, rings) in design
//! pixels, painted at any scale by pixel coverage and outlined by growing them.

use std::borrow::Cow;

use crate::raster::Canvas;
use crate::shape::{Px, centre, draw, scale};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
    Help,
}

#[derive(Clone, Debug)]
pub enum Piece {
    Rect(f64, f64, f64, f64),
    Poly(Cow<'static, [(f64, f64)]>),
    Line(Cow<'static, [(f64, f64)]>, f64),
    /// A circle round (cx, cy), radius r to the middle of its stroke, the
    /// stroke this wide.
    Ring(f64, f64, f64, f64),
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub w: i32,
    pub h: i32,
    pub pieces: Cow<'static, [Piece]>,
}

const GRID: i32 = 8;

impl Mark {
    /// How far (`x`, `y`) lies outside the mark, in design px; less than
    /// 0 inside it.
    pub fn dist(&self, x: f64, y: f64) -> f64 {
        self.pieces
            .iter()
            .map(|p| piece_dist(p, x, y))
            .fold(f64::INFINITY, f64::min)
    }

    /// How much of the device pixel at (`x`, `y`) from the mark's top
    /// left, at `d` device px per design px, the mark grown by `grow`
    /// design px covers, 0..1: 1 for a pixel wholly inside a whole-pixel
    /// edge at d=1, partial where an edge slants or curves through it.
    pub fn cover(&self, x: i32, y: i32, d: i32, grow: f64) -> f64 {
        let k = f64::from(d);
        let reach = std::f64::consts::FRAC_1_SQRT_2 / k;
        let mid = self.dist((f64::from(x) + 0.5) / k, (f64::from(y) + 0.5) / k) - grow;
        if mid >= reach {
            return 0.0;
        }
        if mid <= -reach {
            return 1.0;
        }
        let hits = (0..GRID * GRID)
            .filter(|i| {
                let px = (f64::from(x) + (f64::from(i % GRID) + 0.5) / f64::from(GRID)) / k;
                let py = (f64::from(y) + (f64::from(i / GRID) + 0.5) / f64::from(GRID)) / k;
                self.dist(px, py) <= grow
            })
            .count();
        hits as f64 / f64::from(GRID * GRID)
    }

    /// The mark painted at `d`: each pixel `ink` at its middle, as design
    /// columns and rows (0 the first one's middle, [`centre`]), at the
    /// mark's cover.
    pub fn canvas(&self, d: i32, ink: impl Fn(f64, f64) -> Px) -> Canvas {
        draw(self.w, self.h, d, |x, y| {
            let (x, y) = (x as i32, y as i32);
            scale(ink(centre(x, d), centre(y, d)), self.cover(x, y, d, 0.0))
        })
    }
}

fn piece_dist(p: &Piece, x: f64, y: f64) -> f64 {
    match *p {
        Piece::Rect(x0, y0, x1, y1) => {
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let qx = (x - cx).abs() - (x1 - x0) / 2.0;
            let qy = (y - cy).abs() - (y1 - y0) / 2.0;
            qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0)
        }
        Piece::Poly(ref pts) => {
            let mut near = f64::INFINITY;
            let mut inside = false;
            for (i, &a) in pts.iter().enumerate() {
                let b = pts[(i + 1) % pts.len()];
                near = near.min(segment(x, y, a, b));
                if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                    inside = !inside;
                }
            }
            if inside { -near } else { near }
        }
        Piece::Line(ref pts, w) => {
            pts.windows(2)
                .map(|s| segment(x, y, s[0], s[1]))
                .fold(f64::INFINITY, f64::min)
                - w / 2.0
        }
        Piece::Ring(cx, cy, r, w) => ((x - cx).hypot(y - cy) - r).abs() - w / 2.0,
    }
}

fn segment(x: f64, y: f64, a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 {
        0.0
    } else {
        (((x - a.0) * dx + (y - a.1) * dy) / len).clamp(0.0, 1.0)
    };
    (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
}

/// The Square frame's ? hook in its 10x10 box, drawn 1.2 px wide.
const HOOK: [(f64, f64); 8] = [
    (2.5, 2.9),
    (2.6, 2.3),
    (3.5, 1.5),
    (6.5, 1.5),
    (7.4, 2.3),
    (7.5, 3.5),
    (4.5, 6.5),
    (4.5, 7.9),
];

/// The Rounded frame's ? hook, drawn 2 px wide from 5 px left of its cell's middle column.
const BOLD_HOOK: [(f64, f64); 10] = [
    (4.0, 2.6),
    (4.3, 1.8),
    (5.0, 1.2),
    (6.0, 1.0),
    (7.0, 1.2),
    (7.7, 1.8),
    (8.0, 2.8),
    (7.8, 3.8),
    (6.0, 5.6),
    (6.0, 6.9),
];

/// `v` design px grown `f` times, to the nearest whole px: a grown glyph's sizes and stroke widths.
fn whole(v: f64, f: f64) -> f64 {
    (v * f).round()
}

fn rects(r: &[[f64; 4]]) -> Vec<Piece> {
    r.iter()
        .map(|&[x0, y0, x1, y1]| Piece::Rect(x0, y0, x1, y1))
        .collect()
}

/// A mark `h` rows tall at least `w` wide, widened to its pieces' right edge.
fn fit(w: f64, h: f64, pieces: Vec<Piece>) -> Mark {
    let right = pieces
        .iter()
        .map(|p| match p {
            Piece::Rect(_, _, x1, _) => *x1,
            Piece::Poly(pts) => pts.iter().map(|p| p.0).fold(0.0, f64::max),
            Piece::Line(pts, w) => pts.iter().map(|p| p.0 + w / 2.0).fold(0.0, f64::max),
            Piece::Ring(cx, _, r, w) => cx + r + w / 2.0,
        })
        .fold(w, f64::max);
    Mark {
        w: right.ceil() as i32,
        h: h as i32,
        pieces: Cow::Owned(pieces),
    }
}

/// `g`'s lines in its 10x10 box (the Square frame's) grown `f` times, each line [`whole`] px wide.
/// Minimize is a bar across the middle, maximize a square, restore a square 8 px wide in front of a
/// second one up and to the right, close an X corner to corner, ? a hook over a dot.
pub fn lines(g: Glyph, f: f64) -> Mark {
    let (w, t) = (whole(10.0, f), whole(1.0, f));
    let o = whole(2.0, f);
    let pieces = match g {
        Glyph::Minimize => {
            let y = (w / 2.0).floor();
            rects(&[[0.0, y, w, y + t]])
        }
        Glyph::Maximize => rects(&[
            [0.0, 0.0, w, t],
            [0.0, w - t, w, w],
            [0.0, 0.0, t, w],
            [w - t, 0.0, w, w],
        ]),
        Glyph::Restore => {
            let a = w - o;
            rects(&[
                [0.0, o, a, o + t],
                [0.0, w - t, a, w],
                [0.0, o, t, w],
                [a - t, o, a, w],
                [o, 0.0, w, t],
                [w - t, 0.0, w, a],
                [o, 0.0, o + t, o],
                [a, a - t, w - t, a],
            ])
        }
        Glyph::Close => vec![
            Piece::Poly(Cow::Owned(vec![
                (0.0, 0.0),
                (t, 0.0),
                (w, w - t),
                (w, w),
                (w - t, w),
                (0.0, t),
            ])),
            Piece::Poly(Cow::Owned(vec![
                (w, 0.0),
                (w, t),
                (t, w),
                (0.0, w),
                (0.0, w - t),
                (w - t, 0.0),
            ])),
        ],
        Glyph::Help => {
            let k = w / 10.0;
            let x = (4.5 * k - t / 2.0).round();
            vec![
                Piece::Line(
                    Cow::Owned(HOOK.iter().map(|&(x, y)| (x * k, y * k)).collect()),
                    1.2 * t,
                ),
                Piece::Rect(x, w - t, x + t, w),
            ]
        }
    };
    fit(w, w, pieces)
}

/// The column of a [`bold`] mark grown `f` times that lies 1 px right of its cell's middle column.
pub fn bold_column(f: f64) -> i32 {
    whole(6.0, f) as i32
}

/// `g`'s bold shape on the Rounded frame grown `f` times, each stroke [`whole`] px wide, and the
/// restored strip's row its middle lies on (a maximized strip lays it a row higher). Minimize is a
/// 10x3 bar, maximize a 10x8 window framed 2 px, restore that window 8 px wide before a 1 px one up
/// and to the right, close an X of strokes 3 px across, ? a 2 px hook over a dot.
pub fn bold(g: Glyph, f: f64) -> (f64, Mark) {
    let s = |v: f64| whole(v, f);
    let c = f64::from(bold_column(f));
    let (w, t) = (s(10.0), s(2.0));
    let (mid, h, pieces) = match g {
        Glyph::Minimize => {
            let x = c - (w / 2.0).floor();
            let h = s(3.0);
            (11.5, h, rects(&[[x, 0.0, x + w, h]]))
        }
        Glyph::Maximize => {
            let (x, h) = (c - (w / 2.0).floor(), s(8.0));
            let r = x + w;
            (
                9.0,
                h,
                rects(&[
                    [x, 0.0, r, t],
                    [x, h - t, r, h],
                    [x, t, x + t, h - t],
                    [r - t, t, r, h - t],
                ]),
            )
        }
        Glyph::Restore => {
            let (x, o, b) = (c - s(6.0), s(2.0), s(1.0));
            let (a, r) = (x + w - o, x + w);
            (
                9.0,
                w,
                rects(&[
                    [x, o, a, o + t],
                    [x, w - t, a, w],
                    [x, o + t, x + t, w - t],
                    [a - t, o + t, a, w - t],
                    [x + o, 0.0, r, b],
                    [r - b, 0.0, r, w - o],
                ]),
            )
        }
        Glyph::Close => {
            let (span, h, t) = (s(9.0), s(8.0), s(3.0));
            let x = c - (span / 2.0).floor();
            let r = x + span;
            (
                9.0,
                h,
                vec![
                    Piece::Poly(Cow::Owned(vec![(x, 0.0), (x + t, 0.0), (r, h), (r - t, h)])),
                    Piece::Poly(Cow::Owned(vec![(r - t, 0.0), (r, 0.0), (x + t, h), (x, h)])),
                ],
            )
        }
        Glyph::Help => {
            let shift = c - 6.0 * f;
            let hook: Vec<(f64, f64)> = BOLD_HOOK
                .iter()
                .map(|&(x, y)| (x * f + shift, y * f))
                .collect();
            // The dot sits clear of the hook's end however the two round.
            let end = hook[hook.len() - 1].1 + t / 2.0;
            let top = s(8.0).max((end - 1e-9).ceil());
            let x = c - (t / 2.0).floor();
            (
                9.0,
                top + t,
                vec![
                    Piece::Line(Cow::Owned(hook), t),
                    Piece::Rect(x, top, x + t, top + t),
                ],
            )
        }
    };
    (mid, fit(s(11.0), h, pieces))
}

pub const MAGNIFIER: Mark = Mark {
    w: 13,
    h: 13,
    pieces: Cow::Borrowed(&[
        Piece::Ring(8.1, 4.5, 3.7, 1.7),
        Piece::Line(Cow::Borrowed(&[(5.4, 7.4), (0.8, 12.0)]), 2.2),
    ]),
};

pub const CLEAR: Mark = Mark {
    w: 7,
    h: 7,
    pieces: Cow::Borrowed(&[
        Piece::Line(Cow::Borrowed(&[(0.6, 0.6), (6.4, 6.4)]), 1.3),
        Piece::Line(Cow::Borrowed(&[(6.4, 0.6), (0.6, 6.4)]), 1.3),
    ]),
};

pub const TRIANGLE_RIGHT: Mark = Mark {
    w: 6,
    h: 11,
    pieces: Cow::Borrowed(&[Piece::Poly(Cow::Borrowed(&[
        (0.0, 0.0),
        (6.0, 5.5),
        (0.0, 11.0),
    ]))]),
};

pub const TRIANGLE_LEFT: Mark = Mark {
    w: 6,
    h: 11,
    pieces: Cow::Borrowed(&[Piece::Poly(Cow::Borrowed(&[
        (6.0, 0.0),
        (0.0, 5.5),
        (6.0, 11.0),
    ]))]),
};

pub const ARROW: Mark = Mark {
    w: 4,
    h: 7,
    pieces: Cow::Borrowed(&[Piece::Poly(Cow::Borrowed(&[
        (0.0, -0.5),
        (4.0, 3.5),
        (0.0, 7.5),
    ]))]),
};

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Glyph; 5] = [
        Glyph::Minimize,
        Glyph::Maximize,
        Glyph::Restore,
        Glyph::Close,
        Glyph::Help,
    ];

    fn covers(m: &Mark, d: i32) -> Vec<f64> {
        (0..m.h * d)
            .flat_map(|y| (0..m.w * d).map(move |x| (x, y)))
            .map(|(x, y)| m.cover(x, y, d, 0.0))
            .collect()
    }

    #[test]
    fn level_and_upright_lines_cover_whole_pixels() {
        for g in [Glyph::Minimize, Glyph::Maximize, Glyph::Restore] {
            for m in [lines(g, 1.0), bold(g, 1.0).1] {
                for d in [1, 2] {
                    let c = covers(&m, d);
                    assert!(c.iter().all(|&v| v == 0.0 || v == 1.0), "{g:?} at {d}");
                }
                let one = covers(&m, 1).iter().filter(|&&v| v == 1.0).count();
                let two = covers(&m, 2).iter().filter(|&&v| v == 1.0).count();
                assert_eq!(two, 4 * one, "{g:?}: four times the pixels at 2");
            }
        }
        let min = lines(Glyph::Minimize, 1.0);
        assert_eq!(min.cover(4, 5, 1, 0.0), 1.0, "the bar's row");
        assert_eq!(min.cover(4, 4, 1, 0.0), 0.0, "above it");
    }

    #[test]
    fn glyphs_are_solid_at_1_and_smooth_at_2() {
        let mut marks: Vec<(String, Mark)> = Vec::new();
        for g in ALL {
            marks.push((format!("lines {g:?}"), lines(g, 1.0)));
            marks.push((format!("bold {g:?}"), bold(g, 1.0).1));
        }
        for (n, m) in [
            ("magnifier", MAGNIFIER),
            ("clear", CLEAR),
            ("right", TRIANGLE_RIGHT),
            ("left", TRIANGLE_LEFT),
            ("arrow", ARROW),
        ] {
            marks.push((n.into(), m));
        }
        for (what, m) in &marks {
            let solid = covers(m, 1).iter().filter(|&&v| v == 1.0).count();
            assert!(solid >= 2, "{what}: {solid} whole pixels at 1");
        }
        for (what, m) in marks.iter().filter(|(n, _)| {
            !["Minimize", "Maximize", "Restore"]
                .iter()
                .any(|g| n.ends_with(g))
        }) {
            let soft = covers(m, 2).iter().filter(|&&v| v > 0.0 && v < 1.0).count();
            assert!(soft >= 4, "{what}: {soft} soft pixels at 2");
        }
    }

    /// The whole pixels a mark covers at 1 in column `x`, from row 0 down, before the first gap.
    fn run_down(m: &Mark, x: i32) -> i32 {
        (0..m.h)
            .take_while(|&y| m.cover(x, y, 1, 0.0) == 1.0)
            .count() as i32
    }

    #[test]
    fn grown_marks_grow_with_the_cell_in_whole_px_strokes() {
        for (f, bar, frame) in [(23.0 / 16.0, (14, 4), 3), (29.0 / 16.0, (18, 5), 4)] {
            let (_, min) = bold(Glyph::Minimize, f);
            let x0 = (0..min.w)
                .find(|&x| min.cover(x, 0, 1, 0.0) == 1.0)
                .unwrap();
            let wide = (x0..min.w)
                .take_while(|&x| min.cover(x, 0, 1, 0.0) == 1.0)
                .count();
            assert_eq!((wide as i32, min.h), bar, "the bar at {f}");
            let (_, max) = bold(Glyph::Maximize, f);
            assert_eq!(run_down(&max, max.w / 2), frame, "the window's top at {f}");
            for g in ALL {
                let (_, design) = bold(g, 1.0);
                let (_, grown) = bold(g, f);
                let want = f64::from(design.h) * f;
                assert!(
                    (f64::from(grown.h) - want).abs() <= 1.0,
                    "{g:?} at {f}: {} rows",
                    grown.h
                );
            }
        }
        for (f, w, t) in [(27.0 / 20.0, 14, 1), (33.0 / 20.0, 17, 2)] {
            let max = lines(Glyph::Maximize, f);
            assert_eq!((max.w, max.h), (w, w), "the box at {f}");
            assert_eq!(run_down(&max, w / 2), t, "the line at {f}");
            assert_eq!(
                run_down(&lines(Glyph::Close, f), 0),
                t,
                "the X's corner at {f}"
            );
        }
    }

    #[test]
    fn grown_marks_are_crisp_at_1_and_smooth_at_2() {
        for f in [23.0 / 16.0, 29.0 / 16.0, 27.0 / 20.0, 33.0 / 20.0] {
            for g in ALL {
                for m in [lines(g, f), bold(g, f).1] {
                    let solid = covers(&m, 1).iter().filter(|&&v| v == 1.0).count();
                    assert!(solid >= 4, "{g:?} at {f}: {solid} whole pixels at 1");
                    let soft = covers(&m, 2)
                        .iter()
                        .filter(|&&v| v > 0.0 && v < 1.0)
                        .count();
                    match g {
                        Glyph::Close | Glyph::Help => {
                            assert!(soft >= 4, "{g:?} at {f}: {soft} soft pixels at 2")
                        }
                        _ => assert_eq!(soft, 0, "{g:?} at {f}: whole-pixel edges"),
                    }
                }
            }
        }
    }

    #[test]
    fn the_close_x_runs_corner_to_corner() {
        let m = lines(Glyph::Close, 1.0);
        for i in 0..10 {
            assert_eq!(m.cover(i, i, 1, 0.0), 1.0, "({i}, {i})");
            assert_eq!(m.cover(9 - i, i, 1, 0.0), 1.0, "({}, {i})", 9 - i);
        }
        assert_eq!(m.cover(0, 5, 1, 0.0), 0.0, "clear at the left middle");
        let (_, b) = bold(Glyph::Close, 1.0);
        assert_eq!(b.cover(3, 0, 1, 0.0), 1.0, "the bold stroke's top");
        assert_eq!(b.cover(6, 4, 1, 0.0), 1.0, "the crossing");
        assert_eq!(b.cover(6, 0, 1, 0.0), 0.0, "between the strokes");
    }

    #[test]
    fn growing_a_mark_outlines_it() {
        let (_, m) = bold(Glyph::Minimize, 1.0);
        assert_eq!(m.cover(5, 3, 1, 1.0), 1.0, "below the bar");
        assert_eq!(m.cover(5, 3, 1, 0.0), 0.0, "off the bar");
        let corner = m.cover(0, 3, 1, 1.0);
        assert!(corner > 0.5 && corner < 1.0, "off its corner: {corner}");
        assert_eq!(m.cover(5, 5, 1, 1.0), 0.0, "2 px off");
    }

    #[test]
    fn the_row_arrow_points_right_from_its_middle_row() {
        for y in 0..7 {
            assert!(ARROW.cover(0, y, 1, 0.0) > 0.8, "back, row {y}");
        }
        assert!(ARROW.cover(3, 3, 1, 0.0) >= 0.5, "point");
        assert!(ARROW.cover(3, 0, 1, 0.0) == 0.0, "clear above the point");
    }
}
