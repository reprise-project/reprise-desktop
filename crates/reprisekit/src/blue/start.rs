//! Reprise Blue's Start pill, drawn from its scheme's faces at the
//! output's scale: a bar flat on the left, its right end rounded over an
//! elliptical curve, a 1 px rim round the top, the end and the bottom, and
//! a face shaded down by the face's `down` gradient and across by its
//! `across` gradient.

use crate::blue::scheme::{Face, Grad, Metrics, Scheme};
use crate::raster::{Canvas, Insets};
use crate::shape::{self, CLEAR, Px, Stops};
use crate::style::{Layout, PartImage, State};

/// The pill's rounded right end on a 20 px right cap: how far each corner's curve reaches in from the right and down from the top (design px). It runs the whole height, so the end reads as a half round.
const END_NARROW: (f64, f64) = (5.5, 15.0);

/// The rounded right end on a 22 px right cap: a little wider, with a
/// short straight run in the middle.
const END_WIDE: (f64, f64) = (6.0, 12.0);

fn end(m: &Metrics) -> (f64, f64) {
    if m.start_right > 20 {
        END_WIDE
    } else {
        END_NARROW
    }
}

const STATES: [State; 3] = [State::Normal, State::Hot, State::Pressed];

/// The taskbar's Start pill in `s` at `d` device px per design px: one
/// [`Metrics::start_width`] x `taskbar_height` frame per state (normal,
/// hot, pressed). Its caps keep their shape and the middle, where the
/// face varies only down the pill, stretches to fit a wider or narrower
/// "start".
pub fn start_button(s: &Scheme, d: i32) -> PartImage {
    let m = s.metrics;
    let (w, h) = (m.start_width(), m.taskbar_height);
    let faces = [
        &s.start_button.normal,
        &s.start_button.hot,
        &s.start_button.pressed,
    ];
    let mut a = Canvas::new((w * d) as u32, (h * d) as u32 * faces.len() as u32);
    for (i, f) in faces.into_iter().enumerate() {
        a.blit_over(0, i as i32 * h * d, &frame(f, &m, d));
    }
    PartImage::new(a, &STATES, Layout::Vertical)
        .sizing(Insets::new(m.start_left, 0, m.start_right, 0))
        .at_scale(d)
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

/// One state's frame: the rim where the outline and the face inset 1 px
/// from the top, the end and the bottom part, the face inside.
fn frame(f: &Face, m: &Metrics, d: i32) -> Canvas {
    let (w, h) = (m.start_width(), m.taskbar_height);
    let (rx, ry) = end(m);
    let k = f64::from(d);
    let (down, across) = (stops(f.down), stops(f.across));
    let (down, across) = (Stops(&down), Stops(&across));
    // The face's colour across the stretchable middle: `across` shades the
    // face relative to it, so the middle stays `down` alone.
    let mid = f64::from(m.start_left + m.start_middle / 2) + 0.5;
    let level = across.at(mid / f64::from(w));
    let (left, right) = (
        f64::from(m.start_left),
        f64::from(m.start_left + m.start_middle),
    );
    let rim = shape::premul(f.rim, 255);
    let (fw, fh) = (f64::from(w) * k, f64::from(h) * k);
    shape::draw(w, h, d, |x, y| {
        let outer = pill(x, y, fw, fh, rx * k, ry * k);
        if outer == 0.0 {
            return CLEAR;
        }
        let inner = pill(
            x,
            y - k,
            fw - k,
            fh - 2.0 * k,
            (rx - 1.0) * k,
            (ry - 1.0) * k,
        );
        let cx = shape::centre(x as i32, d) + 0.5;
        let cy = shape::centre(y as i32, d) + 0.5;
        // The caps take `across` up to the column inside the rim (the rim
        // draws the end itself); the middle takes none of it.
        let side = if (left..=right).contains(&cx) {
            level
        } else {
            across.at(cx.min(f64::from(w) - 1.5) / f64::from(w))
        };
        let fill = shade(down.at(cy / f64::from(h)), side, level);
        shape::shares(&[(rim, outer - inner), (fill, inner)])
    })
}

fn shade(base: Px, across: Px, level: Px) -> Px {
    (
        255.0,
        std::array::from_fn(|i| (base.1[i] + across.1[i] - level.1[i]).clamp(0.0, 255.0)),
    )
}

/// Coverage of the device pixel at (`x`, `y`) by a `w` x `h` bar at the
/// origin, flat on the left, its two right corners rounded over ellipses
/// `rx` across and `ry` down (all device px). A pixel with all four
/// corners inside is whole (the bar is convex); any other is sampled 8x8.
fn pill(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> f64 {
    if x >= w || y >= h || x + 1.0 <= 0.0 || y + 1.0 <= 0.0 {
        return 0.0;
    }
    let inside = |px: f64, py: f64| {
        if px < 0.0 || py < 0.0 || px > w || py > h {
            return false;
        }
        let ex = px - (w - rx);
        let ey = if py < ry {
            ry - py
        } else if py > h - ry {
            py - (h - ry)
        } else {
            0.0
        };
        ex <= 0.0 || ey <= 0.0 || (ex / rx).powi(2) + (ey / ry).powi(2) <= 1.0
    };
    if [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        .iter()
        .all(|&(dx, dy)| inside(x + dx, y + dy))
    {
        return 1.0;
    }
    let mut n = 0;
    for j in 0..8 {
        for i in 0..8 {
            if inside(
                x + (f64::from(i) + 0.5) / 8.0,
                y + (f64::from(j) + 0.5) / 8.0,
            ) {
                n += 1;
            }
        }
    }
    f64::from(n) / 64.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::Variant;
    use crate::raster::Rect;

    fn fnv(w: i32, h: i32, px: &[u32]) -> u64 {
        let mut x = 0xCBF2_9CE4_8422_2325_u64;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                x = (x ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
            }
        };
        eat(w as u32);
        eat(h as u32);
        px.iter().for_each(|&p| eat(p));
        x
    }

    fn frame_of(img: &PartImage, state: State) -> (i32, i32, Vec<u32>) {
        let r: Rect = img.frame(state);
        let c = img.atlas.copy_rect(r);
        (r.width, r.height, c.pixels)
    }

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn start_pill_is_its_metrics_wide_and_30_tall_at_every_scale() {
        for v in Variant::ALL {
            let m = v.scheme().metrics;
            let one = start_button(v.scheme(), 1);
            let two = start_button(v.scheme(), 2);
            let want = if m.start_right == 20 { 97 } else { 99 };
            assert_eq!(m.start_width(), want, "{v:?} pill width");
            for st in STATES {
                let f = one.frame(st);
                assert_eq!((f.width, f.height), (want, 30), "{v:?} {st:?} at 1");
                let f = two.frame(st);
                assert_eq!((f.width, f.height), (2 * want, 60), "{v:?} {st:?} at 2");
            }
            assert_eq!(one.sizing, Insets::new(10, 0, m.start_right, 0), "{v:?}");
            assert_eq!(two.sizing, one.sizing, "{v:?}: sizing stays in design px");
        }
    }

    #[test]
    fn start_pill_states_differ_and_its_end_is_round() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let img = start_button(v.scheme(), d);
                let frames: Vec<_> = STATES.iter().map(|&s| frame_of(&img, s)).collect();
                assert_ne!(frames[0].2, frames[1].2, "{v:?} d={d}: hot differs");
                assert_ne!(frames[0].2, frames[2].2, "{v:?} d={d}: pressed differs");
                assert_ne!(frames[1].2, frames[2].2, "{v:?} d={d}: hot and pressed");
                for (st, (w, h, px)) in STATES.iter().zip(&frames) {
                    let at = |x: i32, y: i32| px[(y * w + x) as usize];
                    assert_eq!(alpha(at(w - 1, 0)), 0, "{v:?} {st:?} top-right clear");
                    assert_eq!(
                        alpha(at(w - 1, h - 1)),
                        0,
                        "{v:?} {st:?} bottom-right clear"
                    );
                    for y in 0..*h {
                        assert_eq!(alpha(at(0, y)), 255, "{v:?} {st:?} left edge row {y}");
                    }
                    assert_eq!(alpha(at(w - 1, h / 2)), 255, "{v:?} {st:?} end's middle");
                    if d == 2 {
                        let cap = (w - 2 * v.scheme().metrics.start_right)..*w;
                        let soft = (0..*h)
                            .flat_map(|y| cap.clone().map(move |x| (x, y)))
                            .filter(|&(x, y)| (1..255).contains(&alpha(at(x, y))))
                            .count();
                        assert!(soft >= 8, "{v:?} {st:?}: the end is smooth at 2 ({soft})");
                    }
                }
            }
        }
    }

    #[test]
    fn start_pill_middle_stretches_cleanly() {
        for v in Variant::ALL {
            let m = v.scheme().metrics;
            for d in [1, 2] {
                let img = start_button(v.scheme(), d);
                for st in STATES {
                    let (w, h, px) = frame_of(&img, st);
                    for y in 0..h {
                        let row = &px[(y * w) as usize..((y + 1) * w) as usize];
                        let first = row[(m.start_left * d) as usize];
                        for x in m.start_left * d..(m.start_left + m.start_middle) * d {
                            assert_eq!(row[x as usize], first, "{v:?} {st:?} d={d} ({x}, {y})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn start_pill_rim_leaves_no_dark_fringe_on_its_end() {
        let lum = |r: u32, g: u32, b: u32| r + g + b;
        for v in Variant::ALL {
            for (st, face) in STATES.iter().zip([
                &v.scheme().start_button.normal,
                &v.scheme().start_button.hot,
                &v.scheme().start_button.pressed,
            ]) {
                let rim = lum(
                    u32::from(face.rim.r),
                    u32::from(face.rim.g),
                    u32::from(face.rim.b),
                );
                let (w, h, px) = frame_of(&start_button(v.scheme(), 2), *st);
                let ch = |p: u32, s: u32| (p >> s) & 0xFF;
                let darkest = px
                    .iter()
                    .filter(|&&p| alpha(p) == 255)
                    .map(|&p| lum(ch(p, 16), ch(p, 8), ch(p, 0)))
                    .min()
                    .unwrap_or(0)
                    .min(rim);
                for y in 0..h {
                    let row = &px[(y * w) as usize..((y + 1) * w) as usize];
                    for (x, &p) in row.iter().enumerate() {
                        let a = alpha(p);
                        if a == 0 || a == 255 {
                            continue;
                        }
                        // Packing rounds each channel by up to half a
                        // level, which unmultiplying scales by 255 / a.
                        let slack = 3 * 255 / (2 * a) + 3;
                        let un = lum(ch(p, 16), ch(p, 8), ch(p, 0)) * 255 / a;
                        assert!(
                            un + slack >= darkest,
                            "{v:?} {st:?} ({x}, {y}): a soft edge pixel darker than rim and face"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn start_pill_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in Variant::ALL {
            for d in [1, 2] {
                let img = start_button(v.scheme(), d);
                for st in STATES {
                    let (w, h, px) = frame_of(&img, st);
                    got.push((format!("{v:?} {d} {st:?}"), fnv(w, h, &px)));
                }
            }
        }
        crate::pins::check("blue/start_pill", &got);
    }
}
