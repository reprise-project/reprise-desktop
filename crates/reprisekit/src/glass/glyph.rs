//! The glyphs Glass draws from shapes, each a [`Mark`]: pieces (rects, polygons, lines, rings) in design
//! pixels, painted at any scale by pixel coverage and outlined by growing them.

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

#[derive(Clone, Copy, Debug)]
pub enum Piece {
    Rect(f64, f64, f64, f64),
    Poly(&'static [(f64, f64)]),
    Line(&'static [(f64, f64)], f64),
    /// A circle round (cx, cy), radius r to the middle of its stroke, the
    /// stroke this wide.
    Ring(f64, f64, f64, f64),
}

#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub w: i32,
    pub h: i32,
    pub pieces: &'static [Piece],
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
        Piece::Poly(pts) => {
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
        Piece::Line(pts, w) => {
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

/// `g`'s 1 px lines in its 10x10 box: the Square frame's. Minimize is a
/// bar across the middle, maximize a square, restore a square 8 px wide
/// in front of a second one up and to the right, close an X corner to
/// corner, ? a hook over a dot.
pub fn lines(g: Glyph) -> Mark {
    let pieces: &'static [Piece] = match g {
        Glyph::Minimize => &[Piece::Rect(0.0, 5.0, 10.0, 6.0)],
        Glyph::Maximize => &[
            Piece::Rect(0.0, 0.0, 10.0, 1.0),
            Piece::Rect(0.0, 9.0, 10.0, 10.0),
            Piece::Rect(0.0, 0.0, 1.0, 10.0),
            Piece::Rect(9.0, 0.0, 10.0, 10.0),
        ],
        Glyph::Restore => &[
            Piece::Rect(0.0, 2.0, 8.0, 3.0),
            Piece::Rect(0.0, 9.0, 8.0, 10.0),
            Piece::Rect(0.0, 2.0, 1.0, 10.0),
            Piece::Rect(7.0, 2.0, 8.0, 10.0),
            Piece::Rect(2.0, 0.0, 10.0, 1.0),
            Piece::Rect(9.0, 0.0, 10.0, 8.0),
            Piece::Rect(2.0, 0.0, 3.0, 2.0),
            Piece::Rect(8.0, 7.0, 9.0, 8.0),
        ],
        Glyph::Close => &[
            Piece::Poly(&[
                (0.0, 0.0),
                (1.0, 0.0),
                (10.0, 9.0),
                (10.0, 10.0),
                (9.0, 10.0),
                (0.0, 1.0),
            ]),
            Piece::Poly(&[
                (10.0, 0.0),
                (10.0, 1.0),
                (1.0, 10.0),
                (0.0, 10.0),
                (0.0, 9.0),
                (9.0, 0.0),
            ]),
        ],
        Glyph::Help => &[
            Piece::Line(
                &[
                    (2.5, 2.9),
                    (2.6, 2.3),
                    (3.5, 1.5),
                    (6.5, 1.5),
                    (7.4, 2.3),
                    (7.5, 3.5),
                    (4.5, 6.5),
                    (4.5, 7.9),
                ],
                1.2,
            ),
            Piece::Rect(4.0, 9.0, 5.0, 10.0),
        ],
    };
    Mark {
        w: 10,
        h: 10,
        pieces,
    }
}

/// `g`'s bold shape on the Rounded frame: the row of a restored strip its
/// top lies on (a maximized strip lays it a row higher), and its mark, drawn
/// white from 5 px left of its cell's middle column. Minimize is a 10x3 bar
/// low in the cell; maximize a 10x8 window with a 2 px frame round a 6x4
/// pane; restore that window 8 px wide in front of a second one up and to
/// the right; close an X of strokes 3 px across; ? a 2 px hook over a dot.
pub fn bold(g: Glyph) -> (i32, Mark) {
    let (row, h, pieces): (i32, i32, &'static [Piece]) = match g {
        Glyph::Minimize => (10, 3, &[Piece::Rect(1.0, 0.0, 11.0, 3.0)]),
        Glyph::Maximize => (
            5,
            8,
            &[
                Piece::Rect(1.0, 0.0, 11.0, 2.0),
                Piece::Rect(1.0, 6.0, 11.0, 8.0),
                Piece::Rect(1.0, 2.0, 3.0, 6.0),
                Piece::Rect(9.0, 2.0, 11.0, 6.0),
            ],
        ),
        Glyph::Restore => (
            4,
            10,
            &[
                Piece::Rect(0.0, 2.0, 8.0, 4.0),
                Piece::Rect(0.0, 8.0, 8.0, 10.0),
                Piece::Rect(0.0, 4.0, 2.0, 8.0),
                Piece::Rect(6.0, 4.0, 8.0, 8.0),
                Piece::Rect(2.0, 0.0, 10.0, 1.0),
                Piece::Rect(9.0, 0.0, 10.0, 8.0),
            ],
        ),
        Glyph::Close => (
            5,
            8,
            &[
                Piece::Poly(&[(2.0, 0.0), (5.0, 0.0), (11.0, 8.0), (8.0, 8.0)]),
                Piece::Poly(&[(8.0, 0.0), (11.0, 0.0), (5.0, 8.0), (2.0, 8.0)]),
            ],
        ),
        Glyph::Help => (
            4,
            10,
            &[
                Piece::Line(
                    &[
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
                    ],
                    2.0,
                ),
                Piece::Rect(5.0, 8.0, 7.0, 10.0),
            ],
        ),
    };
    (row, Mark { w: 11, h, pieces })
}

/// How many times its design size a glyph drawn in `rows` rows is, when its design fills `base`:
/// the nearest whole multiple, so its strokes stay whole design pixels.
pub fn multiple(rows: i32, base: i32) -> i32 {
    ((rows + base / 2) / base).max(1)
}

pub const MAGNIFIER: Mark = Mark {
    w: 13,
    h: 13,
    pieces: &[
        Piece::Ring(8.1, 4.5, 3.7, 1.7),
        Piece::Line(&[(5.4, 7.4), (0.8, 12.0)], 2.2),
    ],
};

pub const CLEAR: Mark = Mark {
    w: 7,
    h: 7,
    pieces: &[
        Piece::Line(&[(0.6, 0.6), (6.4, 6.4)], 1.3),
        Piece::Line(&[(6.4, 0.6), (0.6, 6.4)], 1.3),
    ],
};

pub const TRIANGLE_RIGHT: Mark = Mark {
    w: 6,
    h: 11,
    pieces: &[Piece::Poly(&[(0.0, 0.0), (6.0, 5.5), (0.0, 11.0)])],
};

pub const TRIANGLE_LEFT: Mark = Mark {
    w: 6,
    h: 11,
    pieces: &[Piece::Poly(&[(6.0, 0.0), (0.0, 5.5), (6.0, 11.0)])],
};

pub const ARROW: Mark = Mark {
    w: 4,
    h: 7,
    pieces: &[Piece::Poly(&[(0.0, -0.5), (4.0, 3.5), (0.0, 7.5)])],
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
            for m in [lines(g), bold(g).1] {
                for d in [1, 2] {
                    let c = covers(&m, d);
                    assert!(c.iter().all(|&v| v == 0.0 || v == 1.0), "{g:?} at {d}");
                }
                let one = covers(&m, 1).iter().filter(|&&v| v == 1.0).count();
                let two = covers(&m, 2).iter().filter(|&&v| v == 1.0).count();
                assert_eq!(two, 4 * one, "{g:?}: four times the pixels at 2");
            }
        }
        let min = lines(Glyph::Minimize);
        assert_eq!(min.cover(4, 5, 1, 0.0), 1.0, "the bar's row");
        assert_eq!(min.cover(4, 4, 1, 0.0), 0.0, "above it");
    }

    #[test]
    fn glyphs_are_solid_at_1_and_smooth_at_2() {
        let mut marks: Vec<(String, Mark)> = Vec::new();
        for g in ALL {
            marks.push((format!("lines {g:?}"), lines(g)));
            marks.push((format!("bold {g:?}"), bold(g).1));
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

    #[test]
    fn the_close_x_runs_corner_to_corner() {
        let m = lines(Glyph::Close);
        for i in 0..10 {
            assert_eq!(m.cover(i, i, 1, 0.0), 1.0, "({i}, {i})");
            assert_eq!(m.cover(9 - i, i, 1, 0.0), 1.0, "({}, {i})", 9 - i);
        }
        assert_eq!(m.cover(0, 5, 1, 0.0), 0.0, "clear at the left middle");
        let (_, b) = bold(Glyph::Close);
        assert_eq!(b.cover(3, 0, 1, 0.0), 1.0, "the bold stroke's top");
        assert_eq!(b.cover(6, 4, 1, 0.0), 1.0, "the crossing");
        assert_eq!(b.cover(6, 0, 1, 0.0), 0.0, "between the strokes");
    }

    #[test]
    fn growing_a_mark_outlines_it() {
        let (_, m) = bold(Glyph::Minimize);
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
