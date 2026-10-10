//! The progress bar's `ProgressBar` and `ProgressChunk` atlases: Normal then Disabled (the scheme gives both one face).
//! The styled drawer in [`crate::look::styled::progress`] steps the chunk across the fill and stretches it
//! to the bar's height, since a nine-grid atlas cannot repeat one axis while stretching the other.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::{Canvas, Fill, Insets};
use crate::shape::{CLEAR, Px, Stops, centre, draw, mix, premul, ringed, smooth};
use crate::style::{Layout, Part, PartImage, State};

const SIDE: i32 = 4;
const END: i32 = 3;
const TROUGH_W: i32 = 9;
const TROUGH_H: i32 = 19;
const RADIUS: f64 = 3.0;
const RINGS: usize = 3;

/// Where the fill's own content rect sits inside the bar: flush with the
/// left border, the top and bottom borders above and below it, and two
/// columns into the right border, painting over it wherever it runs there.
const FILL_LEFT: i32 = SIDE;
const FILL_TOP: i32 = END;
const FILL_RIGHT: i32 = 2;
const FILL_BOTTOM: i32 = END;

const BLOCK_W: i32 = 6;
const BLOCK_H: i32 = 7;

/// The gap after each of `v`'s blocks, where the trough shows: 1 px in
/// Slate, Sapphire and Obsidian, 2 px in the others.
fn gap(v: Variant) -> i32 {
    match v {
        Variant::Slate | Variant::Sapphire | Variant::Obsidian => 1,
        Variant::Blue | Variant::Olive | Variant::Silver | Variant::Ember => 2,
    }
}

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let states = [State::Normal, State::Disabled];

    let troughs = [&s.progress_bar.normal, &s.progress_bar.disabled];
    let bar = PartImage::new(
        stack(troughs.map(|f| trough(f, d))),
        &states,
        Layout::Vertical,
    )
    .sizing(Insets::new(SIDE, END, SIDE, END))
    .fill(Fill::Tile)
    .content(Insets::new(FILL_LEFT, FILL_TOP, FILL_RIGHT, FILL_BOTTOM))
    .at_scale(d);

    let blocks = [&s.progress_chunk.normal, &s.progress_chunk.disabled];
    // Default zero sizing and `Fill::Stretch`: the drawer asks for one block
    // at a time and the frame stretches to that call's height.
    let chunk = PartImage::new(
        stack(blocks.map(|f| block(f, gap(v), d))),
        &states,
        Layout::Vertical,
    )
    .at_scale(d);

    vec![(Part::ProgressBar, bar), (Part::ProgressChunk, chunk)]
}

fn stack<const N: usize>(frames: [Canvas; N]) -> Canvas {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut atlas = Canvas::new(w, h * N as u32);
    for (i, f) in frames.iter().enumerate() {
        atlas.blit_over(0, (h * i as u32) as i32, f);
    }
    atlas
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), premul(c, 255)))
        .collect()
}

/// One trough frame at `d`. Ring `i` on the top and left takes row `i`'s
/// colour from the face's profile down the trough; on the bottom and right
/// it takes the colour of row `i` counted up from the bottom. Near the
/// top-right and bottom-left corners the two blend by which side is
/// closer.
fn trough(f: &Face, d: i32) -> Canvas {
    let down = stops(f.down);
    let down = Stops(&down);
    let h = f64::from(TROUGH_H);
    let (right, bottom) = (f64::from(TROUGH_W - 1), h - 1.0);
    let row = |r: f64| down.at((r + 0.5) / h);
    let fill = row(bottom / 2.0);
    let px = ringed(TROUGH_W, TROUGH_H, RADIUS, RINGS, d, |x, y, i| {
        if i == RINGS {
            return fill;
        }
        let i = i as f64;
        let (near, far) = (x.min(y), (right - x).min(bottom - y));
        let k = smooth(0.5 + (far - near) / 2.0);
        mix(row(bottom - i), row(i), k)
    });
    let wd = TROUGH_W * d;
    draw(TROUGH_W, TROUGH_H, d, |x, y| {
        px[(y as i32 * wd + x as i32) as usize]
    })
}

fn block(f: &Face, gap: i32, d: i32) -> Canvas {
    let down = stops(f.down);
    let down = Stops(&down);
    draw(BLOCK_W + gap, BLOCK_H, d, |x, y| {
        if x as i32 >= BLOCK_W * d {
            return CLEAR;
        }
        down.at((centre(y as i32, d) + 0.5) / f64::from(BLOCK_H))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::widgets::draw_progress;
    use crate::raster::Rect;
    use crate::theme::Color;

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

    fn luma(p: u32) -> u32 {
        (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF)
    }

    fn bar(v: Variant, h: i32, fraction: f64, s: i32) -> Canvas {
        let theme = crate::interfaces::blue_theme(v).scaled(s);
        let mut c = Canvas::new((110 * s) as u32, ((h + 6) * s) as u32);
        c.fill_rect(
            Rect::new(0, 0, 110 * s, (h + 6) * s),
            Color::rgb(0xEC, 0xE9, 0xD8),
        );
        let r = Rect::new(3, 3, 101, h);
        draw_progress(&mut c, &theme.paint(), r, fraction, true);
        c
    }

    #[test]
    fn progress_parts_pin_at_scale_1_and_2() {
        const LIGHT: [u64; 2] = [0x569f_9805_207f_1eb2, 0x4f46_2251_171c_004b];
        const GREY: [u64; 2] = [0xafdc_afa0_2895_a8f2, 0x353b_6ad5_ded9_ad8b];
        const SLATE: [u64; 2] = [0x5f64_ace7_2bb9_80a4, 0xbfe2_580b_921a_8087];
        let pin = |bar: [u64; 2], chunk: [u64; 2]| [bar[0], bar[1], chunk[0], chunk[1]];
        let want: [(Variant, [u64; 4]); 7] = [
            (
                Variant::Blue,
                pin(LIGHT, [0x5019_c2f5_4ecf_ceeb, 0x6f3e_2405_1682_cb89]),
            ),
            (
                Variant::Olive,
                pin(LIGHT, [0xa45c_69b4_fad3_5a8b, 0x0301_e094_298a_7329]),
            ),
            (
                Variant::Silver,
                pin(LIGHT, [0x1d23_8ad0_f92b_0d33, 0x014a_7a59_dd6f_51f9]),
            ),
            (Variant::Slate, pin(GREY, SLATE)),
            (Variant::Sapphire, pin(GREY, SLATE)),
            (Variant::Obsidian, pin(GREY, SLATE)),
            (
                Variant::Ember,
                pin(GREY, [0x03ef_c33c_b2e7_c4d3, 0xadec_b100_7565_f439]),
            ),
        ];
        let got = want.map(|(v, _)| {
            let h = |d, p| fnv(&part(v, d, p).atlas);
            (
                v,
                [
                    h(1, Part::ProgressBar),
                    h(2, Part::ProgressBar),
                    h(1, Part::ProgressChunk),
                    h(2, Part::ProgressChunk),
                ],
            )
        });
        assert_eq!(got, want);
    }

    #[test]
    fn every_part_keeps_its_design_insets_at_scale_2() {
        for v in Variant::ALL {
            for p in [Part::ProgressBar, Part::ProgressChunk] {
                let (one, two) = (part(v, 1, p), part(v, 2, p));
                assert_eq!(two.scale, 2, "{v:?} {p:?}");
                assert_eq!(two.sizing, one.sizing, "{v:?} {p:?} sizing");
                assert_eq!(two.content, one.content, "{v:?} {p:?} content");
                assert_eq!(two.states, one.states, "{v:?} {p:?} states");
                assert_eq!(
                    (two.atlas.width, two.atlas.height),
                    (2 * one.atlas.width, 2 * one.atlas.height),
                    "{v:?} {p:?} atlas"
                );
            }
        }
    }

    #[test]
    fn the_trough_has_round_clear_corners_without_a_dark_fringe() {
        for v in Variant::ALL {
            let rim = v.scheme().progress_bar.normal.rim.argb();
            for d in [1, 2] {
                let a = part(v, d, Part::ProgressBar).atlas;
                let (w, h) = (TROUGH_W * d, TROUGH_H * d);
                for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                    assert!(
                        a.get(x, y) >> 24 < 0x80,
                        "{v:?} d={d}: corner ({x}, {y}) is clear"
                    );
                }
                let mut over = Canvas::new(w as u32, h as u32);
                over.fill_rect(Rect::new(0, 0, w, h), Color::rgb(0xFF, 0xFF, 0xFF));
                over.blit_over(0, 0, &a);
                for y in 0..h {
                    for x in 0..w {
                        assert!(
                            luma(over.get(x, y)) + 3 >= luma(rim),
                            "{v:?} d={d}: ({x}, {y}) is no darker than the rim"
                        );
                    }
                }
                if d == 2 {
                    let partial = (0..3 * d)
                        .flat_map(|y| (0..3 * d).map(move |x| (x, y)))
                        .any(|(x, y)| (1..255).contains(&(a.get(x, y) >> 24)));
                    assert!(partial, "{v:?}: the corner curve is smooth at 2");
                }
            }
        }
    }

    #[test]
    fn the_trough_sinks_deeper_at_the_top_and_left() {
        let white = 0xFFFF_FFFF;
        for v in Variant::ALL {
            let f = &v.scheme().progress_bar.normal;
            let a = part(v, 1, Part::ProgressBar).atlas;
            let mid = TROUGH_H / 2;
            assert_eq!(a.get(4, mid), white, "{v:?}: white behind the fill");
            assert_eq!(a.get(0, mid), f.rim.argb(), "{v:?}: left rim");
            assert_eq!(a.get(4, 0), f.rim.argb(), "{v:?}: top rim");
            assert_eq!(a.get(1, mid), f.inner.argb(), "{v:?}: left inner ring");
            assert_eq!(a.get(4, 1), f.inner.argb(), "{v:?}: top inner ring");
            let light = a.get(2, mid);
            assert!(
                luma(light) > luma(f.inner.argb()),
                "{v:?}: second ring lighter"
            );
            assert!(luma(light) < luma(white), "{v:?}: second ring still shaded");
            assert_eq!(a.get(4, 2), light, "{v:?}: top second ring");
            assert_eq!(a.get(7, mid), light, "{v:?}: right ring is the light one");
            assert_eq!(a.get(4, TROUGH_H - 2), light, "{v:?}: bottom ring too");
            assert_eq!(a.get(6, mid), white, "{v:?}: right sinks one ring");
            assert_eq!(
                a.get(4, TROUGH_H - 3),
                white,
                "{v:?}: bottom sinks one ring"
            );
        }
    }

    #[test]
    fn a_block_shades_down_and_leaves_its_gap_clear() {
        for v in Variant::ALL {
            let f = &v.scheme().progress_chunk.normal;
            for d in [1, 2] {
                let a = part(v, d, Part::ProgressChunk).atlas;
                for y in 0..BLOCK_H * d {
                    for x in 0..BLOCK_W * d {
                        assert_eq!(a.get(x, y) >> 24, 0xFF, "{v:?} d={d}: ({x}, {y})");
                    }
                    for x in BLOCK_W * d..(BLOCK_W + gap(v)) * d {
                        assert_eq!(a.get(x, y), 0, "{v:?} d={d}: gap ({x}, {y})");
                    }
                }
            }
            let one = part(v, 1, Part::ProgressChunk).atlas;
            assert_eq!(one.get(0, 0), f.down[0].1.argb(), "{v:?}: top row");
            let two = part(v, 2, Part::ProgressChunk).atlas;
            let rows: Vec<u32> = (0..BLOCK_H * 2).map(|y| two.get(0, y)).collect();
            let steps = rows.windows(2).filter(|w| w[0] != w[1]).count();
            assert!(
                steps > BLOCK_H as usize,
                "{v:?}: the block shades smoothly at 2"
            );
        }
    }

    #[test]
    fn each_variant_fills_with_its_own_blocks() {
        let hash = |v| fnv(&part(v, 1, Part::ProgressChunk).atlas);
        let slate = hash(Variant::Slate);
        for v in [Variant::Sapphire, Variant::Obsidian] {
            assert_eq!(hash(v), slate, "{v:?} fills as slate does");
        }
        let own = [
            Variant::Blue,
            Variant::Olive,
            Variant::Silver,
            Variant::Slate,
            Variant::Ember,
        ];
        let mut seen = Vec::new();
        for v in own {
            assert!(!seen.contains(&hash(v)), "{v:?} has its own block");
            seen.push(hash(v));
        }
        for v in Variant::ALL {
            let gap = part(v, 1, Part::ProgressChunk).atlas.width as i32 - BLOCK_W;
            let want = match v {
                Variant::Slate | Variant::Sapphire | Variant::Obsidian => 1,
                _ => 2,
            };
            assert_eq!(gap, want, "{v:?}'s gap");
        }
    }

    #[test]
    fn disabled_repeats_normal() {
        for v in Variant::ALL {
            for p in [Part::ProgressBar, Part::ProgressChunk] {
                let img = part(v, 2, p);
                let (n, off) = (img.frame(State::Normal), img.frame(State::Disabled));
                assert!(n != off, "{v:?} {p:?}: two frames");
                assert!(
                    img.atlas.copy_rect(n) == img.atlas.copy_rect(off),
                    "{v:?} {p:?}"
                );
            }
        }
    }

    #[test]
    fn a_bar_at_scale_2_fills_as_far_as_at_scale_1() {
        for v in Variant::ALL {
            let filled = |s: i32| {
                let (full, empty) = (bar(v, 16, 0.5, s), bar(v, 16, 0.0, s));
                let y = (3 + 8) * s;
                (0..110 * s)
                    .filter(|&x| full.get(x, y) != empty.get(x, y))
                    .count()
            };
            let one = filled(1);
            assert!(one > 30, "{v:?}: half the bar fills");
            assert_eq!(filled(2), 2 * one, "{v:?}");
        }
    }
}
