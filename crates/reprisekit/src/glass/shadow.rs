//! A Glass window's shadow: black at a falling alpha around the frame, never lightening what lies beneath. It reaches 13, 12, 18 and 19 px (left, up, right, down) of an active window's frame and 7, 7, 9 and 9 px of an inactive one's.
//! Each side has its own darkness just outside the frame ([`Design`]) and fades out over its reach as the square of the distance left; round the corners the shade follows the curve, turning from one side's reach and darkness to the other's.
//! Each state is eight pieces: four fixed corners and four one-pixel edge profiles stretched along their side; `place` says where each goes around a frame. A maximized window has no shadow.

use std::sync::LazyLock;

use crate::backdrop::corner_radius;
use crate::raster::{Canvas, Insets, Rect};

#[derive(Clone, Debug)]
pub struct Alpha {
    pub w: i32,
    pub h: i32,
    pub a: Vec<u8>,
}

impl Alpha {
    pub fn get(&self, x: i32, y: i32) -> u8 {
        self.a[(y * self.w + x) as usize]
    }
}

pub const TOP_LEFT: usize = 0;
pub const TOP_RIGHT: usize = 1;
pub const BOTTOM_LEFT: usize = 2;
pub const BOTTOM_RIGHT: usize = 3;
pub const TOP: usize = 4;
pub const BOTTOM: usize = 5;
pub const LEFT: usize = 6;
pub const RIGHT: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Design {
    pub margins: Insets,
    pub edge: [f64; 4],
}

pub const ACTIVE_DESIGN: Design = Design {
    margins: Insets::new(13, 12, 18, 19),
    edge: [0.49, 0.45, 0.62, 0.64],
};

pub const INACTIVE_DESIGN: Design = Design {
    margins: Insets::new(7, 7, 9, 9),
    edge: [0.22, 0.22, 0.29, 0.29],
};

const FALLOFF: f64 = 2.0;

/// How far a corner piece runs on along the frame past the frame's
/// corner, so the corner's curve and its outer pixels lie inside it.
pub const CORNER_RUN: i32 = 12;

/// One state's shadow: how far past each side of the frame it reaches,
/// and its pieces at scale 1. A corner covers its margins and runs on
/// over the frame's corner, clear there but for the pixels outside the
/// frame's rounded corner; an edge profile is as thick as its margin.
#[derive(Clone, Debug)]
pub struct Shadow {
    pub margins: Insets,
    pub pieces: [Alpha; 8],
}

impl Shadow {
    fn new(design: Design) -> Shadow {
        Shadow {
            margins: design.margins,
            pieces: std::array::from_fn(|i| {
                let c = draw(&design, i, 1);
                Alpha {
                    w: c.width as i32,
                    h: c.height as i32,
                    a: c.pixels.iter().map(|&p| (p >> 24) as u8).collect(),
                }
            }),
        }
    }
}

pub static ACTIVE: LazyLock<Shadow> = LazyLock::new(|| Shadow::new(ACTIVE_DESIGN));
pub static INACTIVE: LazyLock<Shadow> = LazyLock::new(|| Shadow::new(INACTIVE_DESIGN));

pub fn shadow(active: bool) -> &'static Shadow {
    if active { &ACTIVE } else { &INACTIVE }
}

pub(super) fn design(active: bool) -> &'static Design {
    if active {
        &ACTIVE_DESIGN
    } else {
        &INACTIVE_DESIGN
    }
}

fn size(d: &Design, i: usize) -> (i32, i32) {
    let m = d.margins;
    let (left, top) = (i == TOP_LEFT || i == BOTTOM_LEFT, i < BOTTOM_LEFT);
    let mx = if left { m.left } else { m.right };
    let my = if top { m.top } else { m.bottom };
    match i {
        TOP => (1, m.top),
        BOTTOM => (1, m.bottom),
        LEFT => (m.left, 1),
        RIGHT => (m.right, 1),
        _ => (mx + CORNER_RUN, my + CORNER_RUN),
    }
}
/// Where one piece shows: `dest` in the frame's coordinates, and `src`,
/// the part of the piece drawn into it (stretched when the sizes differ).
/// An empty `dest` shows nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placed {
    pub dest: Rect,
    pub src: Rect,
}

/// Where each piece of the shadow goes around `frame`, in `frame`'s own
/// coordinates: the corners at the frame's corners, the edges stretched
/// between them. On a frame too small for two corners side by side, each
/// corner is cut at the frame's middle, so none darkens a pixel twice.
/// Nothing for a maximized window.
pub fn place(frame: Rect, active: bool, maximized: bool) -> [Placed; 8] {
    let mut out = [Placed::default(); 8];
    if maximized || frame.is_empty() {
        return out;
    }
    let s = shadow(active);
    let m = s.margins;
    let p = &s.pieces;
    let (x0, x1) = (frame.x - m.left, frame.x + frame.width + m.right);
    let (y0, y1) = (frame.y - m.top, frame.y + frame.height + m.bottom);
    let (mx, my) = (frame.x + frame.width / 2, frame.y + frame.height / 2);
    // A corner from `a` (its outer edge) `len` long, cut at `mid`: its
    // span and the piece's first pixel shown.
    let near = |a: i32, len: i32, mid: i32| (a, (a + len).min(mid).max(a), 0);
    let far = |b: i32, len: i32, mid: i32| {
        let start = (b - len).max(mid).min(b);
        (start, b, start - (b - len))
    };
    let corner = |(xa, xb, sx): (i32, i32, i32), (ya, yb, sy): (i32, i32, i32)| Placed {
        dest: Rect::new(xa, ya, xb - xa, yb - ya),
        src: Rect::new(sx, sy, xb - xa, yb - ya),
    };
    let whole = |a: &Alpha| Rect::new(0, 0, a.w, a.h);
    let span = |a: i32, b: i32| (b - a).max(0);

    let tl = corner(near(x0, p[TOP_LEFT].w, mx), near(y0, p[TOP_LEFT].h, my));
    let tr = corner(far(x1, p[TOP_RIGHT].w, mx), near(y0, p[TOP_RIGHT].h, my));
    let bl = corner(
        near(x0, p[BOTTOM_LEFT].w, mx),
        far(y1, p[BOTTOM_LEFT].h, my),
    );
    let br = corner(
        far(x1, p[BOTTOM_RIGHT].w, mx),
        far(y1, p[BOTTOM_RIGHT].h, my),
    );
    out[TOP_LEFT] = tl;
    out[TOP_RIGHT] = tr;
    out[BOTTOM_LEFT] = bl;
    out[BOTTOM_RIGHT] = br;
    let top_x = tl.dest.x + tl.dest.width;
    out[TOP] = Placed {
        dest: Rect::new(top_x, y0, span(top_x, tr.dest.x), m.top),
        src: whole(&p[TOP]),
    };
    let bottom_x = bl.dest.x + bl.dest.width;
    out[BOTTOM] = Placed {
        dest: Rect::new(bottom_x, y1 - m.bottom, span(bottom_x, br.dest.x), m.bottom),
        src: whole(&p[BOTTOM]),
    };
    let left_y = tl.dest.y + tl.dest.height;
    out[LEFT] = Placed {
        dest: Rect::new(x0, left_y, m.left, span(left_y, bl.dest.y)),
        src: whole(&p[LEFT]),
    };
    let right_y = tr.dest.y + tr.dest.height;
    out[RIGHT] = Placed {
        dest: Rect::new(x1 - m.right, right_y, m.right, span(right_y, br.dest.y)),
        src: whole(&p[RIGHT]),
    };
    out
}

/// Piece `i` of the active or inactive shadow as premultiplied ARGB at `scale` device pixels per pixel, a scale-1 canvas of the device size to upload as is. Each pixel is the shade at its middle. A corner is clear where the frame covers a pixel whole (at scale 1 inside the frame's stepped corner [`super::CORNER`], above it inside the round corner [`corner_radius`]), so no unshaded sliver shows.
pub fn piece(active: bool, i: usize, scale: i32) -> Canvas {
    draw(design(active), i, scale)
}

fn draw(d: &Design, i: usize, scale: i32) -> Canvas {
    let s = scale.max(1);
    let (w, h) = size(d, i);
    let m = d.margins;
    let (left, top) = (i == TOP_LEFT || i == BOTTOM_LEFT, i < BOTTOM_LEFT);
    let r = f64::from(corner_radius(super::CORNER));
    let k = f64::from(s);
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    for y in 0..h * s {
        for x in 0..w * s {
            let (px, py) = ((f64::from(x) + 0.5) / k, (f64::from(y) + 0.5) / k);
            let a = match i {
                TOP => shade(d.edge[1], f64::from(m.top), f64::from(m.top) - py),
                BOTTOM => shade(d.edge[3], f64::from(m.bottom), py),
                LEFT => shade(d.edge[0], f64::from(m.left), f64::from(m.left) - px),
                RIGHT => shade(d.edge[2], f64::from(m.right), px),
                _ => {
                    // The pixel turned to the top-left corner's way round,
                    // counted from the frame's corner in device px: (0, 0)
                    // is the frame's corner pixel.
                    let fx = if left {
                        x - m.left * s
                    } else {
                        (w - m.right) * s - 1 - x
                    };
                    let fy = if top {
                        y - m.top * s
                    } else {
                        (h - m.bottom) * s - 1 - y
                    };
                    if under_frame(fx, fy, s, r) {
                        0.0
                    } else {
                        let qx = r - (f64::from(fx) + 0.5) / k;
                        let qy = r - (f64::from(fy) + 0.5) / k;
                        let x_side = if left {
                            (d.edge[0], f64::from(m.left))
                        } else {
                            (d.edge[2], f64::from(m.right))
                        };
                        let y_side = if top {
                            (d.edge[1], f64::from(m.top))
                        } else {
                            (d.edge[3], f64::from(m.bottom))
                        };
                        round_shade(qx, qy, r, x_side, y_side)
                    }
                }
            };
            let a = (a * 255.0).round().clamp(0.0, 255.0) as u32;
            c.put_raw(x, y, a << 24);
        }
    }
    c
}

/// Whether the frame covers device pixel (`fx`, `fy`) of its corner
/// whole at scale `s`, counted from the frame's corner pixel: at scale 1
/// every pixel but the stepped corner's outer ones, above it every pixel
/// the round corner of radius `r` design px covers whole.
fn under_frame(fx: i32, fy: i32, s: i32, r: f64) -> bool {
    if fx < 0 || fy < 0 {
        return false;
    }
    if s == 1 {
        let outside = super::CORNER.get(fy as usize).copied().unwrap_or(0);
        return fx >= outside;
    }
    let far = 1.0e5;
    crate::shape::rounded(f64::from(fx), f64::from(fy), far, far, r * f64::from(s)) >= 1.0
}

/// The shade `out` design px out from a side whose first pixel out is
/// `edge` dark and whose shade reaches `reach` px: full up to that first
/// pixel's middle, falling off to nothing at the reach.
pub(super) fn shade(edge: f64, reach: f64, out: f64) -> f64 {
    let left = ((reach - out) / (reach - 0.5)).clamp(0.0, 1.0);
    edge * left.powf(FALLOFF)
}

/// The shade round a corner whose curve has radius `r`, at (`qx`, `qy`)
/// from the curve's centre toward the outside (design px): the sides'
/// (darkness, reach) are `x` and `y`. Off the corner's square it is the
/// side's shade; on it the distance out of the curve, with the reach and
/// darkness turning from one side's to the other's with the direction.
fn round_shade(qx: f64, qy: f64, r: f64, x: (f64, f64), y: (f64, f64)) -> f64 {
    match (qx > 0.0, qy > 0.0) {
        (true, true) => {
            let len = qx.hypot(qy);
            let (cx, cy) = (qx / len, qy / len);
            let reach = x.1 * y.1 / ((y.1 * cx).hypot(x.1 * cy));
            let edge = x.0 * cx * cx + y.0 * cy * cy;
            shade(edge, reach, len - r)
        }
        (true, false) => shade(x.0, x.1, qx - r),
        (false, true) => shade(y.0, y.1, qy - r),
        (false, false) => x.0.max(y.0),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn at(frame: Rect, active: bool, x: i32, y: i32) -> Option<u8> {
        let placed = place(frame, active, false);
        placed.iter().enumerate().find_map(|(i, p)| {
            p.dest.contains(x, y).then(|| {
                let a = &shadow(active).pieces[i];
                // Edges stretch along their side: a 1 px run reads its only pixel.
                let sx = p.src.x + (x - p.dest.x) * p.src.width / p.dest.width;
                let sy = p.src.y + (y - p.dest.y) * p.src.height / p.dest.height;
                a.get(sx, sy)
            })
        })
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    #[test]
    fn an_800_by_500_window_at_400_200_places_its_eight_pieces() {
        let frame = Rect::new(400, 200, 800, 500);
        let p = place(frame, true, false);
        let dest: Vec<Rect> = p.iter().map(|p| p.dest).collect();
        assert_eq!(
            dest,
            vec![
                Rect::new(387, 188, 25, 24),
                Rect::new(1188, 188, 30, 24),
                Rect::new(387, 688, 25, 31),
                Rect::new(1188, 688, 30, 31),
                Rect::new(412, 188, 776, 12),
                Rect::new(412, 700, 776, 19),
                Rect::new(387, 212, 13, 476),
                Rect::new(1200, 212, 18, 476),
            ],
            "corners at the frame's corners, edges between them"
        );
        let whole: Vec<Rect> = ACTIVE
            .pieces
            .iter()
            .map(|a| Rect::new(0, 0, a.w, a.h))
            .collect();
        let src: Vec<Rect> = p.iter().map(|p| p.src).collect();
        assert_eq!(src, whole, "every piece shows whole");
        let inactive = place(frame, false, false);
        assert_eq!(
            inactive[TOP_LEFT].dest,
            Rect::new(393, 193, 19, 19),
            "inactive: 7 px left and up"
        );
        assert_eq!(
            inactive[BOTTOM_RIGHT].dest,
            Rect::new(1188, 688, 21, 21),
            "inactive: 9 px right and down"
        );
        assert_eq!(inactive[TOP].dest, Rect::new(412, 193, 776, 7));
        assert_eq!(inactive[RIGHT].dest, Rect::new(1200, 212, 9, 476));
    }

    #[test]
    fn a_maximized_window_has_no_shadow() {
        let p = place(Rect::new(0, 0, 1920, 1040), true, true);
        assert!(p.iter().all(|p| p.dest.is_empty()), "no piece shows");
    }

    #[test]
    fn the_shadow_at_the_frame_edge_is_as_dark_as_designed() {
        let frame = Rect::new(400, 200, 800, 500);
        let sides = [(800, 199), (800, 700), (399, 450), (1200, 450)];
        for (active, want) in [
            (true, [0.45, 0.64, 0.49, 0.62]),
            (false, [0.22, 0.29, 0.22, 0.29]),
        ] {
            for ((x, y), w) in sides.into_iter().zip(want) {
                let a = at(frame, active, x, y).expect("the edge is covered");
                let got = f64::from(a) / 255.0;
                assert!(
                    (got - w).abs() < 0.006,
                    "active {active} at ({x}, {y}): alpha {w}, got {got:.3}"
                );
            }
        }
    }

    #[test]
    fn each_side_fades_to_nothing_at_its_reach() {
        for active in [true, false] {
            let sh = shadow(active);
            let m = sh.margins;
            let profile = |i: usize| -> Vec<u8> { sh.pieces[i].a.clone() };
            let mut top = profile(TOP);
            top.reverse();
            let mut left = profile(LEFT);
            left.reverse();
            for (what, run, reach) in [
                ("top", top, m.top),
                ("left", left, m.left),
                ("bottom", profile(BOTTOM), m.bottom),
                ("right", profile(RIGHT), m.right),
            ] {
                assert_eq!(run.len(), reach as usize, "{what}: as thick as its margin");
                assert!(
                    run.windows(2).all(|w| w[0] > w[1] || w[1] == 0),
                    "active {active} {what}: falls off, {run:?}"
                );
                assert!(
                    run[run.len() - 1] <= 1,
                    "active {active} {what}: all but gone"
                );
            }
        }
    }

    #[test]
    fn nothing_darkens_the_frame_but_its_corners_outer_pixels() {
        let outside = |fx: i32, fy: i32, w: i32, h: i32| {
            let row = fy.min(h - 1 - fy);
            let col = fx.min(w - 1 - fx);
            [5, 3, 2, 1, 1, 1]
                .get(row as usize)
                .is_some_and(|&n| col < n)
        };
        for active in [true, false] {
            let frame = Rect::new(0, 0, 800, 500);
            for p in place(frame, active, false).iter() {
                for y in p.dest.y..p.dest.y + p.dest.height {
                    for x in p.dest.x..p.dest.x + p.dest.width {
                        if !frame.contains(x, y) {
                            continue;
                        }
                        let a = at(frame, active, x, y).unwrap();
                        if outside(x, y, 800, 500) {
                            assert!(a > 0, "({x}, {y}) outside the corner is shaded");
                        } else {
                            assert_eq!(a, 0, "({x}, {y}) inside the frame is clear");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn each_corner_meets_its_edges_without_a_seam() {
        for active in [true, false] {
            let sh = shadow(active);
            let m = sh.margins;
            let p = &sh.pieces;
            for (i, col, row) in [
                (TOP_LEFT, p[TOP_LEFT].w - 1, p[TOP_LEFT].h - 1),
                (TOP_RIGHT, 0, p[TOP_RIGHT].h - 1),
                (BOTTOM_LEFT, p[BOTTOM_LEFT].w - 1, 0),
                (BOTTOM_RIGHT, 0, 0),
            ] {
                let c = &p[i];
                let (top, left) = (i < BOTTOM_LEFT, i == TOP_LEFT || i == BOTTOM_LEFT);
                let (edge, y0) = if top {
                    (&p[TOP], 0)
                } else {
                    (&p[BOTTOM], c.h - m.bottom)
                };
                for y in 0..edge.h {
                    assert_eq!(c.get(col, y0 + y), edge.get(0, y), "{i} column, row {y}");
                }
                let (side, x0) = if left {
                    (&p[LEFT], 0)
                } else {
                    (&p[RIGHT], c.w - m.right)
                };
                for x in 0..side.w {
                    assert_eq!(c.get(x0 + x, row), side.get(x, 0), "{i} row, column {x}");
                }
            }
        }
    }

    #[test]
    fn a_corner_at_scale_2_shades_all_the_round_frame_leaves_open() {
        let r = 14.0;
        for active in [true, false] {
            let sh = shadow(active);
            let m = sh.margins;
            for i in [TOP_LEFT, TOP_RIGHT, BOTTOM_LEFT, BOTTOM_RIGHT] {
                let a = &sh.pieces[i];
                let c = piece(active, i, 2);
                assert_eq!((c.width, c.height), (2 * a.w as u32, 2 * a.h as u32));
                let (left, top) = (i == TOP_LEFT || i == BOTTOM_LEFT, i < BOTTOM_LEFT);
                for y in 0..2 * a.h {
                    for x in 0..2 * a.w {
                        let fx = if left {
                            x - 2 * m.left
                        } else {
                            2 * (a.w - m.right) - 1 - x
                        };
                        let fy = if top {
                            y - 2 * m.top
                        } else {
                            2 * (a.h - m.bottom) - 1 - y
                        };
                        if !(0..14).contains(&fx) || !(0..14).contains(&fy) {
                            continue;
                        }
                        let got = c.get(x, y) >> 24;
                        let what = format!("active {active}, piece {i} ({x}, {y})");
                        let covered =
                            crate::shape::rounded(f64::from(fx), f64::from(fy), 1.0e5, 1.0e5, r);
                        if covered >= 1.0 {
                            assert_eq!(got, 0, "{what}: clear under the frame");
                        } else {
                            assert!(got > 0, "{what}: shaded where the frame leaves it open");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_piece_at_scale_2_is_drawn_at_scale_2() {
        for active in [true, false] {
            for i in 0..8 {
                let one = piece(active, i, 1);
                let two = piece(active, i, 2);
                assert_eq!((two.width, two.height), (2 * one.width, 2 * one.height));
                assert!(two.pixels.iter().all(|p| p & 0x00FF_FFFF == 0), "black");
                let steps = (0..two.height as i32 / 2)
                    .flat_map(|y| (0..two.width as i32 / 2).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let block = [(0, 0), (1, 0), (0, 1), (1, 1)]
                            .map(|(dx, dy)| two.get(2 * x + dx, 2 * y + dy));
                        block.iter().any(|&p| p != block[0])
                    })
                    .count();
                assert!(
                    steps > 0,
                    "active {active} piece {i}: shaded within a pixel"
                );
            }
        }
        let top = piece(true, TOP, 2);
        assert_eq!((top.width, top.height), (2, 24));
        assert_eq!(top.get(0, 23), top.get(1, 23), "an edge is one shade along");
    }

    #[test]
    fn the_shadow_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for active in [true, false] {
            for scale in [1, 2] {
                got.push(
                    (0..8)
                        .map(|i| fnv(&piece(active, i, scale)))
                        .collect::<Vec<_>>(),
                );
            }
        }
        #[rustfmt::skip]
        let want: Vec<Vec<u64>> = vec![
            vec![0xa9c0b46d56662aa7, 0x97b076074191b7b1, 0x0ac03dd1a1f2949f, 0x3111ce6cf8ae3850, 0x6bb46ec54e50dcce, 0xd45ee115212744db, 0x270e9accfaeccd7c, 0x37f2dc00909d30c7],
            vec![0x049d3ab81a36ab0a, 0x83ccbc068a1dbc9e, 0xc87b31ca6bb338d6, 0xefc0184907ceb84f, 0x5eb7081cc154ba3f, 0xe23b0d625c4df541, 0x67978fc3733df3fd, 0xaa6b04b211aca3c3],
            vec![0x5e0f0ca29d61b9bb, 0x4e7cddd2ade754ac, 0x54597c73b3888ffc, 0x019c94d00e4716bd, 0xa148843709d3e9e3, 0x9d8cbee7104f87c5, 0x29733ea7f87ee643, 0x68d5e521db213c45],
            vec![0x1b7ea30f5821d477, 0x7b191739fb392ee7, 0x38105a483f36b1d7, 0xd36dfe1e5956b36a, 0xf78e09df9d092729, 0x12d2c61e7771ec45, 0xcd2ad6502a6943a9, 0x374d75c3f6f51275],
        ];
        assert_eq!(got, want, "the shadow's pieces");
    }
}
