//! The toolbar's parts: [`Part::ToolbarButton`] and [`Part::ToolbarDrop`] each hold six 9x38 frames stacked in the order of [`STATES`]; [`Part::ToolbarSeparator`] is one 6x4 frame. Frames are drawn from the scheme's faces at the style's scale.
//! A lit frame is a 1 px rim round a face, corners rounded over 3 px. The face takes the scheme's `down` shading for its row plus its `across` shading for its column (less the across shading's middle). Hot is near-white at the top, greying down and darkening along the right edge; Pressed is darker with a grey rim; Checked is white inside a blue rim, warming under the pointer (CheckedHot). The nine-grid keeps four columns and rows of each edge and stretches the rest, so one frame draws Explorer's 38 px strip and the file dialog's 22 px one.
//! Normal and Disabled are empty: a button at rest shows the rebar behind it.
//! The separator is a 1 px line two columns into a 6 px slot, from two rows under the band's top to one row over its bottom.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::metrics::{ButtonPads, ToolbarMetrics};
use crate::raster::{Canvas, Insets, Rect};
use crate::shape::{Px, Stops, draw, premul, ringed};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const W: i32 = 9;
const H: i32 = 38;

const RADIUS: f64 = 3.0;

/// How much of each edge the nine-grid keeps: the rounded corners and
/// the face's edge shading.
const EDGE: i32 = 4;

pub const STATES: [State; 6] = [
    State::Normal,
    State::Hot,
    State::Pressed,
    State::Disabled,
    State::Checked,
    State::CheckedHot,
];

const SEP_W: i32 = 6;
const SEP_H: i32 = 4;
const SEP_X: i32 = 2;
const SEP_TOP: i32 = 2;

pub(super) fn parts(v: Variant, scale: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let b = &s.toolbar_button;
    let dr = &s.toolbar_drop;
    vec![
        (
            Part::ToolbarButton,
            button([&b.hot, &b.pressed, &b.checked, &b.checked_hot], scale),
        ),
        (
            Part::ToolbarDrop,
            button([&dr.hot, &dr.pressed, &dr.checked, &dr.checked_hot], scale),
        ),
        (
            Part::ToolbarSeparator,
            separator(s.toolbar_separator.normal.rim, scale),
        ),
    ]
}

/// Blue's big strip: 38 px tall from 2 px in; a labelled button's label
/// 4 px past its icon box and 8 short of its end; an icon-only button's
/// icon 5 px in with 3 after it (32 px in all); a labelled split button's
/// icon 6 px in, its label 5 px past the box and 6 short of its 14 px
/// drop part; an empty-labelled one's icon 8 px in with 5 after it; the
/// arrow dead centre.
pub(super) fn metrics() -> ToolbarMetrics {
    ToolbarMetrics {
        height: H,
        lead: 2,
        labelled: ButtonPads {
            inset: 3,
            gap: 4,
            trail: 8,
        },
        bare: ButtonPads {
            inset: 5,
            gap: 0,
            trail: 3,
        },
        split: ButtonPads {
            inset: 6,
            gap: 5,
            trail: 6,
        },
        split_empty: ButtonPads {
            inset: 8,
            gap: 0,
            trail: 5,
        },
        split_drop: 14,
        split_arrow_dy: 0,
    }
}

/// Label and arrow inks from the scheme: held label, greyed label etched over a white copy, greyed drop arrow.
pub(in crate::blue) fn inks(v: Variant) -> [(Part, State, Color); 3] {
    let k = &v.scheme().inks;
    [
        (Part::ToolbarButton, State::Pressed, k.toolbar_pressed),
        (Part::ToolbarButton, State::Disabled, k.toolbar_disabled),
        (Part::ToolbarDrop, State::Disabled, k.toolbar_drop_disabled),
    ]
}

fn button(lit: [&Face; 4], d: i32) -> PartImage {
    let mut a = Canvas::new((W * d) as u32, (H * d * STATES.len() as i32) as u32);
    let mut lit = lit.into_iter();
    for (i, state) in STATES.into_iter().enumerate() {
        if matches!(state, State::Normal | State::Disabled) {
            continue;
        }
        let face = lit.next().expect("one face per lit state");
        let f = frame(face, d);
        let top = i as i32 * H * d;
        for y in 0..H * d {
            for x in 0..W * d {
                a.put_raw(x, top + y, f.get(x, y));
            }
        }
    }
    PartImage::new(a, &STATES, Layout::Vertical)
        .sizing(Insets::new(EDGE, EDGE, EDGE, EDGE))
        .at_scale(d)
}

/// One lit frame at `d`: the rim, then the face shaded by `down` for its
/// row and `across` for its column. Shading is read at the pixel's
/// middle, held to the face's own rows and columns so the rim colour at
/// the gradients' ends never bleeds into it.
fn frame(face: &Face, d: i32) -> Canvas {
    let rim = premul(face.rim, 255);
    let down = stops(face.down);
    let across = stops(face.across);
    let (down, across) = (Stops(&down), Stops(&across));
    let middle = across.at(0.5);
    let along = |v: f64, n: i32| (v.clamp(1.0, f64::from(n - 2)) + 0.5) / f64::from(n);
    let px = ringed(W, H, RADIUS, 1, d, |x, y, ring| {
        if ring == 0 {
            return rim;
        }
        let (row, col) = (down.at(along(y, H)), across.at(along(x, W)));
        (255.0, [0, 1, 2].map(|i| row.1[i] + col.1[i] - middle.1[i]))
    });
    draw(W, H, d, |x, y| px[(y as i32 * W * d + x as i32) as usize])
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    debug_assert!(!g.is_empty(), "a lit toolbar face shades its fill");
    g.iter()
        .map(|&(t, c)| (f64::from(t), premul(c, 255)))
        .collect()
}

/// The separator slot at `d`: transparent but for its line, which
/// stretches down the band. The line's column is part of the fixed left
/// edge, so a slot wider than 6 px widens the clear columns after it,
/// never the line.
fn separator(line: Color, d: i32) -> PartImage {
    let mut a = Canvas::new((SEP_W * d) as u32, (SEP_H * d) as u32);
    a.fill_rect(Rect::new(SEP_X * d, SEP_TOP * d, d, d), line);
    PartImage::new(a, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(
            SEP_X + 1,
            SEP_TOP,
            SEP_W - SEP_X - 1,
            SEP_H - SEP_TOP - 1,
        ))
        .at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(v: Variant, part: Part, d: i32) -> PartImage {
        parts(v, d)
            .into_iter()
            .find(|(p, _)| *p == part)
            .expect("a toolbar part")
            .1
    }

    fn frame(v: Variant, state: State, d: i32) -> Canvas {
        let img = part(v, Part::ToolbarButton, d);
        let f = img.frame(state);
        let mut c = Canvas::new(f.width as u32, f.height as u32);
        for y in 0..f.height {
            for x in 0..f.width {
                c.put_raw(x, y, img.atlas.get(f.x + x, f.y + y));
            }
        }
        c
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

    const LIT: [State; 4] = [
        State::Hot,
        State::Pressed,
        State::Checked,
        State::CheckedHot,
    ];

    #[test]
    fn parts_are_painted_at_the_style_scale() {
        for v in Variant::ALL {
            for d in [1, 2] {
                for (p, w, h) in [
                    (Part::ToolbarButton, W, H * 6),
                    (Part::ToolbarDrop, W, H * 6),
                    (Part::ToolbarSeparator, SEP_W, SEP_H),
                ] {
                    let img = part(v, p, d);
                    assert_eq!(img.scale, d, "{v:?} {p:?} at {d}");
                    assert_eq!(
                        (img.atlas.width as i32, img.atlas.height as i32),
                        (w * d, h * d),
                        "{v:?} {p:?} at {d}"
                    );
                    assert_eq!(img.sizing, part(v, p, 1).sizing, "{v:?} {p:?} at {d}");
                }
            }
        }
    }

    #[test]
    fn normal_and_disabled_are_empty() {
        for v in Variant::ALL {
            for d in [1, 2] {
                for state in [State::Normal, State::Disabled] {
                    let f = frame(v, state, d);
                    assert!(
                        f.pixels.iter().all(|p| p >> 24 == 0),
                        "{v:?} {state:?} at {d} is transparent"
                    );
                }
            }
        }
    }

    #[test]
    fn lit_frames_hold_a_rim_and_a_face_and_differ() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let frames: Vec<Canvas> = LIT.iter().map(|&s| frame(v, s, d)).collect();
                for (state, f) in LIT.iter().zip(&frames) {
                    let rim = f.get(0, 10 * d);
                    assert_eq!(rim >> 24, 0xFF, "{v:?} {state:?} at {d}: rim");
                    assert_eq!(f.get(4 * d, 10 * d) >> 24, 0xFF, "{v:?} {state:?}: face");
                    assert_ne!(rim, f.get(4 * d, 10 * d), "{v:?} {state:?}: rim vs face");
                }
                for i in 0..frames.len() {
                    for j in i + 1..frames.len() {
                        assert_ne!(
                            frames[i].pixels, frames[j].pixels,
                            "{v:?} at {d}: {:?} vs {:?}",
                            LIT[i], LIT[j]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn faces_shade_as_the_scheme_says() {
        let luma = |p: u32| ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF);
        for v in Variant::ALL {
            let c = frame(v, State::Checked, 1);
            assert_eq!(c.get(4, 18), 0xFFFF_FFFF, "{v:?} latched face");
            let h = frame(v, State::Hot, 1);
            assert!(
                luma(h.get(4, 2)) > luma(h.get(4, 34)),
                "{v:?} hot greys down"
            );
            assert!(
                luma(h.get(2, 18)) > luma(h.get(7, 18)),
                "{v:?} hot darkens right"
            );
        }
    }

    #[test]
    fn lit_frames_round_their_corners_smoothly() {
        for v in Variant::ALL {
            for state in LIT {
                for d in [1, 2] {
                    let f = frame(v, state, d);
                    let (w, h) = (W * d, H * d);
                    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                        assert!(
                            f.get(x, y) >> 24 <= 8,
                            "{v:?} {state:?} at {d}: corner {x},{y}"
                        );
                    }
                }
                let f = frame(v, state, 2);
                let partial = (0..6)
                    .flat_map(|y| (0..6).map(move |x| (x, y)))
                    .filter(|&(x, y)| (1..255).contains(&(f.get(x, y) >> 24)))
                    .count();
                assert!(partial >= 3, "{v:?} {state:?}: smooth corner at 2");
            }
        }
    }

    #[test]
    fn the_corner_curve_has_no_dark_fringe() {
        let luma = |p: u32| ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF);
        for v in Variant::ALL {
            for state in LIT {
                let f = frame(v, state, 2);
                let rim = luma(f.get(0, 20));
                for (x, y) in [(1, 1), (2, 1), (1, 2), (2, 2), (3, 2), (2, 3)] {
                    let p = f.get(x, y);
                    let a = p >> 24;
                    if a == 0 {
                        continue;
                    }
                    // Brightness against the rim at its alpha, allowing one
                    // level of rounding per premultiplied channel.
                    assert!(
                        (luma(p) + 3) * 255 >= rim * a,
                        "{v:?} {state:?}: ({x}, {y}) {p:08X} darker than the rim"
                    );
                }
            }
        }
    }

    #[test]
    fn the_separator_is_a_one_pixel_line() {
        for v in Variant::ALL {
            let ink = v.scheme().toolbar_separator.normal.rim.argb();
            for d in [1, 2] {
                let img = part(v, Part::ToolbarSeparator, d);
                let inked: Vec<(i32, i32)> = (0..SEP_H * d)
                    .flat_map(|y| (0..SEP_W * d).map(move |x| (x, y)))
                    .filter(|&(x, y)| img.atlas.get(x, y) >> 24 != 0)
                    .collect();
                assert_eq!(inked.len() as i32, d * d, "{v:?} at {d}");
                for (x, y) in inked {
                    assert_eq!(img.atlas.get(x, y), ink, "{v:?} at {d}");
                    assert_eq!((x / d, y / d), (SEP_X, SEP_TOP), "{v:?} at {d}");
                }
            }
        }
    }

    #[test]
    fn the_separator_line_stays_one_column_wide_in_any_slot() {
        use crate::look::styled::fixture::one_part_style;
        let ink = Variant::Blue.scheme().toolbar_separator.normal.rim.argb();
        let style = one_part_style(
            Part::ToolbarSeparator,
            part(Variant::Blue, Part::ToolbarSeparator, 1),
        );
        for (w, h) in [(6, 38), (8, 22), (12, 22)] {
            let mut c = Canvas::new(w as u32, h as u32);
            let r = Rect::new(0, 0, w, h);
            style.draw(&mut c, r, Part::ToolbarSeparator, State::Normal, 1);
            for y in 0..h {
                for x in 0..w {
                    let line = x == SEP_X && (SEP_TOP..h - 1).contains(&y);
                    assert_eq!(c.get(x, y) == ink, line, "{w}x{h} slot ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_label_inks_follow_the_scheme() {
        for v in Variant::ALL {
            let k = &v.scheme().inks;
            let [pressed, disabled, arrow] = inks(v);
            assert_eq!(pressed.2, k.toolbar_pressed, "{v:?}: a held label");
            assert_eq!(disabled.2, k.toolbar_disabled, "{v:?}: a greyed label");
            assert_eq!(
                arrow.2, k.toolbar_drop_disabled,
                "{v:?}: a greyed drop arrow"
            );
        }
    }

    #[test]
    fn toolbar_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in Variant::ALL {
            for d in [1, 2] {
                for p in [
                    Part::ToolbarButton,
                    Part::ToolbarDrop,
                    Part::ToolbarSeparator,
                ] {
                    got.push(format!(
                        "{v:?} {p:?} {d}: {:016x}",
                        fnv(&part(v, p, d).atlas)
                    ));
                }
            }
        }
        let want: &[&str] = &[
            "Blue ToolbarButton 1: ca372671a79bb0eb",
            "Blue ToolbarDrop 1: ca372671a79bb0eb",
            "Blue ToolbarSeparator 1: e73550ce215650fd",
            "Blue ToolbarButton 2: c7c490e92f0143c6",
            "Blue ToolbarDrop 2: c7c490e92f0143c6",
            "Blue ToolbarSeparator 2: 43dd96efbe6320b9",
            "Olive ToolbarButton 1: ca372671a79bb0eb",
            "Olive ToolbarDrop 1: ca372671a79bb0eb",
            "Olive ToolbarSeparator 1: e73550ce215650fd",
            "Olive ToolbarButton 2: c7c490e92f0143c6",
            "Olive ToolbarDrop 2: c7c490e92f0143c6",
            "Olive ToolbarSeparator 2: 43dd96efbe6320b9",
            "Silver ToolbarButton 1: 32cd85e2cf6257ab",
            "Silver ToolbarDrop 1: 32cd85e2cf6257ab",
            "Silver ToolbarSeparator 1: 9eaeb5cb576bb0af",
            "Silver ToolbarButton 2: 3cca1f9377941b5a",
            "Silver ToolbarDrop 2: 3cca1f9377941b5a",
            "Silver ToolbarSeparator 2: efb6750b648b7a01",
            "Slate ToolbarButton 1: 499be67a95d8a2ec",
            "Slate ToolbarDrop 1: 499be67a95d8a2ec",
            "Slate ToolbarSeparator 1: 78f5e2c7fc78980a",
            "Slate ToolbarButton 2: 40a9a5fb12ef0640",
            "Slate ToolbarDrop 2: 40a9a5fb12ef0640",
            "Slate ToolbarSeparator 2: 5fd6553bf4b82169",
            "Sapphire ToolbarButton 1: 7f3b755710236e9c",
            "Sapphire ToolbarDrop 1: 7f3b755710236e9c",
            "Sapphire ToolbarSeparator 1: 78f5e2c7fc78980a",
            "Sapphire ToolbarButton 2: a237dd4f7aa8debf",
            "Sapphire ToolbarDrop 2: a237dd4f7aa8debf",
            "Sapphire ToolbarSeparator 2: 5fd6553bf4b82169",
            "Obsidian ToolbarButton 1: 7f3b755710236e9c",
            "Obsidian ToolbarDrop 1: 7f3b755710236e9c",
            "Obsidian ToolbarSeparator 1: 78f5e2c7fc78980a",
            "Obsidian ToolbarButton 2: a237dd4f7aa8debf",
            "Obsidian ToolbarDrop 2: a237dd4f7aa8debf",
            "Obsidian ToolbarSeparator 2: 5fd6553bf4b82169",
            "Ember ToolbarButton 1: a48fc04f1dea09b5",
            "Ember ToolbarDrop 1: a48fc04f1dea09b5",
            "Ember ToolbarSeparator 1: 78f5e2c7fc78980a",
            "Ember ToolbarButton 2: 83a6ebeb67c30360",
            "Ember ToolbarDrop 2: 83a6ebeb67c30360",
            "Ember ToolbarSeparator 2: 5fd6553bf4b82169",
            "Cobalt ToolbarButton 1: 7f3b755710236e9c",
            "Cobalt ToolbarDrop 1: 7f3b755710236e9c",
            "Cobalt ToolbarSeparator 1: 78f5e2c7fc78980a",
            "Cobalt ToolbarButton 2: a237dd4f7aa8debf",
            "Cobalt ToolbarDrop 2: a237dd4f7aa8debf",
            "Cobalt ToolbarSeparator 2: 5fd6553bf4b82169",
            "Carbon ToolbarButton 1: 7f3b755710236e9c",
            "Carbon ToolbarDrop 1: 7f3b755710236e9c",
            "Carbon ToolbarSeparator 1: 78f5e2c7fc78980a",
            "Carbon ToolbarButton 2: a237dd4f7aa8debf",
            "Carbon ToolbarDrop 2: a237dd4f7aa8debf",
            "Carbon ToolbarSeparator 2: 5fd6553bf4b82169",
            "Steel ToolbarButton 1: f230a442806a406a",
            "Steel ToolbarDrop 1: f230a442806a406a",
            "Steel ToolbarSeparator 1: 9eaeb5cb576bb0af",
            "Steel ToolbarButton 2: c6831e86ad82e662",
            "Steel ToolbarDrop 2: c6831e86ad82e662",
            "Steel ToolbarSeparator 2: efb6750b648b7a01",
        ];
        assert_eq!(got, want);
    }
}
