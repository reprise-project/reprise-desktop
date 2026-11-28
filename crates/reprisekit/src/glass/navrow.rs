//! Glass's nav row parts, drawn from shapes at the style's scale: the pill and the round Back and
//! Forward buttons with the recent places drop after them, which keep their colours over the
//! frame's glass in either mode, and the search field with its magnifier and clear button,
//! which dark Glass remaps as it does every field.

use crate::raster::{Canvas, Insets};
use crate::shape::{CLEAR, Glyph, Px, Stops, centre, draw, ellipse, glyph, over, polygon, premul};
use crate::shape::{rounded, shares, stroke};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const ORB: i32 = 25;
pub(super) const PILL_W: i32 = 57;
pub(super) const PILL_H: i32 = 29;
const RECENT_W: i32 = 15;
const RECENT_H: i32 = 21;
const FIELD_W: i32 = 7;
const FIELD_H: i32 = 22;
const GLASS_SIDE: i32 = 12;
const CLEAR_W: i32 = 21;
const CLEAR_H: i32 = 20;
const CROSS: i32 = 8;

pub(super) const BUTTON_STATES: [State; 4] =
    [State::Normal, State::Hot, State::Pressed, State::Disabled];
pub(super) const FIELD_STATES: [State; 3] = [State::Normal, State::Hot, State::Focused];
pub(super) const CLEAR_STATES: [State; 3] = [State::Normal, State::Hot, State::Pressed];

/// A part's painter at a scale.
pub(super) type Painter = fn(i32) -> PartImage;

const fn rgb(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const WHITE: Color = rgb(0xFFFFFF);
const BLACK: Color = rgb(0x000000);

fn solid(v: u32) -> Px {
    premul(rgb(v), 0xFF)
}

/// One orb state: its ring top and bottom, the gloss over its upper half and the deep blue
/// under the split, and the cyan its lower rim glows in.
pub(super) struct Orb {
    pub ring: [u32; 2],
    pub upper: [(f64, u32); 3],
    pub lower: [(f64, u32); 4],
    pub rim: Option<u32>,
    pub arrow: (u32, u8),
}

const ORB_REST: Orb = Orb {
    ring: [0x4B6280, 0x073788],
    upper: [(1.0, 0xDCE8F5), (5.0, 0xBDD4EC), (10.5, 0x5282C2)],
    lower: [
        (11.5, 0x2F62AB),
        (12.5, 0x011B5B),
        (18.0, 0x0153A0),
        (23.0, 0x0A68B8),
    ],
    rim: Some(0x44A8EE),
    arrow: (0xFFFFFF, 0xFF),
};

const ORB_HOT: Orb = Orb {
    ring: [0x4B6280, 0x0A3F94],
    upper: [(1.0, 0xE2F0FB), (5.0, 0xD0E7F7), (10.5, 0x68A6E3)],
    lower: [
        (11.5, 0x2C6FC0),
        (12.5, 0x013888),
        (18.0, 0x0178E2),
        (23.0, 0x2A95F0),
    ],
    rim: Some(0x54C6FC),
    arrow: (0xFFFFFF, 0xFF),
};

const ORB_PRESSED: Orb = Orb {
    ring: [0x3E5270, 0x062C6E],
    upper: [(1.0, 0xB5C4D4), (5.0, 0x98ACC1), (10.5, 0x4C688F)],
    lower: [
        (11.5, 0x20406E),
        (12.5, 0x01072E),
        (18.0, 0x015392),
        (23.0, 0x0A5E9E),
    ],
    rim: None,
    arrow: (0xFFFFFF, 0xFF),
};

const ORB_GREYED: Orb = Orb {
    ring: [0x506A8C, 0x3E5878],
    upper: [(1.0, 0xDCE5EE), (4.0, 0xC7D6E3), (10.5, 0x8FA4BD)],
    lower: [
        (11.5, 0x7D92AE),
        (13.0, 0x627C9E),
        (18.0, 0x6F87A7),
        (23.0, 0x8296B1),
    ],
    rim: None,
    arrow: (0xE8EDF2, 0xA0),
};

/// The orb's arrow pointing left, in design px over the 25 px box: a 12 px head on a 4 px shaft.
const ARROW: [(f64, f64); 7] = [
    (6.0, 12.5),
    (12.0, 6.5),
    (12.0, 10.5),
    (18.5, 10.5),
    (18.5, 14.5),
    (12.0, 14.5),
    (12.0, 18.5),
];
const ARROW_SHADOW: u8 = 0x50;

const PILL_RING: (u32, u8) = (0x3F5779, 0xD0);
const PILL_FACE: [(u32, u8); 2] = [(0x8FA8CA, 0x90), (0x4F6E99, 0xA0)];

const RECENT_FILL: u32 = 0x398CC9;
const RECENT_LINE: u32 = 0x052959;
const RECENT_GREYED: [u32; 2] = [0x8A9AB0, 0x44546A];

/// The search field's edge, darker at the top; the alpha of its edge, inner line and face per
/// [`FIELD_STATES`]; then the magnifier's ink and its lens's white.
pub(super) struct FieldInks {
    pub edge: [(f64, u32); 2],
    pub alpha: [[u8; 3]; 3],
    pub magnifier: u32,
    pub lens: u8,
}

const FIELD: FieldInks = FieldInks {
    edge: FIELD_EDGE,
    alpha: FIELD_ALPHA,
    magnifier: GLASS_INK,
    lens: GLASS_LENS,
};

const FIELD_EDGE: [(f64, u32); 2] = [(0.0, 0x414952), (21.0, 0x8091A5)];
/// The edge's, the inner line's and the face's alpha for Normal, Hot and Focused.
const FIELD_ALPHA: [[u8; 3]; 3] = [[0xE0, 0xE8, 0xCC], [0xF0, 0xF4, 0xED], [0xF0, 0xFF, 0xFF]];

const GLASS_INK: u32 = 0x2D6FB6;
const GLASS_LENS: u8 = 0x60;

/// The clear button's cross, and its box's line and face top and bottom, Hot then Pressed.
pub(super) struct ClearInks {
    pub cross: u32,
    pub boxes: [[u32; 3]; 2],
}

const CLEAR_INKS: ClearInks = ClearInks {
    cross: CROSS_INK,
    boxes: CLEAR_BOX,
};

const CROSS_INK: u32 = 0x3A5F8F;
const CLEAR_BOX: [[u32; 3]; 2] = [
    [0x8EC5EA, 0xEAF6FD, 0xD3EEFC],
    [0x6E9DC0, 0xC2E4F6, 0xA9D9F2],
];

/// The parts over the frame's glass, which keep their light art in dark Glass.
pub fn glass_painters() -> [(Part, Painter); 4] {
    [
        (Part::NavPill, pill_part),
        (Part::NavBack, |d| orb_part(false, d)),
        (Part::NavForward, |d| orb_part(true, d)),
        (Part::NavRecent, recent_part),
    ]
}

/// The search field's parts, remapped in dark Glass with the other fields.
pub fn painters() -> [(Part, Painter); 3] {
    [
        (Part::SearchField, |d| field_part(&FIELD, d)),
        (Part::SearchGlass, |d| glass_part(&FIELD, d)),
        (Part::SearchClear, |d| clear_part(&CLEAR_INKS, d)),
    ]
}

pub(super) fn stack(frames: impl IntoIterator<Item = Canvas>) -> Canvas {
    let frames: Vec<Canvas> = frames.into_iter().collect();
    let w = frames.first().map_or(0, |f| f.width);
    let h: u32 = frames.iter().map(|f| f.height).sum();
    let mut a = Canvas::new(w, h);
    let mut y = 0;
    for f in &frames {
        a.blit_over(0, y, f);
        y += f.height as i32;
    }
    a
}

fn orb_inks(state: State) -> &'static Orb {
    match state {
        State::Hot => &ORB_HOT,
        State::Pressed => &ORB_PRESSED,
        State::Disabled => &ORB_GREYED,
        _ => &ORB_REST,
    }
}

fn orb_part(forward: bool, d: i32) -> PartImage {
    orbs(orb_inks, forward, d)
}

/// Back's (or, `forward`, Forward's) ball in each of [`BUTTON_STATES`], inked by `inks`.
pub(super) fn orbs(inks: fn(State) -> &'static Orb, forward: bool, d: i32) -> PartImage {
    let d = d.max(1);
    PartImage::new(
        stack(BUTTON_STATES.map(|s| orb(inks(s), forward, d))),
        &BUTTON_STATES,
        Layout::Vertical,
    )
    .at_scale(d)
}

fn stops<const N: usize>(s: &[(f64, u32); N]) -> [(f64, Px); N] {
    s.map(|(t, v)| (t, solid(v)))
}

/// A 25 px glass ball: a 1 px ring, a gloss over the top half that breaks at the middle into a
/// deep blue lightening downwards with a glow along its lower rim, and the arrow.
fn orb(o: &Orb, forward: bool, d: i32) -> Canvas {
    let k = f64::from(d);
    let r = f64::from(ORB) / 2.0;
    let (upper, lower) = (stops(&o.upper), stops(&o.lower));
    let ring = [
        (0.0, solid(o.ring[0])),
        (f64::from(ORB - 1), solid(o.ring[1])),
    ];
    let flip = |pts: &[(f64, f64)], dy: f64| -> Vec<(f64, f64)> {
        pts.iter()
            .map(|&(x, y)| {
                let x = if forward { f64::from(ORB) - x } else { x };
                (x * k, (y + dy) * k)
            })
            .collect()
    };
    let (arrow, shadow) = (flip(&ARROW, 0.0), flip(&ARROW, 1.0));
    draw(ORB, ORB, d, |x, y| {
        let outer = ellipse(x, y, r * k, r * k, r * k, r * k);
        if outer <= 0.0 {
            return CLEAR;
        }
        let inner = ellipse(x, y, r * k, r * k, (r - 1.0) * k, (r - 1.0) * k);
        let (cx, cy) = (centre(x as i32, d), centre(y as i32, d));
        let mut body = if cy < 11.0 {
            Stops(&upper).at(cy)
        } else {
            Stops(&lower).at(cy)
        };
        if let Some(rim) = o.rim {
            let from_edge = r - 0.5 - (cx - (r - 0.5)).hypot(cy - (r - 0.5));
            let low = ((cy - 13.0) / 8.0).clamp(0.0, 1.0);
            let near = (1.0 - from_edge / 3.0).clamp(0.0, 1.0);
            body = crate::shape::mix(body, solid(rim), low * near * 0.85);
        }
        let ball = shares(&[(Stops(&ring).at(cy), outer - inner), (body, inner)]);
        let dark = crate::shape::scale(premul(BLACK, ARROW_SHADOW), polygon(x, y, &shadow));
        let ink = crate::shape::scale(premul(rgb(o.arrow.0), o.arrow.1), polygon(x, y, &arrow));
        over(ink, over(dark, ball))
    })
}

fn pill_part(d: i32) -> PartImage {
    let d = d.max(1);
    let k = f64::from(d);
    let (w, h) = (f64::from(PILL_W), f64::from(PILL_H));
    let face = PILL_FACE.map(|(v, a)| premul(rgb(v), a));
    let line = premul(rgb(PILL_RING.0), PILL_RING.1);
    let atlas = draw(PILL_W, PILL_H, d, |x, y| {
        let outer = rounded(x, y, w * k, h * k, h / 2.0 * k);
        let inner = rounded(
            x - k,
            y - k,
            (w - 2.0) * k,
            (h - 2.0) * k,
            (h / 2.0 - 1.0) * k,
        );
        let t = centre(y as i32, d) / (h - 1.0);
        let fill = Stops(&[(0.0, face[0]), (1.0, face[1])]).at(t);
        shares(&[(line, outer - inner), (fill, inner)])
    });
    PartImage::new(atlas, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(15, 0, 15, 0))
        .at_scale(d)
}

fn recent_part(d: i32) -> PartImage {
    recent(&RECENT, d)
}

/// The recent places drop's triangle and its outline, and greyed, the two of them.
pub(super) struct RecentInks {
    pub fill: u32,
    pub line: u32,
    pub greyed: [u32; 2],
}

const RECENT: RecentInks = RecentInks {
    fill: RECENT_FILL,
    line: RECENT_LINE,
    greyed: RECENT_GREYED,
};

/// The drop's 7 x 4 triangle, its outline round it, and the plate it lights on.
pub(super) fn recent(inks: &RecentInks, d: i32) -> PartImage {
    let d = d.max(1);
    let k = f64::from(d);
    let mid = f64::from(RECENT_H) / 2.0;
    let tri = |grow: f64| -> Vec<(f64, f64)> {
        let (l, r, top) = (4.0 - grow, 11.0 + grow, mid - 2.0 - grow * 0.5);
        [(l, top), (r, top), (7.5, mid + 2.0 + grow)]
            .map(|(a, b)| (a * k, b * k))
            .to_vec()
    };
    let (outline, fill) = (tri(1.0), tri(0.0));
    let frames = BUTTON_STATES.map(|state| {
        let [ink, line] = match state {
            State::Disabled => inks.greyed.map(solid),
            _ => [solid(inks.fill), solid(inks.line)],
        };
        let plate = match state {
            State::Hot => Some((premul(WHITE, 0x90), [0x60, 0x28])),
            State::Pressed => Some((premul(BLACK, 0x50), [0x00, 0x00])),
            _ => None,
        };
        draw(RECENT_W, RECENT_H, d, |x, y| {
            let base = plate.map_or(CLEAR, |(edge, face)| {
                let (w, h) = (f64::from(RECENT_W) * k, f64::from(RECENT_H) * k);
                let outer = rounded(x, y, w, h, 2.0 * k);
                let inner = rounded(x - k, y - k, w - 2.0 * k, h - 2.0 * k, k);
                let t = centre(y as i32, d) / f64::from(RECENT_H - 1);
                let a = f64::from(face[0]) + (f64::from(face[1]) - f64::from(face[0])) * t;
                let fill = match state {
                    State::Pressed => premul(BLACK, 0x28),
                    _ => premul(WHITE, a.round() as u8),
                };
                shares(&[(edge, outer - inner), (fill, inner)])
            });
            let edge = crate::shape::scale(line, polygon(x, y, &outline));
            let body = crate::shape::scale(ink, polygon(x, y, &fill));
            over(body, over(edge, base))
        })
    });
    PartImage::new(stack(frames), &BUTTON_STATES, Layout::Vertical).at_scale(d)
}

/// A 1 px edge darker at the top, a light line inside it and a white face the glass shows
/// through until the field holds the keyboard.
pub(super) fn field_part(inks: &FieldInks, d: i32) -> PartImage {
    let d = d.max(1);
    let edge = stops(&inks.edge);
    let frames = FIELD_STATES
        .iter()
        .zip(inks.alpha)
        .map(|(_, [e, line, face])| {
            draw(FIELD_W, FIELD_H, d, |x, y| {
                let (col, row) = (x as i32 / d, y as i32 / d);
                let ring = col.min(row).min(FIELD_W - 1 - col).min(FIELD_H - 1 - row);
                match ring {
                    0 => crate::shape::scale(
                        Stops(&edge).at(centre(y as i32, d)),
                        f64::from(e) / 255.0,
                    ),
                    1 => premul(WHITE, line),
                    _ => premul(WHITE, face),
                }
            })
        });
    PartImage::new(stack(frames), &FIELD_STATES, Layout::Vertical)
        .sizing(Insets::new(2, 2, 2, 2))
        .at_scale(d)
}

/// A lens ring with a handle down to the right.
pub(super) fn glass_part(inks: &FieldInks, d: i32) -> PartImage {
    let d = d.max(1);
    let k = f64::from(d);
    let ink = solid(inks.magnifier);
    let (cx, cy) = (5.0 * k, 5.0 * k);
    let atlas = draw(GLASS_SIDE, GLASS_SIDE, d, |x, y| {
        let out = ellipse(x, y, cx, cy, 4.25 * k, 4.25 * k);
        let inn = ellipse(x, y, cx, cy, 2.75 * k, 2.75 * k);
        let handle = stroke(x, y, &[(8.0 * k, 8.0 * k), (11.2 * k, 11.2 * k)], 2.2 * k);
        let ring = (out - inn).max(handle).clamp(0.0, 1.0);
        let lens = crate::shape::scale(premul(WHITE, inks.lens), inn);
        over(crate::shape::scale(ink, ring), lens)
    });
    PartImage::new(atlas, &[State::Normal], Layout::Vertical).at_scale(d)
}

/// The cross alone at rest, on a light box while hot and a deeper one while held.
pub(super) fn clear_part(inks: &ClearInks, d: i32) -> PartImage {
    let d = d.max(1);
    let k = f64::from(d);
    let cross = glyph(Glyph::Cross, CROSS, d, 1.5, solid(inks.cross));
    let frames = CLEAR_STATES.map(|state| {
        let inks = match state {
            State::Hot => Some(inks.boxes[0]),
            State::Pressed => Some(inks.boxes[1]),
            _ => None,
        };
        let mut c = draw(CLEAR_W, CLEAR_H, d, |x, y| {
            let Some([line, top, bottom]) = inks else {
                return CLEAR;
            };
            let (w, h) = (f64::from(CLEAR_W) * k, f64::from(CLEAR_H) * k);
            let outer = rounded(x, y, w, h, k);
            let inner = rounded(x - k, y - k, w - 2.0 * k, h - 2.0 * k, 0.0);
            let t = centre(y as i32, d) / f64::from(CLEAR_H - 1);
            let face = Stops(&[(0.0, solid(top)), (1.0, solid(bottom))]).at(t);
            shares(&[(solid(line), outer - inner), (face, inner)])
        });
        let at = |n: i32| (n - CROSS) / 2 * d;
        c.blit_over(at(CLEAR_W), at(CLEAR_H), &cross);
        c
    });
    PartImage::new(stack(frames), &CLEAR_STATES, Layout::Vertical).at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Mode, Settings, style};
    use crate::pins::fnv;
    use crate::raster::Rect;

    fn frame(mode: Mode, part: Part, state: State, d: i32) -> Canvas {
        let st = style(&Settings {
            mode,
            ..Settings::default()
        })
        .at_scale(d);
        let img = st.part(part).expect("the part is built");
        let r = img.frame(state);
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        for y in 0..r.height {
            for x in 0..r.width {
                c.put_raw(x, y, img.atlas.get(r.x + x, r.y + y));
            }
        }
        c
    }

    const ALL: [(Part, &[State]); 7] = [
        (Part::NavPill, &[State::Normal]),
        (Part::NavBack, &BUTTON_STATES),
        (Part::NavForward, &BUTTON_STATES),
        (Part::NavRecent, &BUTTON_STATES),
        (Part::SearchField, &FIELD_STATES),
        (Part::SearchGlass, &[State::Normal]),
        (Part::SearchClear, &CLEAR_STATES),
    ];

    #[test]
    fn nav_row_parts_pin_per_state_mode_and_scale() {
        let mut got = Vec::new();
        for mode in Mode::ALL {
            for (part, states) in ALL {
                for &state in states {
                    for d in [1, 2] {
                        let key = format!("{mode:?} {part:?} {state:?} @{d}");
                        got.push((key, fnv(&frame(mode, part, state, d))));
                    }
                }
            }
        }
        crate::pins::check("glass/navrow", &got);
    }

    #[test]
    fn each_button_state_draws_differently() {
        for part in [Part::NavBack, Part::NavForward, Part::NavRecent] {
            let drawn: Vec<u64> = BUTTON_STATES
                .iter()
                .map(|&s| fnv(&frame(Mode::Light, part, s, 1)))
                .collect();
            for i in 0..drawn.len() {
                assert!(
                    !drawn[i + 1..].contains(&drawn[i]),
                    "{part:?} {:?} is its own face",
                    BUTTON_STATES[i]
                );
            }
        }
    }

    #[test]
    fn the_orbs_corners_are_clear_and_their_curve_is_soft_at_scale_2() {
        for part in [Part::NavBack, Part::NavForward, Part::NavPill] {
            let c = frame(Mode::Light, part, State::Normal, 2);
            let (w, h) = (c.width as i32, c.height as i32);
            for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                assert_eq!(c.get(x, y) >> 24, 0, "{part:?} corner ({x}, {y})");
            }
            let partial = (0..w)
                .flat_map(|x| (0..h).map(move |y| (x, y)))
                .filter(|&(x, y)| (1..0xFF).contains(&(c.get(x, y) >> 24)))
                .count();
            assert!(partial >= 8, "{part:?}: an anti-aliased edge, {partial} px");
        }
    }

    #[test]
    fn forward_is_back_mirrored() {
        for state in BUTTON_STATES {
            let (b, f) = (
                frame(Mode::Light, Part::NavBack, state, 1),
                frame(Mode::Light, Part::NavForward, state, 1),
            );
            for y in 0..ORB {
                for x in 0..ORB {
                    let (l, r) = (b.get(x, y), f.get(ORB - 1 - x, y));
                    let off = |s: u32| ((l >> s) & 0xFF).abs_diff((r >> s) & 0xFF);
                    assert!(
                        [0, 8, 16, 24].into_iter().all(|s| off(s) <= 0x30),
                        "{state:?} ({x}, {y}): {l:08X} vs {r:08X}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_orbs_keep_their_colours_in_dark_glass_and_the_search_field_darkens() {
        for part in [Part::NavBack, Part::NavPill, Part::NavRecent] {
            assert_eq!(
                fnv(&frame(Mode::Light, part, State::Normal, 1)),
                fnv(&frame(Mode::Dark, part, State::Normal, 1)),
                "{part:?} over the glass"
            );
        }
        let (light, dark) = (
            frame(Mode::Light, Part::SearchField, State::Focused, 1),
            frame(Mode::Dark, Part::SearchField, State::Focused, 1),
        );
        let lum = |p: u32| ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF);
        assert!(
            lum(dark.get(3, 11)) < lum(light.get(3, 11)) / 2,
            "a dark face"
        );
    }

    #[test]
    fn the_search_frame_doubles_its_insets_at_scale_2() {
        let one = style(&Settings::default());
        let two = one.at_scale(2);
        let (a, b) = (
            one.part(Part::SearchField).unwrap(),
            two.part(Part::SearchField).unwrap(),
        );
        assert_eq!(a.sizing, b.sizing, "design insets");
        assert_eq!(b.scale, 2);
        assert_eq!(
            (b.atlas.width, b.atlas.height),
            (2 * a.atlas.width, 2 * a.atlas.height)
        );
        let mut c = Canvas::new(110, 22);
        one.draw(
            &mut c,
            Rect::new(0, 0, 109, 22),
            Part::SearchField,
            State::Focused,
            1,
        );
        assert_eq!(c.get(50, 11), 0xFFFF_FFFF, "a focused field is white");
        assert!(
            c.get(108, 11) >> 24 > 0xC0,
            "its right edge lands at the rect's"
        );
    }
}
