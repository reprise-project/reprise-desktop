//! The rebar's parts: [`Part::Rebar`], the background behind every band (one 320x13 frame), and
//! [`Part::RebarBand`], the line along the top of each band after the first (one 1x2 frame).

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::metrics::RebarMetrics;
use crate::raster::{Canvas, Fill, Insets};
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

/// The background's design size, stretched to whatever the rebar is.
const W: i32 = 320;
const H: i32 = 13;

/// The bottom rows the stretch keeps whole.
const BOTTOM: i32 = 4;

/// Rows from here down are whole design rows: at any scale each takes its row's middle.
const SHADE_ROW: i32 = H - 2;

/// The smallest gap in summed channels between the middle rows and `inner` that the down shading
/// leans across; nearer than this the face has no shading.
const LEAN_MIN: f64 = 8.0;

const LINE_W: i32 = 1;
const LINE_H: i32 = 2;

pub(super) fn metrics(v: Variant) -> RebarMetrics {
    let band_top = match v {
        Variant::Blue | Variant::Olive | Variant::Silver | Variant::Ember => 0,
        Variant::Slate | Variant::Sapphire | Variant::Obsidian | Variant::Cobalt => 2,
    };
    RebarMetrics {
        top: 0,
        first_line: false,
        closing: 0,
        band_top,
    }
}

pub(super) fn inks(v: Variant) -> Vec<(Part, State, Color)> {
    vec![(Part::RebarBand, State::Normal, v.scheme().inks.rebar_label)]
}

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    vec![
        (Part::Rebar, background(&s.rebar.normal, d)),
        (Part::RebarBand, band_line(&s.rebar_band.normal, d)),
    ]
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn light(p: Px) -> f64 {
    p.1.iter().sum()
}

fn background(face: &Face, d: i32) -> PartImage {
    let across = stops(face.across);
    let down = stops(face.down);
    let (across, down) = (Stops(&across), Stops(&down));
    let inner = shape::premul(face.inner, 255);
    let rim = shape::premul(face.rim, 255);
    let closes = face.down.last().is_some_and(|&(_, c)| c == face.rim);
    let mid = (0..H - BOTTOM)
        .map(|r| light(down.at((f64::from(r) + 0.5) / f64::from(H))))
        .sum::<f64>()
        / f64::from(H - BOTTOM);
    let reach = light(inner) - mid;
    let lean = |t: f64| {
        if reach.abs() < LEAN_MIN {
            0.0
        } else {
            (light(down.at(t)) - mid) / reach
        }
    };
    let canvas = shape::draw(W, H, d, |x, y| {
        let row = y as i32 / d;
        if closes && row == H - 1 {
            return rim;
        }
        let v = if row >= SHADE_ROW {
            f64::from(row)
        } else {
            shape::centre(y as i32, d)
        };
        let u = shape::centre(x as i32, d);
        let t = |p: f64, n: i32| (p + 0.5) / f64::from(n);
        shape::mix(across.at(t(u, W)), inner, lean(t(v, H)))
    });
    PartImage::new(canvas, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(0, 0, 0, BOTTOM))
        .fill(Fill::StretchCentred)
        .at_scale(d)
}

fn band_line(face: &Face, d: i32) -> PartImage {
    let down = stops(face.down);
    let down = Stops(&down);
    let canvas: Canvas = shape::draw(LINE_W, LINE_H, d, |_, y| {
        let row = y as i32 / d;
        down.at((f64::from(row) + 0.5) / f64::from(LINE_H))
    });
    PartImage::new(canvas, &[State::Normal], Layout::Vertical).at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rect;

    fn part(v: Variant, part: Part, d: i32) -> PartImage {
        parts(v, d)
            .into_iter()
            .find(|(p, _)| *p == part)
            .expect("the part")
            .1
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        };
        eat(c.width);
        eat(c.height);
        for &p in &c.pixels {
            eat(p);
        }
        h
    }

    fn drawn(v: Variant, w: i32, h: i32, s: i32) -> Canvas {
        let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
        crate::blue::style_for(v).at_scale(s).draw(
            &mut c,
            Rect::new(0, 0, w * s, h * s),
            Part::Rebar,
            State::Normal,
            s,
        );
        c
    }

    fn channels(px: u32) -> i32 {
        [16, 8, 0].iter().map(|s| ((px >> s) & 0xFF) as i32).sum()
    }

    #[test]
    fn rebar_pins_at_scale_1_and_2() {
        let want: [(Variant, [u64; 4]); 8] = [
            (
                Variant::Blue,
                [
                    0x8d7e919b16cf0c3b,
                    0x2e2d7989cb987f94,
                    0xb9ea95dc1000a761,
                    0x378fc45603f1eec3,
                ],
            ),
            (
                Variant::Olive,
                [
                    0x8d7e919b16cf0c3b,
                    0x2e2d7989cb987f94,
                    0xb9ea95dc1000a761,
                    0x378fc45603f1eec3,
                ],
            ),
            (
                Variant::Silver,
                [
                    0x02f2060fcfe629b7,
                    0xd5555043c43cae75,
                    0xb9ea95dc1000a761,
                    0x378fc45603f1eec3,
                ],
            ),
            (
                Variant::Slate,
                [
                    0x39c1c16eed6e8e77,
                    0x55db844d2d91ca95,
                    0x221848b77a31edc9,
                    0xa3852e3d97c059fb,
                ],
            ),
            (
                Variant::Sapphire,
                [
                    0x5c18e2541baf7cd4,
                    0x5d60bb1ff0df8139,
                    0x6506ae63d400f32b,
                    0x5835cd7bf850573b,
                ],
            ),
            (
                Variant::Obsidian,
                [
                    0x5c18e2541baf7cd4,
                    0x5d60bb1ff0df8139,
                    0x6506ae63d400f32b,
                    0x5835cd7bf850573b,
                ],
            ),
            (
                Variant::Ember,
                [
                    0xcc73d0111c8e4504,
                    0x546b4610a9ae9b65,
                    0x044881221d36f702,
                    0xad4a8407cd59bd23,
                ],
            ),
            (
                Variant::Cobalt,
                [
                    0x5c18e2541baf7cd4,
                    0x5d60bb1ff0df8139,
                    0x6506ae63d400f32b,
                    0x5835cd7bf850573b,
                ],
            ),
        ];
        let mut got = Vec::new();
        for (v, _) in want {
            let mut h = [0; 4];
            for (i, d) in [1, 2].into_iter().enumerate() {
                h[i] = fnv(&part(v, Part::Rebar, d).atlas);
                h[2 + i] = fnv(&part(v, Part::RebarBand, d).atlas);
            }
            got.push((v, h));
        }
        assert_eq!(got, want, "rebar hashes: {got:#x?}");
    }

    #[test]
    fn both_parts_double_their_size_and_keep_their_sizing() {
        for v in Variant::ALL {
            for p in [Part::Rebar, Part::RebarBand] {
                let (one, two) = (part(v, p, 1), part(v, p, 2));
                assert_eq!(
                    (two.atlas.width, two.atlas.height),
                    (2 * one.atlas.width, 2 * one.atlas.height),
                    "{v:?} {p:?}: size"
                );
                assert_eq!((one.scale, two.scale), (1, 2), "{v:?} {p:?}: scale");
                assert_eq!(one.sizing, two.sizing, "{v:?} {p:?}: sizing");
            }
            let one = part(v, Part::Rebar, 1);
            assert_eq!((one.atlas.width, one.atlas.height), (W as u32, H as u32));
            assert_eq!(one.sizing, Insets::new(0, 0, 0, BOTTOM), "{v:?}");
        }
    }

    #[test]
    fn the_background_runs_light_to_dark_and_closes_with_a_line() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let a = part(v, Part::Rebar, d).atlas;
                let (w, h) = (a.width as i32, a.height as i32);
                assert!(
                    a.pixels.iter().all(|p| p >> 24 == 0xFF),
                    "{v:?} at {d}: opaque"
                );
                assert!(
                    channels(a.get(0, 0)) > channels(a.get(w - 1, 0)),
                    "{v:?} at {d}: lighter at the left"
                );
                let bottom = a.get(0, h - 1);
                let flat = (0..w).all(|x| a.get(x, h - 1) == bottom);
                let darker = channels(bottom) < channels(a.get(0, h - 1 - d)) - 30;
                let closes = v != Variant::Silver;
                assert_eq!(flat && darker, closes, "{v:?} at {d}: closing line");
            }
        }
    }

    #[test]
    fn navy_darkens_toward_its_bottom() {
        for v in Variant::ALL {
            let a = part(v, Part::Rebar, 1).atlas;
            let shades = matches!(v, Variant::Blue | Variant::Olive);
            for x in [0, W / 2, W - 1] {
                let (top, low) = (channels(a.get(x, 0)), channels(a.get(x, SHADE_ROW - 1)));
                assert_eq!(top > low + 8, shades, "{v:?} at x {x}: {top} over {low}");
                if !shades {
                    assert!((top - low).abs() <= 1, "{v:?} at x {x}: flat");
                }
            }
            if shades {
                assert!(
                    channels(a.get(W - 1, 4)) < channels(a.get(W - 2, 4)),
                    "{v:?}: darker last column"
                );
            }
        }
    }

    #[test]
    fn the_background_stretches_with_its_bottom_rows_whole() {
        for v in Variant::ALL {
            for s in [1, 2] {
                let a = drawn(v, 792, 86, s);
                let src = part(v, Part::Rebar, s).atlas;
                let (w, h) = (792 * s, 86 * s);
                for x in [0, 400 * s, w - 1] {
                    let sx = (2 * x + 1) * W * s / (2 * w);
                    for k in 1..=BOTTOM * s {
                        let (y, sy) = (h - k, H * s - k);
                        assert_eq!(a.get(x, y), src.get(sx, sy), "{v:?} at {s}: {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn each_variant_has_its_band_line_and_label() {
        for v in Variant::ALL {
            let face = &v.scheme().rebar_band.normal;
            for d in [1, 2] {
                let a = part(v, Part::RebarBand, d).atlas;
                let shadow = face.down[0].1.argb();
                let highlight = face.down[face.down.len() - 1].1.argb();
                assert_ne!(shadow, highlight, "{v:?}: two rows");
                for y in 0..2 * d {
                    let want = if y < d { shadow } else { highlight };
                    for x in 0..d {
                        assert_eq!(a.get(x, y), want, "{v:?} at {d}: {x},{y}");
                    }
                }
            }
            let [(part, state, ink)] = inks(v)[..] else {
                panic!("{v:?}: one ink")
            };
            assert_eq!((part, state), (Part::RebarBand, State::Normal));
            assert_eq!(ink, v.scheme().inks.rebar_label, "{v:?}: the band label");
        }
    }

    #[test]
    fn slate_sapphire_obsidian_and_cobalt_lower_a_bands_content() {
        for v in Variant::ALL {
            let want = match v {
                Variant::Slate | Variant::Sapphire | Variant::Obsidian | Variant::Cobalt => 2,
                _ => 0,
            };
            assert_eq!(metrics(v).band_top, want, "{v:?}");
        }
    }
}
