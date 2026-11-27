//! Reprise Glass's Onyx Start panel ([`super::Style::Onyx`]): smoked glass that darkens towards its
//! foot round a white pane, pale blue lit rows on the pane, glass lit places, a bevelled picture
//! frame, and the power, lock and arrow buttons as one outlined group. [`parts`] makes the
//! `Part::Start*` parts that differ from Glass's; [`panel`] and [`buttons`] paint at the size they
//! fill, which grows with the text.

use std::borrow::Cow;

use super::glyph::{self, Mark, Piece};
use super::items::{DARK_SELECTED, DARK_SELECTED_HOT};
use super::panel::LightBox;
use super::startparts::{
    ARROW_OUTLINE, ARROW_WHITE, FIELD_GRID, FieldInks, Ink, along, canvas, cell, field_in, one,
    opaque, px, row_box, row_part, rule, share,
};
use super::{Mode, Settings};
use crate::raster::{Canvas, Insets, Rect};
use crate::shape::{
    self, CLEAR, Px, centre, mix, over, pack, ringed, rounded, rounded4, smooth, unpack,
};
use crate::style::{Part, PartImage, State};

const FRAME_R: f64 = 6.0;
const FRAME_LINE: Ink = (230.0, 0x121518);
/// The frame's light line inside its dark one, at the top and at the foot.
const FRAME_LIGHT: [Ink; 2] = [(235.0, 0xA0A2A3), (235.0, 0x7C7F82)];
/// The smoke down the panel, by the share of its height above.
const SMOKE: [(f64, Ink); 5] = [
    (0.0, (214.0, 0x5A5D60)),
    (0.16, (214.0, 0x4A4D50)),
    (0.5, (204.0, 0x23272C)),
    (0.9, (196.0, 0x080B10)),
    (1.0, (196.0, 0x070A10)),
];

fn smoke(t: f64) -> Px {
    let stops: Vec<(f64, Px)> = SMOKE.iter().map(|&(t, ink)| (t, px(ink))).collect();
    shape::Stops(&stops).at(t)
}

/// The panel's frame and glass over `r` (device px) at `d` device px per design px: a dark line,
/// a light one inside it, then the smoke; the top corners round, the foot square on the bar.
pub fn panel(c: &mut Canvas, r: Rect, d: i32) {
    let k = f64::from(d);
    let (w, h) = (f64::from(r.width), f64::from(r.height));
    let cov = |x: f64, y: f64, i: f64| {
        let rr = ((FRAME_R - i) * k).max(0.0);
        rounded4(
            x - i * k,
            y - i * k,
            w - 2.0 * i * k,
            h - 2.0 * i * k,
            [rr, rr, 0.0, 0.0],
        )
    };
    for y in 0..r.height {
        let t = (f64::from(y) + 0.5) / h;
        let light = mix(px(FRAME_LIGHT[0]), px(FRAME_LIGHT[1]), t);
        let glass = smoke(t);
        for x in 0..r.width {
            let (fx, fy) = (f64::from(x), f64::from(y));
            let [c0, c1, c2] = [cov(fx, fy, 0.0), cov(fx, fy, 1.0), cov(fx, fy, 2.0)];
            if c0 == 0.0 {
                continue;
            }
            let p = shape::shares(&[(px(FRAME_LINE), c0 - c1), (light, c1 - c2), (glass, c2)]);
            let (cx, cy) = (r.x + x, r.y + y);
            c.put_raw(cx, cy, pack(over(p, unpack(c.get(cx, cy)))));
        }
    }
}

const PANE: (i32, i32) = (14, 14);
const PANE_R: f64 = 4.0;
const PANE_RINGS: [Ink; 2] = [(130.0, 0xFFFFFF), (255.0, 0x3A3A3A)];
const PANE_GRID: Insets = Insets::new(6, 6, 6, 6);

fn pane(d: i32) -> Canvas {
    let (w, h) = PANE;
    let pixels = ringed(w, h, PANE_R, 2, d, |_, _, i| match i {
        0 | 1 => px(PANE_RINGS[i]),
        _ => opaque(0xFFFFFF),
    });
    canvas(w * d, &pixels)
}

const HOT_LEFT: LightBox = row_box(0xB7E7FC, [0xF3FAFE, 0xDFF2FD], [0xE9F7FE, 0xC5E9FB]);
const HOT_ALL: LightBox = row_box(0xB3E4F9, [0xF0F7FB, 0xDCEFFA], [0xE5F3FA, 0xC2E5F8]);

const HOT_RIGHT: (i32, i32) = (12, 33);
const HOT_RIGHT_GRID: Insets = Insets::new(5, 16, 5, 16);
const HOT_RIGHT_R: f64 = 4.0;
const RIGHT_GLOW: Ink = (40.0, 0xFFFFFF);
const RIGHT_OUTLINE: Ink = (255.0, 0x000000);
/// The lit place's light line inside its outline: top, sides, foot.
const RIGHT_EDGE: [Ink; 3] = [(128.0, 0xFFFFFF), (110.0, 0xFFFFFF), (96.0, 0xFFFFFF)];
/// Its face by design row: lighter glass fading down the upper half, a shade at the top of the
/// lower half rising to light again at its foot.
const RIGHT_FACE: [(i32, Ink); 6] = [
    (3, (74.0, 0xFFFFFF)),
    (16, (29.0, 0xFFFFFF)),
    (17, (32.0, 0x000000)),
    (21, (0.0, 0x000000)),
    (22, (0.0, 0xFFFFFF)),
    (29, (29.0, 0xFFFFFF)),
];

fn hot_right(d: i32) -> Canvas {
    let (w, h) = HOT_RIGHT;
    let pixels = ringed(w, h, HOT_RIGHT_R, 3, d, |x, y, i| match i {
        0 => px(RIGHT_GLOW),
        1 => px(RIGHT_OUTLINE),
        2 => {
            let (x, y) = (cell(x), cell(y));
            if y <= 2 {
                px(RIGHT_EDGE[0])
            } else if y >= h - 3 {
                px(RIGHT_EDGE[2])
            } else if x <= 2 || x >= w - 3 {
                px(RIGHT_EDGE[1])
            } else {
                along(&RIGHT_FACE, f64::from(y))
            }
        }
        _ => along(&RIGHT_FACE, y),
    });
    canvas(w * d, &pixels)
}

const RULE_LEFT: Ink = (255.0, 0xE2E2E2);
const RULE_RIGHT: [Ink; 2] = [(181.0, 0x000000), (74.0, 0xFFFFFF)];

const FIELD: FieldInks = FieldInks {
    halo: [50.0, 90.0, 64.0, 60.0],
    top: 0x636363,
    bottom: 0x737577,
    left: [0x4B4C49, 0x5A5B59],
    right: [0x57585A, 0x737476],
    radius: 2.0,
};

const FRAME: i32 = 58;
const FRAME_RADIUS: f64 = 3.5;
const FRAME_OUTLINE: Ink = (200.0, 0x1E2022);
/// The rim inside the outline runs white from the top left to cyan at the bottom right.
const FRAME_RIM: [Ink; 2] = [(240.0, 0xFFFFFF), (240.0, 0x14C2DC)];
const FRAME_GLASS: Ink = (40.0, 0xFFFFFF);
const FRAME_INNER: Ink = (220.0, 0xDCDCDC);
/// The rings round the 48 px hole the picture shows through: outline, rim, two of glass, the
/// light line.
const FRAME_RINGS: usize = 5;
/// The picture's offset in the frame, design px.
pub const PICTURE_AT: (i32, i32) = (5, 5);

fn picture_frame(d: i32) -> Canvas {
    let last = f64::from(FRAME - 1);
    let pixels = ringed(
        FRAME,
        FRAME,
        FRAME_RADIUS,
        FRAME_RINGS,
        d,
        |x, y, i| match i {
            0 => px(FRAME_OUTLINE),
            1 => {
                let t = smooth((x / last + y / last - 1.0) / 0.25);
                mix(px(FRAME_RIM[0]), px(FRAME_RIM[1]), t)
            }
            2 | 3 => px(FRAME_GLASS),
            4 => px(FRAME_INNER),
            _ => CLEAR,
        },
    );
    canvas(FRAME * d, &pixels)
}

/// The Onyx parts over Glass's `glass`: the pane and its rules, the lit rows and places, the field
/// and the picture frame. Dark Glass remaps the pane's pieces and takes its own lit rows.
pub(super) fn parts(s: &Settings, d: i32, glass: Vec<(Part, PartImage)>) -> Vec<(Part, PartImage)> {
    let dark = s.mode == Mode::Dark;
    let pane_part = |img: PartImage| {
        if dark {
            super::dark::remap_part(img)
        } else {
            img
        }
    };
    let [hot_left, hot_all, lit_result] = if dark {
        [&DARK_SELECTED_HOT, &DARK_SELECTED_HOT, &DARK_SELECTED]
    } else {
        [&HOT_LEFT, &HOT_ALL, &HOT_LEFT]
    };
    let none = Insets::ZERO;
    let onyx = vec![
        (Part::StartPane, pane_part(one(pane(d), PANE_GRID, d))),
        (Part::StartHotLeft, row_part(hot_left, 36, d)),
        (Part::StartHotAll, row_part(hot_all, 23, d)),
        (Part::StartLitResult, row_part(lit_result, 22, d)),
        (Part::StartHotRight, one(hot_right(d), HOT_RIGHT_GRID, d)),
        (
            Part::StartRuleLeft,
            pane_part(one(rule(&[RULE_LEFT]), none, 1)),
        ),
        (Part::StartRuleRight, one(rule(&RULE_RIGHT), none, 1)),
        (
            Part::StartSearchField,
            pane_part(one(field_in(&FIELD, d), FIELD_GRID, d)),
        ),
        (Part::StartPictureFrame, one(picture_frame(d), none, d)),
    ];
    glass
        .into_iter()
        .map(|(part, img)| match onyx.iter().find(|(p, _)| *p == part) {
            Some((_, mine)) => (part, mine.clone()),
            None => (part, img),
        })
        .collect()
}

/// A button's face, top to bottom: its light top row, its upper half's and lower half's
/// gradients, its light foot row.
struct Face {
    top: u32,
    upper: [u32; 2],
    lower: [u32; 2],
    foot: u32,
}

const POWER: [Face; 3] = [
    Face {
        top: 0xC4BA98,
        upper: [0xAA9D6C, 0x766C62],
        lower: [0x47392E, 0x5F502F],
        foot: 0x8F856D,
    },
    Face {
        top: 0xE3CA91,
        upper: [0xD3AB4F, 0x957F57],
        lower: [0x63460E, 0xBA7E07],
        foot: 0xD4AE63,
    },
    Face {
        top: 0x615434,
        upper: [0x7F6934, 0x5D5139],
        lower: [0x322407, 0x5C3F03],
        foot: 0x372602,
    },
];

/// The lock button's and the arrow's faces.
const SMOKED: [Face; 3] = [
    Face {
        top: 0xC6C8C9,
        upper: [0xAFB1B3, 0x696B6D],
        lower: [0x34373A, 0x5A5D61],
        foot: 0x8B8D90,
    },
    Face {
        top: 0xBACADB,
        upper: [0x91ABC5, 0x667074],
        lower: [0x1F2E33, 0x066CAF],
        foot: 0x63A3CD,
    },
    Face {
        top: 0x4A535C,
        upper: [0x5A6979, 0x40474A],
        lower: [0x10161A, 0x033659],
        foot: 0x022235,
    },
];

const GROUP_R: f64 = 3.0;
const GROUP_HALO: Ink = (77.0, 0xFFFFFF);
const GROUP_LINE: Px = (255.0, [0.0; 3]);
/// How much white a face's side columns take.
const SIDE_LIGHT: f64 = 0.25;
const GLYPH_SHADOW: Ink = (150.0, 0x000000);
const GLYPH_WHITE: u32 = 0xF4F4F4;

fn face_of(kind: &'static [Face; 3], state: State) -> &'static Face {
    &kind[match state {
        State::Hot => 1,
        State::Pressed => 2,
        _ => 0,
    }]
}

/// The power symbol: a ring open at the top and a bar down into the gap.
fn power_mark() -> Mark {
    let (cx, cy, r) = (6.5, 7.0, 4.8);
    let arc: Vec<(f64, f64)> = (0..=24)
        .map(|k| {
            let a = (35.0 + 290.0 * f64::from(k) / 24.0).to_radians();
            (cx + r * a.sin(), cy - r * a.cos())
        })
        .collect();
    Mark {
        w: 13,
        h: 13,
        pieces: Cow::Owned(vec![
            Piece::Line(Cow::Owned(arc), 1.8),
            Piece::Line(Cow::Borrowed(&[(6.5, 0.6), (6.5, 6.4)]), 1.8),
        ]),
    }
}

/// A padlock: its body and the shackle over it.
fn lock_mark() -> Mark {
    let (cx, cy, r) = (6.0, 3.6, 3.1);
    let mut shackle = vec![(cx - r, 6.0)];
    shackle.extend((0..=12).map(|k| {
        let a = (-90.0 + 180.0 * f64::from(k) / 12.0).to_radians();
        (cx + r * a.sin(), cy - r * a.cos())
    }));
    shackle.push((cx + r, 6.0));
    Mark {
        w: 12,
        h: 13,
        pieces: Cow::Owned(vec![
            Piece::Rect(0.5, 5.5, 11.5, 12.5),
            Piece::Line(Cow::Owned(shackle), 1.6),
        ]),
    }
}

/// Paints mark `m` at `(x, y)` (device px) at `k` device px per design px: a dark edge grown round
/// it, then the mark in `ink`.
fn mark(c: &mut Canvas, m: &Mark, (x, y): (i32, i32), k: i32, ink: impl Fn(f64) -> Px, edge: Ink) {
    for dy in -k..(m.h + 1) * k {
        for dx in -k..(m.w + 1) * k {
            let (cx, cy) = (x + dx, y + dy);
            let under = unpack(c.get(cx, cy));
            let shadow = shape::scale(px(edge), m.cover(dx, dy, k, 1.0));
            let top = shape::scale(ink(centre(dy, k)), m.cover(dx, dy, k, 0.0));
            c.put_raw(cx, cy, pack(over(top, over(shadow, under))));
        }
    }
}

/// The power, lock and arrow buttons over `group` (device px, the outline's box) at `d` device px
/// per design px, split at the dividers `cuts` (their device x), each in its own state; their
/// glyphs drawn at `k` device px per design px.
pub fn buttons(c: &mut Canvas, group: Rect, cuts: [i32; 2], states: [State; 3], d: i32, k: i32) {
    let dk = f64::from(d);
    let (w, h) = (f64::from(group.width), f64::from(group.height));
    let rows = h / dk;
    let mid = (rows / 2.0).floor();
    let segments = [
        (group.x + d, cuts[0], &POWER, states[0]),
        (cuts[0] + d, cuts[1], &SMOKED, states[1]),
        (cuts[1] + d, group.right() - d, &SMOKED, states[2]),
    ];
    let face_at = |x: i32, yd: f64| -> Px {
        if cuts.iter().any(|&cut| (cut..cut + d).contains(&x)) {
            return GROUP_LINE;
        }
        let Some(&(x0, x1, kind, state)) = segments.iter().find(|s| (s.0..s.1).contains(&x)) else {
            return GROUP_LINE;
        };
        let f = face_of(kind, state);
        let row = cell(yd);
        let body = if row <= 1 {
            opaque(f.top)
        } else if f64::from(row) >= rows - 2.0 {
            opaque(f.foot)
        } else if yd < mid {
            mix(
                opaque(f.upper[0]),
                opaque(f.upper[1]),
                share(yd, 2, mid as i32 - 1),
            )
        } else {
            let foot = (rows - 3.0) as i32;
            mix(
                opaque(f.lower[0]),
                opaque(f.lower[1]),
                share(yd, mid as i32, foot),
            )
        };
        let side = x < x0 + d || x >= x1 - d;
        if side && row > 1 && f64::from(row) < rows - 2.0 {
            mix(body, opaque(0xFFFFFF), SIDE_LIGHT)
        } else {
            body
        }
    };
    let cov = |x: f64, y: f64, i: f64| {
        rounded(
            x - i * dk,
            y - i * dk,
            w - 2.0 * i * dk,
            h - 2.0 * i * dk,
            ((GROUP_R - i) * dk).max(0.0),
        )
    };
    for y in -d..group.height + d {
        let yd = centre(y, d);
        for x in -d..group.width + d {
            let (fx, fy) = (f64::from(x), f64::from(y));
            let halo = rounded(
                fx + dk,
                fy + dk,
                w + 2.0 * dk,
                h + 2.0 * dk,
                (GROUP_R + 1.0) * dk,
            );
            if halo == 0.0 {
                continue;
            }
            let [c0, c1] = [cov(fx, fy, 0.0), cov(fx, fy, 1.0)];
            let inner = if c1 > 0.0 {
                face_at(group.x + x, yd)
            } else {
                CLEAR
            };
            let p = shape::shares(&[
                (px(GROUP_HALO), halo - c0),
                (GROUP_LINE, c0 - c1),
                (inner, c1),
            ]);
            let (cx, cy) = (group.x + x, group.y + y);
            c.put_raw(cx, cy, pack(over(p, unpack(c.get(cx, cy)))));
        }
    }
    let centred = |x0: i32, x1: i32, m: &Mark| {
        let x = x0 + (x1 - x0 - m.w * k) / 2;
        let y = group.y + (group.height - m.h * k) / 2;
        (x, y)
    };
    let white = |_| opaque(GLYPH_WHITE);
    let power = power_mark();
    mark(
        c,
        &power,
        centred(segments[0].0, segments[0].1, &power),
        k,
        white,
        GLYPH_SHADOW,
    );
    let lock = lock_mark();
    mark(
        c,
        &lock,
        centred(segments[1].0, segments[1].1, &lock),
        k,
        white,
        GLYPH_SHADOW,
    );
    let arrow = glyph::ARROW;
    let arrow_ink = |y: f64| along(&ARROW_WHITE, y);
    let at = centred(segments[2].0, segments[2].1, &arrow);
    mark(c, &arrow, at, k, arrow_ink, (ARROW_OUTLINE, 0x000000));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::Style;

    fn fnv(c: &Canvas) -> u64 {
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        words.fold(0xcbf2_9ce4_8422_2325, |h, w| {
            w.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    fn onyx(mode: Mode) -> Settings {
        Settings {
            style: Style::Onyx,
            mode,
            ..Settings::default()
        }
    }

    /// The parts Onyx paints over Glass's, hashed, in `mode` at `d`.
    fn own(mode: Mode, d: i32) -> Vec<(String, u64)> {
        let glass = super::super::startpanel::parts(
            &Settings {
                mode,
                ..Settings::default()
            },
            d,
        );
        super::super::startpanel::parts(&onyx(mode), d)
            .iter()
            .zip(&glass)
            .filter(|((_, a), (_, b))| a.atlas != b.atlas)
            .map(|((p, img), _)| (format!("{p:?} {d}"), fnv(&img.atlas)))
            .collect()
    }

    #[test]
    fn onyx_keeps_its_pieces_at_scale_1_and_2_light_and_dark() {
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            for d in [1, 2] {
                let mine = own(mode, d);
                // Dark Glass's lit rows are Onyx's too.
                let want = if mode == Mode::Light { 9 } else { 6 };
                assert_eq!(mine.len(), want, "{mode:?} at {d}: {mine:?}");
                got.extend(mine.into_iter().map(|(k, h)| (format!("{mode:?} {k}"), h)));
            }
        }
        crate::pins::check("glass/onyx_start_parts", &got);
    }

    #[test]
    fn onyx_paints_the_parts_glass_names() {
        let names = |s: &Settings| -> Vec<Part> {
            super::super::startpanel::parts(s, 1)
                .into_iter()
                .map(|(p, _)| p)
                .collect()
        };
        assert_eq!(names(&onyx(Mode::Light)), names(&Settings::default()));
    }

    #[test]
    fn the_smoke_darkens_down_the_panel_its_top_corners_round_and_its_foot_square() {
        for d in [1, 2] {
            let (w, h) = (406 * d, 506 * d);
            let mut c = Canvas::new(w as u32, h as u32);
            panel(&mut c, Rect::new(0, 0, w, h), d);
            let level = |y: i32| {
                let p = c.get(200 * d, y * d);
                ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF)
            };
            assert!(level(40) > level(250) && level(250) > level(480), "at {d}");
            assert_eq!(c.get(0, 0), 0, "at {d}: the top corner is clear");
            assert_eq!(c.get(w - 1, 0), 0, "at {d}: both top corners are");
            assert!(c.get(0, h - 1) >> 24 > 200, "at {d}: the foot is square");
            assert!(
                c.get(200 * d, 300 * d) >> 24 < 255,
                "at {d}: the glass lets light through"
            );
        }
        let mut c = Canvas::new(40, 40);
        panel(&mut c, Rect::new(0, 0, 40, 40), 2);
        crate::glass::panel::assert_round_corner("the frame", &c, (0, 0), 12.0, 0xC0, 4);
    }

    #[test]
    fn a_lit_place_and_the_picture_frame_round_their_corners_at_scale_2() {
        let c = hot_right(2);
        crate::glass::panel::assert_round_corner("a lit place", &c, (0, 0), 8.0, 0x20, 4);
        let c = picture_frame(2);
        crate::glass::panel::assert_round_corner("the frame", &c, (0, 0), 7.0, 0, 4);
        let hole = c.get(2 * FRAME / 2, 2 * FRAME / 2);
        assert_eq!(hole, 0, "the picture shows through the hole");
    }

    #[test]
    fn the_panes_nine_grid_doubles_at_scale_2() {
        let parts = super::super::startpanel::parts(&onyx(Mode::Light), 2);
        let pane = &parts.iter().find(|(p, _)| *p == Part::StartPane).unwrap().1;
        assert_eq!((pane.atlas.width, pane.scale), (28, 2));
        assert_eq!(pane.sizing, PANE_GRID, "insets stay in design px");
    }

    const STATES: [(&str, [State; 3]); 7] = [
        ("rest", [State::Normal; 3]),
        ("power hot", [State::Hot, State::Normal, State::Normal]),
        ("power held", [State::Pressed, State::Normal, State::Normal]),
        ("lock hot", [State::Normal, State::Hot, State::Normal]),
        ("lock held", [State::Normal, State::Pressed, State::Normal]),
        ("arrow hot", [State::Normal, State::Normal, State::Hot]),
        ("arrow held", [State::Normal, State::Normal, State::Pressed]),
    ];

    /// The buttons at `d`, the group's outline at (2, 2) in a canvas with room for its halo.
    fn group(states: [State; 3], d: i32) -> Canvas {
        let mut c = Canvas::new((131 * d) as u32, (26 * d) as u32);
        let g = Rect::new(2 * d, 2 * d, 127 * d, 22 * d);
        buttons(&mut c, g, [g.x + 52 * d, g.x + 105 * d], states, d, d);
        c
    }

    #[test]
    fn the_buttons_keep_each_state_at_scale_1_and_2() {
        let mut got = Vec::new();
        for d in [1, 2] {
            let drawn: Vec<Canvas> = STATES.iter().map(|&(_, s)| group(s, d)).collect();
            for (i, a) in drawn.iter().enumerate() {
                for b in &drawn[i + 1..] {
                    assert_ne!(a, b, "at {d}: each state looks its own");
                }
                assert_eq!(a.get(0, 0), 0, "at {d}: the corner is clear");
            }
            got.extend(
                STATES
                    .iter()
                    .zip(&drawn)
                    .map(|((n, _), c)| (format!("{n} {d}"), fnv(c))),
            );
        }
        crate::pins::check("glass/onyx_start_buttons", &got);
        let c = group([State::Normal; 3], 2);
        // The halo rounds over the outline's radius and a design px.
        crate::glass::panel::assert_round_corner("the group", &c, (2, 2), 8.0, 0x20, 3);
    }

    #[test]
    fn a_lit_button_lights_alone_and_the_dividers_stay_black() {
        let rest = group([State::Normal; 3], 1);
        let lit = group([State::Normal, State::Hot, State::Normal], 1);
        let column = |c: &Canvas, x: i32| (4..22).map(|y| c.get(x, y)).collect::<Vec<_>>();
        assert_eq!(
            column(&rest, 20),
            column(&lit, 20),
            "the power button stays"
        );
        assert_ne!(
            column(&rest, 60),
            column(&lit, 60),
            "the lock button lights"
        );
        assert_eq!(column(&rest, 115), column(&lit, 115), "the arrow stays");
        for x in [2 + 52, 2 + 105] {
            assert!(
                column(&lit, x).iter().all(|&p| p == 0xFF00_0000),
                "divider at {x}"
            );
        }
    }
}
