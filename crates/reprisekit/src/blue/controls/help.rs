//! The dialog caption's ? box, [`Part::HelpButton`]: 21x21, drawn at the style's scale. It never
//! disables, so it carries no Disabled frame. Sizes below are design px.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::Canvas;
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};

const S: i32 = 21;

const CORNER_R: f64 = 2.75;

const STROKE: f64 = 2.0;

const CURL_CENTRE: (f64, f64) = (10.5, 8.0);

const CURL_R: f64 = 3.0;

/// Degrees from three o'clock, counter-clockwise: from nine o'clock over the top to past three.
const CURL_FROM: f64 = 180.0;
const CURL_TO: f64 = -40.0;

const CURL_STEPS: usize = 16;

const NECK: (f64, f64) = (10.0, 12.0);
const STEM_END: (f64, f64) = (10.0, 14.0);

const DOT: (f64, f64, f64) = (9.0, 15.0, 2.0);

fn frames(v: Variant) -> [(State, &'static Face); 4] {
    let f = &v.scheme().help_button;
    [
        (State::Normal, &f.normal),
        (State::Hot, &f.hot),
        (State::Pressed, &f.pressed),
        (State::Inactive, &f.inactive),
    ]
}

pub(super) fn parts(v: Variant, scale: i32) -> Vec<(Part, PartImage)> {
    parts_sized(v, scale, S)
}

/// [`parts`] with the box `side` design px square, for a caption grown with the text size.
pub(super) fn parts_sized(v: Variant, scale: i32, side: i32) -> Vec<(Part, PartImage)> {
    let frames = frames(v);
    let px = side * scale;
    let mut a = Canvas::new(px as u32, (px * frames.len() as i32) as u32);
    for (i, (_, face)) in frames.iter().enumerate() {
        a.blit_over(0, i as i32 * px, &help_box(face, scale, side));
    }
    let states: Vec<State> = frames.iter().map(|(s, _)| *s).collect();
    let mut img = PartImage::new(a, &states, Layout::Vertical).at_scale(scale);
    img.true_size = true;
    vec![(Part::HelpButton, img)]
}

/// One frame of the box, `side` design px square, at `d` device px per design px. The mark and
/// the corners grow with `side`, the rim stays 1 px.
fn help_box(face: &Face, d: i32, side: i32) -> Canvas {
    let rim = shape::premul(face.rim, 255);
    let down = stops(face.down);
    let across = stops(face.across);
    let fill = Fill::new(&down, &across);
    let grow = f64::from(side) / f64::from(S);
    let px = shape::ringed(side, side, CORNER_R * grow, 1, d, |x, y, ring| match ring {
        0 => rim,
        _ => fill.at(x / grow, y / grow),
    });
    let mark = face.mark.map(|c| shape::premul(c, 255));
    let k = f64::from(d) * grow;
    let curl = question_line(k);
    let w = side * d;
    shape::draw(side, side, d, |x, y| {
        let under = px[(y as i32 * w + x as i32) as usize];
        match mark {
            Some(ink) => shape::over(shape::scale(ink, question(x, y, k, &curl)), under),
            None => under,
        }
    })
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

struct Fill<'a> {
    down: Stops<'a>,
    across: Stops<'a>,
    mid: Px,
}

impl<'a> Fill<'a> {
    fn new(down: &'a [(f64, Px)], across: &'a [(f64, Px)]) -> Fill<'a> {
        let (down, across) = (Stops(down), Stops(across));
        let mid = shape::mix(down.at(0.5), across.at(0.5), 0.5);
        Fill { down, across, mid }
    }

    /// The profiles' first and last pixels are the rim, so the fill reads them from the ring
    /// inside it inward.
    fn at(&self, x: f64, y: f64) -> Px {
        let t = |v: f64| ((v + 0.5) / f64::from(S)).clamp(1.5 / 21.0, 19.5 / 21.0);
        let (a, b, m) = (self.down.at(t(y)).1, self.across.at(t(x)).1, self.mid.1);
        let c = |i: usize| (a[i] + b[i] - m[i]).clamp(0.0, 255.0);
        (255.0, [c(0), c(1), c(2)])
    }
}

/// The "?" curl, neck and stem as one line through device px at `k` device px per design px.
fn question_line(k: f64) -> Vec<(f64, f64)> {
    let (cx, cy) = CURL_CENTRE;
    let mut pts: Vec<(f64, f64)> = (0..=CURL_STEPS)
        .map(|i| {
            let a = (CURL_FROM + (CURL_TO - CURL_FROM) * i as f64 / CURL_STEPS as f64).to_radians();
            ((cx + CURL_R * a.cos()) * k, (cy - CURL_R * a.sin()) * k)
        })
        .collect();
    pts.push((NECK.0 * k, NECK.1 * k));
    pts.push((STEM_END.0 * k, STEM_END.1 * k));
    pts
}

fn question(x: f64, y: f64, k: f64, curl: &[(f64, f64)]) -> f64 {
    let (dx, dy, ds) = DOT;
    let line = shape::stroke(x, y, curl, STROKE * k);
    let dot = shape::rounded(x - dx * k, y - dy * k, ds * k, ds * k, 0.0);
    line.max(dot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        eat(c.width);
        eat(c.height);
        for &p in &c.pixels {
            eat(p);
        }
        h
    }

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    fn each(d: i32) -> Vec<(Variant, State, Canvas)> {
        let mut out = Vec::new();
        for v in Variant::ALL {
            for (state, face) in frames(v) {
                out.push((v, state, help_box(face, d, S)));
            }
        }
        out
    }

    #[test]
    fn the_box_is_21_design_px_at_every_scale_with_four_frames() {
        for d in [1, 2, 3] {
            for v in Variant::ALL {
                let (part, img) = parts(v, d).remove(0);
                assert_eq!(part, Part::HelpButton);
                assert_eq!(img.scale, d, "{v:?} painted at {d}");
                assert!(img.true_size, "{v:?} true to its size");
                assert_eq!(
                    img.states,
                    [State::Normal, State::Hot, State::Pressed, State::Inactive]
                );
                assert_eq!(
                    (img.atlas.width, img.atlas.height),
                    (21 * d as u32, 84 * d as u32)
                );
            }
        }
    }

    #[test]
    fn the_atlas_keeps_the_corners_translucent() {
        let (_, img) = parts(Variant::Blue, 1).remove(0);
        for i in 0..4 {
            let p = img.atlas.get(0, i * S);
            assert!(alpha(p) < 32, "frame {i}: corner alpha {:#x}", alpha(p));
        }
    }

    #[test]
    fn every_frame_leaves_its_corners_clear() {
        for d in [1, 2] {
            let e = (S * d - 1) as u32;
            for (v, state, c) in each(d) {
                for (x, y) in [(0, 0), (e, 0), (0, e), (e, e)] {
                    assert!(
                        alpha(c.get(x as i32, y as i32)) < 32,
                        "{v:?} {state:?} at {d}: corner ({x}, {y}) shows the caption"
                    );
                }
            }
        }
    }

    #[test]
    fn every_state_draws_its_own_frame() {
        for v in Variant::ALL {
            let f: Vec<u64> = frames(v)
                .iter()
                .map(|(_, face)| fnv(&help_box(face, 1, S)))
                .collect();
            for i in 0..f.len() {
                for j in i + 1..f.len() {
                    assert_ne!(f[i], f[j], "{v:?}: frames {i} and {j} are alike");
                }
            }
        }
    }

    /// At 2 the rounded corners are smooth: partly covered pixels along every curve, never a
    /// doubled scale-1 corner.
    #[test]
    fn the_corners_shade_smoothly_at_scale_2() {
        for (v, state, c) in each(2) {
            let e = S * 2 - 1;
            for (cx, cy, sx, sy) in [(0, 0, 1, 1), (e, 0, -1, 1), (0, e, 1, -1), (e, e, -1, -1)] {
                let partial = (0..4).any(|i| {
                    let a = alpha(c.get(cx + sx * i, cy));
                    let b = alpha(c.get(cx, cy + sy * i));
                    (1..255).contains(&a) || (1..255).contains(&b)
                });
                assert!(
                    partial,
                    "{v:?} {state:?}: corner at ({cx}, {cy}) has no soft edge"
                );
            }
        }
    }

    /// The "?" holds solid ink in its curl, its stem and its dot at 1, so it never smudges into the
    /// glass.
    #[test]
    fn the_question_mark_is_solid_at_scale_1() {
        let curl = question_line(1.0);
        let full = |x: i32, y: i32| question(f64::from(x), f64::from(y), 1.0, &curl) >= 0.999;
        assert!((8..=12).any(|x| full(x, 4)), "the curl's top");
        assert!((6..=7).any(|x| full(x, 7)), "the curl's left end");
        assert!((12..=14).any(|x| full(x, 7)), "the curl's right side");
        assert!(full(9, 13) && full(10, 13), "the stem");
        assert!(
            full(9, 15) && full(10, 15) && full(9, 16) && full(10, 16),
            "the dot"
        );
        assert!(!full(9, 14) && !full(10, 14), "a gap between stem and dot");
    }

    /// The rim and the glass share the pixels along the curve: a pixel on the curve is never darker
    /// than both its neighbours along the diagonal into the box.
    #[test]
    fn no_dark_fringe_along_the_corners() {
        let lum = |p: u32| {
            let a = alpha(p).max(1) as f64;
            let ch = |s: u32| f64::from((p >> s) & 0xFF) * 255.0 / a;
            0.3 * ch(16) + 0.59 * ch(8) + 0.11 * ch(0)
        };
        for d in [1, 2] {
            for (v, state, c) in each(d) {
                for i in 1..(3 * d) {
                    let (o, p, q) = (c.get(i - 1, i - 1), c.get(i, i), c.get(i + 1, i + 1));
                    if alpha(o) == 0 || alpha(p) == 255 {
                        continue;
                    }
                    assert!(
                        lum(p) + 1.0 >= lum(o).min(lum(q)),
                        "{v:?} {state:?} at {d}: ({i}, {i}) is a dark fringe"
                    );
                }
            }
        }
    }

    #[test]
    fn help_box_pins_at_scale_1_and_2() {
        let got: Vec<_> = [1, 2]
            .into_iter()
            .flat_map(|d| {
                each(d)
                    .into_iter()
                    .map(move |(v, s, c)| (format!("{v:?} {s:?} {d}"), fnv(&c)))
            })
            .collect();
        crate::pins::check("blue/help_box", &got);
    }

    #[test]
    fn a_grown_box_is_its_side_with_clear_corners_smooth_at_2() {
        for side in [28, 34] {
            for d in [1, 2] {
                let (_, img) = parts_sized(Variant::Blue, d, side).remove(0);
                let n = side * d;
                assert_eq!(
                    (img.atlas.width, img.atlas.height),
                    (n as u32, (4 * n) as u32)
                );
                for i in 0..4 {
                    let at = |x: i32, y: i32| alpha(img.atlas.get(x, i * n + y));
                    for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
                        assert!(at(x, y) < 32, "frame {i} {side} at {d}: ({x}, {y})");
                    }
                    let partial = (0..4 * d)
                        .flat_map(|y| (0..4 * d).map(move |x| (x, y)))
                        .filter(|&(x, y)| (1..255).contains(&at(x, y)))
                        .count();
                    assert!(partial >= 2, "frame {i} {side} at {d}: corner smooths");
                }
            }
        }
    }

    #[test]
    fn grown_help_box_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in Variant::ALL {
            for (state, face) in frames(v) {
                for side in [28, 34] {
                    for d in [1, 2] {
                        let c = help_box(face, d, side);
                        got.push((format!("{v:?} {state:?} {side} {d}"), fnv(&c)));
                    }
                }
            }
        }
        crate::pins::check("blue/help_box_grown", &got);
    }
}
