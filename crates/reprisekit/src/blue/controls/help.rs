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
    let frames = frames(v);
    let side = S * scale;
    let mut a = Canvas::new(side as u32, (side * frames.len() as i32) as u32);
    for (i, (_, face)) in frames.iter().enumerate() {
        a.blit_over(0, i as i32 * side, &help_box(face, scale));
    }
    let states: Vec<State> = frames.iter().map(|(s, _)| *s).collect();
    let mut img = PartImage::new(a, &states, Layout::Vertical).at_scale(scale);
    img.true_size = true;
    vec![(Part::HelpButton, img)]
}

/// One frame of the box at `d` device px per design px.
fn help_box(face: &Face, d: i32) -> Canvas {
    let rim = shape::premul(face.rim, 255);
    let down = stops(face.down);
    let across = stops(face.across);
    let fill = Fill::new(&down, &across);
    let px = shape::ringed(S, S, CORNER_R, 1, d, |x, y, ring| match ring {
        0 => rim,
        _ => fill.at(x, y),
    });
    let mark = face.mark.map(|c| shape::premul(c, 255));
    let k = f64::from(d);
    let curl = question_line(k);
    let w = S * d;
    shape::draw(S, S, d, |x, y| {
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
                out.push((v, state, help_box(face, d)));
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
                .map(|(_, face)| fnv(&help_box(face, 1)))
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
        let got: Vec<String> = [1, 2]
            .into_iter()
            .flat_map(each)
            .map(|(v, s, c)| format!("{v:?} {s:?} {:016x}", fnv(&c)))
            .collect();
        let want: &[&str] = &[
            "Blue Normal 1101d6fc13fa71b4",
            "Blue Hot 213d3f26a245267d",
            "Blue Pressed 0129e22f2bc217f7",
            "Blue Inactive bcb988012b6fd61c",
            "Olive Normal cd6bd2de1f03a76b",
            "Olive Hot ec60673dc1c441cf",
            "Olive Pressed ed8e9b2a64021d80",
            "Olive Inactive 33d9108ce0180bfe",
            "Silver Normal 87fa118c6e8c056c",
            "Silver Hot 6a97f5c142017fd4",
            "Silver Pressed 3124253af78ec1b4",
            "Silver Inactive 08de93106af0d179",
            "Slate Normal 4d80543aadd74b9a",
            "Slate Hot c8d7006f30de893a",
            "Slate Pressed ee97c473f50af555",
            "Slate Inactive 24de4fac13a6e2fc",
            "Sapphire Normal e8f80eab12531f96",
            "Sapphire Hot 3080c5d69535b7e6",
            "Sapphire Pressed 67454bd67d667851",
            "Sapphire Inactive a371518991b28247",
            "Obsidian Normal 5215afc84f535cc7",
            "Obsidian Hot 6a46f602eb08baff",
            "Obsidian Pressed f3bb514b5c091744",
            "Obsidian Inactive 730610612e484195",
            "Ember Normal f440af9ec6d79547",
            "Ember Hot 11f3d21cbd940541",
            "Ember Pressed 8f8d88c8e48156e7",
            "Ember Inactive 63f13d00a8029a6d",
            "Cobalt Normal 567cd5febf38ae53",
            "Cobalt Hot 862bf56915ce1c48",
            "Cobalt Pressed ab28369672b9d2f7",
            "Cobalt Inactive 5a342402953dd939",
            "Blue Normal 40ee211c14fee9ce",
            "Blue Hot 82a810bd2decb3b9",
            "Blue Pressed 91416ef3906a7be2",
            "Blue Inactive 92d5787b32b1d296",
            "Olive Normal 0623bc903e48618d",
            "Olive Hot e670c7bdc42c0679",
            "Olive Pressed f820b997d4924f74",
            "Olive Inactive 51cbda9d2b031fc2",
            "Silver Normal 00238b44de54d8cb",
            "Silver Hot 6ebdf82d58d674b2",
            "Silver Pressed e1dc8097bb7c8ef9",
            "Silver Inactive 0be481a3d683ad11",
            "Slate Normal c111767e6c7cd82d",
            "Slate Hot f40d203dcf48f418",
            "Slate Pressed 0d5722bb03079e0d",
            "Slate Inactive 4c8c8086d443bcce",
            "Sapphire Normal 31bb395fa741c88c",
            "Sapphire Hot 090a85bae1c0b428",
            "Sapphire Pressed 90e42860d663186d",
            "Sapphire Inactive 652d739339726480",
            "Obsidian Normal a1ca621815669314",
            "Obsidian Hot f2625b038bb8dcae",
            "Obsidian Pressed 6b33cf4d38ea0e1c",
            "Obsidian Inactive a2c06f831ac25a5e",
            "Ember Normal 4845ee8e203009bc",
            "Ember Hot f88d1c3f122783c2",
            "Ember Pressed 7133c4ba02f2e65d",
            "Ember Inactive e2447af689f7d1a2",
            "Cobalt Normal 6487bf418b449768",
            "Cobalt Hot 17a04bc93969e35d",
            "Cobalt Pressed af9b9309e70bf01d",
            "Cobalt Inactive 5d90d872209f59ba",
        ];
        if got != want {
            panic!("pins:\n{}", got.join("\n"));
        }
    }
}
