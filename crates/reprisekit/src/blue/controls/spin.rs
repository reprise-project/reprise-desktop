//! The spinner's [`Part::SpinUp`] and [`Part::SpinDown`] atlases: four frames each (Normal, Hot,
//! Pressed, Disabled), drawn from shapes at the style's scale in the colours of the scheme's spin faces.
//!
//! Each button is a 15x8 box, corners rounded over 1.5 px: a 1 px rim, a 1 px ring inside it, and a
//! fill shaded top to bottom. The top and bottom rows of both rings take the face's `down` colours for
//! their row, the side columns its `across` colours for their column, so a scheme's highlight row or
//! white gap row (Slate's, where the two buttons meet) comes from its face. A chevron 8 px wide and 4
//! tall sits over the middle, its arms meeting at a point with a notch cut under it; its 45-degree
//! edges run through pixel corners, so at scale 1 every core pixel is solid ink with a soft edge
//! outside it. The down button's chevron is the up one turned over.
//!
//! A spinner attached to an edit, or beside a year field's taller box, lays its button out wider or
//! taller: column 3, clear of the chevron, is the only stretchable column, and one plain row (row 1, or
//! row 6 in Slate's down button) the only stretchable row, so no size repeats a slice of the chevron.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::{Canvas, Insets};
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};

const W: i32 = 15;
const H: i32 = 8;

const CORNER: f64 = 1.5;

/// The chevron, pointing up, design px: a flat top one pixel wide, arms down at 45 degrees to the
/// bottom row, the sides cut upright at columns 4 and 11 so the stretchable column 3 stays clear.
const CHEVRON: [(f64, f64); 6] = [
    (7.0, 2.0),
    (8.0, 2.0),
    (11.0, 5.0),
    (11.0, 6.0),
    (4.0, 6.0),
    (4.0, 5.0),
];

const NOTCH: [(f64, f64); 3] = [(7.5, 4.6), (8.9, 6.0), (6.1, 6.0)];

/// How much of the rim's colour a greyed chevron keeps when its face names no ink: the rim darkened a little.
const GREYED_INK: f64 = 0.86;

const STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Disabled];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    Up,
    Down,
}

pub(super) fn parts(v: Variant, scale: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let up = [
        &s.spin_up.normal,
        &s.spin_up.hot,
        &s.spin_up.pressed,
        &s.spin_up.disabled,
    ];
    let down = [
        &s.spin_down.normal,
        &s.spin_down.hot,
        &s.spin_down.pressed,
        &s.spin_down.disabled,
    ];
    vec![
        (Part::SpinUp, button(up, Dir::Up, 1, scale)),
        (Part::SpinDown, button(down, Dir::Down, seam_down(v), scale)),
    ]
}

/// The down button's stretchable row: 6, the bottom of its face, in Slate (whose down button opens with
/// the white row the two buttons share); row 1 everywhere else.
fn seam_down(v: Variant) -> i32 {
    match v {
        Variant::Slate => 6,
        _ => 1,
    }
}

/// One button's four frames stacked top to bottom at `d`, with column 3 and row `seam` its only stretchable seams.
fn button(faces: [&Face; 4], dir: Dir, seam: i32, d: i32) -> PartImage {
    let mut atlas = Canvas::new((W * d) as u32, (H * d * 4) as u32);
    for (i, face) in faces.iter().enumerate() {
        atlas.blit_over(0, i as i32 * H * d, &frame(face, dir, d));
    }
    PartImage::new(atlas, &STATES, Layout::Vertical)
        .sizing(Insets::new(3, seam, 11, H - seam - 1))
        .at_scale(d)
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn at_pixel(g: &[(f64, Px)], i: f64, n: i32) -> Px {
    Stops(g).at((i.floor() + 0.5) / f64::from(n))
}

fn frame(face: &Face, dir: Dir, d: i32) -> Canvas {
    let (down, across) = (stops(face.down), stops(face.across));
    let ink = shape::premul(face.mark.unwrap_or(greyed(face)), 255);
    let k = f64::from(d);
    let body = shape::ringed(W, H, CORNER, 2, d, |x, y, ring| {
        let (row, col) = (y.round(), x.round());
        let level = row < 0.5 + ring as f64 || row > f64::from(H) - 1.5 - ring as f64;
        match ring {
            // The rings' top and bottom rows take their row's colour down the face, their sides their column's across it.
            0 | 1 if level => at_pixel(&down, row, H),
            0 | 1 => at_pixel(&across, col, W),
            _ => Stops(&down).at((y + 0.5) / f64::from(H)),
        }
    });
    let pts = |p: &[(f64, f64)]| -> Vec<(f64, f64)> {
        p.iter()
            .map(|&(x, y)| match dir {
                Dir::Up => (x * k, y * k),
                Dir::Down => (x * k, (f64::from(H) - y) * k),
            })
            .collect()
    };
    let (chevron, notch) = (pts(&CHEVRON), pts(&NOTCH));
    shape::draw(W, H, d, |x, y| {
        let under = body[(y as i32 * W * d + x as i32) as usize];
        let cover = (shape::polygon(x, y, &chevron) - shape::polygon(x, y, &notch)).max(0.0);
        shape::over(shape::scale(ink, cover), under)
    })
}

fn greyed(face: &Face) -> crate::theme::Color {
    let c = face.rim;
    let f = |v: u8| (f64::from(v) * GREYED_INK).round() as u8;
    crate::theme::Color::rgb(f(c.r), f(c.g), f(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rect;

    fn img(v: Variant, part: Part, d: i32) -> PartImage {
        parts(v, d).into_iter().find(|(p, _)| *p == part).unwrap().1
    }

    fn px(img: &PartImage, state: State, x: i32, y: i32) -> u32 {
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    fn fnv(img: &PartImage, state: State) -> u64 {
        let f = img.frame(state);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        eat(f.width as u32);
        eat(f.height as u32);
        for y in 0..f.height {
            for x in 0..f.width {
                eat(img.atlas.get(f.x + x, f.y + y));
            }
        }
        h
    }

    const PARTS: [Part; 2] = [Part::SpinUp, Part::SpinDown];

    #[test]
    fn each_frame_is_15_by_8_design_px_at_every_scale() {
        for v in Variant::ALL {
            for part in PARTS {
                for d in [1, 2] {
                    let img = img(v, part, d);
                    assert_eq!(img.scale, d);
                    for state in STATES {
                        let f = img.frame(state);
                        assert_eq!((f.width, f.height), (W * d, H * d), "{v:?} {part:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_seams_are_the_same_design_insets_at_scale_2() {
        for v in Variant::ALL {
            for part in PARTS {
                assert_eq!(img(v, part, 1).sizing, img(v, part, 2).sizing, "{v:?}");
            }
        }
    }

    #[test]
    fn every_state_draws_its_own_frame() {
        for v in Variant::ALL {
            for part in PARTS {
                for d in [1, 2] {
                    let img = img(v, part, d);
                    for (i, a) in STATES.iter().enumerate() {
                        for b in &STATES[i + 1..] {
                            assert_ne!(fnv(&img, *a), fnv(&img, *b), "{v:?} {part:?} {a:?} {b:?}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_corners_are_rounded_clear_and_soft_at_scale_2() {
        for v in Variant::ALL {
            for part in PARTS {
                for state in STATES {
                    let one = img(v, part, 1);
                    assert!(
                        alpha(px(&one, state, 0, 0)) < 255,
                        "{v:?} {part:?} {state:?}"
                    );
                    assert_eq!(
                        alpha(px(&one, state, 7, 0)),
                        255,
                        "{v:?} {part:?} {state:?}"
                    );
                    let two = img(v, part, 2);
                    assert!(
                        alpha(px(&two, state, 0, 0)) < alpha(px(&one, state, 0, 0)),
                        "{v:?} {part:?} {state:?}: the 2x corner pixel is fainter"
                    );
                    let soft = (0..9).any(|i| {
                        let a = alpha(px(&two, state, i % 3, i / 3));
                        a > 0 && a < 255
                    });
                    assert!(
                        soft,
                        "{v:?} {part:?} {state:?}: the 2x corner is anti-aliased"
                    );
                }
            }
        }
    }

    fn solid_ink(img: &PartImage, state: State, ink: u32) -> Vec<(i32, i32)> {
        let f = img.frame(state);
        (0..f.height)
            .flat_map(|y| (0..f.width).map(move |x| (x, y)))
            .filter(|&(x, y)| px(img, state, x, y) == ink)
            .collect()
    }

    fn ink(v: Variant, part: Part, state: State) -> u32 {
        let s = v.scheme();
        let face = match (part, state) {
            (Part::SpinUp, State::Normal) => &s.spin_up.normal,
            (Part::SpinUp, State::Hot) => &s.spin_up.hot,
            (Part::SpinUp, State::Pressed) => &s.spin_up.pressed,
            (Part::SpinUp, _) => &s.spin_up.disabled,
            (_, State::Normal) => &s.spin_down.normal,
            (_, State::Hot) => &s.spin_down.hot,
            (_, State::Pressed) => &s.spin_down.pressed,
            _ => &s.spin_down.disabled,
        };
        face.mark.unwrap_or(greyed(face)).argb()
    }

    /// At scale 1 both arms and the point hold solid ink: the chevron never thins to a grey smudge.
    #[test]
    fn the_chevron_is_solid_ink_at_scale_1() {
        for v in Variant::ALL {
            for part in PARTS {
                let img = img(v, part, 1);
                for state in STATES {
                    let hits = solid_ink(&img, state, ink(v, part, state));
                    let (point, left, right) = match part {
                        Part::SpinUp => ((7, 2), (4, 5), (10, 5)),
                        _ => ((7, 5), (4, 2), (10, 2)),
                    };
                    for p in [point, left, right] {
                        assert!(hits.contains(&p), "{v:?} {part:?} {state:?} {p:?}");
                    }
                    let notch = (7, 7 - point.1);
                    assert!(!hits.contains(&notch), "{v:?} {part:?} {state:?} notch");
                }
            }
        }
    }

    #[test]
    fn the_up_and_down_chevrons_point_opposite_ways() {
        for v in Variant::ALL {
            let (up, down) = (img(v, Part::SpinUp, 1), img(v, Part::SpinDown, 1));
            let (iu, id) = (
                ink(v, Part::SpinUp, State::Normal),
                ink(v, Part::SpinDown, State::Normal),
            );
            let u = solid_ink(&up, State::Normal, iu);
            let dn = solid_ink(&down, State::Normal, id);
            assert!(u.contains(&(7, 2)) && !u.contains(&(4, 2)), "{v:?} up");
            assert!(dn.contains(&(7, 5)) && !dn.contains(&(4, 5)), "{v:?} down");
        }
    }

    /// The stretchable column 3 and row carry no ink in any state, so a wider or taller button never repeats the chevron.
    #[test]
    fn the_seams_stay_clear_of_the_chevron() {
        for v in Variant::ALL {
            for part in PARTS {
                for d in [1, 2] {
                    let img = img(v, part, d);
                    let seam = img.sizing.top;
                    for state in STATES {
                        let f = img.frame(state);
                        for y in 0..H * d {
                            for x in 3 * d..4 * d {
                                assert_eq!(
                                    px(&img, state, x, y),
                                    px(&img, state, 2 * d, y),
                                    "{v:?} {part:?} {state:?} column 3 ({x}, {y})"
                                );
                            }
                        }
                        for y in seam * d..(seam + 1) * d {
                            let row: Vec<u32> = (2 * d..(W - 2) * d)
                                .map(|x| px(&img, state, x, y))
                                .collect();
                            assert!(
                                row.windows(2).all(|w| w[0] == w[1]),
                                "{v:?} {part:?} {state:?} d {d}: seam row {seam} is plain, {f:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The rim, ring and fill share their edge pixels without darkening them: round every corner, each
    /// part-covered pixel's colour (alpha taken out) is no darker than the darkest its face shades the box with.
    #[test]
    fn the_rounded_corners_leave_no_dark_fringe() {
        let luma = |c: crate::theme::Color| u32::from(c.r) + u32::from(c.g) + u32::from(c.b);
        let face_of = |v: Variant, part: Part, state: State| {
            let s = v.scheme();
            let f = match part {
                Part::SpinUp => [
                    &s.spin_up.normal,
                    &s.spin_up.hot,
                    &s.spin_up.pressed,
                    &s.spin_up.disabled,
                ],
                _ => [
                    &s.spin_down.normal,
                    &s.spin_down.hot,
                    &s.spin_down.pressed,
                    &s.spin_down.disabled,
                ],
            };
            *f[STATES.iter().position(|&t| t == state).unwrap()]
        };
        for v in Variant::ALL {
            for part in PARTS {
                for d in [1, 2] {
                    let img = img(v, part, d);
                    for state in STATES {
                        let face = face_of(v, part, state);
                        let darkest = face
                            .down
                            .iter()
                            .chain(face.across)
                            .map(|&(_, c)| luma(c))
                            .min()
                            .unwrap();
                        let f = img.frame(state);
                        let corners = [
                            (0, 0),
                            (f.width - 2 * d, 0),
                            (0, f.height - 2 * d),
                            (f.width - 2 * d, f.height - 2 * d),
                        ];
                        for (cx, cy) in corners {
                            for y in cy..cy + 2 * d {
                                for x in cx..cx + 2 * d {
                                    let p = px(&img, state, x, y);
                                    let a = alpha(p);
                                    if a < 32 || a == 255 {
                                        continue;
                                    }
                                    let ch = |s: u32| ((p >> s) & 0xFF) * 255 / a;
                                    assert!(
                                        ch(16) + ch(8) + ch(0) + 3 >= darkest,
                                        "{v:?} {part:?} {state:?} d {d} ({x}, {y}) {p:08x} darkest {darkest}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_taller_slate_down_button_stretches_the_bottom_of_its_face() {
        let draw = |v: Variant, part: Part, h: i32| {
            let mut c = Canvas::new(W as u32, 12);
            crate::blue::style_for(v).draw(&mut c, Rect::new(0, 0, W, h), part, State::Normal, 1);
            c
        };
        for (v, part, seam) in [
            (Variant::Slate, Part::SpinDown, 6),
            (Variant::Slate, Part::SpinUp, 1),
            (Variant::Sapphire, Part::SpinDown, 1),
            (Variant::Ember, Part::SpinDown, 1),
            (Variant::Cobalt, Part::SpinDown, 1),
        ] {
            let (short, tall) = (draw(v, part, H), draw(v, part, H + 2));
            for x in 0..W {
                for y in 0..=seam {
                    assert_eq!(tall.get(x, y), short.get(x, y), "{v:?} {part:?} ({x}, {y})");
                }
                for y in seam + 1..seam + 3 {
                    assert_eq!(
                        tall.get(x, y),
                        short.get(x, seam),
                        "{v:?} {part:?} ({x}, {y})"
                    );
                }
                for y in seam + 3..H + 2 {
                    assert_eq!(
                        tall.get(x, y),
                        short.get(x, y - 2),
                        "{v:?} {part:?} ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn spin_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in Variant::ALL {
            for part in PARTS {
                for d in [1, 2] {
                    let img = img(v, part, d);
                    for state in STATES {
                        got.push(format!(
                            "{v:?} {part:?} {state:?} {d} {:016x}",
                            fnv(&img, state)
                        ));
                    }
                }
            }
        }
        let want: &[&str] = &[
            "Blue SpinUp Normal 1 03e6e84875544c99",
            "Blue SpinUp Hot 1 2387a0bd6afee4f1",
            "Blue SpinUp Pressed 1 d1cde92cb79bd5a4",
            "Blue SpinUp Disabled 1 3248259d93154d6c",
            "Blue SpinUp Normal 2 be433b4c9efabb91",
            "Blue SpinUp Hot 2 5f16aec800ec43dc",
            "Blue SpinUp Pressed 2 2938edb47d18ca70",
            "Blue SpinUp Disabled 2 8a96e230b752355b",
            "Blue SpinDown Normal 1 52898e9ec18b70b1",
            "Blue SpinDown Hot 1 7c3a027c585c0195",
            "Blue SpinDown Pressed 1 dac9ed3d8f512611",
            "Blue SpinDown Disabled 1 00adebd40a8d2997",
            "Blue SpinDown Normal 2 a8ea2dc1d25a3702",
            "Blue SpinDown Hot 2 f2a5a2d4ffdbf5c5",
            "Blue SpinDown Pressed 2 bd1c6800ccd53455",
            "Blue SpinDown Disabled 2 90f08efd79f80bb7",
            "Olive SpinUp Normal 1 fb687f434234f193",
            "Olive SpinUp Hot 1 6aaf27e4fbb1b98d",
            "Olive SpinUp Pressed 1 1dca35eb7ef74b0b",
            "Olive SpinUp Disabled 1 3248259d93154d6c",
            "Olive SpinUp Normal 2 60787f63ed5fd440",
            "Olive SpinUp Hot 2 7fd8dea7bddf9035",
            "Olive SpinUp Pressed 2 94e6fe0ff8e8039e",
            "Olive SpinUp Disabled 2 8a96e230b752355b",
            "Olive SpinDown Normal 1 81aef30492eb389d",
            "Olive SpinDown Hot 1 0f0db53a9bdfe186",
            "Olive SpinDown Pressed 1 709d805402689211",
            "Olive SpinDown Disabled 1 00adebd40a8d2997",
            "Olive SpinDown Normal 2 1c2986011555dbcd",
            "Olive SpinDown Hot 2 3b3d34345518db87",
            "Olive SpinDown Pressed 2 8ee686fa014e5d14",
            "Olive SpinDown Disabled 2 90f08efd79f80bb7",
            "Silver SpinUp Normal 1 786a92064af50939",
            "Silver SpinUp Hot 1 d0780bfe93dda67b",
            "Silver SpinUp Pressed 1 52e5f0a754994547",
            "Silver SpinUp Disabled 1 745ef6793c94aedd",
            "Silver SpinUp Normal 2 5b8487a702b95c08",
            "Silver SpinUp Hot 2 a418fde1f77ea6da",
            "Silver SpinUp Pressed 2 f9aa886922e62f6e",
            "Silver SpinUp Disabled 2 3ad8da5af05fe14b",
            "Silver SpinDown Normal 1 c51fd6f402da0fbf",
            "Silver SpinDown Hot 1 5ab77eb7b9740e84",
            "Silver SpinDown Pressed 1 a31fa7a83aade87c",
            "Silver SpinDown Disabled 1 00adebd40a8d2997",
            "Silver SpinDown Normal 2 9a5bded16dc2fbff",
            "Silver SpinDown Hot 2 9931fa201db1f7b0",
            "Silver SpinDown Pressed 2 0d7abda99c10bd8a",
            "Silver SpinDown Disabled 2 90f08efd79f80bb7",
            "Slate SpinUp Normal 1 461ec2567adc02f6",
            "Slate SpinUp Hot 1 e21d15465a20b468",
            "Slate SpinUp Pressed 1 38e496dc4c970618",
            "Slate SpinUp Disabled 1 82f76e5909d2649c",
            "Slate SpinUp Normal 2 2980b0ef0b7b5f5d",
            "Slate SpinUp Hot 2 3add9f29930ca0e8",
            "Slate SpinUp Pressed 2 924db94c6afc4ad3",
            "Slate SpinUp Disabled 2 2fb9527c5b31e443",
            "Slate SpinDown Normal 1 05f59b1fa56faa07",
            "Slate SpinDown Hot 1 64d566782ebcde23",
            "Slate SpinDown Pressed 1 5431041a8c07793a",
            "Slate SpinDown Disabled 1 f827ea1db41ff70c",
            "Slate SpinDown Normal 2 54e4e10d73b389b5",
            "Slate SpinDown Hot 2 577a58be72149fd4",
            "Slate SpinDown Pressed 2 e22cd1d71d9764ef",
            "Slate SpinDown Disabled 2 6e9ef254cd51c503",
            "Sapphire SpinUp Normal 1 f2eb90198b194ae6",
            "Sapphire SpinUp Hot 1 61e0f078a005b0e7",
            "Sapphire SpinUp Pressed 1 d8005c71b061239c",
            "Sapphire SpinUp Disabled 1 7538ca7b3592986b",
            "Sapphire SpinUp Normal 2 0d04a01046f5988a",
            "Sapphire SpinUp Hot 2 5f5fa0cd5dd0ca30",
            "Sapphire SpinUp Pressed 2 134b1ad09c0be932",
            "Sapphire SpinUp Disabled 2 1d2b33e7f4141bde",
            "Sapphire SpinDown Normal 1 dd2ddeb7b9e45198",
            "Sapphire SpinDown Hot 1 80892ec66a1d2143",
            "Sapphire SpinDown Pressed 1 e15db21b3d0d7b66",
            "Sapphire SpinDown Disabled 1 7f3edc081a519fd5",
            "Sapphire SpinDown Normal 2 05d84b0fc1cb6de6",
            "Sapphire SpinDown Hot 2 e54e7e7f8db59a43",
            "Sapphire SpinDown Pressed 2 3f9c90da12a4c9ba",
            "Sapphire SpinDown Disabled 2 d4c8e1f9afdfc221",
            "Obsidian SpinUp Normal 1 f2eb90198b194ae6",
            "Obsidian SpinUp Hot 1 61e0f078a005b0e7",
            "Obsidian SpinUp Pressed 1 d8005c71b061239c",
            "Obsidian SpinUp Disabled 1 7538ca7b3592986b",
            "Obsidian SpinUp Normal 2 0d04a01046f5988a",
            "Obsidian SpinUp Hot 2 5f5fa0cd5dd0ca30",
            "Obsidian SpinUp Pressed 2 134b1ad09c0be932",
            "Obsidian SpinUp Disabled 2 1d2b33e7f4141bde",
            "Obsidian SpinDown Normal 1 dd2ddeb7b9e45198",
            "Obsidian SpinDown Hot 1 80892ec66a1d2143",
            "Obsidian SpinDown Pressed 1 e15db21b3d0d7b66",
            "Obsidian SpinDown Disabled 1 7f3edc081a519fd5",
            "Obsidian SpinDown Normal 2 05d84b0fc1cb6de6",
            "Obsidian SpinDown Hot 2 e54e7e7f8db59a43",
            "Obsidian SpinDown Pressed 2 3f9c90da12a4c9ba",
            "Obsidian SpinDown Disabled 2 d4c8e1f9afdfc221",
            "Ember SpinUp Normal 1 753b96820315af6d",
            "Ember SpinUp Hot 1 5897692f0995dd2e",
            "Ember SpinUp Pressed 1 59c4d71292bb9108",
            "Ember SpinUp Disabled 1 e05dd7fcffa63ff2",
            "Ember SpinUp Normal 2 b19a2aeb4372851c",
            "Ember SpinUp Hot 2 adc0c33536862b8a",
            "Ember SpinUp Pressed 2 a796274fa83fe942",
            "Ember SpinUp Disabled 2 57c81ad55b99aef9",
            "Ember SpinDown Normal 1 72ca5233161b55d6",
            "Ember SpinDown Hot 1 a74fd149e8bf0bb9",
            "Ember SpinDown Pressed 1 b320223a085703e0",
            "Ember SpinDown Disabled 1 0abf9cbc4b4e52c3",
            "Ember SpinDown Normal 2 129b872d1bc2164b",
            "Ember SpinDown Hot 2 bcc44137fc3f8aed",
            "Ember SpinDown Pressed 2 27c46b0544205bfd",
            "Ember SpinDown Disabled 2 213fd73dfaa280c8",
            "Cobalt SpinUp Normal 1 f2eb90198b194ae6",
            "Cobalt SpinUp Hot 1 61e0f078a005b0e7",
            "Cobalt SpinUp Pressed 1 d8005c71b061239c",
            "Cobalt SpinUp Disabled 1 7538ca7b3592986b",
            "Cobalt SpinUp Normal 2 0d04a01046f5988a",
            "Cobalt SpinUp Hot 2 5f5fa0cd5dd0ca30",
            "Cobalt SpinUp Pressed 2 134b1ad09c0be932",
            "Cobalt SpinUp Disabled 2 1d2b33e7f4141bde",
            "Cobalt SpinDown Normal 1 dd2ddeb7b9e45198",
            "Cobalt SpinDown Hot 1 80892ec66a1d2143",
            "Cobalt SpinDown Pressed 1 e15db21b3d0d7b66",
            "Cobalt SpinDown Disabled 1 7f3edc081a519fd5",
            "Cobalt SpinDown Normal 2 05d84b0fc1cb6de6",
            "Cobalt SpinDown Hot 2 e54e7e7f8db59a43",
            "Cobalt SpinDown Pressed 2 3f9c90da12a4c9ba",
            "Cobalt SpinDown Disabled 2 d4c8e1f9afdfc221",
            "Carbon SpinUp Normal 1 f2eb90198b194ae6",
            "Carbon SpinUp Hot 1 61e0f078a005b0e7",
            "Carbon SpinUp Pressed 1 d8005c71b061239c",
            "Carbon SpinUp Disabled 1 7538ca7b3592986b",
            "Carbon SpinUp Normal 2 0d04a01046f5988a",
            "Carbon SpinUp Hot 2 5f5fa0cd5dd0ca30",
            "Carbon SpinUp Pressed 2 134b1ad09c0be932",
            "Carbon SpinUp Disabled 2 1d2b33e7f4141bde",
            "Carbon SpinDown Normal 1 dd2ddeb7b9e45198",
            "Carbon SpinDown Hot 1 80892ec66a1d2143",
            "Carbon SpinDown Pressed 1 e15db21b3d0d7b66",
            "Carbon SpinDown Disabled 1 7f3edc081a519fd5",
            "Carbon SpinDown Normal 2 05d84b0fc1cb6de6",
            "Carbon SpinDown Hot 2 e54e7e7f8db59a43",
            "Carbon SpinDown Pressed 2 3f9c90da12a4c9ba",
            "Carbon SpinDown Disabled 2 d4c8e1f9afdfc221",
        ];
        assert_eq!(got, want);
    }
}
