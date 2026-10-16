//! The combo box drop button's [`Part::ComboDrop`] atlas: Normal, Hot, Pressed and Disabled frames,
//! drawn from the scheme's [`ComboDropFaces`] at the style's scale.

use crate::blue::Variant;
use crate::blue::scheme::{ComboDropFaces, Face, Grad};
use crate::raster::{Canvas, Insets};
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};

const STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Disabled];

#[derive(Clone, Copy, Debug, PartialEq)]
struct Cut {
    w: i32,
    h: i32,
    out: i32,
    rings: usize,
    radius: f64,
    seam: i32,
    chevron: f64,
}

const PLAIN: Cut = Cut {
    w: 15,
    h: 17,
    out: 0,
    rings: 2,
    radius: 1.5,
    seam: 4,
    chevron: 6.0,
};

const FRAMED: Cut = Cut {
    w: 17,
    h: 19,
    out: 1,
    rings: 3,
    radius: 2.0,
    seam: 14,
    chevron: 8.0,
};

const CHEVRON_HALF: f64 = 4.5;
const CHEVRON_DEPTH: f64 = 6.0;
const CHEVRON_ARM: f64 = 3.0;

const GREYED_SHADE: f64 = 0.86;

fn cut(v: Variant) -> Cut {
    match v {
        Variant::Blue | Variant::Olive => PLAIN,
        _ => FRAMED,
    }
}

/// The button's four frames in `v`'s colours at `scale`, stacked one
/// above the other. Sideways only the column just past the left rings
/// stretches; up and down only the cut's seam row.
pub(super) fn parts(v: Variant, scale: i32) -> Vec<(Part, PartImage)> {
    let c = cut(v);
    let atlas = atlas(&v.scheme().combo_drop, c, scale);
    let o = c.out;
    let img = PartImage::new(atlas, &STATES, Layout::Vertical)
        .sizing(Insets::new(2 + o, c.seam, 12 + o, c.h - c.seam - 1))
        .outset(Insets::new(o, o, o, o))
        .at_scale(scale);
    vec![(Part::ComboDrop, img)]
}

fn atlas(faces: &ComboDropFaces, c: Cut, d: i32) -> Canvas {
    let frames = [
        frame(&faces.normal, c, false, d),
        frame(&faces.hot, c, false, d),
        frame(&faces.pressed, c, false, d),
        frame(&faces.disabled, c, c.out > 0, d),
    ];
    let (fw, fh) = (c.w * d, c.h * d);
    shape::draw(c.w, c.h * STATES.len() as i32, d, |x, y| {
        let (x, y) = (x as i32, y as i32);
        frames[(y / fh) as usize][((y % fh) * fw + x) as usize]
    })
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn frame(f: &Face, c: Cut, well: bool, d: i32) -> Vec<Px> {
    let (down, across) = (stops(f.down), stops(f.across));
    let (w, h) = (f64::from(c.w), f64::from(c.h));
    let down = |row: f64| Stops(&down).at((row + 0.5) / h);
    let across = |col: f64| Stops(&across).at((col + 0.5) / w);
    let n = c.rings as f64;
    let body = shape::ringed(c.w, c.h, c.radius, c.rings, d, |x, y, i| {
        if i == c.rings {
            return down(y.clamp(n, h - 1.0 - n));
        }
        let i = i as f64;
        let col = if x < w / 2.0 { i } else { w - 1.0 - i };
        let row = if y < h / 2.0 { i } else { h - 1.0 - i };
        let side = across(col);
        let end = down(row);
        let (dx, dy) = ((x - col).abs(), (y - row).abs());
        if dx < dy {
            side
        } else if dy < dx {
            end
        } else {
            shape::mix(side, end, 0.5)
        }
    });
    let under = shape::premul(f.rim, 255);
    let ink = match f.mark {
        Some(m) => shape::premul(m, 255),
        None => shape::scale(shape::premul(f.inner, 255), GREYED_SHADE),
    };
    let ink = (255.0, ink.1);
    let k = f64::from(d);
    let (m, t) = (f64::from(c.out) + 7.5, c.chevron);
    let (hw, arm, dp) = (CHEVRON_HALF, CHEVRON_ARM, CHEVRON_DEPTH);
    let pts = [
        (m - hw, t),
        (m - (dp - arm), t),
        (m, t + dp - arm),
        (m + (dp - arm), t),
        (m + hw, t),
        (m + hw, t + dp - hw),
        (m, t + dp),
        (m - hw, t + dp - hw),
    ]
    .map(|(x, y)| (x * k, y * k));
    let fw = c.w * d;
    body.into_iter()
        .enumerate()
        .map(|(i, px)| {
            let px = if well { shape::over(px, under) } else { px };
            let (x, y) = ((i as i32 % fw) as f64, (i as i32 / fw) as f64);
            shape::over(shape::scale(ink, shape::polygon(x, y, &pts)), px)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::widgets::draw_combo_drop;
    use crate::pointer::Interaction;
    use crate::raster::Rect;

    fn part(v: Variant, d: i32) -> PartImage {
        parts(v, d).into_iter().next().unwrap().1
    }

    fn px(img: &PartImage, state: State, x: i32, y: i32) -> u32 {
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn ink(v: Variant, state: State) -> u32 {
        let f = match state {
            State::Hot => &v.scheme().combo_drop.hot,
            State::Pressed => &v.scheme().combo_drop.pressed,
            State::Disabled => &v.scheme().combo_drop.disabled,
            _ => &v.scheme().combo_drop.normal,
        };
        let p = match f.mark {
            Some(m) => shape::premul(m, 255),
            None => shape::scale(shape::premul(f.inner, 255), GREYED_SHADE),
        };
        shape::pack((255.0, p.1))
    }

    fn fnv(img: &PartImage) -> u64 {
        let a = &img.atlas;
        let mut bytes = Vec::new();
        bytes.extend(a.width.to_le_bytes());
        bytes.extend(a.height.to_le_bytes());
        for p in &a.pixels {
            bytes.extend(p.to_le_bytes());
        }
        bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    #[test]
    fn combo_drop_pins_at_scale_1_and_2() {
        let got: Vec<(Variant, [u64; 2])> = Variant::ALL
            .into_iter()
            .map(|v| (v, [fnv(&part(v, 1)), fnv(&part(v, 2))]))
            .collect();
        let want: [(Variant, [u64; 2]); 10] = [
            (Variant::Blue, [3779598193147821126, 16455731398898575776]),
            (Variant::Olive, [8204549873019638842, 2223000614753538460]),
            (Variant::Silver, [14545375529455236018, 1260758399300977047]),
            (Variant::Slate, [3166328381793163028, 2656914979319323568]),
            (
                Variant::Sapphire,
                [15655336059566156223, 3625015956340176215],
            ),
            (
                Variant::Obsidian,
                [15655336059566156223, 3625015956340176215],
            ),
            (Variant::Ember, [5319559610858569370, 2405021278666821556]),
            (Variant::Cobalt, [15655336059566156223, 3625015956340176215]),
            (Variant::Carbon, [15655336059566156223, 3625015956340176215]),
            (Variant::Steel, [4422310586742569329, 12701878169192758583]),
        ];
        assert_eq!(got, want, "FNV-1a of (w, h, pixels) at d=1 and d=2");
    }

    #[test]
    fn each_cut_sizes_its_frames_and_insets_by_the_scale() {
        for v in Variant::ALL {
            let c = cut(v);
            let (one, two) = (part(v, 1), part(v, 2));
            for (d, img) in [(1, &one), (2, &two)] {
                assert_eq!(img.scale, d, "{v:?}");
                for state in STATES {
                    let f = img.frame(state);
                    assert_eq!((f.width, f.height), (c.w * d, c.h * d), "{v:?} {state:?}");
                }
            }
            assert_eq!(one.sizing, two.sizing, "{v:?}: design-px insets");
            assert_eq!(one.outset, two.outset, "{v:?}");
            let o = c.out;
            assert_eq!(one.outset, Insets::new(o, o, o, o), "{v:?}");
            assert_eq!(c.w - 2 * o, 15, "{v:?}: 15 px wide on the rect");
        }
    }

    #[test]
    fn every_state_draws_its_own_frame() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let img = part(v, d);
                let frames: Vec<Vec<u32>> = STATES
                    .iter()
                    .map(|&s| {
                        let f = img.frame(s);
                        (0..f.height)
                            .flat_map(|y| (0..f.width).map(move |x| (x, y)))
                            .map(|(x, y)| px(&img, s, x, y))
                            .collect()
                    })
                    .collect();
                for i in 0..4 {
                    for j in i + 1..4 {
                        assert_ne!(frames[i], frames[j], "{v:?} d={d} {i} vs {j}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_corners_are_round_and_smooth_at_scale_2() {
        for v in Variant::ALL {
            let c = cut(v);
            for d in [1, 2] {
                let img = part(v, d);
                for state in [State::Normal, State::Hot, State::Pressed] {
                    let a = px(&img, state, 0, 0) >> 24;
                    assert!(a < 255, "{v:?} d={d} {state:?}: corner alpha {a}");
                }
                let a = px(&img, State::Disabled, 0, 0) >> 24;
                assert_eq!(a == 255, c.out > 0, "{v:?} d={d}: greyed corner alpha {a}");
            }
            let img = part(v, 2);
            let partial = (0..4)
                .flat_map(|y| (0..4).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let a = px(&img, State::Normal, x, y) >> 24;
                    a > 0 && a < 255
                })
                .count();
            assert!(partial >= 2, "{v:?}: AA corner at d=2 ({partial})");
        }
    }

    #[test]
    fn the_chevron_is_solid_at_scale_1_and_smooth_at_scale_2() {
        for v in Variant::ALL {
            let (o, top) = (cut(v).out, cut(v).chevron as i32);
            for state in STATES {
                let ink = ink(v, state);
                let img = part(v, 1);
                for (x, y) in [(4 + o, top + 1), (7 + o, top + 4), (10 + o, top + 1)] {
                    assert_eq!(px(&img, state, x, y), ink, "{v:?} {state:?} ({x}, {y})");
                }
                // At d=2 the arm's lower edge runs corner to corner
                // through device pixel (2o + 8, 2 top + 5): half ink, half
                // face, between the ink above it and the face below.
                let img = part(v, 2);
                let (x, y) = (2 * o + 8, 2 * top + 5);
                let (above, edge, below) = (
                    px(&img, state, x, y - 1),
                    px(&img, state, x, y),
                    px(&img, state, x, y + 1),
                );
                assert_eq!(above, ink, "{v:?} {state:?}: ink above the edge");
                assert!(
                    differs(edge, above) && differs(edge, below),
                    "{v:?} {state:?}: edge {edge:08X} between {above:08X} and {below:08X}"
                );
            }
        }
    }

    fn differs(a: u32, b: u32) -> bool {
        (0..4).any(|i| ((a >> (8 * i)) & 0xFF).abs_diff((b >> (8 * i)) & 0xFF) > 4)
    }

    #[test]
    fn the_seams_carry_no_chevron() {
        for v in Variant::ALL {
            let c = cut(v);
            let img = part(v, 1);
            for state in STATES {
                let ink = ink(v, state);
                let n = c.rings as i32;
                for y in n..c.h - n {
                    assert_ne!(
                        px(&img, state, 2 + c.out, y),
                        ink,
                        "{v:?} {state:?} row {y}"
                    );
                }
                for x in n..c.w - n {
                    assert_ne!(px(&img, state, x, c.seam), ink, "{v:?} {state:?} col {x}");
                }
            }
        }
    }

    #[test]
    fn the_button_lights_under_the_pointer_and_darkens_held() {
        let luma = |p: u32| {
            let (_, c) = shape::unpack(p);
            c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11
        };
        for v in Variant::ALL {
            let o = cut(v).out;
            let img = part(v, 1);
            let at = |s, x, y| luma(px(&img, s, x + o, y + o));
            let (face, rim) = ((7, 3), (-o, 8));
            let normal = (
                at(State::Normal, face.0, face.1),
                at(State::Normal, rim.0, rim.1),
            );
            let hot = (at(State::Hot, face.0, face.1), at(State::Hot, rim.0, rim.1));
            assert!(
                hot.0 > normal.0 + 4.0 || hot.1 < normal.1 - 4.0,
                "{v:?}: hot {hot:?} vs {normal:?}"
            );
            let held = at(State::Pressed, 7, 2).min(at(State::Pressed, 7, 1));
            assert!(held < at(State::Normal, 7, 2), "{v:?}: held top");
        }
    }

    #[test]
    fn a_taller_button_keeps_the_framed_chevron_in_place() {
        let draw = |v: Variant, h: i32| {
            let theme = crate::interfaces::blue_theme(v);
            let mut c = Canvas::new(21, 26);
            draw_combo_drop(
                &mut c,
                &theme.paint(),
                Rect::new(3, 3, 15, h),
                true,
                Interaction::Idle,
                true,
            );
            c
        };
        for v in [
            Variant::Silver,
            Variant::Slate,
            Variant::Sapphire,
            Variant::Obsidian,
            Variant::Ember,
            Variant::Cobalt,
            Variant::Carbon,
        ] {
            let (short, tall) = (draw(v, 17), draw(v, 18));
            // The frame's row `seam` lands at y = 2 + seam.
            for y in 0..2 + FRAMED.seam + 1 {
                for x in 0..21 {
                    assert_eq!(tall.get(x, y), short.get(x, y), "{v:?} ({x}, {y})");
                }
            }
            for y in 2 + FRAMED.seam + 1..26 {
                for x in 0..21 {
                    assert_eq!(tall.get(x, y), short.get(x, y - 1), "{v:?} ({x}, {y})");
                }
            }
        }
        for v in [Variant::Blue, Variant::Olive] {
            let (short, tall) = (draw(v, 17), draw(v, 18));
            assert_eq!(
                tall.get(10, 13),
                short.get(10, 12),
                "{v:?}: the point moves"
            );
            assert_eq!(tall.get(10, 6), short.get(10, 6), "{v:?}: the top stays");
        }
    }

    #[test]
    fn a_framed_button_covers_the_well_round_its_rect() {
        let theme = crate::interfaces::blue_theme(Variant::Silver);
        let under = crate::theme::Color::rgb(1, 2, 3);
        for (h, enabled, ptr) in [
            (17, true, Interaction::Idle),
            (18, true, Interaction::Hot),
            (17, false, Interaction::Idle),
        ] {
            let r = Rect::new(3, 3, 15, h);
            let mut c = Canvas::new(21, 25);
            c.fill_rect(c.bounds(), under);
            draw_combo_drop(&mut c, &theme.paint(), r, enabled, ptr, true);
            let outer = r.inset(-1);
            for y in 0..25 {
                for x in 0..21 {
                    if !outer.contains(x, y) {
                        assert_eq!(c.get(x, y), under.argb(), "{h} {enabled}: ({x}, {y})");
                    }
                }
            }
            if !enabled {
                assert_eq!(c.get(2, 2), 0xFFFF_FFFF, "the well's white in the corner");
                assert_eq!(c.get(2, 10), 0xFFFF_FFFF, "the well's white on the left");
            }
        }
    }
}
