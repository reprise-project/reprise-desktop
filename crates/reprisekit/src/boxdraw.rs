//! Box-drawing and block-element characters (U+2500..U+259F) drawn from geometry as coverage
//! masks filling a whole cell, so lines in neighbouring cells meet without a gap.

use crate::shape;

/// Whether [`mask`] draws `ch`.
pub fn covers(ch: char) -> bool {
    ('\u{2500}'..='\u{259F}').contains(&ch)
}

/// A light line's width in device px for a cell `h` device px tall at `scale`: whole design
/// pixels, so scale 2 doubles it.
pub fn light(h: i32, scale: i32) -> i32 {
    let s = scale.max(1);
    s * (h / s / 16).max(1)
}

/// `ch`'s coverage (0..=255) over a `w` x `h` device-pixel cell at `scale`, row by row; `None`
/// outside U+2500..U+259F.
pub fn mask(ch: char, w: i32, h: i32, scale: i32) -> Option<Vec<u8>> {
    if !covers(ch) || w <= 0 || h <= 0 {
        return None;
    }
    let mut m = Mask {
        w,
        h,
        px: vec![0; (w * h) as usize],
    };
    let c = ch as u32;
    let light = light(h, scale);
    match c {
        0x2580..=0x259F => block(&mut m, c),
        0x256D..=0x2570 => arc(&mut m, c, light),
        0x2571..=0x2573 => diagonal(&mut m, c, light),
        0x2504..=0x250B | 0x254C..=0x254F => dashes(&mut m, c, light),
        _ => lines(&mut m, arms(c)?, light),
    }
    Some(m.px)
}

struct Mask {
    w: i32,
    h: i32,
    px: Vec<u8>,
}

impl Mask {
    /// Cover x0..x1 by y0..y1 fully, cut to the cell.
    fn fill(&mut self, (x0, x1): (i32, i32), (y0, y1): (i32, i32), cov: u8) {
        let (x0, x1) = (x0.clamp(0, self.w), x1.clamp(0, self.w));
        for y in y0.clamp(0, self.h)..y1.clamp(0, self.h) {
            let row = (y * self.w) as usize;
            self.px[row + x0 as usize..row + x1.max(x0) as usize].fill(cov);
        }
    }

    fn paint(&mut self, cover: impl Fn(f64, f64) -> f64) {
        for y in 0..self.h {
            for x in 0..self.w {
                let k = cover(f64::from(x), f64::from(y)).clamp(0.0, 1.0);
                let p = &mut self.px[(y * self.w + x) as usize];
                *p = (*p).max((k * 255.0).round() as u8);
            }
        }
    }
}

/// An arm's weight.
#[derive(Clone, Copy, PartialEq, Eq)]
enum W {
    None,
    Light,
    Heavy,
    Double,
}

/// Up, right, down and left arm weights for U+2500..U+257F, `0` none, `1` light, `2` heavy,
/// `3` double; `----` is drawn some other way (dashes, arcs, diagonals).
const ARMS: &str = concat!(
    "0101,0202,1010,2020,----,----,----,----,",
    "----,----,----,----,0110,0210,0120,0220,",
    "0011,0012,0021,0022,1100,1200,2100,2200,",
    "1001,1002,2001,2002,1110,1210,2110,1120,",
    "2120,2210,1220,2220,1011,1012,2011,1021,",
    "2021,2012,1022,2022,0111,0112,0211,0212,",
    "0121,0122,0221,0222,1101,1102,1201,1202,",
    "2101,2102,2201,2202,1111,1112,1211,1212,",
    "2111,1121,2121,2112,2211,1122,1221,2212,",
    "1222,2122,2221,2222,----,----,----,----,",
    "0303,3030,0310,0130,0330,0013,0031,0033,",
    "1300,3100,3300,1003,3001,3003,1310,3130,",
    "3330,1013,3031,3033,0313,0131,0333,1303,",
    "3101,3303,1313,3131,3333,----,----,----,",
    "----,----,----,----,0001,1000,0100,0010,",
    "0002,2000,0200,0020,0201,1020,0102,2010,",
);

fn arms(c: u32) -> Option<[W; 4]> {
    let i = c.checked_sub(0x2500)? as usize * 5;
    let entry = ARMS.as_bytes().get(i..i + 4)?;
    let mut out = [W::None; 4];
    for (o, &b) in out.iter_mut().zip(entry) {
        *o = match b {
            b'0' => W::None,
            b'1' => W::Light,
            b'2' => W::Heavy,
            b'3' => W::Double,
            _ => return None,
        };
    }
    Some(out)
}

/// The `t`-wide span centred in `len`.
fn span(len: i32, t: i32) -> (i32, i32) {
    let s = (len - t) / 2;
    (s, s + t)
}

/// Lines from the cell's middle to its edges; a double arm is two light strokes a light stroke
/// apart, whose inner strokes stop at another double arm's near stroke and outer ones at its far.
fn lines(m: &mut Mask, [up, right, down, left]: [W; 4], lt: i32) {
    let ht = lt * 2;
    let thick = |a: W| match a {
        W::None => 0,
        W::Light | W::Double => lt,
        W::Heavy => ht,
    };
    let core = |len: i32, a: W, b: W, across: (W, W)| {
        if a == W::Double || b == W::Double {
            span(len, 3 * lt)
        } else {
            let t = thick(a).max(thick(b));
            span(
                len,
                if t > 0 {
                    t
                } else {
                    thick(across.0).max(thick(across.1))
                },
            )
        }
    };
    let vx = core(m.w, up, down, (left, right));
    let hy = core(m.h, left, right, (up, down));
    let both = |a: W, b: W| a == W::Double && b == W::Double;
    let (w, h) = (m.w, m.h);

    for (arm, toward_end) in [(left, true), (right, false)] {
        let ends = |near_double: bool| {
            if toward_end {
                (0, if near_double { vx.0 + lt } else { vx.1 })
            } else {
                (if near_double { vx.1 - lt } else { vx.0 }, w)
            }
        };
        match arm {
            W::None => {}
            W::Double => {
                let (y0, y1) = span(h, 3 * lt);
                m.fill(ends(up == W::Double), (y0, y0 + lt), 255);
                m.fill(ends(down == W::Double), (y1 - lt, y1), 255);
            }
            _ => m.fill(ends(both(up, down)), span(h, thick(arm)), 255),
        }
    }
    for (arm, toward_end) in [(up, true), (down, false)] {
        let ends = |near_double: bool| {
            if toward_end {
                (0, if near_double { hy.0 + lt } else { hy.1 })
            } else {
                (if near_double { hy.1 - lt } else { hy.0 }, h)
            }
        };
        match arm {
            W::None => {}
            W::Double => {
                let (x0, x1) = span(w, 3 * lt);
                m.fill((x0, x0 + lt), ends(left == W::Double), 255);
                m.fill((x1 - lt, x1), ends(right == W::Double), 255);
            }
            _ => m.fill(span(w, thick(arm)), ends(both(left, right)), 255),
        }
    }
}

/// Dashed lines: U+2504..U+250B (three and four dashes) and U+254C..U+254F (two).
fn dashes(m: &mut Mask, c: u32, lt: i32) {
    let (n, i) = match c {
        0x2504..=0x2507 => (3, c - 0x2504),
        0x2508..=0x250B => (4, c - 0x2508),
        _ => (2, c - 0x254C),
    };
    let t = if i % 2 == 1 { lt * 2 } else { lt };
    let across = i >= 2;
    let len = if across { m.h } else { m.w };
    for k in 0..n {
        let a = len * k / n;
        let b = len * (k + 1) / n;
        let dash = (a, a + (b - a + 1) / 2);
        if across {
            m.fill(span(m.w, t), dash, 255);
        } else {
            m.fill(dash, span(m.h, t), 255);
        }
    }
}

/// Rounded corners U+256D..U+2570: a quarter circle joining the light arms.
fn arc(m: &mut Mask, c: u32, lt: i32) {
    let (sx, sy) = match c {
        0x256D => (1.0, 1.0),
        0x256E => (-1.0, 1.0),
        0x256F => (-1.0, -1.0),
        _ => (1.0, -1.0),
    };
    let (w, h) = (f64::from(m.w), f64::from(m.h));
    let (vx, hy) = (span(m.w, lt), span(m.h, lt));
    let cx = f64::from(vx.0 + vx.1) / 2.0;
    let cy = f64::from(hy.0 + hy.1) / 2.0;
    let rx = if sx > 0.0 { w - cx } else { cx };
    let ry = if sy > 0.0 { h - cy } else { cy };
    let lt = f64::from(lt);
    // Short of the edge so the arm leaves the cell straight and meets its neighbour's line.
    let r = (rx.min(ry) - lt).max(1.0);
    let (ox, oy) = (cx + sx * r, cy + sy * r);
    let edge_y = if sy > 0.0 { h + 1.0 } else { -1.0 };
    let edge_x = if sx > 0.0 { w + 1.0 } else { -1.0 };
    let mut pts = vec![(cx, edge_y)];
    pts.extend((0..=16).map(|i| {
        let t = f64::from(i) / 16.0 * std::f64::consts::FRAC_PI_2;
        (ox - sx * r * t.cos(), oy - sy * r * t.sin())
    }));
    pts.push((edge_x, cy));
    m.paint(|x, y| shape::stroke(x, y, &pts, lt));
}

/// Diagonals U+2571..U+2573, corner to corner.
fn diagonal(m: &mut Mask, c: u32, lt: i32) {
    let (w, h) = (f64::from(m.w), f64::from(m.h));
    let lt = f64::from(lt);
    let rising = [(w + 1.0, -1.0 - h / w), (-1.0, h + 1.0 + h / w)];
    let falling = [(-1.0, -1.0 - h / w), (w + 1.0, h + 1.0 + h / w)];
    let lines: &[[(f64, f64); 2]] = match c {
        0x2571 => &[rising],
        0x2572 => &[falling],
        _ => &[rising, falling],
    };
    m.paint(|x, y| {
        lines
            .iter()
            .map(|l| shape::stroke(x, y, l, lt))
            .fold(0.0, f64::max)
    });
}

/// Block elements U+2580..U+259F: eighths, halves, quadrants and shades.
fn block(m: &mut Mask, c: u32) {
    let (w, h) = (m.w, m.h);
    let eighth = |len: i32, k: i32| (len * k + 4) / 8;
    let (mx, my) = (eighth(w, 4), h - eighth(h, 4));
    let full = (0, w);
    match c {
        0x2580 => m.fill(full, (0, my), 255),
        0x2581..=0x2588 => m.fill(full, (h - eighth(h, (c - 0x2580) as i32), h), 255),
        0x2589..=0x258F => m.fill((0, eighth(w, (0x2590 - c) as i32)), (0, h), 255),
        0x2590 => m.fill((mx, w), (0, h), 255),
        0x2591..=0x2593 => m.fill(full, (0, h), [64, 128, 191][(c - 0x2591) as usize]),
        0x2594 => m.fill(full, (0, eighth(h, 1)), 255),
        0x2595 => m.fill((w - eighth(w, 1), w), (0, h), 255),
        _ => {
            let (ul, ur, ll, lr) = match c {
                0x2596 => (0, 0, 1, 0),
                0x2597 => (0, 0, 0, 1),
                0x2598 => (1, 0, 0, 0),
                0x2599 => (1, 0, 1, 1),
                0x259A => (1, 0, 0, 1),
                0x259B => (1, 1, 1, 0),
                0x259C => (1, 1, 0, 1),
                0x259D => (0, 1, 0, 0),
                0x259E => (0, 1, 1, 0),
                _ => (0, 1, 1, 1),
            };
            for (on, xs, ys) in [
                (ul, (0, mx), (0, my)),
                (ur, (mx, w), (0, my)),
                (ll, (0, mx), (my, h)),
                (lr, (mx, w), (my, h)),
            ] {
                if on == 1 {
                    m.fill(xs, ys, 255);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(m: &[u8], w: i32, x: i32, y: i32) -> u8 {
        m[(y * w + x) as usize]
    }

    #[test]
    fn every_char_in_the_range_draws_ink() {
        for c in 0x2500..=0x259F {
            let ch = char::from_u32(c).unwrap();
            for (w, h) in [(7, 15), (8, 16), (16, 32)] {
                let m = mask(ch, w, h, 1).unwrap();
                assert_eq!(m.len(), (w * h) as usize);
                assert!(m.iter().any(|&p| p > 0), "U+{c:04X} at {w}x{h} draws ink");
            }
        }
        assert!(mask('A', 8, 16, 1).is_none(), "A is left to the font");
    }

    #[test]
    fn light_horizontal_runs_edge_to_edge_on_one_row() {
        let m = mask('─', 8, 16, 1).unwrap();
        let rows: Vec<i32> = (0..16)
            .filter(|&y| (0..8).all(|x| at(&m, 8, x, y) == 255))
            .collect();
        assert_eq!(rows, vec![7], "one full row through the middle");
        assert_eq!(m.iter().filter(|&&p| p > 0).count(), 8, "and nothing else");
    }

    #[test]
    fn double_corner_has_outer_and_inner_strokes() {
        let m = mask('╔', 9, 17, 1).unwrap();
        let (x0, _) = span(9, 3);
        let (y0, _) = span(17, 3);
        assert_eq!(at(&m, 9, x0, y0), 255, "outer corner");
        assert_eq!(at(&m, 9, x0 + 1, y0 + 1), 0, "gap between the strokes");
        assert_eq!(at(&m, 9, x0 + 2, y0 + 2), 255, "inner corner");
        assert_eq!(
            at(&m, 9, x0 + 1, y0 + 3),
            0,
            "the inner stroke does not cross the gap"
        );
    }

    #[test]
    fn lower_and_upper_halves_tile_the_cell() {
        for h in [15, 16, 17, 32] {
            let up = mask('▀', 8, h, 1).unwrap();
            let down = mask('▄', 8, h, 1).unwrap();
            assert!(
                up.iter()
                    .zip(&down)
                    .all(|(a, b)| a.max(b) == &255 && a.min(b) == &0),
                "every pixel in exactly one half at height {h}"
            );
        }
    }

    #[test]
    fn arcs_join_the_straight_lines() {
        let (w, h) = (16, 32);
        let arc = mask('╭', w, h, 2).unwrap();
        let line = mask('─', w, h, 2).unwrap();
        let bar = mask('│', w, h, 2).unwrap();
        let right = w - 1;
        assert!((0..h).all(|y| at(&arc, w, right, y) == at(&line, w, right, y)));
        assert!((0..w).all(|x| at(&arc, w, x, h - 1) == at(&bar, w, x, h - 1)));
        assert!(
            arc.iter().any(|&p| p > 0 && p < 255),
            "the curve is smoothed"
        );
    }
}
