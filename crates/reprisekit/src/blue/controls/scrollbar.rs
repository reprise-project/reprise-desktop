//! Blue's scrollbar parts, drawn from the scheme's `scroll_*` faces at the output's scale. The bar is 17 px thick; every part draws as a nine-grid whose one stretchable seam is a plain column and row, so a bar laid out 16 px thick drops just that seam.

use std::f64::consts::SQRT_2;

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::{Canvas, Insets};
use crate::shape::{self, CLEAR, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};

/// The bar's thickness and an arrow button's length, design px.
const THICK: i32 = 17;
const THUMB: i32 = 22;
const GRIP: (i32, i32) = (7, 8);
const RADIUS: f64 = 2.0;
const RINGS: usize = 2;
/// The chevron of an upright up arrow, a line through its middle in design px: a 45° bend at column 8's middle.
const CHEVRON: [(f64, f64); 3] = [(5.0, 10.0), (8.5, 6.5), (12.0, 10.0)];
/// The chevron's width square to its run: 1.5 px level, so a row is 3 px across with a half-covered pixel at each side.
const CHEVRON_WIDTH: f64 = 1.5 * SQRT_2;
const ARROW_SEAM: (i32, i32) = (3, 3);
const THUMB_SEAM: (i32, i32) = (7, 11);

const STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Disabled];

const TRACK_STATES: [State; 3] = [State::Normal, State::Pressed, State::Disabled];

/// Which way a part lies: upright runs down a vertical bar, lying along a horizontal one. Every part is drawn upright and turned.
#[derive(Clone, Copy, PartialEq)]
enum Lie {
    Upright,
    Down,
}

/// A face's two shadings for an upright part: `side` across the bar, `along` down it.
struct Shading {
    side: Vec<(f64, Px)>,
    along: Vec<(f64, Px)>,
}

impl Shading {
    fn of(face: &Face, lie: Lie) -> Shading {
        let (side, along) = match lie {
            Lie::Upright => (face.across, face.down),
            Lie::Down => (face.down, face.across),
        };
        Shading {
            side: stops(side),
            along: stops(along),
        }
    }

    fn side(&self, p: f64, n: i32) -> Px {
        Stops(&self.side).at((p + 0.5) / f64::from(n))
    }

    fn along(&self, p: f64, n: i32) -> Px {
        Stops(&self.along).at((p + 0.5) / f64::from(n))
    }

    fn darkest(&self) -> Px {
        let luma = |p: &Px| 0.3 * p.1[0] + 0.59 * p.1[1] + 0.11 * p.1[2];
        self.side
            .iter()
            .chain(&self.along)
            .map(|&(_, p)| p)
            .min_by(|a, b| luma(a).total_cmp(&luma(b)))
            .unwrap_or(CLEAR)
    }
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

/// `a` plus `b` less `base`, per channel, held to 0..=255: two shadings laid one over the other from where they cross.
fn sum(a: Px, b: Px, base: Px) -> Px {
    (
        255.0,
        std::array::from_fn(|i| (a.1[i] + b.1[i] - base.1[i]).clamp(0.0, 255.0)),
    )
}

/// A button `len` design px long, shaded by `sh` and turned to `lie`: track edge in column 0, rounded button over columns 1..17, and `face`'s chevron up (`Some(false)`), down (`Some(true)`) or none.
fn button(
    face: &Face,
    sh: &Shading,
    lie: Lie,
    len: i32,
    seam: (i32, i32),
    chevron: Option<bool>,
    d: i32,
) -> Canvas {
    let w = THICK - 1;
    let edge = sh.side(0.0, THICK);
    let base = shape::mix(
        sh.side(f64::from(seam.0), THICK),
        sh.along(f64::from(seam.1), len),
        0.5,
    );
    let (last_x, last_y) = (f64::from(w - 1), f64::from(len - 1));
    let px = shape::ringed(w, len, RADIUS, RINGS, d, |x, y, i| {
        if i == RINGS {
            return sum(sh.side(x + 1.0, THICK), sh.along(y, len), base);
        }
        let ring = i as f64;
        // A ring pixel takes the colour of the side it lies nearest; a corner pixel, half of each.
        let (across, from_side) = if x < last_x - x {
            (sh.side(1.0 + ring, THICK), x)
        } else {
            (sh.side(f64::from(w) - ring, THICK), last_x - x)
        };
        let (down, from_end) = if y < last_y - y {
            (sh.along(ring, len), y)
        } else {
            (sh.along(last_y - ring, len), last_y - y)
        };
        match from_side - from_end {
            t if t.abs() < 0.5 => shape::mix(across, down, 0.5),
            t if t < 0.0 => across,
            _ => down,
        }
    });
    let ink = face
        .mark
        .map(|c| shape::premul(c, 255))
        .unwrap_or_else(|| sh.darkest());
    let k = f64::from(d);
    let line: Option<Vec<(f64, f64)>> = chevron.map(|flip| {
        CHEVRON
            .iter()
            .map(|&(x, y)| (x * k, if flip { f64::from(len) - y } else { y } * k))
            .collect()
    });
    let upright = shape::draw(THICK, len, d, |x, y| {
        let bx = x as i32 - d;
        let under = if bx < 0 {
            edge
        } else {
            shape::over(px[(y as i32 * w * d + bx) as usize], edge)
        };
        match &line {
            Some(pts) => shape::over(
                shape::scale(ink, shape::stroke(x, y, pts, CHEVRON_WIDTH * k)),
                under,
            ),
            None => under,
        }
    });
    turn(upright, lie)
}

/// A grip upright (7 across, 8 along): four ridges, a light line over columns 0..6 above a dark one over columns 1..7.
fn grip(face: Option<&Face>, lie: Lie, d: i32) -> Canvas {
    let (w, h) = GRIP;
    let Some(face) = face else {
        return turn(Canvas::new((w * d) as u32, (h * d) as u32), lie);
    };
    let sh = Shading::of(face, lie);
    let (light, dark) = (sh.along(0.0, h), sh.along(1.0, h));
    let upright = shape::draw(w, h, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        match (row % 2, col) {
            (0, c) if c < w - 1 => light,
            (1, c) if c >= 1 => dark,
            _ => CLEAR,
        }
    });
    turn(upright, lie)
}

/// A track upright (17 across, 1 along): the face's fade across the bar, its edge columns held whole.
fn track(face: &Face, lie: Lie, d: i32) -> Canvas {
    let sh = Shading::of(face, lie);
    let upright = shape::draw(THICK, 1, d, |x, _| {
        let col = x as i32 / d;
        let p = if col == 0 || col == THICK - 1 {
            f64::from(col)
        } else {
            shape::centre(x as i32, d).clamp(1.0, f64::from(THICK - 2))
        };
        sh.side(p, THICK)
    });
    turn(upright, lie)
}

fn turn(c: Canvas, lie: Lie) -> Canvas {
    if lie == Lie::Upright {
        return c;
    }
    let mut t = Canvas::new(c.height, c.width);
    for y in 0..c.height as i32 {
        for x in 0..c.width as i32 {
            t.put_raw(y, x, c.get(x, y));
        }
    }
    t
}

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let up = &s.scroll_arrow_up;
    let down = &s.scroll_arrow_down;
    let arrow = |faces: [&Face; 4], greyed: &Face, lie: Lie, flip: bool| {
        let mut frames: Vec<Canvas> = faces
            .iter()
            .map(|f| {
                button(
                    f,
                    &Shading::of(f, lie),
                    lie,
                    THICK,
                    ARROW_SEAM,
                    Some(flip),
                    d,
                )
            })
            .collect();
        if lie == Lie::Down && faces[3] == faces[0] {
            // No greyed face of its own: grey as the upright arrow does.
            let sh = Shading::of(greyed, Lie::Upright);
            frames[3] = button(greyed, &sh, lie, THICK, ARROW_SEAM, Some(flip), d);
        }
        atlas(&frames, &STATES, d).sizing(Insets::new(3, 3, 13, 13))
    };
    let thumb = |faces: [&Face; 4], lie: Lie| {
        let frames =
            faces.map(|f| button(f, &Shading::of(f, lie), lie, THUMB, THUMB_SEAM, None, d));
        atlas(&frames, &STATES, d)
    };
    let left = &s.scroll_arrow_left;
    let right = &s.scroll_arrow_right;
    let (tv, th) = (&s.scroll_thumb_v, &s.scroll_thumb_h);
    let (gv, gh) = (&s.scroll_grip_v, &s.scroll_grip_h);
    let (kv, kh) = (&s.scroll_track_v, &s.scroll_track_h);
    let grips =
        |faces: [Option<&Face>; 4], lie: Lie| atlas(&faces.map(|f| grip(f, lie, d)), &STATES, d);
    let tracks =
        |faces: [&Face; 3], lie: Lie| atlas(&faces.map(|f| track(f, lie, d)), &TRACK_STATES, d);
    vec![
        (
            Part::ScrollArrowUp,
            arrow(
                [&up.normal, &up.hot, &up.pressed, &up.disabled],
                &up.disabled,
                Lie::Upright,
                false,
            ),
        ),
        (
            Part::ScrollArrowDown,
            arrow(
                [&down.normal, &down.hot, &down.pressed, &down.disabled],
                &down.disabled,
                Lie::Upright,
                true,
            ),
        ),
        (
            Part::ScrollArrowLeft,
            arrow(
                [&left.normal, &left.hot, &left.pressed, &left.disabled],
                &up.disabled,
                Lie::Down,
                false,
            ),
        ),
        (
            Part::ScrollArrowRight,
            arrow(
                [&right.normal, &right.hot, &right.pressed, &right.disabled],
                &down.disabled,
                Lie::Down,
                true,
            ),
        ),
        (
            Part::ScrollThumbV,
            thumb(
                [&tv.normal, &tv.hot, &tv.pressed, &tv.disabled],
                Lie::Upright,
            )
            .sizing(Insets::new(7, 5, 9, 5))
            .content(Insets::new(0, 3, 0, 3)),
        ),
        (
            Part::ScrollThumbH,
            thumb([&th.normal, &th.hot, &th.pressed, &th.disabled], Lie::Down)
                .sizing(Insets::new(5, 7, 5, 9))
                .content(Insets::new(3, 0, 3, 0)),
        ),
        (
            Part::ScrollGripV,
            grips(
                [
                    Some(&gv.normal),
                    Some(&gv.hot),
                    Some(&gv.pressed),
                    gv.disabled.as_ref(),
                ],
                Lie::Upright,
            ),
        ),
        (
            Part::ScrollGripH,
            grips(
                [
                    Some(&gh.normal),
                    Some(&gh.hot),
                    Some(&gh.pressed),
                    gh.disabled.as_ref(),
                ],
                Lie::Down,
            ),
        ),
        (
            Part::ScrollTrackV,
            tracks([&kv.normal, &kv.pressed, &kv.disabled], Lie::Upright)
                .sizing(Insets::new(11, 0, 5, 0)),
        ),
        (
            Part::ScrollTrackH,
            tracks([&kh.normal, &kh.pressed, &kh.disabled], Lie::Down)
                .sizing(Insets::new(0, 11, 0, 5)),
        ),
    ]
}

fn atlas(frames: &[Canvas], states: &[State], d: i32) -> PartImage {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut a = Canvas::new(w, h * frames.len() as u32);
    for (i, f) in frames.iter().enumerate() {
        a.blit_canvas(0, i as i32 * h as i32, f);
    }
    PartImage::new(a, states, Layout::Vertical).at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rect;

    const PARTS: [Part; 10] = [
        Part::ScrollArrowUp,
        Part::ScrollArrowDown,
        Part::ScrollArrowLeft,
        Part::ScrollArrowRight,
        Part::ScrollThumbV,
        Part::ScrollThumbH,
        Part::ScrollGripV,
        Part::ScrollGripH,
        Part::ScrollTrackV,
        Part::ScrollTrackH,
    ];

    fn part(v: Variant, p: Part, d: i32) -> PartImage {
        parts(v, d).into_iter().find(|(q, _)| *q == p).unwrap().1
    }

    fn frame(img: &PartImage, state: State) -> Rect {
        let f = img.frame(state);
        Rect::new(f.x, f.y, f.width, f.height)
    }

    fn px(img: &PartImage, state: State, x: i32, y: i32) -> u32 {
        let f = frame(img, state);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn pixels(img: &PartImage, state: State) -> Vec<u32> {
        let f = frame(img, state);
        (0..f.height)
            .flat_map(|y| (0..f.width).map(move |x| (x, y)))
            .map(|(x, y)| img.atlas.get(f.x + x, f.y + y))
            .collect()
    }

    fn luma(p: u32) -> f64 {
        let ch = |s: u32| f64::from((p >> s) & 0xFF);
        0.3 * ch(16) + 0.59 * ch(8) + 0.11 * ch(0)
    }

    fn fnv(img: &PartImage) -> u64 {
        [img.atlas.width, img.atlas.height]
            .iter()
            .chain(&img.atlas.pixels)
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    #[test]
    fn every_part_keeps_its_size_and_nine_grid_at_scale_1_and_2() {
        for v in Variant::ALL {
            for (p, size) in [
                (Part::ScrollArrowUp, (17, 17)),
                (Part::ScrollArrowDown, (17, 17)),
                (Part::ScrollArrowLeft, (17, 17)),
                (Part::ScrollArrowRight, (17, 17)),
                (Part::ScrollThumbV, (17, 22)),
                (Part::ScrollThumbH, (22, 17)),
                (Part::ScrollGripV, (7, 8)),
                (Part::ScrollGripH, (8, 7)),
                (Part::ScrollTrackV, (17, 1)),
                (Part::ScrollTrackH, (1, 17)),
            ] {
                let one = part(v, p, 1);
                for d in [1, 2] {
                    let img = part(v, p, d);
                    assert_eq!(img.scale, d, "{v:?} {p:?} is painted at {d}");
                    assert_eq!(img.sizing, one.sizing, "{v:?} {p:?} at {d}");
                    assert_eq!(img.content, one.content, "{v:?} {p:?} at {d}");
                    for &state in &img.states {
                        let f = frame(&img, state);
                        assert_eq!(
                            (f.width, f.height),
                            (size.0 * d, size.1 * d),
                            "{v:?} {p:?} {state:?} at {d}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_state_draws_its_own_frame_where_its_face_is_its_own() {
        for v in Variant::ALL {
            let s = v.scheme();
            for p in &PARTS[..4] {
                let img = part(v, *p, 1);
                for (i, &a) in STATES.iter().enumerate() {
                    for &b in &STATES[i + 1..] {
                        assert_ne!(
                            pixels(&img, a),
                            pixels(&img, b),
                            "{v:?} {p:?} {a:?} vs {b:?}"
                        );
                    }
                }
            }
            let (tv, th) = (&s.scroll_thumb_v, &s.scroll_thumb_h);
            let (gv, gh) = (&s.scroll_grip_v, &s.scroll_grip_h);
            for (p, own) in [
                (
                    Part::ScrollThumbV,
                    [
                        tv.hot != tv.normal,
                        tv.pressed != tv.hot,
                        tv.disabled != tv.normal,
                    ],
                ),
                (
                    Part::ScrollThumbH,
                    [
                        th.hot != th.normal,
                        th.pressed != th.hot,
                        th.disabled != th.normal,
                    ],
                ),
                (
                    Part::ScrollGripV,
                    [
                        gv.hot != gv.normal,
                        gv.pressed != gv.hot,
                        gv.disabled != Some(gv.normal),
                    ],
                ),
                (
                    Part::ScrollGripH,
                    [
                        gh.hot != gh.normal,
                        gh.pressed != gh.hot,
                        gh.disabled != Some(gh.normal),
                    ],
                ),
            ] {
                let img = part(v, p, 1);
                let f = |st| pixels(&img, st);
                assert_eq!(f(State::Hot) != f(State::Normal), own[0], "{v:?} {p:?} lit");
                assert_eq!(
                    f(State::Pressed) != f(State::Hot),
                    own[1],
                    "{v:?} {p:?} held"
                );
                assert_eq!(
                    f(State::Disabled) != f(State::Normal),
                    own[2],
                    "{v:?} {p:?} greyed"
                );
            }
            for p in [Part::ScrollTrackV, Part::ScrollTrackH] {
                let img = part(v, p, 1);
                assert_ne!(
                    pixels(&img, State::Normal),
                    pixels(&img, State::Pressed),
                    "{v:?} {p:?}"
                );
            }
        }
    }

    #[test]
    fn a_buttons_corners_are_round_and_clean_at_scale_2() {
        for v in Variant::ALL {
            for p in [Part::ScrollArrowUp, Part::ScrollThumbV] {
                let img = part(v, p, 2);
                for &state in &STATES {
                    let f = frame(&img, state);
                    let at = |x: i32, y: i32| px(&img, state, x, y);
                    let edge = at(0, 0);
                    let (r, b) = (f.width - 1, f.height - 1);
                    for (x, y) in [(r, 0), (r, b), (2, 0), (2, b)] {
                        assert_eq!(at(x, y), edge, "{v:?} {p:?} {state:?} ({x}, {y})");
                    }
                    let mut distinct: Vec<u32> = (0..4)
                        .flat_map(|y| (0..4).map(move |x| (r - x, y)))
                        .map(|(x, y)| at(x, y))
                        .collect();
                    distinct.sort_unstable();
                    distinct.dedup();
                    assert!(distinct.len() >= 3, "{v:?} {p:?} {state:?}: a smooth curve");
                    let floor = [edge, at(f.width / 2, 0), at(r, f.height / 2)]
                        .map(luma)
                        .into_iter()
                        .fold(f64::MAX, f64::min);
                    for y in 0..6 {
                        for x in r - 5..=r {
                            if y > 1 && x < r - 1 {
                                continue;
                            }
                            let q = at(x, y);
                            assert!(
                                luma(q) >= floor - 1.0,
                                "{v:?} {p:?} {state:?}: {q:08X} at ({x}, {y}) is a dark fringe"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_chevron_arm_is_solid_at_1_and_smooth_at_2() {
        for v in Variant::ALL {
            let s = v.scheme();
            for (p, face, pts) in [
                (
                    Part::ScrollArrowUp,
                    &s.scroll_arrow_up.normal,
                    [(7, 7), (9, 7)],
                ),
                (
                    Part::ScrollArrowDown,
                    &s.scroll_arrow_down.normal,
                    [(7, 9), (9, 9)],
                ),
                (
                    Part::ScrollArrowLeft,
                    &s.scroll_arrow_left.normal,
                    [(7, 7), (7, 9)],
                ),
                (
                    Part::ScrollArrowRight,
                    &s.scroll_arrow_right.normal,
                    [(9, 7), (9, 9)],
                ),
            ] {
                let ink = shape::pack(shape::premul(face.mark.unwrap(), 255));
                let img = part(v, p, 1);
                for (x, y) in pts {
                    assert_eq!(px(&img, State::Normal, x, y), ink, "{v:?} {p:?} ({x}, {y})");
                }
                let two = pixels(&part(v, p, 2), State::Normal);
                let whole = two.iter().filter(|&&q| q == ink).count();
                assert!(whole >= 30, "{v:?} {p:?}: {whole} whole ink pixels at 2");
            }
        }
    }

    #[test]
    fn an_arrows_seam_carries_no_chevron() {
        for v in Variant::ALL {
            let face = &v.scheme().scroll_arrow_up.normal;
            let sh = Shading::of(face, Lie::Upright);
            for d in [1, 2] {
                let with = button(face, &sh, Lie::Upright, THICK, ARROW_SEAM, Some(false), d);
                let without = button(face, &sh, Lie::Upright, THICK, ARROW_SEAM, None, d);
                let ink = shape::pack(shape::premul(face.mark.unwrap(), 255));
                let blends = (with.pixels.iter().zip(&without.pixels))
                    .filter(|&(a, b)| a != b && *a != ink)
                    .count();
                assert!(blends > 0, "{v:?} at {d}: the chevron has soft sides");
                for i in 0..THICK * d {
                    for j in 3 * d..4 * d {
                        assert_eq!(with.get(j, i), without.get(j, i), "{v:?} column at {d}");
                        assert_eq!(with.get(i, j), without.get(i, j), "{v:?} row at {d}");
                    }
                }
            }
        }
    }

    #[test]
    fn grips_are_ridges_and_tracks_keep_whole_edges() {
        for v in Variant::ALL {
            let g = part(v, Part::ScrollGripV, 1);
            let light = px(&g, State::Normal, 0, 0);
            let dark = px(&g, State::Normal, 6, 1);
            assert_ne!(light, dark, "{v:?}");
            for row in [0, 2, 4, 6] {
                assert_eq!(
                    px(&g, State::Normal, 6, row),
                    0,
                    "{v:?} right of a light line"
                );
                assert_eq!(
                    px(&g, State::Normal, 0, row + 1),
                    0,
                    "{v:?} left of a dark line"
                );
                assert_eq!(px(&g, State::Normal, 3, row), light, "{v:?}");
                assert_eq!(px(&g, State::Normal, 3, row + 1), dark, "{v:?}");
            }
            let h = part(v, Part::ScrollGripH, 1);
            assert_eq!(px(&h, State::Normal, 0, 6), 0, "{v:?} a lying grip turns");
            let t = part(v, Part::ScrollTrackV, 2);
            for st in TRACK_STATES {
                assert_eq!(px(&t, st, 0, 0), px(&t, st, 1, 0), "{v:?} {st:?} left edge");
                assert_eq!(
                    px(&t, st, 32, 0),
                    px(&t, st, 33, 0),
                    "{v:?} {st:?} right edge"
                );
            }
        }
    }

    #[test]
    fn scrollbar_parts_pin_at_scale_1_and_2() {
        let got: Vec<(Variant, i32, [u64; 10])> = Variant::ALL
            .into_iter()
            .flat_map(|v| [1, 2].map(|d| (v, d, PARTS.map(|p| fnv(&part(v, p, d))))))
            .collect();
        for (v, d, h) in &got {
            println!("(Variant::{v:?}, {d}, {h:#018x?}),");
        }
        assert_eq!(got.len(), PINS.len());
        for (g, want) in got.iter().zip(PINS) {
            assert_eq!(*g, want, "{:?} at {}", g.0, g.1);
        }
    }

    const PINS: [(Variant, i32, [u64; 10]); 16] = [
        (
            Variant::Blue,
            1,
            [
                0x7d44_023f_0745_88ee,
                0xbde0_a020_4a37_c5c6,
                0xcb04_69df_2206_71b1,
                0x21a9_1912_2f94_4203,
                0xf287_b676_fc70_6190,
                0x2ca0_2345_fbb6_1c1b,
                0x6c72_da3f_62fe_f182,
                0xece8_314d_7169_ae31,
                0x90a0_5827_5dda_7c86,
                0x9e97_91c0_d4a2_b666,
            ],
        ),
        (
            Variant::Blue,
            2,
            [
                0x7d14_ffa7_81f3_7e48,
                0x7873_0deb_c995_ef2c,
                0xdb38_a54e_a4f6_8e7d,
                0xf263_867b_dc76_fe21,
                0xcc01_59e4_6191_770f,
                0x10f2_a179_b0da_cc39,
                0xbeed_1147_6cfc_ae4b,
                0x903e_654b_36e2_a48d,
                0xb21c_ad4d_c0ae_7eb9,
                0x9f1d_79c3_1185_e5c5,
            ],
        ),
        (
            Variant::Olive,
            1,
            [
                0x0ba5_bf76_ad6d_f10e,
                0x4047_1ff0_3cdf_df82,
                0x4712_a4e0_2b45_985b,
                0x7d6e_a2f3_cee1_81e6,
                0xb654_cd35_4ca0_ca2c,
                0x820e_8813_3435_c24f,
                0x29f9_11b3_fcf5_a5c2,
                0x0045_817c_464f_5b51,
                0x90a0_5827_5dda_7c86,
                0x9e97_91c0_d4a2_b666,
            ],
        ),
        (
            Variant::Olive,
            2,
            [
                0xc2dd_aa1f_4097_cff7,
                0x57db_32ae_579a_aa57,
                0x6a91_35af_5e91_eed6,
                0x49a3_dd15_e2c7_f4e3,
                0x840c_82d6_194b_1843,
                0x4fdb_40b6_84ac_0c81,
                0x24a4_7690_e40c_9a4b,
                0x534e_8bc4_1503_1e0d,
                0xb21c_ad4d_c0ae_7eb9,
                0x9f1d_79c3_1185_e5c5,
            ],
        ),
        (
            Variant::Silver,
            1,
            [
                0x0272_b89d_a900_fb6d,
                0x2b34_6818_20b9_dc9f,
                0x1777_ecab_241f_401b,
                0x3158_1e53_7937_4e99,
                0xa62c_db21_ab89_b960,
                0x4f9c_a286_f647_e6ff,
                0x87dd_fe71_9217_44c2,
                0xf6ab_8efc_1335_23b1,
                0xbff3_aae8_18fd_2ff1,
                0x29ff_5a5f_7507_5811,
            ],
        ),
        (
            Variant::Silver,
            2,
            [
                0x2ea0_e284_995f_3b72,
                0xb2da_9cc1_213d_f0fa,
                0x56f4_7e63_6f78_bcaf,
                0x3950_2701_f5ac_9e77,
                0x7f88_7bda_430b_72cf,
                0xe230_e62c_8a34_cba1,
                0xcdad_adad_7b6e_ef4b,
                0x1ffa_4d04_652f_1c8d,
                0xb5f2_c6e7_7d1f_54a9,
                0xda27_a2ef_900d_5a45,
            ],
        ),
        (
            Variant::Slate,
            1,
            [
                0xa2f1_2cea_686b_9da9,
                0xa29d_d099_3cd2_a0f3,
                0x3c61_cc9e_23de_fd0d,
                0x602d_ff1e_7de5_b354,
                0xf5ba_70b7_0547_bb6b,
                0x684c_515a_a7fb_5e70,
                0xca16_0fc8_034d_e0e2,
                0x0796_340c_c8c8_48c1,
                0x897c_db3c_cb2d_08fe,
                0xa2cc_a80d_ee4c_933e,
            ],
        ),
        (
            Variant::Slate,
            2,
            [
                0x21ec_3657_fe5e_a573,
                0x81f7_d6ba_cba4_2fb7,
                0x284f_a7b5_0910_eb27,
                0xc399_9a2a_1a4c_4955,
                0x877d_0adc_dfe7_1879,
                0x35df_4fd9_42cc_dccd,
                0xf420_e05a_2fd9_99cb,
                0x18c7_aeab_9735_6a8d,
                0xa57e_caa1_4ebe_afc1,
                0x4012_632d_09aa_f701,
            ],
        ),
        (
            Variant::Sapphire,
            1,
            [
                0x5fa1_0dca_4415_3d0c,
                0xbf92_05a7_fba0_9a07,
                0xc975_5671_ac85_f238,
                0x3ee3_cf2c_f2db_71e0,
                0x4fa3_f1b9_1fe3_0932,
                0x6b61_2bbd_3224_ea76,
                0xbc70_ce05_ee4b_0c62,
                0xfb19_3ea2_afe0_34a1,
                0xcc4d_994d_a1b6_d511,
                0x5e76_0dd6_45de_1a44,
            ],
        ),
        (
            Variant::Sapphire,
            2,
            [
                0x0085_150f_501b_3586,
                0xc16c_6f8f_41cd_b32c,
                0x4c5a_42db_0603_9966,
                0xe98a_04d2_fc32_d9f4,
                0x47db_b5ef_8412_044d,
                0xfe87_6669_1e4c_0345,
                0x9a91_c51a_4f71_53cb,
                0x1431_8bb8_dbf4_fe0d,
                0xb98f_b2ba_e2e4_4e91,
                0x3585_3b6d_c4d0_159d,
            ],
        ),
        (
            Variant::Obsidian,
            1,
            [
                0x5fa1_0dca_4415_3d0c,
                0xbf92_05a7_fba0_9a07,
                0xc975_5671_ac85_f238,
                0x3ee3_cf2c_f2db_71e0,
                0x4fa3_f1b9_1fe3_0932,
                0x6b61_2bbd_3224_ea76,
                0xbc70_ce05_ee4b_0c62,
                0xfb19_3ea2_afe0_34a1,
                0xcc4d_994d_a1b6_d511,
                0x5e76_0dd6_45de_1a44,
            ],
        ),
        (
            Variant::Obsidian,
            2,
            [
                0x0085_150f_501b_3586,
                0xc16c_6f8f_41cd_b32c,
                0x4c5a_42db_0603_9966,
                0xe98a_04d2_fc32_d9f4,
                0x47db_b5ef_8412_044d,
                0xfe87_6669_1e4c_0345,
                0x9a91_c51a_4f71_53cb,
                0x1431_8bb8_dbf4_fe0d,
                0xb98f_b2ba_e2e4_4e91,
                0x3585_3b6d_c4d0_159d,
            ],
        ),
        (
            Variant::Ember,
            1,
            [
                0xa2f1_2cea_686b_9da9,
                0xa29d_d099_3cd2_a0f3,
                0x3c61_cc9e_23de_fd0d,
                0x602d_ff1e_7de5_b354,
                0xf5ba_70b7_0547_bb6b,
                0x684c_515a_a7fb_5e70,
                0xca16_0fc8_034d_e0e2,
                0x0796_340c_c8c8_48c1,
                0x897c_db3c_cb2d_08fe,
                0xa2cc_a80d_ee4c_933e,
            ],
        ),
        (
            Variant::Ember,
            2,
            [
                0x21ec_3657_fe5e_a573,
                0x81f7_d6ba_cba4_2fb7,
                0x284f_a7b5_0910_eb27,
                0xc399_9a2a_1a4c_4955,
                0x877d_0adc_dfe7_1879,
                0x35df_4fd9_42cc_dccd,
                0xf420_e05a_2fd9_99cb,
                0x18c7_aeab_9735_6a8d,
                0xa57e_caa1_4ebe_afc1,
                0x4012_632d_09aa_f701,
            ],
        ),
        (
            Variant::Cobalt,
            1,
            [
                0x5fa1_0dca_4415_3d0c,
                0xbf92_05a7_fba0_9a07,
                0xc975_5671_ac85_f238,
                0x3ee3_cf2c_f2db_71e0,
                0x4fa3_f1b9_1fe3_0932,
                0x6b61_2bbd_3224_ea76,
                0xbc70_ce05_ee4b_0c62,
                0xfb19_3ea2_afe0_34a1,
                0xcc4d_994d_a1b6_d511,
                0x5e76_0dd6_45de_1a44,
            ],
        ),
        (
            Variant::Cobalt,
            2,
            [
                0x0085_150f_501b_3586,
                0xc16c_6f8f_41cd_b32c,
                0x4c5a_42db_0603_9966,
                0xe98a_04d2_fc32_d9f4,
                0x47db_b5ef_8412_044d,
                0xfe87_6669_1e4c_0345,
                0x9a91_c51a_4f71_53cb,
                0x1431_8bb8_dbf4_fe0d,
                0xb98f_b2ba_e2e4_4e91,
                0x3585_3b6d_c4d0_159d,
            ],
        ),
    ];
}
