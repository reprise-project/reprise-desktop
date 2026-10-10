//! Glass's field frame, drop-down list, combo drop button and spin button
//! parts, drawn at the output scale.

use crate::glass::Mode;
use crate::glass::dark::remap_color;
use crate::raster::{Canvas, Insets};
use crate::shape::{
    CLEAR, Glyph, Px, Stops, centre, draw, glyph, over, premul, rounded, rounded4, scale, shares,
};
use crate::style::{DropList, DropShadow, Layout, Part, PartImage, State};
use crate::theme::Color;

const DROP_LIST: DropList = DropList {
    top: Color::rgb(0x80, 0x83, 0x8B),
    sides: Color::rgb(0xC4, 0xC5, 0xD1),
    bottom: Color::rgb(0xD0, 0xD9, 0xE4),
    top_corners: Color::rgb(0xDC, 0xDE, 0xDF),
    bottom_corners: Color::rgb(0xF0, 0xEF, 0xF3),
    shadow: Some(&DROP_SHADOW),
};

/// The list's shadow, shared by menus and balloons: half black, softened
/// over 1.2 px, from a box 2 px past the list's right and bottom edges
/// and 7 px in from its top and left, so each strip fades in a few px
/// along and out to nothing 5 px past the list.
const DROP_SHADOW: DropShadow = DropShadow {
    opacity: 0.5,
    blur: 1.2,
    spread: 2.0,
    inset: 7.0,
};

pub fn drop_list(mode: Mode) -> DropList {
    match mode {
        Mode::Light => DROP_LIST,
        Mode::Dark => DropList {
            top: remap_color(DROP_LIST.top),
            sides: remap_color(DROP_LIST.sides),
            bottom: remap_color(DROP_LIST.bottom),
            top_corners: remap_color(DROP_LIST.top_corners),
            bottom_corners: remap_color(DROP_LIST.bottom_corners),
            ..DROP_LIST
        },
    }
}

const STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Disabled];

const FIELD_STATES: [State; 4] = [State::Normal, State::Hot, State::Focused, State::Disabled];

type Line = [Color; 4];

const FIELD_LINES: [Line; 4] = [
    [
        Color::rgb(0xA9, 0xAA, 0xB1),
        Color::rgb(0xE1, 0xE2, 0xE9),
        Color::rgb(0xDA, 0xDD, 0xE5),
        Color::rgb(0xE2, 0xE8, 0xEE),
    ],
    [
        Color::rgb(0x53, 0x8C, 0xBE),
        Color::rgb(0xC3, 0xD7, 0xEC),
        Color::rgb(0xB5, 0xD1, 0xE9),
        Color::rgb(0xC5, 0xDF, 0xF1),
    ],
    [
        Color::rgb(0x3C, 0x73, 0xA9),
        Color::rgb(0xB3, 0xCB, 0xE6),
        Color::rgb(0xA1, 0xC4, 0xE2),
        Color::rgb(0xB5, 0xD5, 0xEC),
    ],
    [Color::rgb(0xAC, 0xAD, 0xAD); 4],
];

const FIELD: i32 = 5;

const FIELD_RADIUS: f64 = 1.5;

/// The field line's inner corners: no more than 1 px, so the stretched
/// middle row and column inside the line stay clear.
const FIELD_INNER_RADIUS: f64 = 1.0;

#[derive(Clone, Copy)]
enum Fill {
    Run([Color; 2]),
    Split {
        at: f64,
        upper: [Color; 2],
        lower: [Color; 2],
    },
}

#[derive(Clone, Copy)]
struct Face {
    line: Color,
    inner_top: Color,
    inner_sides: [Color; 2],
    inner_bottom: Color,
    fill: Fill,
}

const HOT_LINE: Color = Color::rgb(0x3B, 0x77, 0xAD);

const HELD_LINE: Color = Color::rgb(0x2B, 0x5B, 0x87);

const HELD_SIDES: [Color; 2] = [Color::rgb(0xA8, 0xB9, 0xC5), Color::rgb(0x51, 0x8B, 0xB3)];

const LIST_CHEVRON_ROW: i32 = 9;

const LIST: [Face; 4] = [
    Face {
        line: Color::rgb(0x6D, 0x6D, 0x6D),
        inner_top: Color::rgb(0xFC, 0xFB, 0xFC),
        inner_sides: [Color::rgb(0xFA, 0xFB, 0xFB), Color::rgb(0xF2, 0xF3, 0xF3)],
        inner_bottom: Color::rgb(0xF2, 0xF3, 0xF3),
        fill: Fill::Split {
            at: LIST_CHEVRON_ROW as f64 + 1.0,
            upper: [Color::rgb(0xF2, 0xF1, 0xF2), Color::rgb(0xEA, 0xE9, 0xEA)],
            lower: [Color::rgb(0xDC, 0xDB, 0xDC), Color::rgb(0xCD, 0xCC, 0xCD)],
        },
    },
    Face {
        line: HOT_LINE,
        inner_top: Color::rgb(0xFA, 0xFD, 0xFD),
        inner_sides: [Color::rgb(0xF8, 0xFD, 0xFE), Color::rgb(0xE8, 0xF5, 0xFD)],
        inner_bottom: Color::rgb(0xE7, 0xF4, 0xFC),
        fill: Fill::Split {
            at: LIST_CHEVRON_ROW as f64 + 1.0,
            upper: [Color::rgb(0xE9, 0xF5, 0xFD), Color::rgb(0xD8, 0xEE, 0xFC)],
            lower: [Color::rgb(0xBC, 0xE2, 0xFD), Color::rgb(0xA4, 0xD4, 0xF5)],
        },
    },
    Face {
        line: HELD_LINE,
        inner_top: Color::rgb(0x94, 0xA6, 0xB2),
        inner_sides: HELD_SIDES,
        inner_bottom: Color::rgb(0x62, 0xA7, 0xD6),
        fill: Fill::Split {
            at: LIST_CHEVRON_ROW as f64 + 2.0,
            upper: [Color::rgb(0xDD, 0xEB, 0xF6), Color::rgb(0xC2, 0xE2, 0xF6)],
            lower: [Color::rgb(0x95, 0xCB, 0xEF), Color::rgb(0x62, 0xA7, 0xD6)],
        },
    },
    Face {
        line: Color::rgb(0xAB, 0xAF, 0xB3),
        inner_top: Color::rgb(0xFC, 0xFB, 0xFC),
        inner_sides: [Color::rgb(0xFC, 0xFB, 0xFC); 2],
        inner_bottom: Color::rgb(0xFC, 0xFB, 0xFC),
        fill: Fill::Run([Color::rgb(0xF3, 0xF4, 0xF4); 2]),
    },
];

const DROP_CHEVRON_ROW: i32 = 10;

const DROP_HOT: Face = Face {
    line: HOT_LINE,
    inner_top: Color::rgb(0xFA, 0xFD, 0xFD),
    inner_sides: [Color::rgb(0xFA, 0xFD, 0xFD); 2],
    inner_bottom: Color::rgb(0xFA, 0xFD, 0xFD),
    fill: Fill::Split {
        at: DROP_CHEVRON_ROW as f64 + 1.0,
        upper: [Color::rgb(0xE9, 0xF5, 0xFD), Color::rgb(0xD8, 0xEE, 0xFC)],
        lower: [Color::rgb(0xBC, 0xE2, 0xFD), Color::rgb(0xA6, 0xD6, 0xF6)],
    },
};

const DROP_HELD: Face = Face {
    line: HELD_LINE,
    inner_top: Color::rgb(0x9B, 0xAC, 0xB8),
    inner_sides: HELD_SIDES,
    inner_bottom: Color::rgb(0x5F, 0xA4, 0xD2),
    fill: Fill::Split {
        at: DROP_CHEVRON_ROW as f64 + 2.0,
        upper: [Color::rgb(0xDE, 0xEB, 0xF7), Color::rgb(0xC2, 0xE2, 0xF6)],
        lower: [Color::rgb(0x95, 0xCB, 0xEF), Color::rgb(0x5F, 0xA4, 0xD2)],
    },
};

const SPIN: [Face; 4] = [
    Face {
        line: Color::rgb(0xA9, 0xAA, 0xB1),
        inner_top: Color::rgb(0xFC, 0xFB, 0xFC),
        inner_sides: [Color::rgb(0xFA, 0xFB, 0xFB), Color::rgb(0xF2, 0xF3, 0xF3)],
        inner_bottom: Color::rgb(0xF2, 0xF3, 0xF3),
        fill: Fill::Run([Color::rgb(0xF2, 0xF1, 0xF2), Color::rgb(0xE7, 0xE6, 0xE7)]),
    },
    Face {
        line: HOT_LINE,
        inner_top: Color::rgb(0xFA, 0xFD, 0xFD),
        inner_sides: [Color::rgb(0xF8, 0xFD, 0xFE), Color::rgb(0xF4, 0xF9, 0xFE)],
        inner_bottom: Color::rgb(0xE8, 0xF4, 0xFC),
        fill: Fill::Run([Color::rgb(0xE8, 0xF5, 0xFD), Color::rgb(0xD0, 0xEA, 0xFC)]),
    },
    Face {
        line: HELD_LINE,
        inner_top: Color::rgb(0x6B, 0x88, 0x9D),
        inner_sides: [Color::rgb(0x6B, 0x88, 0x9D), Color::rgb(0x50, 0x78, 0x95)],
        inner_bottom: Color::rgb(0x8E, 0xC6, 0xEA),
        fill: Fill::Run([Color::rgb(0xC0, 0xE0, 0xF6), Color::rgb(0x92, 0xC8, 0xEB)]),
    },
    Face {
        line: Color::rgb(0xBC, 0xBD, 0xBD),
        inner_top: Color::rgb(0xED, 0xEC, 0xED),
        inner_sides: [Color::rgb(0xED, 0xEC, 0xED); 2],
        inner_bottom: Color::rgb(0xED, 0xEC, 0xED),
        fill: Fill::Run([Color::rgb(0xED, 0xEC, 0xED); 2]),
    },
];

#[derive(Clone, Copy)]
struct Chevron {
    glyph: Glyph,
    size: i32,
    at: (i32, i32),
    ink: [Color; 2],
}

const BLACK: Color = Color::rgb(0, 0, 0);

const GREYED_CHEVRON: Color = Color::rgb(0xAC, 0xAD, 0xAD);

const LIST_CHEVRON: Chevron = Chevron {
    glyph: Glyph::ArrowDown,
    size: 9,
    at: (10, LIST_CHEVRON_ROW - 3),
    ink: [BLACK; 2],
};

const DROP_CHEVRON: Chevron = Chevron {
    glyph: Glyph::ArrowDown,
    size: 9,
    at: (4, DROP_CHEVRON_ROW - 3),
    ink: [BLACK; 2],
};

const SPIN_CHEVRON: Chevron = Chevron {
    glyph: Glyph::ArrowUp,
    size: 7,
    at: (4, 1),
    ink: [Color::rgb(0x6C, 0x7D, 0xD0), Color::rgb(0x4B, 0x52, 0x75)],
};

const SPIN_GREYED_INK: [Color; 2] = [Color::rgb(0xC1, 0xC1, 0xC3), Color::rgb(0x79, 0x7B, 0x81)];

const LIST_SIZE: (i32, i32) = (23, 21);
const LIST_RADIUS: f64 = 3.0;

const DROP_SIZE: (i32, i32) = (17, 23);
const DROP_RADII: [f64; 4] = [0.0, FIELD_RADIUS, FIELD_RADIUS, 0.0];

const SPIN_SIZE: (i32, i32) = (15, 9);

pub fn parts(d: i32) -> [(Part, PartImage); 5] {
    let d = d.max(1);
    let field = PartImage {
        border_only: true,
        ..PartImage::new(
            stack(FIELD_LINES.iter().map(|l| field_frame(l, d))),
            &FIELD_STATES,
            Layout::Vertical,
        )
        .sizing(Insets::new(2, 2, 2, 2))
        .at_scale(d)
    };
    // The list's 4 left columns and its right 18 (the chevron's among
    // them) stay put. A taller list repeats row 7, in the light half, and
    // row 13, in the shaded half, one each for a 23 px list, so its face
    // steps and its chevron sit a row lower, in the middle; the rows
    // between them (the chevron's among them) keep their shape.
    let list = PartImage::new(
        stack(STATES.iter().zip(&LIST).map(|(&st, f)| {
            let chevron = Chevron {
                ink: chevron_ink(st),
                ..LIST_CHEVRON
            };
            button(LIST_SIZE, [LIST_RADIUS; 4], d, Some(f), chevron)
        })),
        &STATES,
        Layout::Vertical,
    )
    .sizing(Insets::new(4, 13, 18, 2))
    .two_bands(7, 13)
    .at_scale(d);
    // The button reaches from a column right of the drop rect's left edge
    // to the field's own right edge, and over the field's top and bottom
    // lines: a 16 px drop rect 2 px inside the field gives the 17 px
    // button. Its top 14 rows hold the chevron.
    let drop = PartImage::new(
        stack(STATES.iter().map(|&st| {
            let face = match st {
                State::Hot => Some(&DROP_HOT),
                State::Pressed => Some(&DROP_HELD),
                _ => None,
            };
            let chevron = Chevron {
                ink: chevron_ink(st),
                ..DROP_CHEVRON
            };
            button(DROP_SIZE, DROP_RADII, d, face, chevron)
        })),
        &STATES,
        Layout::Vertical,
    )
    .sizing(Insets::new(2, 14, 2, 2))
    .outset(Insets::new(-1, 2, 2, 2))
    .at_scale(d);
    let spin = |glyph: Glyph| {
        let frames = STATES.iter().zip(&SPIN).map(|(&st, f)| {
            let ink = match st {
                State::Disabled => SPIN_GREYED_INK,
                _ => SPIN_CHEVRON.ink,
            };
            let chevron = Chevron {
                glyph,
                ink,
                ..SPIN_CHEVRON
            };
            button(SPIN_SIZE, [0.0; 4], d, Some(f), chevron)
        });
        PartImage::new(stack(frames), &STATES, Layout::Vertical)
            .sizing(Insets::new(2, 6, 2, 2))
            .at_scale(d)
    };
    [
        (Part::EditBorder, field),
        (Part::ComboList, list),
        (Part::ComboDrop, drop),
        (Part::SpinUp, spin(Glyph::ArrowUp)),
        (Part::SpinDown, spin(Glyph::ArrowDown)),
    ]
}

fn chevron_ink(state: State) -> [Color; 2] {
    match state {
        State::Disabled => [GREYED_CHEVRON; 2],
        _ => [BLACK; 2],
    }
}

fn solid(c: Color) -> Px {
    premul(c, 255)
}

fn run(c: [Color; 2], a: f64, b: f64, y: f64) -> Px {
    let t = if b != a { (y - a) / (b - a) } else { 0.0 };
    Stops(&[(0.0, solid(c[0])), (1.0, solid(c[1]))]).at(t)
}

fn field_frame(line: &Line, d: i32) -> Canvas {
    let k = f64::from(d);
    let n = f64::from(FIELD);
    draw(FIELD, FIELD, d, |x, y| {
        let cover = rounded(x, y, n * k, n * k, FIELD_RADIUS * k)
            - rounded(
                x - k,
                y - k,
                (n - 2.0) * k,
                (n - 2.0) * k,
                FIELD_INNER_RADIUS * k,
            );
        if cover <= 0.0 {
            return CLEAR;
        }
        let (cx, cy) = (centre(x as i32, d), centre(y as i32, d));
        let far = [cy, cx, n - 1.0 - cx, n - 1.0 - cy];
        let near = far.iter().copied().fold(f64::INFINITY, f64::min);
        let sides: Vec<usize> = (0..4).filter(|&i| far[i] == near).collect();
        let each = cover.clamp(0.0, 1.0) / sides.len() as f64;
        let parts: Vec<(Px, f64)> = sides.iter().map(|&i| (solid(line[i]), each)).collect();
        shares(&parts)
    })
}

fn button(
    size: (i32, i32),
    radii: [f64; 4],
    d: i32,
    face: Option<&Face>,
    chevron: Chevron,
) -> Canvas {
    let (w, h) = size;
    let base = face.map(|f| {
        let last = f64::from(h - 3);
        ringed4(w, h, radii, 2, d, |_, y, ring| match (ring, f.fill) {
            (0, _) => solid(f.line),
            (1, _) if y < 1.5 => solid(f.inner_top),
            (1, _) if y > f64::from(h) - 2.5 => solid(f.inner_bottom),
            (1, _) => run(f.inner_sides, 1.0, f64::from(h - 2), y),
            (_, Fill::Run(c)) => run(c, 2.0, last, y),
            (_, Fill::Split { at, upper, .. }) if y < at - 0.5 => run(upper, 2.0, at - 1.0, y),
            (_, Fill::Split { at, lower, .. }) => run(lower, at, last, y),
        })
    });
    let mask = glyph(chevron.glyph, chevron.size, d, 1.0, solid(BLACK));
    let s = chevron.size;
    let (n, top) = (s / 2, (s - s / 2) / 2);
    let (wide, tip) = match chevron.glyph {
        Glyph::ArrowDown => (s - top - n, s - 1 - top),
        _ => (top + n - 1, top),
    };
    let (ox, oy) = (chevron.at.0 * d, chevron.at.1 * d);
    let row = (w * d) as usize;
    draw(w, h, d, |x, y| {
        let (xi, yi) = (x as i32, y as i32);
        let under = base
            .as_ref()
            .map_or(CLEAR, |b| b[yi as usize * row + xi as usize]);
        let (gx, gy) = (xi - ox, yi - oy);
        if gx < 0 || gy < 0 || gx >= mask.width as i32 || gy >= mask.height as i32 {
            return under;
        }
        let cover = f64::from(mask.get(gx, gy) >> 24) / 255.0;
        if cover == 0.0 {
            return under;
        }
        let ink = run(chevron.ink, f64::from(wide), f64::from(tip), centre(gy, d));
        over(scale(ink, cover), under)
    })
}

fn ringed4(
    w: i32,
    h: i32,
    r: [f64; 4],
    n: usize,
    d: i32,
    paint: impl Fn(f64, f64, usize) -> Px,
) -> Vec<Px> {
    let k = f64::from(d);
    let mut out = Vec::with_capacity((w * h * d * d) as usize);
    for y in 0..h * d {
        for x in 0..w * d {
            let cover = |i: usize| {
                let i = i as f64;
                rounded4(
                    f64::from(x) - i * k,
                    f64::from(y) - i * k,
                    f64::from(w) * k - 2.0 * i * k,
                    f64::from(h) * k - 2.0 * i * k,
                    r.map(|r| ((r - i) * k).max(0.0)),
                )
            };
            let mut parts = Vec::with_capacity(n + 1);
            let mut outer = cover(0);
            for i in 0..=n {
                let inner = if i < n { cover(i + 1) } else { 0.0 };
                if outer > inner {
                    parts.push((paint(centre(x, d), centre(y, d), i), outer - inner));
                }
                outer = inner;
            }
            out.push(shares(&parts));
        }
    }
    out
}

fn stack(frames: impl Iterator<Item = Canvas>) -> Canvas {
    let frames: Vec<Canvas> = frames.collect();
    let (w, h) = (frames[0].width, frames[0].height);
    let mut a = Canvas::new(w, h * frames.len() as u32);
    for (i, f) in frames.iter().enumerate() {
        a.blit_over(0, (i as u32 * h) as i32, f);
    }
    a
}

#[cfg(test)]
mod tests {
    use crate::controls::combo::{self, Combo};
    use crate::controls::updown;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::{
        ComboLook, draw_combo_drop, draw_combo_frame, draw_disabled_field, draw_edit_field,
    };
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, State};
    use crate::theme::Color;

    const FACE: u32 = 0xEFEEEF;

    #[test]
    fn the_list_shadow_fades_out_and_in_and_spares_the_list() {
        let sh = super::DROP_SHADOW;
        let (w, h) = (100.0, 60.0);
        let a = |x: f64, y: f64| (sh.alpha(w, h, x, y) * 255.0).round() as i32;
        let out: Vec<i32> = (0..5).map(|o| a(w + o as f64 + 0.5, 30.5)).collect();
        assert_eq!(out, [114, 84, 43, 13, 2], "next to the list and out");
        assert!(out.windows(2).all(|p| p[0] > p[1]), "fades out, {out:?}");
        assert!(out[4] <= 3, "nearly clear at the reach, {out:?}");
        assert_eq!(a(w + 5.5, 30.5), 0, "nothing past the reach");
        let down: Vec<i32> = (0..10).map(|y| a(w + 0.5, y as f64 + 0.5)).collect();
        assert!(down.windows(2).all(|p| p[0] <= p[1]), "fades in, {down:?}");
        assert!(down[3] < 10 && down[9] >= 105, "{down:?}");
        assert_eq!(a(50.5, 30.5), 0, "clear under the list");
        assert!(a(w + 0.5, h + 0.5) > a(w + 2.5, h + 2.5));
    }

    fn inputs(state: State) -> (bool, Interaction) {
        match state {
            State::Disabled => (false, Interaction::Idle),
            State::Hot => (true, Interaction::Hot),
            State::Pressed => (true, Interaction::Pressed),
            _ => (true, Interaction::Idle),
        }
    }

    fn field(state: State) -> (Canvas, Rect) {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let r = Rect::new(2, 2, 60, 21);
        let mut c = face(64, 25);
        let inner = match state {
            State::Disabled => draw_disabled_field(&mut c, &p, r),
            State::Focused => draw_edit_field(&mut c, &p, r, p.cs.window, true, Interaction::Idle),
            _ => draw_edit_field(&mut c, &p, r, p.cs.window, false, inputs(state).1),
        };
        (c, inner)
    }

    #[track_caller]
    fn pin(c: &Canvas, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(x, y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    fn face(w: u32, h: u32) -> Canvas {
        let mut c = Canvas::new(w, h);
        c.fill_rect(c.bounds(), Color::rgb(0xEF, 0xEE, 0xEF));
        c
    }

    const FIELD_PINS: [(State, [u32; 4]); 4] = [
        (State::Normal, [0xA9AAB1, 0xE1E2E9, 0xDADDE5, 0xE2E8EE]),
        (State::Hot, [0x538CBE, 0xC3D7EC, 0xB5D1E9, 0xC5DFF1]),
        (State::Focused, [0x3C73A9, 0xB3CBE6, 0xA1C4E2, 0xB5D5EC]),
        (State::Disabled, [0xACADAD; 4]),
    ];

    fn lum(p: u32) -> u32 {
        (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF)
    }

    #[test]
    fn every_glass_field_frame_state_is_a_1px_line_round_the_fill() {
        let r = Rect::new(2, 2, 60, 21);
        for (state, want) in FIELD_PINS {
            let (c, inner) = field(state);
            let what = format!("{state:?}");
            for ((x, y), rgb) in [(30, 2), (2, 12), (61, 12), (30, 22)].into_iter().zip(want) {
                pin(&c, x, y, rgb, &what);
            }
            let corner = lum(c.get(2, 2));
            assert!(
                corner > lum(want[0]) && corner < lum(0xFFFFFF),
                "{what}: a soft corner, {:08X}",
                c.get(2, 2)
            );
            let tint = lum(c.get(3, 3));
            assert!(
                tint < lum(0xFFFFFF) && tint > lum(want[0]),
                "{what}: the inner corner tinted, {:08X}",
                c.get(3, 3)
            );
            pin(
                &c,
                30,
                3,
                0xFFFFFF,
                &format!("{what}: the ring is the fill"),
            );
            pin(&c, 1, 1, FACE, &format!("{what}: nothing outside the rect"));
            assert_eq!(inner, r.inset(3), "the text goes 3 px in");
        }
    }

    #[test]
    fn the_clipboard_history_s_field_holds_the_focus_under_glass() {
        use crate::controls::cliphistory::{self, ClipHistory};
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = theme_for(Interface::Glass);
        let m = ClipHistory::new(Vec::new());
        let c = cliphistory::draw(&theme, &text, &m, false, None);
        let f = cliphistory::layout(&m, &theme.look.controls()).field;
        let at = crate::flyout::margin(&theme);
        pin(
            &c,
            at.left + f.x + f.width / 2,
            at.top + f.y,
            0x3C73A9,
            "the field's focused top line",
        );
    }

    #[test]
    fn a_focused_glass_edit_draws_its_3d7bad_frame() {
        use crate::controls::form::{self, Form, Item};
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = theme_for(Interface::Glass);
        let mut f = Form::new(vec![Item::edit("e", Rect::new(2, 2, 60, 21), "x", true)]);
        f.focus_id("e");
        let mut c = face(64, 25);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &f,
            false,
            theme.fonts.size_px,
        );
        pin(&c, 30, 2, 0x3C73A9, "the focused edit's top line");
    }

    fn list(state: State) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let r = Rect::new(2, 2, 140, 21);
        let drop = Rect::new(r.right() - 18, r.y + 2, 16, 17);
        let mut c = face(146, 25);
        c.fill_rect(r, p.cs.window);
        let (enabled, ptr) = inputs(state);
        let look = ComboLook {
            enabled,
            focused: false,
            ptr,
            editable: false,
        };
        draw_combo_frame(&mut c, &p, r, drop, p.cs.window, look);
        draw_combo_drop(&mut c, &p, drop, enabled, ptr, false);
        c
    }

    const LIST_PINS: [(State, [u32; 5]); 4] = [
        (
            State::Normal,
            [0x6D6D6D, 0xFCFBFC, 0xF2F1F2, 0xCDCCCD, 0x000000],
        ),
        (
            State::Hot,
            [0x3B77AD, 0xFAFDFD, 0xE9F5FD, 0xA4D4F5, 0x000000],
        ),
        (
            State::Pressed,
            [0x2B5B87, 0x94A6B2, 0xDDEBF6, 0x62A7D6, 0x000000],
        ),
        (
            State::Disabled,
            [0xABAFB3, 0xFCFBFC, 0xF3F4F4, 0xF3F4F4, 0xACADAD],
        ),
    ];

    #[test]
    fn a_glass_drop_down_list_is_one_button_with_its_chevron() {
        for (state, want) in LIST_PINS {
            let c = list(state);
            let what = format!("{state:?}");
            for ((x, y), rgb) in [(60, 2), (8, 3), (8, 4), (8, 20), (130, 11)]
                .into_iter()
                .zip(want)
            {
                pin(&c, x, y, rgb, &what);
            }
            pin(
                &c,
                136,
                11,
                want[4],
                &format!("{what}: the chevron's top row"),
            );
            pin(
                &c,
                133,
                14,
                want[4],
                &format!("{what}: the chevron's point"),
            );
            assert!(
                lum(c.get(2, 2)) >= lum(0xE7E6E7),
                "{what}: the rounded corner shows the face"
            );
        }
    }

    fn editable(drop_state: State) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let r = Rect::new(2, 2, 140, 21);
        let drop = Rect::new(r.right() - 18, r.y + 2, 16, 17);
        let mut c = face(146, 25);
        c.fill_rect(r, p.cs.window);
        let look = ComboLook {
            enabled: true,
            focused: false,
            ptr: Interaction::Idle,
            editable: true,
        };
        draw_combo_frame(&mut c, &p, r, drop, p.cs.window, look);
        let (enabled, ptr) = inputs(drop_state);
        draw_combo_drop(&mut c, &p, drop, enabled, ptr, true);
        c
    }

    #[test]
    fn an_editable_glass_combo_shows_its_chevron_alone_at_rest() {
        let c = editable(State::Normal);
        pin(&c, 130, 12, 0x000000, "the chevron's top row");
        pin(&c, 133, 15, 0x000000, "the chevron's point");
        pin(&c, 125, 5, 0xFFFFFF, "no button round it");
        pin(&c, 125, 2, 0xA9AAB1, "the field's top line runs on");
        pin(&c, 141, 12, 0xDADDE5, "the field's right line runs on");
        assert!(
            lum(c.get(140, 3)) < lum(0xFFFFFF),
            "the field's inner corner by the button keeps its tint"
        );
        let greyed = editable(State::Disabled);
        pin(&greyed, 130, 12, 0xACADAD, "the greyed chevron");
    }

    #[test]
    fn a_held_drop_button_keeps_its_unfocused_field_lit() {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let r = Rect::new(2, 2, 140, 21);
        let drop = Rect::new(r.right() - 18, r.y + 2, 16, 17);
        let mut c = face(146, 25);
        c.fill_rect(r, p.cs.window);
        let look = ComboLook {
            enabled: true,
            focused: false,
            ptr: Interaction::Pressed,
            editable: true,
        };
        draw_combo_frame(&mut c, &p, r, drop, p.cs.window, look);
        draw_combo_drop(&mut c, &p, drop, true, Interaction::Pressed, true);
        pin(&c, 60, 2, 0x538CBE, "the field's top line stays hot");
        pin(&c, 125, 12, 0x2B5B87, "the held button's left line");
    }

    #[test]
    fn a_hot_drop_button_s_right_end_shows_the_field_s_frame() {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let r = Rect::new(2, 2, 140, 21);
        let drop = Rect::new(r.right() - 18, r.y + 2, 16, 17);
        let lit = |focused: bool| {
            let mut c = face(146, 25);
            c.fill_rect(r, p.cs.window);
            let look = ComboLook {
                enabled: true,
                focused,
                ptr: Interaction::Hot,
                editable: true,
            };
            draw_combo_frame(&mut c, &p, r, drop, p.cs.window, look);
            draw_combo_drop(&mut c, &p, drop, true, Interaction::Hot, true);
            c
        };
        let (focused, unfocused) = (lit(true), lit(false));
        for (x, y) in [(141, 2), (141, 22)] {
            assert_ne!(
                focused.get(x, y),
                unfocused.get(x, y),
                "({x}, {y}) shows the field's corner"
            );
            assert_ne!(focused.get(x, y), 0xFF3B_77AD, "({x}, {y}) is not all line");
        }
        for c in [&focused, &unfocused] {
            pin(c, 139, 2, 0x3B77AD, "the button's own top line");
            pin(c, 141, 12, 0x3B77AD, "the button's right line");
        }
    }

    #[test]
    fn a_held_drop_button_is_outlined_along_its_top() {
        let c = editable(State::Pressed);
        pin(&c, 133, 2, 0x2B5B87, "the top line, as dark as the sides");
        pin(&c, 133, 3, 0x9BACB8, "the shade under it");
        pin(&c, 133, 4, 0xDEEBF7, "then the pale top of the fill");
        assert!(lum(c.get(133, 18)) < lum(0xC2E2F6), "the deeper fill below");
    }

    #[test]
    fn a_hot_or_held_drop_button_covers_the_field_s_right_end() {
        for (state, line, fill) in [
            (State::Hot, 0x3B77AD, 0xE9F5FD),
            (State::Pressed, 0x2B5B87, 0xDEEBF7),
        ] {
            let c = editable(state);
            let what = format!("{state:?}");
            pin(&c, 125, 12, line, &format!("{what}: its left line"));
            pin(&c, 133, 4, fill, &format!("{what}: its fill"));
            pin(&c, 130, 12, 0x000000, &format!("{what}: the chevron"));
            pin(
                &c,
                124,
                12,
                0xFFFFFF,
                &format!("{what}: the field left of it"),
            );
            pin(
                &c,
                142,
                12,
                FACE,
                &format!("{what}: nothing past the field"),
            );
        }
    }

    fn count(c: &Canvas, r: Rect, rgb: u32) -> usize {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == 0xFF00_0000 | rgb)
            .count()
    }

    fn focused_list(focus: bool, open: bool) -> Canvas {
        focused_list_with(focus, open, None)
    }

    fn focused_list_with(focus: bool, open: bool, icon: Option<Canvas>) -> Canvas {
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = theme_for(Interface::Glass);
        let mut cb = Combo::new(&["(None)", "Blank"], 0, 8);
        cb.open = open.then_some(0);
        cb.icon = icon;
        let lay = combo::layout(&cb, Rect::new(2, 2, 201, 23), &theme.look.controls());
        let mut c = face(206, 28);
        combo::draw(
            &mut c,
            &text,
            &theme.paint(),
            &cb,
            &lay,
            focus,
            true,
            crate::textedit::Caret::Unfocused,
            combo::ComboLit::IDLE,
            theme.fonts.size_px,
        );
        c
    }

    #[test]
    fn a_focused_glass_drop_down_list_rings_its_plain_item() {
        let (plain, focused) = (focused_list(false, false), focused_list(true, false));
        assert_eq!(
            count(&focused, Rect::new(2, 2, 201, 23), 0x2E8CFF),
            0,
            "no highlight"
        );
        let inverted = |x: i32, y: i32| (plain.get(x, y) ^ 0x00FF_FFFF) == focused.get(x, y);
        let (l, t, r, b) = (2 + 3, 2 + 3, 2 + 201 - 21, 2 + 23 - 4);
        for (x, y) in [(l + 1, t), (l, t + 1), (r, t), (l + 1, b), (r, b)] {
            assert!(inverted(x, y), "a dot at ({x}, {y})");
        }
        for (x, y) in [(l, t), (l + 2, t - 1), (r + 1, t + 2), (l - 1, t + 2)] {
            assert_eq!(plain.get(x, y), focused.get(x, y), "no dot at ({x}, {y})");
        }
        let text = Rect::new(l + 1, t + 1, r - l - 1, b - t - 1);
        assert_eq!(
            count(&plain, text, 0x000000),
            count(&focused, text, 0x000000),
            "the item inks black as it does unfocused"
        );
        let (open, open_plain) = (focused_list(true, true), focused_list(false, true));
        assert_eq!(
            open.get(l + 1, t),
            open_plain.get(l + 1, t),
            "open, no ring"
        );
    }

    #[test]
    fn a_focused_glass_list_with_an_icon_rings_right_of_it() {
        let icon = || {
            let mut i = Canvas::new(16, 16);
            i.fill_rect(Rect::new(0, 0, 16, 16), Color::rgb(0x1F, 0x7C, 0x24));
            Some(i)
        };
        let theme = theme_for(Interface::Glass);
        let cb = Combo {
            icon: icon(),
            ..Combo::new(&["x"], 0, 4)
        };
        let lay = combo::layout(&cb, Rect::new(2, 2, 201, 23), &theme.look.controls());
        let ir = lay.icon.expect("an icon rect");
        let (plain, focused) = (
            focused_list_with(false, false, icon()),
            focused_list_with(true, false, icon()),
        );
        let changed: Vec<(i32, i32)> = (0..28)
            .flat_map(|y| (0..206).map(move |x| (x, y)))
            .filter(|&(x, y)| plain.get(x, y) != focused.get(x, y))
            .collect();
        assert!(!changed.is_empty(), "a ring");
        let left = changed.iter().map(|p| p.0).min().unwrap();
        assert!(left > ir.right(), "the ring starts right of the icon");
        assert!(left <= ir.right() + 2, "a pixel or two right of it");
    }

    #[test]
    fn a_glass_greyed_combo_or_spinner_inks_its_text_6d6d6d() {
        use crate::controls::form::{self, Form, Item};
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = theme_for(Interface::Glass);
        let mut spin = updown::UpDown::new(10, 1, 9999);
        spin.enabled = false;
        let mut list = Combo::new(&["Greyed"], 0, 4);
        list.enabled = false;
        let f = Form::new(vec![
            Item::new(
                "c",
                Rect::new(2, 2, 120, combo::COMBO_H),
                form::Control::Combo(list),
            ),
            Item::new(
                "s",
                Rect::new(2, 30, updown::WIDTH, updown::HEIGHT),
                form::Control::UpDown(spin),
            ),
        ]);
        let mut c = face(126, 56);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &f,
            false,
            theme.fonts.size_px,
        );
        for (what, r) in [
            ("the greyed list", Rect::new(6, 5, 90, 15)),
            ("the greyed spinner", Rect::new(5, 33, 30, 14)),
        ] {
            assert!(count(&c, r, 0x6B6B6B) > 0, "{what}'s text inks 6B6B6B");
            assert_eq!(
                count(&c, r, 0x9D9E9E),
                0,
                "{what}'s text is not etched: no 9D9E9E"
            );
        }
    }

    #[test]
    fn a_greyed_glass_edit_inks_its_value_6d6d6d() {
        use crate::controls::form::{self, Form, Item};
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let draw = |i: Interface| {
            let theme = theme_for(i);
            let f = Form::new(vec![Item::edit("e", Rect::new(2, 2, 60, 21), "10", false)]);
            let mut c = face(64, 25);
            form::draw(
                &mut c,
                &text,
                &theme.paint(),
                &f,
                false,
                theme.fonts.size_px,
            );
            c
        };
        let inside = Rect::new(4, 4, 56, 17);
        let glass = draw(Interface::Glass);
        assert!(count(&glass, inside, 0x6B6B6B) > 0, "the value inks 6B6B6B");
        assert_eq!(count(&glass, inside, 0x000000), 0, "nothing inks black");
        assert!(count(&glass, inside, 0xFFFFFF) > 0, "on white");
        let blue = draw(Interface::Blue);
        assert!(
            count(&blue, inside, 0x000000) > 0,
            "Blue's value stays black"
        );
    }

    #[test]
    fn a_greyed_glass_spinner_draws_the_greyed_frame_round_white() {
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = theme_for(Interface::Glass);
        let mut ud = updown::UpDown::new(10, 1, 9999);
        ud.enabled = false;
        let lay = updown::layout(2, 2);
        let mut c = face(56, 24);
        updown::draw(
            &mut c,
            &text,
            &theme.paint(),
            &ud,
            &lay,
            false,
            None,
            theme.fonts.size_px,
        );
        pin(&c, 2, 12, 0xACADAD, "the greyed field's left line");
        pin(&c, 20, 2, 0xACADAD, "the greyed field's top line");
        pin(&c, 4, 10, 0xFFFFFF, "the greyed field is white");
        pin(
            &c,
            lay.up.x,
            lay.up.y,
            0xBCBDBD,
            "the greyed up button's corner",
        );
    }

    const SPIN_PINS: [(State, [u32; 3], [u32; 3]); 4] = [
        (
            State::Normal,
            [0xA9AAB1, 0xF2F1F2, 0x5C68A3],
            [0xA9AAB1, 0xF2F1F2, 0x5C68A3],
        ),
        (
            State::Hot,
            [0x3B77AD, 0xE8F5FD, 0x5C68A3],
            [0x3B77AD, 0xE8F5FD, 0x5C68A3],
        ),
        (
            State::Pressed,
            [0x2B5B87, 0xC0E0F6, 0x5C68A3],
            [0x2B5B87, 0xC0E0F6, 0x5C68A3],
        ),
        (
            State::Disabled,
            [0xBCBDBD, 0xEDECED, 0x9D9EA2],
            [0xBCBDBD, 0xEDECED, 0x9D9EA2],
        ),
    ];

    #[test]
    fn every_glass_spin_button_state_draws_its_own_box() {
        let st = crate::glass::style(&crate::glass::Settings::default());
        for (state, up, down) in SPIN_PINS {
            for (part, want) in [(Part::SpinUp, up), (Part::SpinDown, down)] {
                let mut c = face(19, 12);
                st.draw(&mut c, Rect::new(2, 2, 15, 8), part, state, 1);
                let what = format!("{part:?} {state:?}");
                for ((x, y), rgb) in [(2, 2), (5, 4), (9, 6)].into_iter().zip(want) {
                    pin(&c, x, y, rgb, &what);
                }
                pin(&c, 9, 9, want[0], &format!("{what}: the bottom line"));
            }
        }
    }

    #[test]
    fn a_glass_spinner_draws_its_field_and_buttons() {
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = theme_for(Interface::Glass);
        let ud = updown::UpDown::new(10, 1, 9999);
        let lay = updown::layout(2, 2);
        let mut c = face(56, 24);
        updown::draw(
            &mut c,
            &text,
            &theme.paint(),
            &ud,
            &lay,
            false,
            None,
            theme.fonts.size_px,
        );
        pin(&c, 20, 2, 0xA9AAB1, "the field's top line");
        pin(&c, lay.up.x, lay.up.y, 0xA9AAB1, "the up button's corner");
        pin(&c, lay.up.x + 7, lay.up.y + 4, 0x5C68A3, "the up chevron");
    }

    #[test]
    fn glass_fields_on_a_scale_2_canvas_double_every_pixel() {
        use crate::look::styled::fixture::doubles;
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let draw = |c: &mut Canvas| {
            c.fill_rect(Rect::new(0, 0, 146, 50), Color::rgb(0xEF, 0xEE, 0xEF));
            draw_edit_field(
                c,
                &p,
                Rect::new(2, 2, 60, 21),
                p.cs.window,
                true,
                Interaction::Idle,
            );
            let r = Rect::new(2, 26, 140, 21);
            let drop = Rect::new(r.right() - 18, r.y + 2, 16, 17);
            c.fill_rect(r, p.cs.window);
            let look = ComboLook {
                enabled: true,
                focused: false,
                ptr: Interaction::Idle,
                editable: false,
            };
            draw_combo_frame(c, &p, r, drop, p.cs.window, look);
            draw_combo_drop(c, &p, drop, true, Interaction::Idle, false);
        };
        let mut one = Canvas::new(146, 50);
        draw(&mut one);
        let mut two = Canvas::with_scale(146, 50, 2);
        draw(&mut two);
        assert!(doubles(&one, &two), "every pixel covers 2x2 device pixels");
    }

    #[test]
    fn an_editable_combo_s_lit_button_doubles_on_a_scale_2_canvas() {
        use crate::look::styled::fixture::doubles;
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let draw = |c: &mut Canvas| {
            c.fill_rect(Rect::new(0, 0, 146, 25), Color::rgb(0xEF, 0xEE, 0xEF));
            let r = Rect::new(2, 2, 140, 21);
            let drop = Rect::new(r.right() - 18, r.y + 2, 16, 17);
            c.fill_rect(r, p.cs.window);
            let look = ComboLook {
                enabled: true,
                focused: true,
                ptr: Interaction::Hot,
                editable: true,
            };
            draw_combo_frame(c, &p, r, drop, p.cs.window, look);
            draw_combo_drop(c, &p, drop, true, Interaction::Hot, true);
        };
        let mut one = Canvas::new(146, 25);
        draw(&mut one);
        let mut two = Canvas::with_scale(146, 25, 2);
        draw(&mut two);
        assert!(doubles(&one, &two), "every pixel covers 2x2 device pixels");
    }

    #[test]
    fn a_lit_drop_button_reaches_the_field_s_edge_at_paint_scale_2() {
        // A scale-1 canvas in device pixels, the paint at 2 per pixel of
        // the design, as the compositor draws at scale 2: the button's
        // reach over the field's line doubles too.
        let mut theme = theme_for(Interface::Glass);
        theme.scale = 2;
        let p = theme.paint();
        let r = Rect::new(4, 4, 280, 42);
        let drop = Rect::new(r.right() - 36, r.y + 4, 32, 34);
        let mut c = face(292, 50);
        c.fill_rect(r, p.cs.window);
        let look = ComboLook {
            enabled: true,
            focused: false,
            ptr: Interaction::Idle,
            editable: true,
        };
        draw_combo_frame(&mut c, &p, r, drop, p.cs.window, look);
        draw_combo_drop(&mut c, &p, drop, true, Interaction::Hot, true);
        pin(&c, 250, 24, 0x3B77AD, "its left line");
        pin(&c, 249, 24, 0xFFFFFF, "the field left of it");
        pin(&c, 266, 4, 0x3B77AD, "its top line over the field's");
        pin(&c, 266, 45, 0x3B77AD, "its bottom line over the field's");
        pin(&c, 284, 24, FACE, "nothing past the field");
    }

    #[test]
    fn glass_fields_at_paint_scale_2_double_every_pixel() {
        use crate::classic::widgets::Arrow;
        use crate::look::styled::fixture::doubles_at_paint_scale_2;
        use crate::look::widgets::{draw_spin_buttons, draw_spin_field};
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            let s = p.s;
            let white = p.cs.window;
            let edits = [
                (false, Interaction::Idle),
                (false, Interaction::Hot),
                (true, Interaction::Idle),
            ];
            for (i, (focused, ptr)) in edits.into_iter().enumerate() {
                let r = Rect::new(2, 2 + 24 * i as i32, 60, 21).scaled(s);
                draw_edit_field(c, p, r, white, focused, ptr);
            }
            draw_disabled_field(c, p, Rect::new(2, 74, 60, 21).scaled(s));
            for (i, state) in [State::Normal, State::Hot, State::Pressed, State::Disabled]
                .into_iter()
                .enumerate()
            {
                let (enabled, ptr) = inputs(state);
                for (col, editable) in [(0, false), (1, true)] {
                    let x = 66 + 84 * col;
                    let y = 2 + 24 * i as i32;
                    let r = Rect::new(x, y, 80, 21).scaled(s);
                    let drop = Rect::new(x + 62, y + 2, 16, 17).scaled(s);
                    c.fill_rect(r, white);
                    let look = ComboLook {
                        enabled,
                        focused: false,
                        ptr,
                        editable,
                    };
                    draw_combo_frame(c, p, r, drop, white, look);
                    draw_combo_drop(c, p, drop, enabled, ptr, editable);
                }
            }
            let lit = [
                (true, None),
                (true, Some((Arrow::Up, Interaction::Hot))),
                (true, Some((Arrow::Down, Interaction::Pressed))),
                (false, None),
            ];
            for (i, (enabled, lit)) in lit.into_iter().enumerate() {
                let y = 2 + 24 * i as i32;
                let r = Rect::new(236, y, 40, 21).scaled(s);
                // The field takes device rects, the buttons logical ones.
                let up = Rect::new(259, y + 2, 15, 8);
                let down = Rect::new(259, y + 11, 15, 8);
                draw_spin_field(c, p, r, up.scaled(s), down.scaled(s), enabled);
                draw_spin_buttons(c, p, up, down, enabled, lit);
            }
        };
        assert!(
            doubles_at_paint_scale_2(Interface::Glass, 280, 98, draw),
            "every Glass field part at s = 2 is its s = 1 pixels doubled"
        );
    }

    fn tall_list(state: State) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let r = Rect::new(2, 2, 140, 23);
        let mut c = face(146, 27);
        let (enabled, ptr) = inputs(state);
        let look = ComboLook {
            enabled,
            focused: false,
            ptr,
            editable: false,
        };
        draw_combo_frame(&mut c, &p, r, r, p.cs.window, look);
        c
    }

    fn column(c: &Canvas, x: i32, y0: i32, n: i32) -> Vec<u32> {
        (y0..y0 + n).map(|y| c.get(x, y) & 0xFF_FFFF).collect()
    }

    #[test]
    fn a_23px_glass_list_grows_a_row_in_each_half() {
        let rest = [
            0x6D6D6D, 0xFCFBFC, 0xF2F1F2, 0xF1F0F1, 0xF0EFF0, 0xEFEEEF, 0xEDECED, 0xECEBEC,
            0xECEBEC, 0xEBEAEB, 0xEAE9EA, 0xDCDBDC, 0xDAD9DA, 0xD8D7D8, 0xD6D5D6, 0xD6D5D6,
            0xD5D4D5, 0xD3D2D3, 0xD1D0D1, 0xCFCECF, 0xCDCCCD, 0xF2F3F3, 0x6D6D6D,
        ];
        assert_eq!(
            column(&tall_list(State::Normal), 60, 2, 23),
            rest,
            "ECEBEC and D6D7D7 each twice, the step to DCDBDC on row 11"
        );
        let hot = column(&tall_list(State::Hot), 60, 2, 23);
        assert_eq!(hot[7], hot[8], "hot: row 7 twice");
        assert_eq!(hot[14], hot[15], "hot: row 13 twice");
        assert_eq!(hot[11], 0xBCE2FD, "hot: the step on row 11");
        let c = tall_list(State::Normal);
        let end = column(&c, 136, 11, 2);
        assert_eq!(
            end,
            [0xEBEAEB, 0x000000],
            "the chevron's top row on row 10, its right end"
        );
        assert_eq!(
            column(&c, 133, 11, 5),
            [0xEBEAEB, 0x000000, 0x000000, 0x000000, 0x000000],
            "the chevron's middle column, rows 10..13"
        );
        let short = column(&list(State::Normal), 60, 2, 21);
        assert_eq!(short[7..10], [0xECEBEC, 0xEBEAEB, 0xEAE9EA]);
        assert_eq!(short[10], 0xDCDBDC, "the 21 px list steps on row 10");
    }

    fn open_list(mode: crate::glass::Mode, s: i32) -> (Canvas, Combo, combo::Layout) {
        let theme = crate::interfaces::theme_with(
            Interface::Glass,
            crate::blue::Variant::default(),
            &crate::glass::Settings {
                mode,
                ..crate::glass::Settings::default()
            },
        );
        let p = crate::look::Paint { s, ..theme.paint() };
        let mut cb = Combo::new(&["Show", "Hide", "Only"], 0, 8);
        cb.toggle();
        let lay = combo::layout(&cb, Rect::new(2, 2, 279, 23), &theme.look.controls());
        let mut c = Canvas::new(300 * s as u32, 80 * s as u32);
        c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        let text = crate::text::TextRenderer::load(&theme.fonts);
        combo::draw_list(
            &mut c,
            &text,
            &p,
            &cb.items,
            cb.open,
            cb.top,
            cb.rows,
            lay.list,
            lay.scrollbar,
            None,
            theme.fonts.size_px * s as u32,
        );
        (c, cb, lay)
    }

    #[test]
    fn a_glass_open_list_has_15px_rows_a_soft_frame_and_a_shadow() {
        let (c, cb, lay) = open_list(crate::glass::Mode::Light, 1);
        assert_eq!(lay.list, Rect::new(2, 25, 279, 47), "three 15 px rows");
        assert_eq!(lay.row_h, 15);
        for (x, y, rgb, what) in [
            (2, 25, 0xDCDEDF, "top-left corner"),
            (3, 25, 0x80838B, "top line"),
            (280, 25, 0xDCDEDF, "top-right corner"),
            (2, 26, 0xC4C5D1, "left line"),
            (280, 70, 0xC4C5D1, "right line"),
            (2, 71, 0xF0EFF3, "bottom-left corner"),
            (3, 71, 0xD0D9E4, "bottom line"),
            (280, 71, 0xF0EFF3, "bottom-right corner"),
            (3, 26, 0x2E8CFF, "the open row's highlight from the frame"),
            (279, 40, 0x2E8CFF, "the highlight's last pixel, row 15"),
            (270, 41, 0xFFFFFF, "the second row"),
            (281, 28, 0xFFFFFF, "no shadow beside the list's top"),
            (281, 29, 0xFDFDFD, "the right shadow's faint start"),
            (281, 30, 0xF3F3F3, "the right shadow fading in"),
            (281, 35, 0x8D8D8D, "the right shadow next to the list"),
            (282, 35, 0xABABAB, "the right shadow, 1 px out"),
            (285, 35, 0xFDFDFD, "the right shadow's outer column"),
            (286, 35, 0xFFFFFF, "past the right shadow"),
            (281, 72, 0x999999, "the corner next to the list's"),
            (285, 76, 0xFFFFFF, "the corner's outer pixel"),
            (5, 72, 0xFFFFFF, "no shadow under the list's left end"),
            (6, 72, 0xFDFDFD, "the bottom shadow's faint start"),
            (7, 72, 0xF3F3F3, "the bottom shadow fading in"),
            (12, 72, 0x8D8D8D, "the bottom shadow under the list"),
            (12, 76, 0xFDFDFD, "the bottom shadow's outer row"),
            (12, 77, 0xFFFFFF, "past the bottom shadow"),
        ] {
            pin(&c, x, y, rgb, what);
        }
        let inked = |y: i32| (3..100).any(|x| c.get(x, y) & 0xFF_FFFF != 0xFFFFFF);
        assert_eq!((41..56).find(|&y| inked(y)), Some(44), "Hide's cap top");
        assert_eq!(combo::hit(&cb, &lay, 50, 40), Some(combo::Hit::Row(0)));
        assert_eq!(combo::hit(&cb, &lay, 50, 41), Some(combo::Hit::Row(1)));
        assert_eq!(combo::hit(&cb, &lay, 50, 70), Some(combo::Hit::Row(2)));
        assert_eq!(combo::hit(&cb, &lay, 50, 71), None, "the frame's bottom");
        use crate::access::{ROOT, Tree};
        use accesskit::Role;
        let mut tree = Tree::window("T", (300, 80));
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        combo::access(&cb, &lay, &text, 11, &mut tree, ROOT, "c");
        let rows: Vec<(f64, f64)> = tree
            .nodes()
            .filter(|(_, n)| n.role() == Role::ListBoxOption)
            .filter_map(|(_, n)| n.bounds().map(|b| (b.y0, b.y1)))
            .collect();
        assert_eq!(rows, [(26.0, 41.0), (41.0, 56.0), (56.0, 71.0)]);
    }

    #[test]
    fn an_open_glass_list_draws_its_button_held_wherever_the_pointer_is() {
        let theme = theme_for(Interface::Glass);
        let mut cb = Combo::new(&["a", "b"], 0, 4);
        let r = Rect::new(2, 2, 140, 21);
        let draw = |cb: &Combo| {
            let mut c = face(146, 25);
            let lay = combo::layout(cb, r, &theme.look.controls());
            let text = crate::text::TextRenderer::load(&theme.fonts);
            combo::draw(
                &mut c,
                &text,
                &theme.paint(),
                cb,
                &lay,
                true,
                false,
                crate::textedit::Caret::Unfocused,
                combo::ComboLit::IDLE,
                theme.fonts.size_px,
            );
            c
        };
        pin(&draw(&cb), 60, 2, 0x6D6D6D, "closed, at rest");
        cb.toggle();
        pin(&draw(&cb), 60, 2, 0x2B5B87, "open: the held outline");
        pin(&draw(&cb), 8, 20, 0x62A7D6, "open: the held face low down");
    }

    #[test]
    fn a_dark_glass_open_list_remaps_its_frame_and_keeps_its_shadow() {
        use crate::glass::dark::remap_color;
        let (c, ..) = open_list(crate::glass::Mode::Dark, 1);
        let rgb = |col: Color| u32::from(col.r) << 16 | u32::from(col.g) << 8 | u32::from(col.b);
        pin(
            &c,
            3,
            25,
            rgb(remap_color(Color::rgb(0x80, 0x83, 0x8B))),
            "top line",
        );
        pin(
            &c,
            2,
            40,
            rgb(remap_color(Color::rgb(0xC4, 0xC5, 0xD1))),
            "left line",
        );
        pin(
            &c,
            3,
            71,
            rgb(remap_color(Color::rgb(0xD0, 0xD9, 0xE4))),
            "bottom line",
        );
        pin(&c, 281, 35, 0x8D8D8D, "the same shadow");
    }

    #[test]
    fn a_glass_open_list_at_scale_2_doubles_its_frame_and_redraws_its_shadow() {
        let (one, ..) = open_list(crate::glass::Mode::Light, 1);
        let (two, ..) = open_list(crate::glass::Mode::Light, 2);
        let shade = |c: &Canvas, x: i32, y: i32| (c.get(x, y) & 0xFF) as i32;
        assert_ne!(shade(&two, 562, 70), shade(&two, 563, 70), "smooth at 2");
        assert!((shade(&two, 563, 71) - shade(&one, 281, 35)).abs() <= 6);
        for y in 20..72 {
            for x in (100..281).chain(0..4) {
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    assert_eq!(
                        two.get(2 * x + dx, 2 * y + dy),
                        one.get(x, y),
                        "({x}, {y}) doubled"
                    );
                }
            }
        }
    }

    #[test]
    fn combo_layout_puts_the_drop_rect_where_the_tests_draw_it() {
        let theme = theme_for(Interface::Glass);
        let cb = Combo::new(&["a"], 0, 4);
        let lay = combo::layout(&cb, Rect::new(2, 2, 140, 21), &theme.look.controls());
        assert_eq!(lay.drop, Rect::new(124, 4, 16, 17), "the drop rect");
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        let mut eat = |b: &[u8]| {
            for &b in b {
                h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        eat(&c.width.to_le_bytes());
        eat(&c.height.to_le_bytes());
        for p in &c.pixels {
            eat(&p.to_le_bytes());
        }
        h
    }

    fn frame(part: Part, state: State, d: i32) -> Canvas {
        let (_, img) = super::parts(d)
            .into_iter()
            .find(|(p, _)| *p == part)
            .unwrap();
        img.atlas.copy_rect(img.frame(state))
    }

    const PARTS: [(Part, (i32, i32)); 5] = [
        (Part::EditBorder, (5, 5)),
        (Part::ComboList, (23, 21)),
        (Part::ComboDrop, (17, 23)),
        (Part::SpinUp, (15, 9)),
        (Part::SpinDown, (15, 9)),
    ];

    fn states(part: Part) -> [State; 4] {
        match part {
            Part::EditBorder => super::FIELD_STATES,
            _ => super::STATES,
        }
    }

    #[test]
    fn glass_fields_pins_at_scale_1_and_2() {
        let got: Vec<(Part, u64, u64)> = super::parts(1)
            .into_iter()
            .zip(super::parts(2))
            .map(|((p, one), (_, two))| (p, fnv(&one.atlas), fnv(&two.atlas)))
            .collect();
        let want: [(Part, u64, u64); 5] = [
            (Part::EditBorder, 8138547461712309727, 17424655265791650494),
            (Part::ComboList, 4051416926766009047, 11988009224700880312),
            (Part::ComboDrop, 1626082823027008929, 6478819219919109750),
            (Part::SpinUp, 1699632776566478333, 13060240260583136675),
            (Part::SpinDown, 14349372546147879217, 9640344259591982766),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn every_field_part_is_its_design_size_at_each_scale() {
        for ((part, one), (_, two)) in super::parts(1).into_iter().zip(super::parts(2)) {
            let (w, h) = PARTS.iter().find(|p| p.0 == part).unwrap().1;
            for (img, d) in [(&one, 1), (&two, 2)] {
                assert_eq!(img.scale, d, "{part:?} at {d}");
                let f = img.frame(State::Normal);
                assert_eq!((f.width, f.height), (w * d, h * d), "{part:?} at {d}");
            }
            assert_eq!(one.sizing, two.sizing, "{part:?}");
            assert_eq!(one.outset, two.outset, "{part:?}");
            assert_eq!(one.two_bands, two.two_bands, "{part:?}");
        }
    }

    #[test]
    fn every_field_part_state_is_its_own_frame() {
        for d in [1, 2] {
            for (part, _) in PARTS {
                let all = states(part);
                let frames: Vec<Canvas> = all.iter().map(|&s| frame(part, s, d)).collect();
                for i in 0..frames.len() {
                    for j in i + 1..frames.len() {
                        assert_ne!(
                            frames[i].pixels, frames[j].pixels,
                            "{part:?} at {d}: {:?} and {:?}",
                            all[i], all[j]
                        );
                    }
                }
            }
        }
    }

    fn soft(c: &Canvas, x0: i32, y0: i32, n: i32) -> bool {
        (y0..y0 + n).any(|y| {
            (x0..x0 + n).any(|x| {
                let a = c.get(x, y) >> 24;
                a > 0 && a < 255
            })
        })
    }

    #[test]
    fn rounded_corners_are_clear_outside_and_soft_at_2() {
        for st in super::STATES {
            let list = frame(Part::ComboList, st, 2);
            let (w, h) = (list.width as i32, list.height as i32);
            for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                assert_eq!(list.get(x, y) >> 24, 0, "{st:?}: list corner ({x}, {y})");
            }
            assert!(soft(&list, 0, 0, 6), "{st:?}: the list's top-left curve");
            assert!(soft(&list, w - 6, h - 6, 6), "{st:?}: bottom-right");
            let corner = frame(Part::ComboList, st, 1).get(0, 0) >> 24;
            assert!(corner < 8, "{st:?}: the curve only grazes the corner at 1");
        }
        for st in [State::Hot, State::Pressed] {
            let drop = frame(Part::ComboDrop, st, 2);
            let (w, h) = (drop.width as i32, drop.height as i32);
            assert_eq!(drop.get(0, 0) >> 24, 255, "{st:?}: square top-left");
            assert_eq!(drop.get(0, h - 1) >> 24, 255, "{st:?}: square bottom-left");
            assert!(drop.get(w - 1, 0) >> 24 < 255, "{st:?}: round top-right");
            assert!(soft(&drop, w - 3, h - 3, 3), "{st:?}: round bottom-right");
        }
        for st in super::FIELD_STATES {
            let f = frame(Part::EditBorder, st, 2);
            assert!(f.get(0, 0) >> 24 < 255, "{st:?}: the field's soft corner");
            assert_eq!(f.get(4, 0) >> 24, 255, "{st:?}: the top line");
            assert_eq!(f.get(4, 4) >> 24, 0, "{st:?}: clear inside");
        }
    }

    #[test]
    fn rounded_edges_leave_no_dark_fringe() {
        let lum = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        for d in [1, 2] {
            for (part, st, line) in [
                (Part::ComboList, State::Normal, 0x6D6D6D),
                (Part::ComboList, State::Hot, 0x3B77AD),
                (Part::ComboDrop, State::Hot, 0x3B77AD),
            ] {
                let f = frame(part, st, d);
                let mut c = face(f.width, f.height);
                c.blit_over(0, 0, &f);
                for y in 0..c.height as i32 {
                    for x in 0..c.width as i32 {
                        let p = c.get(x, y);
                        if f.get(x, y) & 0xFF_FFFF == 0 && f.get(x, y) >> 24 > 0 {
                            continue;
                        }
                        let chevron = (8 * d..14 * d).contains(&y) && (4 * d..19 * d).contains(&x);
                        assert!(
                            chevron || lum(p) >= lum(line),
                            "{part:?} {st:?} at {d}: ({x}, {y}) is {p:08X}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_chevron_row_is_whole_at_1_and_soft_at_2() {
        let dark = |c: &Canvas, y: i32| {
            (0..c.width as i32).any(|x| {
                let p = c.get(x, y);
                p >> 24 == 255 && (p & 0xFF) < 0xD8 && (p >> 8 & 0xFF) < 0xC8
            })
        };
        for (part, rows) in [
            (Part::ComboList, 9..13),
            (Part::ComboDrop, 10..14),
            (Part::SpinUp, 3..6),
            (Part::SpinDown, 3..6),
        ] {
            for st in super::STATES {
                let one = frame(part, st, 1);
                for y in rows.clone() {
                    assert!(dark(&one, y), "{part:?} {st:?}: chevron row {y}");
                }
            }
        }
        let drop = frame(Part::ComboDrop, State::Normal, 2);
        assert_eq!(
            drop.get(17, 21),
            0xFF00_0000,
            "the drop chevron's middle at 2"
        );
        assert!(soft(&drop, 8, 20, 20), "its slanted sides are soft at 2");
    }

    #[test]
    fn a_glass_list_is_drawn_anew_at_scale_2() {
        let theme = theme_for(Interface::Glass).scaled(2);
        let p = theme.paint();
        let r = Rect::new(4, 4, 280, 42);
        let drop = Rect::new(r.right() - 36, r.y + 4, 32, 34);
        let mut c = face(292, 50);
        let look = ComboLook {
            enabled: true,
            focused: false,
            ptr: Interaction::Idle,
            editable: false,
        };
        draw_combo_frame(&mut c, &p, r, drop, p.cs.window, look);
        pin(&c, 120, 4, 0x6D6D6D, "the outline's first row");
        pin(&c, 120, 5, 0x6D6D6D, "and its second");
        let corner: Vec<u32> = (4..8).map(|i| c.get(i, 4 + (7 - i)) & 0xFF).collect();
        assert!(
            corner.iter().any(|&v| v > 0x70 && v < 0xF0),
            "a soft corner: {corner:X?}"
        );
    }
}
