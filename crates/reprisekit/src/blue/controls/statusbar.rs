//! A status bar's parts: [`Part::StatusBar`] and [`Part::StatusPane`],
//! drawn from the scheme's colours at the style's scale.
//!
//! The bar is 23 rows: a dark top line, a few rows fading into the face,
//! a flat run, then rows shading down to a slightly lighter last row
//! (the scheme's `status_bar.down`). Across, its left 50 columns fade in
//! from a paler face and its right 17 columns ease into the right end's
//! colour (`status_bar.across`); one column between them stretches.
//!
//! A pane is its divider down rows 4 to 20 in the last two columns of the
//! pane's rect, with nothing elsewhere, so the bar shows through. The
//! divider is a shadow line, translucent black, so it follows the bar's
//! own shading; where the scheme gives a second colour across (navy,
//! olive green, silver) a white line sits after it, else the bar shows
//! through that column.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::{Canvas, Insets};
use crate::shape::{CLEAR, Px, Stops, centre, draw, premul};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

/// The bar's frame: its left end, one middle column, its right end.
const LEFT: i32 = 50;
const RIGHT: i32 = 17;
const BAR_W: i32 = LEFT + 1 + RIGHT;
const BAR_H: i32 = 23;
/// The rows at the top and bottom that keep their own shading; the flat
/// run between them stretches for a bar of another height.
const TOP: i32 = 5;
const BOTTOM: i32 = 9;
/// Where the face's flat run is sampled, as a share of the bar's height:
/// the left and right ends' colours are offsets from it.
const FLAT_AT: f64 = 0.5;
/// A pane's frame: one clear column and its divider's two columns.
const PANE_W: i32 = 3;
const DIVIDER: i32 = 2;
/// The divider's first and last rows.
const DIVIDER_TOP: i32 = 4;
const DIVIDER_BOTTOM: i32 = 20;
/// Where a pane's icon and text box starts, under the bar's top: two rows
/// lower than on Classic's bar.
const CONTENT_TOP: i32 = 4;
/// Where a pane's icon box ends, over the bar's bottom: on a 23 px bar the
/// box is the 16 px icon's own rows, so the icon starts on the content top.
const CONTENT_BOTTOM: i32 = 3;

/// This surface's parts in `v`'s colours, painted at `d` device pixels
/// per design pixel.
pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let bar = PartImage::new(
        bar(&s.status_bar.normal, d),
        &[State::Normal],
        Layout::Vertical,
    )
    .sizing(Insets::new(LEFT, TOP, RIGHT, BOTTOM))
    .at_scale(d);
    let pane = PartImage::new(
        pane(&s.status_pane.normal, &s.status_bar.normal, d),
        &[State::Normal],
        Layout::Vertical,
    )
    .sizing(Insets::new(0, TOP, DIVIDER, BOTTOM))
    .content(Insets::new(0, CONTENT_TOP, 0, CONTENT_BOTTOM))
    .at_scale(d);
    vec![(Part::StatusBar, bar), (Part::StatusPane, pane)]
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), premul(c, 255)))
        .collect()
}

/// `a` plus `k` of the step from `from` to `to`, channel by channel.
fn shift(a: Px, from: Px, to: Px, k: f64) -> Px {
    (
        a.0,
        std::array::from_fn(|i| a.1[i] + (to.1[i] - from.1[i]) * k),
    )
}

/// The bar, 68 x 23 design px at `d`. Each device row takes the face's
/// colour down the bar at its own middle, the top line and the last row
/// held as whole design rows so they stay crisp; the left and right ends
/// shift that colour by how far the face's ends differ from its flat run.
fn bar(f: &Face, d: i32) -> Canvas {
    let down = stops(f.down);
    let across = stops(f.across);
    let (down, across) = (Stops(&down), Stops(&across));
    let h = f64::from(BAR_H);
    let flat = down.at(FLAT_AT);
    let (left, right) = (across.at(0.0), across.at(1.0));
    draw(BAR_W, BAR_H, d, |x, y| {
        let (u, v) = (centre(x as i32, d), centre(y as i32, d));
        // The top line holds row 0's colour, the last row its own; the
        // rows between run the gradient, kept clear of both.
        let row = if v < 0.5 {
            0.0
        } else if v > h - 1.5 {
            h - 1.0
        } else {
            v.clamp(1.0, h - 2.0)
        };
        let px = down.at((row + 0.5) / h);
        let l = 1.0 - u / f64::from(LEFT - 1);
        let r = (u - f64::from(LEFT)) / f64::from(RIGHT);
        if l > 0.0 {
            shift(px, flat, left, l.min(1.0))
        } else if r > 0.0 {
            shift(px, flat, right, r.min(1.0))
        } else {
            px
        }
    })
}

/// How dark the divider's shadow line is: the alpha of black that turns
/// the bar's flat face into the scheme's shadow colour.
fn shadow_alpha(shadow: Color, face: Px) -> f64 {
    let luma = |c: [f64; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let s = premul(shadow, 255).1;
    (255.0 * (1.0 - luma(s) / luma(face.1))).clamp(0.0, 255.0)
}

/// A pane, 3 x 23 design px at `d`: the divider in its last two columns,
/// rows 4 to 20, clear elsewhere.
fn pane(p: &Face, bar: &Face, d: i32) -> Canvas {
    let face = Stops(&stops(bar.down)).at(FLAT_AT);
    let shadow = (shadow_alpha(p.across[0].1, face), [0.0; 3]);
    let light = match p.across {
        [_, .., (_, c)] => premul(*c, 255),
        _ => CLEAR,
    };
    draw(PANE_W, BAR_H, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        if !(DIVIDER_TOP..=DIVIDER_BOTTOM).contains(&row) {
            return CLEAR;
        }
        match col - (PANE_W - DIVIDER) {
            0 => shadow,
            1 => light,
            _ => CLEAR,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    fn part(v: Variant, d: i32, p: Part) -> PartImage {
        parts(v, d).into_iter().find(|(q, _)| *q == p).unwrap().1
    }

    /// Navy, olive green and silver draw a white line after the shadow.
    fn lit(v: Variant) -> bool {
        matches!(v, Variant::Blue | Variant::Olive | Variant::Silver)
    }

    /// Each scheme's bar and pane at scale 1 and 2, as FNV-1a hashes of
    /// (width, height, pixels): [bar 1, bar 2, pane 1, pane 2].
    #[test]
    fn status_parts_pin_at_scale_1_and_2() {
        let want: [(Variant, [u64; 4]); 7] = [
            (
                Variant::Blue,
                [
                    0x26a6_3e7a_70e8_885f,
                    0xc3ef_d340_8747_37fe,
                    0x345f_4c33_58d6_f40f,
                    0x70a8_e232_a792_36fd,
                ],
            ),
            (
                Variant::Olive,
                [
                    0x66a5_acfc_c0da_061d,
                    0x59a9_0a19_ad78_0025,
                    0xa864_78be_b447_735e,
                    0xedfe_cca9_de47_9b1d,
                ],
            ),
            (
                Variant::Silver,
                [
                    0x46bb_332e_1b2d_f1cf,
                    0x0455_01f6_d6a4_3960,
                    0x1a44_e373_d7d1_ce2d,
                    0x53aa_96d6_d037_c73d,
                ],
            ),
            (
                Variant::Slate,
                [
                    0xa13c_31ff_30ea_a6f6,
                    0x0b55_34d1_9b18_3523,
                    0x25a1_8c18_34c6_bdaf,
                    0x29d6_854f_9f58_5d0d,
                ],
            ),
            (
                Variant::Sapphire,
                [
                    0xf1f9_912d_e5c5_3906,
                    0x9223_af80_4aee_0003,
                    0x25a1_8c18_34c6_bdaf,
                    0x29d6_854f_9f58_5d0d,
                ],
            ),
            (
                Variant::Obsidian,
                [
                    0xf1f9_912d_e5c5_3906,
                    0x9223_af80_4aee_0003,
                    0x25a1_8c18_34c6_bdaf,
                    0x29d6_854f_9f58_5d0d,
                ],
            ),
            (
                Variant::Ember,
                [
                    0xa13c_31ff_30ea_a6f6,
                    0x0b55_34d1_9b18_3523,
                    0x25a1_8c18_34c6_bdaf,
                    0x29d6_854f_9f58_5d0d,
                ],
            ),
        ];
        let got = want.map(|(v, _)| {
            let h = |d, p| fnv(&part(v, d, p).atlas);
            (
                v,
                [
                    h(1, Part::StatusBar),
                    h(2, Part::StatusBar),
                    h(1, Part::StatusPane),
                    h(2, Part::StatusPane),
                ],
            )
        });
        assert_eq!(got, want, "{got:#x?}");
    }

    /// At `d` both parts are `d` times their design size, their nine-grid
    /// and content insets still in design pixels.
    #[test]
    fn status_parts_are_painted_at_the_style_scale() {
        for v in Variant::ALL {
            for d in [1, 2, 3] {
                let du = d as u32;
                let bar = part(v, d, Part::StatusBar);
                assert_eq!(
                    (bar.atlas.width, bar.atlas.height),
                    (68 * du, 23 * du),
                    "{v:?} bar at {d}"
                );
                assert_eq!(bar.sizing, Insets::new(50, 5, 17, 9), "{v:?} at {d}");
                let pane = part(v, d, Part::StatusPane);
                assert_eq!(
                    (pane.atlas.width, pane.atlas.height),
                    (3 * du, 23 * du),
                    "{v:?} pane at {d}"
                );
                assert_eq!(pane.sizing, Insets::new(0, 5, 2, 9), "{v:?} at {d}");
                assert_eq!(pane.content, Insets::new(0, 4, 0, 3), "{v:?} at {d}");
            }
        }
    }

    /// The bar is opaque; its top line is the darkest row and keeps whole
    /// design rows at scale 2, as does the last row, while the shading and
    /// the left end's fade are worked out per device pixel, not doubled.
    #[test]
    fn the_bar_is_redrawn_at_scale_2() {
        let luma = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        for v in Variant::ALL {
            let one = part(v, 1, Part::StatusBar).atlas;
            let two = part(v, 2, Part::StatusBar).atlas;
            assert!(one.pixels.iter().all(|p| p >> 24 == 0xFF), "{v:?}: opaque");
            assert!(
                (1..23).all(|y| luma(one.get(50, 0)) < luma(one.get(50, y))),
                "{v:?}: the top line is darkest"
            );
            assert_eq!(two.get(100, 0), two.get(100, 1), "{v:?}: a whole line");
            assert_ne!(two.get(100, 1), two.get(100, 2), "{v:?}: under the line");
            assert_eq!(two.get(100, 44), two.get(100, 45), "{v:?}: the last row");
        }
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let two = part(v, 2, Part::StatusBar).atlas;
            assert!(
                (0..50).any(|c| two.get(2 * c, 20) != two.get(2 * c + 1, 20)),
                "{v:?}: the left fade steps inside a design column"
            );
            assert!(
                (2..21).any(|r| two.get(100, 2 * r) != two.get(100, 2 * r + 1)),
                "{v:?}: the shading steps inside a design row"
            );
        }
    }

    /// A pane is clear but for its divider down rows 4 to 20: a
    /// translucent black shadow line in the middle column, then a white
    /// line in navy, olive green and silver, the bar showing through that
    /// column in the other schemes. Whole design pixels at every scale.
    #[test]
    fn the_pane_is_its_divider() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let c = part(v, d, Part::StatusPane).atlas;
                for y in 0..23 * d {
                    let on = (4..=20).contains(&(y / d));
                    for x in 0..3 * d {
                        let p = c.get(x, y);
                        match (on, x / d) {
                            (true, 1) => assert!(
                                (0x10..0x60).contains(&(p >> 24)) && p & 0xFF_FFFF == 0,
                                "{v:?} at {d}: shadow ({x}, {y}) {p:08X}"
                            ),
                            (true, 2) if lit(v) => {
                                assert_eq!(p, 0xFFFF_FFFF, "{v:?} at {d}: white ({x}, {y})");
                            }
                            _ => assert_eq!(p >> 24, 0, "{v:?} at {d}: clear ({x}, {y})"),
                        }
                    }
                }
            }
        }
    }
}
