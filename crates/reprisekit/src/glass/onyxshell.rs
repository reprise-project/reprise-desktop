//! Onyx's shell window parts ([`super::Style::Onyx`]): Back and Forward as two round glass
//! buttons in a silver rim with no pill round them, a pale address and search field, a deep
//! blue-teal command bar glossy over its top half with white labels, and its own panes: a pale
//! details pane shading to blue at its right end under Onyx's light bands, blue group headings
//! over a light blue rule.
//!
//! The address's boxes and the help mark are Glass's. The command bar keeps its dark band in
//! either mode; dark mode lifts its lit box. The round buttons keep their colours over the
//! frame's glass; the fields and the panes darken as dark Glass's do ([`dark`]).

use super::navrow::{self, ClearInks, FieldInks, Orb, Painter, RecentInks};
use super::{Mode, Settings, commandbar, crumbs, dark, streak};
use crate::raster::{Canvas, Insets, Rect};
use crate::shape::{CLEAR, Px, Stops, centre, draw, mix, over, pack, premul, rounded, shares};
use crate::style::{Layout, PaneLook, Part, PartImage, State};
use crate::theme::Color;

const ORB_REST: Orb = Orb {
    ring: [0x8FA3B8, 0x5D7690],
    upper: [(1.0, 0xF2F7FC), (5.0, 0xD9E7F4), (10.5, 0x9CBAD8)],
    lower: [
        (11.5, 0x5D8FC4),
        (12.5, 0x2E64A3),
        (18.0, 0x3F86CB),
        (23.0, 0x7FC1EE),
    ],
    rim: Some(0x8FD8FF),
    arrow: (0xFFFFFF, 0xFF),
};

const ORB_HOT: Orb = Orb {
    ring: [0x8FA3B8, 0x4F7FB0],
    upper: [(1.0, 0xF8FBFE), (5.0, 0xE6F1FB), (10.5, 0xADCBE9)],
    lower: [
        (11.5, 0x6BA3DC),
        (12.5, 0x3474BE),
        (18.0, 0x4C9AE4),
        (23.0, 0x95D2FA),
    ],
    rim: Some(0xA6E6FF),
    arrow: (0xFFFFFF, 0xFF),
};

const ORB_PRESSED: Orb = Orb {
    ring: [0x7F93A8, 0x3E5F84],
    upper: [(1.0, 0xCBD8E5), (5.0, 0xB4C7DA), (10.5, 0x7C9CBD)],
    lower: [
        (11.5, 0x3F6E9F),
        (12.5, 0x1C4677),
        (18.0, 0x2C6299),
        (23.0, 0x4F8CC2),
    ],
    rim: None,
    arrow: (0xFFFFFF, 0xFF),
};

/// Greyed, a ball is pale silver glass under a faint arrow.
const ORB_GREYED: Orb = Orb {
    ring: [0x9AABBC, 0x8799AA],
    upper: [(1.0, 0xEEF3F8), (4.0, 0xE2EAF2), (10.5, 0xD6E2EC)],
    lower: [
        (11.5, 0xCCDBE8),
        (13.0, 0xC8D7E5),
        (18.0, 0xCEDEEC),
        (23.0, 0xDBE9F7),
    ],
    rim: None,
    arrow: (0xF6F9FC, 0xD0),
};

const RECENT: RecentInks = RecentInks {
    fill: 0x3E4E60,
    line: 0xDDE6EE,
    greyed: [0xB5C2CF, 0x8C9AAA],
};

const FIELD_EDGE: [(f64, u32); 2] = [(0.0, 0x575D63), (21.0, 0xA9B4C0)];

const SEARCH: FieldInks = FieldInks {
    edge: FIELD_EDGE,
    alpha: [[0xE0, 0xE6, 0xB4], [0xF0, 0xF0, 0xD8], [0xF0, 0xFF, 0xFF]],
    magnifier: 0x1F5FA8,
    lens: 0x80,
};

const SEARCH_CLEAR: ClearInks = ClearInks {
    cross: 0x3B5B7F,
    boxes: [
        [0x7DA2CE, 0xF4F9FE, 0xD2E6F8],
        [0x5A82AF, 0xC9DDF1, 0xB0CDEA],
    ],
};

const fn px(v: u32) -> Px {
    crumbs::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const CRUMB_FIELD: crumbs::FieldInks = crumbs::FieldInks {
    edge: [(0.0, px(0x575D63)), (21.0, px(0xA9B4C0))],
    edge_focused: [(0.0, px(0x3D7BAD)), (21.0, px(0xB7D9ED))],
    face: [180, 216],
    line: [230, 240],
};

/// The command bar's band, top to bottom: a dark top line, a light line, the gloss over its
/// top half, the deep body lightening to its foot, a light line and a dark foot line.
const BAND_TOP_LINE: Color = Color::rgb(0x4E, 0x62, 0x74);
const BAND_LIGHT_LINE: Color = Color::rgb(0xB1, 0xC9, 0xD8);
const BAND_GLOSS: [Color; 2] = [Color::rgb(0x80, 0xA7, 0xBF), Color::rgb(0x3D, 0x71, 0x91)];
const BAND_BODY: [Color; 2] = [Color::rgb(0x04, 0x33, 0x4F), Color::rgb(0x4B, 0x92, 0xAA)];
const BAND_FOOT_LIGHT: Color = Color::rgb(0x94, 0xC1, 0xCE);
const BAND_FOOT_LINE: Color = Color::rgb(0x03, 0x29, 0x41);
/// Dark mode deepens the body by this share of black, the gloss by half of it.
const BAND_DARK_SHADE: f64 = 0.35;

/// A command's label and arrows on the band, and greyed, a dim grey-blue.
pub const COMMAND_INK: Color = Color::rgb(0xFF, 0xFF, 0xFF);
pub const COMMAND_GREYED_INK: Color = Color::rgb(0x7F, 0x95, 0xAC);
/// The outlines of the glyphs drawn on the band ([`crate::art::cmdglyph::canvas_inked`]).
pub const COMMAND_GLYPH_INK: Color = Color::rgb(0xE8, 0xF0, 0xF8);

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const BLACK: Color = Color::rgb(0, 0, 0);

/// A command box's inks over the band, alphas as (top, bottom) pairs. The line inside the outline
/// is white; `None` runs the face out to the outline.
struct BoxInks {
    edge: (Color, [u8; 2]),
    line: Option<[u8; 2]>,
    face: Color,
    upper: [u8; 2],
    lower: [u8; 2],
    /// Black down the two rows under the top and the two columns in from the left.
    shade: Option<[[u8; 2]; 2]>,
}

const BOX_STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Focused];
/// Lit: a dark outline round a bright line, the upper half whitened, the lower half barely.
const BOX_HOT: BoxInks = BoxInks {
    edge: (BLACK, [0x4D, 0x4D]),
    line: Some([0xBA, 0x7C]),
    face: WHITE,
    upper: [0x73, 0x21],
    lower: [0x12, 0x14],
    shade: None,
};
/// Held: a darker outline, the band dimmed under a shade along the top and left.
const BOX_HELD: BoxInks = BoxInks {
    edge: (BLACK, [0xA6, 0xA6]),
    line: None,
    face: BLACK,
    upper: [0x26, 0x0B],
    lower: [0x13, 0x0A],
    shade: Some([[0x4D, 0x38], [0x2E, 0x1F]]),
};
/// The keyboard's box: a white outline and a faint light inside.
const BOX_FOCUSED: BoxInks = BoxInks {
    edge: (WHITE, [0xC0, 0xA0]),
    line: None,
    face: WHITE,
    upper: [0x14, 0x0C],
    lower: [0x00, 0x0C],
    shade: None,
};
/// Dark mode raises a lit box's alphas by this much.
const BOX_DARK_LIFT: u8 = 0x18;

/// The details pane: a blue-grey line shading the face's top rows, the face white at its left to
/// pale blue at its right end, which Onyx's bands light ([`streak::ONYX_LIGHT`]).
const DETAILS_LINE: Color = Color::rgb(0x98, 0xB4, 0xC4);
/// The line's share in the face's first row, and the rows the shade takes to fade out.
const DETAILS_SHADE: f64 = 0.48;
const DETAILS_SHADE_ROWS: f64 = 8.0;
const DETAILS_FACE: [Color; 3] = [
    Color::rgb(0xF2, 0xFB, 0xFE),
    Color::rgb(0xEC, 0xFB, 0xFE),
    Color::rgb(0xBC, 0xDA, 0xF0),
];
/// Where along the pane the face turns from its middle colour towards its right end's.
const DETAILS_TURN: f64 = 0.45;
/// How wide the bands' flourish is at the pane's right end, and the white a band at
/// [`streak::ONYX_PEAK`] lays over the face there.
const FLOURISH_W: f64 = 220.0;
const FLOURISH_ALPHA: f64 = 0.55;
const DARK_FLOURISH_ALPHA: f64 = 0.22;
const DETAILS_LABEL: Color = Color::rgb(0x73, 0x73, 0x73);
const DETAILS_VALUE: Color = Color::rgb(0x00, 0x00, 0x00);
const DARK_DETAILS_LABEL: Color = Color::rgb(0xA4, 0xAE, 0xBE);
const DARK_DETAILS_VALUE: Color = Color::rgb(0xE6, 0xEC, 0xF2);
const NAV_EDGE: Color = Color::rgb(0xA7, 0xBA, 0xC5);
const PREVIEW_EDGE: Color = Color::rgb(0xDD, 0xE4, 0xE8);
const HEADING: Color = Color::rgb(0x00, 0x33, 0x99);
const HEADING_RULE: Color = Color::rgb(0xB0, 0xBF, 0xDE);
const DARK_HEADING: Color = Color::rgb(0x7F, 0xA8, 0xF0);
const DARK_HEADING_RULE: Color = Color::rgb(0x3A, 0x4F, 0x78);

/// `st` with Onyx's shell parts, inks and panes, painted at `d`.
pub(super) fn restyle(st: crate::style::Style, s: &Settings, d: i32) -> crate::style::Style {
    let dark_mode = s.mode == Mode::Dark;
    let remap = move |img: PartImage| match dark_mode {
        true => dark::remap_part(img),
        false => img,
    };
    let kept: [(Part, Painter); 4] = [
        (Part::NavPill, pill),
        (Part::NavBack, |d| navrow::orbs(orb_inks, false, d)),
        (Part::NavForward, |d| navrow::orbs(orb_inks, true, d)),
        (Part::NavRecent, |d| navrow::recent(&RECENT, d)),
    ];
    let darkened: [(Part, Painter); 4] = [
        (Part::SearchField, |d| navrow::field_part(&SEARCH, d)),
        (Part::SearchGlass, |d| navrow::glass_part(&SEARCH, d)),
        (Part::SearchClear, |d| navrow::clear_part(&SEARCH_CLEAR, d)),
        (Part::CrumbField, |d| crumbs::field_part(&CRUMB_FIELD, d)),
    ];
    let st = kept.into_iter().fold(st, |st, (part, paint)| {
        st.with_painted(part, move || paint(d))
    });
    let st = darkened.into_iter().fold(st, |st, (part, paint)| {
        st.with_painted(part, move || remap(paint(d)))
    });
    let mode = s.mode;
    st.with_painted(Part::CommandBar, move || band(mode, d))
        .with_painted(Part::CommandButton, move || boxes(mode, d))
        .with_painted(Part::CommandHelp, move || commandbar::help(d))
        .with_text_color(Part::CommandButton, State::Normal, COMMAND_INK)
        .with_text_color(Part::CommandButton, State::Disabled, COMMAND_GREYED_INK)
        .with_text_color(Part::CommandBar, State::Normal, COMMAND_GLYPH_INK)
        .with_panes(panes(mode))
}

fn orb_inks(state: State) -> &'static Orb {
    match state {
        State::Hot => &ORB_HOT,
        State::Pressed => &ORB_PRESSED,
        State::Disabled => &ORB_GREYED,
        _ => &ORB_REST,
    }
}

/// No pill: Back and Forward stand apart.
fn pill(d: i32) -> PartImage {
    let d = d.max(1);
    let atlas = draw(navrow::PILL_W, navrow::PILL_H, d, |_, _| CLEAR);
    PartImage::new(atlas, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(15, 0, 15, 0))
        .at_scale(d)
}

fn solid(c: Color) -> Px {
    premul(c, 0xFF)
}

fn shaded(c: Color, k: f64) -> Px {
    mix(solid(c), solid(Color::rgb(0, 0, 0)), k)
}

fn band(mode: Mode, d: i32) -> PartImage {
    let (h, top, bottom) = (
        commandbar::BAR_H,
        commandbar::BAR_TOP,
        commandbar::BAR_BOTTOM,
    );
    let deep = match mode {
        Mode::Light => 0.0,
        Mode::Dark => BAND_DARK_SHADE,
    };
    let gloss = [
        (2.0, shaded(BAND_GLOSS[0], deep / 2.0)),
        (f64::from(top - 1), shaded(BAND_GLOSS[1], deep / 2.0)),
    ];
    let body = [
        (f64::from(top), shaded(BAND_BODY[0], deep)),
        (f64::from(h - bottom), shaded(BAND_BODY[1], deep)),
    ];
    let w = commandbar::BAR_W;
    let atlas = draw(w, h, d, |x, y| {
        let row = (y as i32) / d;
        let face = match row {
            0 => solid(BAND_TOP_LINE),
            1 => solid(BAND_LIGHT_LINE),
            r if r == h - 1 => solid(BAND_FOOT_LINE),
            r if r == h - 2 => solid(BAND_FOOT_LIGHT),
            r if r < top => Stops(&gloss).at(centre(y as i32, d)),
            _ => Stops(&body).at(centre(y as i32, d)),
        };
        let col = (x as i32) / d;
        match col == 0 || col == w - 1 {
            true => over(premul(Color::rgb(0xFF, 0xFF, 0xFF), 0x30), face),
            false => face,
        }
    });
    PartImage::new(atlas, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(1, top, 1, bottom))
        .at_scale(d)
}

fn box_inks(state: State, mode: Mode) -> Option<BoxInks> {
    let lift = |a: [u8; 2]| match mode {
        Mode::Light => a,
        Mode::Dark => a.map(|v| match v {
            0 => 0,
            v => v.saturating_add(BOX_DARK_LIFT),
        }),
    };
    match state {
        State::Hot => Some(BoxInks {
            edge: (BOX_HOT.edge.0, lift(BOX_HOT.edge.1)),
            line: BOX_HOT.line.map(lift),
            upper: lift(BOX_HOT.upper),
            lower: lift(BOX_HOT.lower),
            ..BOX_HOT
        }),
        State::Pressed => Some(BOX_HELD),
        State::Focused => Some(BOX_FOCUSED),
        _ => None,
    }
}

/// The boxes clear at rest, lit, held and focused: a rounded outline, a line inside it, and a
/// face split across the middle.
fn boxes(mode: Mode, d: i32) -> PartImage {
    use commandbar::{BOX_H, BOX_RADIUS, BOX_STEP, BOX_W};
    let k = f64::from(d);
    let fh = BOX_H * d;
    let n = BOX_STATES.len() as i32;
    let (w, h) = (f64::from(BOX_W) * k, f64::from(BOX_H) * k);
    let atlas = draw(BOX_W, BOX_H * n, d, |x, y| {
        let i = (y as i32 / fh) as usize;
        let Some(inks) = box_inks(BOX_STATES[i], mode) else {
            return CLEAR;
        };
        let y = y - f64::from(i as i32 * fh);
        let row = centre(y as i32, d);
        let t = row / f64::from(BOX_H - 1);
        let tint = |ink: Color, alpha: f64| premul(ink, alpha.round().clamp(0.0, 255.0) as u8);
        let between = |[p, q]: [u8; 2], t: f64| f64::from(p) + (f64::from(q) - f64::from(p)) * t;
        let outer = rounded(x, y, w, h, BOX_RADIUS * k);
        let inner = rounded(
            x - k,
            y - k,
            w - 2.0 * k,
            h - 2.0 * k,
            (BOX_RADIUS - 1.0) * k,
        );
        let body = rounded(x - 2.0 * k, y - 2.0 * k, w - 4.0 * k, h - 4.0 * k, 0.0);
        let face = if row < BOX_STEP - 0.5 {
            let s = ((row - 2.0) / (BOX_STEP - 3.0)).clamp(0.0, 1.0);
            tint(inks.face, between(inks.upper, 1.0 - (1.0 - s) * (1.0 - s)))
        } else {
            let s = (row - BOX_STEP) / (f64::from(BOX_H) - 3.0 - BOX_STEP);
            tint(inks.face, between(inks.lower, s))
        };
        let col = (x as i32) / d;
        let face = match inks.shade {
            Some([top, left]) => {
                let by_row = usize::try_from((row.round() as i32) - 1).ok();
                let by_col = usize::try_from(col - 1).ok();
                let a = [
                    by_row.and_then(|r| top.get(r)),
                    by_col.and_then(|c| left.get(c)),
                ]
                .into_iter()
                .flatten()
                .max();
                a.map_or(face, |&a| premul(BLACK, a))
            }
            None => face,
        };
        let line = match inks.line {
            Some(a) => tint(WHITE, between(a, t)),
            None => face,
        };
        shares(&[
            (tint(inks.edge.0, between(inks.edge.1, t)), outer - inner),
            (line, inner - body),
            (face, body),
        ])
    });
    PartImage::new(atlas, &BOX_STATES, Layout::Vertical)
        .sizing(Insets::new(
            commandbar::BOX_END,
            commandbar::BOX_TOP,
            commandbar::BOX_END,
            commandbar::BOX_BOTTOM,
        ))
        .at_scale(d)
}

fn panes(mode: Mode) -> PaneLook {
    match mode {
        Mode::Light => PaneLook {
            nav_edge: NAV_EDGE,
            preview_edge: PREVIEW_EDGE,
            details: details_light,
            details_label: DETAILS_LABEL,
            details_value: DETAILS_VALUE,
            heading: HEADING,
            heading_rule: rule_light,
        },
        Mode::Dark => PaneLook {
            nav_edge: dark::remap_color(NAV_EDGE),
            preview_edge: dark::remap_color(PREVIEW_EDGE),
            details: details_dark,
            details_label: DARK_DETAILS_LABEL,
            details_value: DARK_DETAILS_VALUE,
            heading: DARK_HEADING,
            heading_rule: rule_dark,
        },
    }
}

fn details_light(c: &mut Canvas, r: Rect, d: i32) {
    details(c, r, d, |v| v, FLOURISH_ALPHA);
}

fn details_dark(c: &mut Canvas, r: Rect, d: i32) {
    details(c, r, d, dark::remap_color, DARK_FLOURISH_ALPHA);
}

/// The backdrop down `r`: design row 0 the line, then the face across, shaded under the line
/// and its right [`FLOURISH_W`] px lit by Onyx's bands fixed to the pane's right end.
fn details(c: &mut Canvas, r: Rect, d: i32, ink: fn(Color) -> Color, flourish: f64) {
    let dev = r.scaled(c.scale());
    let k = f64::from(d.max(1));
    let line = solid(ink(DETAILS_LINE));
    let face = DETAILS_FACE.map(|v| solid(ink(v)));
    let width = f64::from(dev.width) / k;
    c.compose_device(dev, |x, y| {
        let (u, v) = (centre(x - dev.x, d), centre(y - dev.y, d));
        if v < 0.5 {
            return pack(line);
        }
        let t = (u / width.max(1.0)).clamp(0.0, 1.0);
        let base = match t < DETAILS_TURN {
            true => mix(face[0], face[1], t / DETAILS_TURN),
            false => mix(face[1], face[2], (t - DETAILS_TURN) / (1.0 - DETAILS_TURN)),
        };
        let fade_in = 1.0 - ((v - 1.0) / DETAILS_SHADE_ROWS).clamp(0.0, 1.0);
        let base = mix(base, line, DETAILS_SHADE * fade_in * fade_in);
        let from_end = width - u;
        if from_end > FLOURISH_W {
            return pack(base);
        }
        let fade = 1.0 - from_end / FLOURISH_W;
        let level = streak::onyx_level(from_end - streak::ONYX_SLOPE * v + FLOURISH_PHASE);
        let lit = ((level - FLOURISH_FLOOR) / (streak::ONYX_PEAK - FLOURISH_FLOOR)).max(0.0);
        let a = flourish * fade * lit;
        pack(over(
            premul(Color::rgb(0xFF, 0xFF, 0xFF), (a * 255.0).round() as u8),
            base,
        ))
    });
}

/// Where along Onyx's bands the flourish starts, and the band level it shows from: the bands
/// between stand apart.
const FLOURISH_PHASE: f64 = 180.0;
const FLOURISH_FLOOR: f64 = 12.0;

fn rule_light(c: &mut Canvas, r: Rect, d: i32) {
    rule(c, r, d, HEADING_RULE);
}

fn rule_dark(c: &mut Canvas, r: Rect, d: i32) {
    rule(c, r, d, DARK_HEADING_RULE);
}

/// One design row across the middle of `r` in `ink`.
fn rule(c: &mut Canvas, r: Rect, d: i32, ink: Color) {
    let dev = r.scaled(c.scale());
    let row = Rect::new(dev.x, dev.y + (dev.height - d) / 2, dev.width, d);
    let ink = pack(solid(ink));
    c.compose_device(row, |_, _| ink);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Style, style};
    use crate::pins::fnv;

    fn settings(style: Style, mode: Mode) -> Settings {
        Settings {
            style,
            mode,
            ..Settings::default()
        }
    }

    fn frame_of(s: &Settings, part: Part, state: State, d: i32) -> Canvas {
        let st = style(s).at_scale(d);
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

    fn frame(mode: Mode, part: Part, state: State, d: i32) -> Canvas {
        frame_of(&settings(Style::Onyx, mode), part, state, d)
    }

    const PARTS: [(Part, &[State]); 12] = [
        (Part::NavPill, &[State::Normal]),
        (Part::NavBack, &navrow::BUTTON_STATES),
        (Part::NavForward, &navrow::BUTTON_STATES),
        (Part::NavRecent, &navrow::BUTTON_STATES),
        (Part::SearchField, &navrow::FIELD_STATES),
        (Part::SearchGlass, &[State::Normal]),
        (Part::SearchClear, &navrow::CLEAR_STATES),
        (
            Part::CrumbField,
            &[State::Normal, State::Hot, State::Focused],
        ),
        (
            Part::CrumbButton,
            &[State::Hot, State::Pressed, State::Checked],
        ),
        (Part::CommandBar, &[State::Normal]),
        (
            Part::CommandButton,
            &[State::Hot, State::Pressed, State::Focused],
        ),
        (Part::CommandHelp, &[State::Normal]),
    ];

    /// The details pane `w` x `h` design px and a heading's rule, drawn at `d`.
    fn panes_drawn(mode: Mode, (w, h): (i32, i32), d: i32) -> Canvas {
        let look = panes(mode);
        let mut c = Canvas::new((w * d) as u32, ((h + 8) * d) as u32).at_scale(d);
        (look.details)(&mut c, Rect::new(0, 0, w, h), d);
        (look.heading_rule)(&mut c, Rect::new(0, h, w, 8), d);
        c
    }

    #[test]
    fn onyx_shell_parts_pin_per_state_mode_and_scale() {
        let mut got = Vec::new();
        for mode in Mode::ALL {
            for (part, states) in PARTS {
                for &state in states {
                    for d in [1, 2] {
                        let key = format!("{mode:?} {part:?} {state:?} @{d}");
                        got.push((key, fnv(&frame(mode, part, state, d))));
                    }
                }
            }
            for d in [1, 2] {
                let key = format!("{mode:?} details pane and rule @{d}");
                got.push((key, fnv(&panes_drawn(mode, (400, 80), d))));
            }
        }
        crate::pins::check("glass/onyxshell", &got);
    }

    #[test]
    fn glass_keeps_its_own_shell_parts() {
        for (part, states) in PARTS {
            let state = states[0];
            let glass = frame_of(&settings(Style::Glass, Mode::Light), part, state, 1);
            let onyx = frame(Mode::Light, part, state, 1);
            match part {
                Part::CommandHelp | Part::CrumbButton => {
                    assert_eq!(fnv(&glass), fnv(&onyx), "{part:?} is Glass's")
                }
                _ => assert_ne!(fnv(&glass), fnv(&onyx), "{part:?} is Onyx's own"),
            }
        }
        let glass = style(&Settings::default());
        assert_eq!(
            glass.text_color(Part::CommandBar, State::Normal),
            None,
            "Glass inks its glyphs in their own colours"
        );
    }

    #[test]
    fn each_state_draws_its_own_face() {
        for mode in Mode::ALL {
            for (part, states) in PARTS {
                let drawn: Vec<u64> = states
                    .iter()
                    .map(|&s| fnv(&frame(mode, part, s, 1)))
                    .collect();
                for i in 0..drawn.len() {
                    assert!(
                        !drawn[i + 1..].contains(&drawn[i]),
                        "{mode:?} {part:?} {:?} is its own face",
                        states[i]
                    );
                }
            }
        }
    }

    #[test]
    fn the_round_buttons_stand_apart_with_clear_corners_and_soft_edges_at_scale_2() {
        for part in [Part::NavBack, Part::NavForward] {
            for state in navrow::BUTTON_STATES {
                let c = frame(Mode::Light, part, state, 2);
                let (w, h) = (c.width as i32, c.height as i32);
                for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                    assert_eq!(c.get(x, y) >> 24, 0, "{part:?} {state:?} corner ({x}, {y})");
                }
                let partial = (0..w)
                    .flat_map(|x| (0..h).map(move |y| (x, y)))
                    .filter(|&(x, y)| (1..0xFF).contains(&(c.get(x, y) >> 24)))
                    .count();
                assert!(partial >= 8, "{part:?} {state:?}: an anti-aliased edge");
            }
        }
        let pill = frame(Mode::Light, Part::NavPill, State::Normal, 1);
        assert!(pill.pixels.iter().all(|&p| p == 0), "no pill joins them");
    }

    #[test]
    fn the_nine_grids_double_their_insets_at_scale_2() {
        for part in [
            Part::CommandBar,
            Part::CommandButton,
            Part::CrumbField,
            Part::CrumbButton,
            Part::SearchField,
        ] {
            let one = style(&settings(Style::Onyx, Mode::Light));
            let two = one.at_scale(2);
            let (a, b) = (one.part(part).unwrap(), two.part(part).unwrap());
            assert_eq!(a.sizing, b.sizing, "{part:?}: design insets");
            assert_eq!(b.scale, 2, "{part:?}");
            assert_eq!(
                (b.atlas.width, b.atlas.height),
                (2 * a.atlas.width, 2 * a.atlas.height),
                "{part:?}"
            );
        }
    }

    fn lum(p: u32) -> u32 {
        ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF)
    }

    #[test]
    fn the_band_is_dark_in_either_mode_and_dark_mode_lifts_the_lit_box() {
        for mode in Mode::ALL {
            let band = frame(mode, Part::CommandBar, State::Normal, 1);
            assert!(lum(band.get(1, 16)) < 3 * 0x50, "{mode:?}: a deep body");
            assert!(
                lum(band.get(1, 1)) > lum(band.get(1, 16)),
                "{mode:?}: a light line on top"
            );
        }
        let hot = |mode| frame(mode, Part::CommandButton, State::Hot, 1).get(3, 6) >> 24;
        assert!(
            hot(Mode::Dark) > hot(Mode::Light),
            "a brighter lit box in dark mode"
        );
        let style = style(&settings(Style::Onyx, Mode::Light));
        assert_eq!(
            style.text_color(Part::CommandButton, State::Normal),
            Some(COMMAND_INK)
        );
        assert_eq!(
            style.text_color(Part::CommandBar, State::Normal),
            Some(COMMAND_GLYPH_INK)
        );
    }

    #[test]
    fn the_flourish_lights_only_the_details_panes_right_end() {
        let c = panes_drawn(Mode::Light, (400, 80), 1);
        let column = |x: i32| (10..80).map(|y| c.get(x, y)).collect::<Vec<_>>();
        let plain = 400 - FLOURISH_W as i32 - 10;
        for x in [5, 100, plain] {
            let col = column(x);
            assert!(col.iter().all(|&p| p == col[0]), "x {x}: the face alone");
        }
        let lit = (400 - 60..400)
            .filter(|&x| column(x).windows(2).any(|w| w[0] != w[1]))
            .count();
        assert!(lit > 30, "the bands cross the right end: {lit} columns");
        assert!(
            lum(c.get(5, 40)) > lum(c.get(399 - FLOURISH_W as i32, 40)),
            "the face shades towards its right end"
        );
    }
}
