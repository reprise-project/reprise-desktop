//! The analog clock on the Date and Time sheet: twelve hour dots on an ellipse and three hands,
//! drawn straight onto the page's face.

use crate::paint::Paint;
use crate::raster::{Canvas, Rect};
use crate::theme::Color;

const DOT: Color = Color::rgb(0, 255, 255);
const DOT_SHADE: Color = Color::rgb(0, 0, 0);
const HAND: Color = Color::rgb(0, 128, 128);
const INSET: i32 = 4;
const HOUR_LEN: f64 = 0.6;
const MINUTE_LEN: f64 = 0.78;
const SECOND_LEN: f64 = 0.8;
/// The hour and minute hands' widths at their base, 100% px: they grow with the face.
const HOUR_W: i32 = 10;
const MINUTE_W: i32 = 8;

/// What the clock shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Face {
    pub h: u32,
    pub m: u32,
    pub s: u32,
}

fn geometry(r: Rect) -> (i32, i32, i32, i32) {
    (
        r.x + r.width / 2,
        r.y + r.height / 2,
        r.width / 2 - INSET,
        r.height / 2 - INSET,
    )
}

/// The top-left of hour dot `k` (0 at twelve, clockwise).
pub fn dot(r: Rect, k: u32) -> (i32, i32) {
    let (cx, cy, rx, ry) = geometry(r);
    let a = f64::from(k) * std::f64::consts::PI / 6.0;
    (
        cx + (f64::from(rx) * a.sin()).round() as i32,
        cy - (f64::from(ry) * a.cos()).round() as i32,
    )
}

/// `angle` in radians clockwise from twelve; `len` a fraction of the ellipse.
fn reach(r: Rect, angle: f64, len: f64) -> (f64, f64) {
    let (cx, cy, rx, ry) = geometry(r);
    (
        f64::from(cx) + f64::from(rx) * len * angle.sin(),
        f64::from(cy) - f64::from(ry) * len * angle.cos(),
    )
}

/// Radians clockwise from twelve.
fn angles(f: Face) -> (f64, f64, f64) {
    let tau = std::f64::consts::TAU;
    let s = f64::from(f.s % 60);
    let m = f64::from(f.m % 60) + s / 60.0;
    let h = f64::from(f.h % 12) + m / 60.0;
    (h / 12.0 * tau, m / 60.0 * tau, s / 60.0 * tau)
}

/// Draw the clock in `r` (logical px) at `p`'s scale. The hands widen with `p.text`; the hour
/// dots and the hands' bevel keep their px.
pub fn draw(c: &mut Canvas, p: &Paint, r: Rect, face: Face) {
    let Paint { cs, s, .. } = *p;
    for k in 0..12 {
        let (x, y) = dot(r, k);
        c.fill_rect(Rect::new(x, y, 1, 2).scaled(s), DOT);
        c.fill_rect(Rect::new(x + 1, y, 1, 2).scaled(s), DOT_SHADE);
    }
    let (ha, ma, sa) = angles(face);
    let (cx, cy, _, _) = geometry(r);
    let centre = (f64::from(cx), f64::from(cy));
    let half = |w: i32| f64::from(p.text.len(w)) / 2.0;
    for (angle, len, half) in [
        (ha, HOUR_LEN, half(HOUR_W)),
        (ma, MINUTE_LEN, half(MINUTE_W)),
    ] {
        let tip = reach(r, angle, len);
        let (dx, dy) = (angle.cos() * half, angle.sin() * half);
        let kite = [
            (centre.0 - dx, centre.1 - dy),
            tip,
            (centre.0 + dx, centre.1 + dy),
        ];
        for (off, ink) in [
            (-1.0, cs.button_hilight),
            (2.0, cs.button_shadow),
            (0.0, HAND),
        ] {
            let moved: Vec<(f64, f64)> = kite.iter().map(|&(x, y)| (x + off, y + off)).collect();
            fill_polygon(c, &moved, ink, s);
        }
    }
    let tip = reach(r, sa, SECOND_LEN);
    invert_line(c, (cx, cy), (tip.0.round() as i32, tip.1.round() as i32), s);
}

fn fill_polygon(c: &mut Canvas, pts: &[(f64, f64)], ink: Color, s: i32) {
    let top = pts.iter().map(|p| p.1).fold(f64::MAX, f64::min).floor() as i32;
    let bottom = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max).ceil() as i32;
    for y in top..=bottom {
        let yc = f64::from(y) + 0.5;
        let mut xs: Vec<f64> = Vec::new();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            if (a.1 <= yc) != (b.1 <= yc) {
                xs.push(a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks(2) {
            if let [x0, x1] = pair {
                let (x0, x1) = ((x0 - 0.5).ceil() as i32, (x1 - 0.5).floor() as i32);
                if x1 >= x0 {
                    c.fill_rect(Rect::new(x0, y, x1 - x0 + 1, 1).scaled(s), ink);
                }
            }
        }
    }
}

fn invert_line(c: &mut Canvas, a: (i32, i32), b: (i32, i32), s: i32) {
    let (mut x, mut y) = a;
    let (dx, dy) = ((b.0 - x).abs(), -(b.1 - y).abs());
    let (sx, sy) = (if x < b.0 { 1 } else { -1 }, if y < b.1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        for py in y * s..(y + 1) * s {
            for px in x * s..(x + 1) * s {
                let v = c.get(px, py);
                c.put_raw(px, py, 0xFF00_0000 | (!v & 0x00FF_FFFF));
            }
        }
        if (x, y) == b {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::Theme;

    const R: Rect = Rect {
        x: 225,
        y: 68,
        width: 132,
        height: 100,
    };

    #[test]
    fn the_dots_sit_on_the_ellipse() {
        assert_eq!(dot(R, 0), (291, 72));
        assert_eq!(dot(R, 1), (322, 78));
        assert_eq!(dot(R, 2), (345, 95));
        assert_eq!(dot(R, 3), (353, 118));
        assert_eq!(dot(R, 6), (291, 164));
        assert_eq!(dot(R, 9), (229, 118));
    }

    #[test]
    fn draws_dots_and_hands() {
        let cs = Theme::default().colors;
        let mut c = Canvas::new(400, 200);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &classic_paint(&cs, 1), R, Face { h: 3, m: 0, s: 0 });
        assert_eq!(c.get(291, 72), DOT.argb());
        assert_eq!(c.get(292, 73), DOT_SHADE.argb());
        assert_eq!(c.get(310, 118), HAND.argb());
        assert_eq!(c.get(290, 90), HAND.argb());
        assert_eq!(c.get(289, 90), cs.button_hilight.argb());
        let mut c = Canvas::new(400, 200);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &classic_paint(&cs, 1),
            R,
            Face { h: 3, m: 0, s: 30 },
        );
        assert_eq!(c.get(291, 150), cs.button_face.inverted().argb());
    }
}
