//! Glass's breadcrumb address parts: the field's frame, translucent on the frame's glass and
//! white while editing, and the box a segment, chevron or field button lights in.

use crate::raster::{Canvas, Insets};
use crate::shape::{Px, Stops, centre, draw, premul};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

/// The field's frame states, in atlas order.
const FIELD_STATES: [State; 3] = [State::Normal, State::Hot, State::Focused];
/// A box's states: lit, pressed or its menu open, and the grey of a lit box's partner.
const BOX_STATES: [State; 3] = [State::Hot, State::Pressed, State::Checked];

const FIELD_W: i32 = 7;
const FIELD_H: i32 = 22;
const BOX_W: i32 = 7;
/// A box fills the field inside its frame's top and bottom lines.
const BOX_H: i32 = FIELD_H - 2;
/// Rows a box keeps at its top and bottom when it stretches: the gloss's split stays put.
const BOX_TOP: i32 = 11;
const BOX_BOTTOM: i32 = 2;

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

const fn rgb(r: u8, g: u8, b: u8) -> Px {
    (255.0, [r as f64, g as f64, b as f64])
}

const FIELD_EDGE: [(f64, Px); 2] = [(0.0, rgb(0x31, 0x3E, 0x4B)), (21.0, rgb(0x64, 0x7E, 0x99))];
const FIELD_EDGE_FOCUSED: [(f64, Px); 2] =
    [(0.0, rgb(0x3D, 0x7B, 0xAD)), (21.0, rgb(0xB7, 0xD9, 0xED))];
/// White over the glass: the face, its 1 px inner line, at rest and lit. Editing is opaque.
const FIELD_FACE_ALPHA: u8 = 150;
const FIELD_FACE_HOT_ALPHA: u8 = 217;
const FIELD_LINE_ALPHA: u8 = 215;
const FIELD_LINE_HOT_ALPHA: u8 = 235;

struct BoxInks {
    edge: Px,
    /// The 1 px line inside the edge, down the box.
    line: [(f64, Px); 2],
    /// The gloss, its two halves split at [`SPLIT`].
    face: [(f64, Px); 4],
    /// A pressed box's shade down its top and left: the two lines inside the edge.
    shade: Option<[Px; 2]>,
}

const SPLIT: f64 = 9.5;

const HOT: BoxInks = BoxInks {
    edge: rgb(0x3C, 0x7F, 0xB1),
    line: [(0.0, rgb(0xF9, 0xFD, 0xFE)), (19.0, rgb(0xEB, 0xF7, 0xFD))],
    face: [
        (1.0, rgb(0xE8, 0xF6, 0xFD)),
        (SPLIT, rgb(0xDD, 0xF1, 0xFC)),
        (SPLIT, rgb(0xBA, 0xE4, 0xFC)),
        (18.0, rgb(0xA6, 0xD9, 0xF4)),
    ],
    shade: None,
};

const PRESSED: BoxInks = BoxInks {
    edge: rgb(0x2C, 0x62, 0x8B),
    line: [(0.0, rgb(0xC2, 0xE4, 0xF6)), (19.0, rgb(0x90, 0xCB, 0xEB))],
    face: [
        (1.0, rgb(0xC2, 0xE4, 0xF6)),
        (SPLIT, rgb(0xC2, 0xE4, 0xF6)),
        (SPLIT, rgb(0xA3, 0xD5, 0xF1)),
        (19.0, rgb(0x90, 0xCB, 0xEB)),
    ],
    shade: Some([rgb(0x6E, 0x8D, 0x9F), rgb(0xA6, 0xC7, 0xD9)]),
};

const PARTNER: BoxInks = BoxInks {
    edge: rgb(0x8E, 0x8F, 0x8F),
    line: [(0.0, rgb(0xFC, 0xFC, 0xFC)), (19.0, rgb(0xF4, 0xF4, 0xF4))],
    face: [
        (1.0, rgb(0xF2, 0xF2, 0xF2)),
        (SPLIT, rgb(0xED, 0xED, 0xED)),
        (SPLIT, rgb(0xDB, 0xDB, 0xDB)),
        (18.0, rgb(0xCF, 0xCF, 0xCF)),
    ],
    shade: None,
};

/// The parts and their painters, each painted the first time it is drawn.
type Paint = fn(i32) -> PartImage;

pub fn painters() -> [(Part, Paint); 2] {
    [
        (Part::CrumbField, field_part),
        (Part::CrumbButton, box_part),
    ]
}

fn stack(frames: impl IntoIterator<Item = Canvas>) -> Canvas {
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

fn field_part(d: i32) -> PartImage {
    let d = d.max(1);
    PartImage::new(
        stack(FIELD_STATES.map(|s| field_frame(s, d))),
        &FIELD_STATES,
        Layout::Vertical,
    )
    .sizing(Insets::new(2, 2, 2, 2))
    .at_scale(d)
}

/// The field: a 1 px edge darker at the top, a light line inside it and a white face.
fn field_frame(state: State, d: i32) -> Canvas {
    let (edge, face, line) = match state {
        State::Focused => (FIELD_EDGE_FOCUSED, 255, 255),
        State::Hot => (FIELD_EDGE, FIELD_FACE_HOT_ALPHA, FIELD_LINE_HOT_ALPHA),
        _ => (FIELD_EDGE, FIELD_FACE_ALPHA, FIELD_LINE_ALPHA),
    };
    draw(FIELD_W, FIELD_H, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        let ring = ring_of(col, row, FIELD_W, FIELD_H);
        match ring {
            0 => Stops(&edge).at(centre(y as i32, d)),
            1 => premul(WHITE, line),
            _ => premul(WHITE, face),
        }
    })
}

fn box_part(d: i32) -> PartImage {
    let d = d.max(1);
    PartImage::new(
        stack([HOT, PRESSED, PARTNER].map(|inks| box_frame(&inks, d))),
        &BOX_STATES,
        Layout::Vertical,
    )
    .sizing(Insets::new(2, BOX_TOP, 2, BOX_BOTTOM))
    .at_scale(d)
}

/// A box: an edge down each side only (the field's own lines close it at the top and bottom),
/// a light line inside, and a gloss split across the middle.
fn box_frame(inks: &BoxInks, d: i32) -> Canvas {
    draw(BOX_W, BOX_H, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        let t = centre(y as i32, d);
        if col == 0 || col == BOX_W - 1 {
            return inks.edge;
        }
        if let Some([outer, inner]) = inks.shade {
            match (col.min(row + 1), row) {
                (1, _) | (_, 0) => return outer,
                (2, _) | (_, 1) => return inner,
                _ => {}
            }
        }
        if ring_of(col, row, BOX_W, BOX_H) <= 1 && inks.shade.is_none() {
            return Stops(&inks.line).at(t);
        }
        Stops(&inks.face).at(t)
    })
}

/// How many whole px (`col`, `row`) lies in from the edge of a `w` x `h` rect.
fn ring_of(col: i32, row: i32, w: i32, h: i32) -> i32 {
    col.min(row).min(w - 1 - col).min(h - 1 - row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Mode, Settings, build_at};

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    fn part(mode: Mode, part: Part, d: i32) -> PartImage {
        let s = Settings {
            mode,
            ..Settings::default()
        };
        build_at(&s, d)
            .part(part)
            .expect("glass paints the part")
            .clone()
    }

    #[test]
    fn crumb_parts_pin_at_scale_1_and_2() {
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            for p in [Part::CrumbField, Part::CrumbButton] {
                for d in [1, 2] {
                    got.push((format!("{mode:?} {p:?} {d}"), fnv(&part(mode, p, d).atlas)));
                }
            }
        }
        crate::pins::check("glass/crumbs", &got);
    }

    #[test]
    fn crumb_parts_double_exactly_at_scale_2() {
        for p in [Part::CrumbField, Part::CrumbButton] {
            let (one, two) = (part(Mode::Light, p, 1), part(Mode::Light, p, 2));
            assert_eq!(two.atlas.width, one.atlas.width * 2, "{p:?}");
            assert_eq!(two.atlas.height, one.atlas.height * 2, "{p:?}");
            assert_eq!(one.sizing, two.sizing, "{p:?} insets are design px");
        }
    }

    #[test]
    fn every_crumb_state_draws_its_own_frame() {
        for (p, n, h) in [
            (Part::CrumbField, FIELD_STATES.len(), FIELD_H),
            (Part::CrumbButton, BOX_STATES.len(), BOX_H),
        ] {
            let img = part(Mode::Light, p, 1);
            let frame = |i: usize| -> Vec<u32> {
                let y0 = i as i32 * h;
                (y0..y0 + h)
                    .flat_map(|y| (0..img.atlas.width as i32).map(move |x| (x, y)))
                    .map(|(x, y)| img.atlas.get(x, y))
                    .collect()
            };
            let frames: Vec<_> = (0..n).map(frame).collect();
            for i in 0..n {
                for j in i + 1..n {
                    assert_ne!(frames[i], frames[j], "{p:?} states {i} and {j} differ");
                }
            }
        }
    }

    #[test]
    fn the_field_at_rest_lets_the_glass_through_and_is_opaque_while_editing() {
        let img = part(Mode::Light, Part::CrumbField, 1);
        let mid = |state: usize| {
            img.atlas
                .get(FIELD_W / 2, state as i32 * FIELD_H + FIELD_H / 2)
        };
        assert_eq!(mid(0) >> 24, u32::from(FIELD_FACE_ALPHA));
        assert_eq!(mid(1) >> 24, u32::from(FIELD_FACE_HOT_ALPHA));
        assert_eq!(mid(2), 0xFFFF_FFFF);
    }

    #[test]
    fn a_lit_box_is_edged_down_its_sides_only() {
        let img = part(Mode::Light, Part::CrumbButton, 1);
        assert_eq!(img.atlas.get(0, 5), 0xFF3C_7FB1);
        assert_eq!(img.atlas.get(BOX_W - 1, 5), 0xFF3C_7FB1);
        assert_ne!(img.atlas.get(BOX_W / 2, 0), 0xFF3C_7FB1);
    }
}
