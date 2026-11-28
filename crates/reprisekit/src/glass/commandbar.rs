//! Glass's command bar: a pale blue band, glassy boxes behind a command under the pointer or held,
//! and the round help mark. Drawn from shapes at the style's scale; dark mode remaps them.

use crate::raster::Insets;
use crate::shape::{Px, Stops, centre, draw, ellipse, premul, rounded, shares, stroke};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

/// The band's height at 100% text, and the rows of it that keep their height: the gradient at the
/// top and the three edge rows at the bottom. The flat rows between stretch.
pub const BAR_H: i32 = 31;
const BAR_TOP: i32 = 15;
const BAR_BOTTOM: i32 = 3;
const BAR_W: i32 = 3;

const BAR_TOP_STOPS: [(f64, Color); 2] = [
    (0.0, Color::rgb(0xFA, 0xFC, 0xFD)),
    (14.0, Color::rgb(0xE6, 0xF0, 0xFA)),
];
const BAR_FLAT_STOPS: [(f64, Color); 2] = [
    (15.0, Color::rgb(0xDC, 0xE6, 0xF4)),
    (27.0, Color::rgb(0xDD, 0xE9, 0xF7)),
];
const BAR_EDGE_ROWS: [Color; 3] = [
    Color::rgb(0xE4, 0xEF, 0xFB),
    Color::rgb(0xCD, 0xDA, 0xEA),
    Color::rgb(0xA0, 0xAF, 0xC3),
];
/// The end columns take white over the band at this alpha, above the edge rows.
const BAR_END_LIGHT: u8 = 0x80;

/// A command's box at 100% text, and its rows that keep their height.
pub const BOX_H: i32 = 24;
const BOX_END: i32 = 3;
const BOX_W: i32 = 2 * BOX_END + 1;
const BOX_TOP: i32 = 12;
const BOX_BOTTOM: i32 = 3;
const BOX_RADIUS: f64 = 2.5;
/// The row where a box's face steps from its pale upper half to its deeper lower half.
const BOX_STEP: f64 = 12.0;

const BOX_STATES: [State; 3] = [State::Normal, State::Hot, State::Pressed];

/// One state's outline (top and bottom), the light line inside it (top and bottom; none lets the
/// face run to the outline), the rows of shade under its top, and its face (upper half top and
/// bottom, lower half top and bottom).
struct BoxInks {
    outline: [Color; 2],
    inner: Option<[Color; 2]>,
    shade: &'static [Color],
    upper: [Color; 2],
    lower: [Color; 2],
}

const HOT: BoxInks = BoxInks {
    outline: [Color::rgb(0xBB, 0xCA, 0xDB), Color::rgb(0xAA, 0xBC, 0xD5)],
    inner: Some([Color::rgb(0xFD, 0xFE, 0xFF), Color::rgb(0xEF, 0xF4, 0xF9)]),
    shade: &[],
    upper: [Color::rgb(0xF8, 0xFB, 0xFE), Color::rgb(0xED, 0xF2, 0xFA)],
    lower: [Color::rgb(0xD7, 0xE4, 0xF4), Color::rgb(0xC1, 0xD2, 0xE8)],
};

/// Held, a box sinks: no light line, two rows of shade under its top, a flatter, deeper face.
const PRESSED: BoxInks = BoxInks {
    outline: HOT.outline,
    inner: None,
    shade: &[Color::rgb(0xC9, 0xD4, 0xE4), Color::rgb(0xD4, 0xDE, 0xEA)],
    upper: [Color::rgb(0xE1, 0xEB, 0xF5), Color::rgb(0xD8, 0xE4, 0xF1)],
    lower: [Color::rgb(0xCF, 0xDB, 0xEC), Color::rgb(0xCF, 0xDC, 0xED)],
};

/// A command's label and arrows; greyed, a muted blue-grey.
pub const INK: Color = Color::rgb(0x1E, 0x39, 0x5B);
pub const GREYED_INK: Color = Color::rgb(0x8D, 0x9A, 0xAB);

/// The help mark: a 16 px disc in a grey rim with a light ring inside it, a blue body lit from
/// above, and a white question mark.
pub const HELP: i32 = 16;
const HELP_RIM: [Color; 2] = [Color::rgb(0x86, 0x88, 0x8C), Color::rgb(0x9C, 0x9E, 0xA2)];
const HELP_RING: Color = Color::rgb(0xD4, 0xD6, 0xDC);
const HELP_BODY_LIT: Color = Color::rgb(0x86, 0xA4, 0xEC);
const HELP_BODY: Color = Color::rgb(0x2A, 0x52, 0xC4);
const HELP_BODY_DEEP: Color = Color::rgb(0x12, 0x34, 0x96);
const HELP_GLOSS: u8 = 0x48;
const HELP_MARK: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// The question mark's hook: a circle's arc round (8, 6.3) of this radius from the left round to
/// lower right, then a stem down to the row where the dot's gap starts.
const HOOK_RADIUS: f64 = 2.3;
const HOOK_STEM_END: f64 = 9.6;
const HOOK_WIDTH: f64 = 1.7;
const DOT_Y: f64 = 11.9;
const DOT_RADIUS: f64 = 1.0;

pub fn parts(d: i32) -> [(Part, PartImage); 3] {
    [
        (Part::CommandBar, band(d)),
        (Part::CommandButton, boxes(d)),
        (Part::CommandHelp, help(d)),
    ]
}

fn c(v: Color) -> Px {
    premul(v, 0xFF)
}

fn band(d: i32) -> PartImage {
    let top = BAR_TOP_STOPS.map(|(t, v)| (t, c(v)));
    let flat = BAR_FLAT_STOPS.map(|(t, v)| (t, c(v)));
    let atlas = draw(BAR_W, BAR_H, d, |x, y| {
        let row = centre(y as i32, d);
        let edge_from = BAR_H - BAR_BOTTOM;
        if row > f64::from(edge_from) - 0.5 {
            let i = (row.round() as i32 - edge_from).clamp(0, BAR_BOTTOM - 1);
            return c(BAR_EDGE_ROWS[i as usize]);
        }
        let face = if row < f64::from(BAR_TOP) - 0.5 {
            Stops(&top).at(row)
        } else {
            Stops(&flat).at(row)
        };
        let col = centre(x as i32, d);
        if col < 0.5 || col > f64::from(BAR_W) - 1.5 {
            crate::shape::over(premul(Color::rgb(0xFF, 0xFF, 0xFF), BAR_END_LIGHT), face)
        } else {
            face
        }
    });
    PartImage::new(atlas, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(1, BAR_TOP, 1, BAR_BOTTOM))
        .at_scale(d)
}

fn inks(state: State) -> Option<&'static BoxInks> {
    match state {
        State::Hot => Some(&HOT),
        State::Pressed => Some(&PRESSED),
        _ => None,
    }
}

fn boxes(d: i32) -> PartImage {
    let k = f64::from(d);
    let fh = BOX_H * d;
    let n = BOX_STATES.len() as i32;
    let (w, h) = (f64::from(BOX_W) * k, f64::from(BOX_H) * k);
    let atlas = draw(BOX_W, BOX_H * n, d, |x, y| {
        let i = (y as i32 / fh) as usize;
        let Some(ink) = inks(BOX_STATES[i]) else {
            return crate::shape::CLEAR;
        };
        let y = y - f64::from(i as i32 * fh);
        let row = centre(y as i32, d);
        let t = row / f64::from(BOX_H - 1);
        let between =
            |pair: [Color; 2], t: f64| Stops(&[(0.0, c(pair[0])), (1.0, c(pair[1]))]).at(t);
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
            between(ink.upper, (row - 2.0) / (BOX_STEP - 3.0))
        } else {
            between(
                ink.lower,
                (row - BOX_STEP) / (f64::from(BOX_H) - 3.0 - BOX_STEP),
            )
        };
        let shaded = (row.round() as i32 - 1).try_into().ok();
        let face = match shaded.and_then(|i: usize| ink.shade.get(i)) {
            Some(&shade) => c(shade),
            None => face,
        };
        let inner_ink = ink.inner.map_or(face, |pair| between(pair, t));
        shares(&[
            (between(ink.outline, t), outer - inner),
            (inner_ink, inner - body),
            (face, body),
        ])
    });
    PartImage::new(atlas, &BOX_STATES, Layout::Vertical)
        .sizing(Insets::new(BOX_END, BOX_TOP, BOX_END, BOX_BOTTOM))
        .at_scale(d)
}

/// The question mark's hook, design pixels: an arc from the left of its circle over the top
/// round to below its right, then the stem.
fn hook() -> Vec<(f64, f64)> {
    let (cx, cy) = (8.0, 6.3);
    let mut pts: Vec<(f64, f64)> = (0..=10)
        .map(|i| {
            let a = std::f64::consts::PI * (1.0 + 1.35 * f64::from(i) / 10.0);
            (cx + HOOK_RADIUS * a.cos(), cy + HOOK_RADIUS * a.sin())
        })
        .collect();
    pts.push((cx, 8.4));
    pts.push((cx, HOOK_STEM_END));
    pts
}

fn help(d: i32) -> PartImage {
    let k = f64::from(d);
    let mid = f64::from(HELP) / 2.0;
    let hook: Vec<(f64, f64)> = hook().into_iter().map(|(x, y)| (x * k, y * k)).collect();
    let rim = [(0.0, c(HELP_RIM[0])), (1.0, c(HELP_RIM[1]))];
    let body = [
        (0.0, c(HELP_BODY_LIT)),
        (0.55, c(HELP_BODY)),
        (1.0, c(HELP_BODY_DEEP)),
    ];
    let atlas = draw(HELP, HELP, d, |x, y| {
        let disc = |r: f64| ellipse(x, y, mid * k, mid * k, r * k, r * k);
        let (outer, ring, inside) = (disc(mid), disc(mid - 1.0), disc(mid - 2.0));
        let cy = centre(y as i32, d);
        let lit = crate::shape::radial(x, y, mid * k, 4.0 * k, 7.0 * k, 9.0 * k, &Stops(&body));
        let gloss = ellipse(x, y, mid * k, 4.2 * k, 4.6 * k, 3.4 * k);
        let face = crate::shape::over(premul(Color::rgb(0xFF, 0xFF, 0xFF), HELP_GLOSS), lit);
        let face = crate::shape::mix(lit, face, gloss);
        let mark = stroke(x, y, &hook, HOOK_WIDTH * k)
            .max(ellipse(
                x,
                y,
                mid * k,
                DOT_Y * k,
                DOT_RADIUS * k,
                DOT_RADIUS * k,
            ))
            .min(1.0);
        let face = crate::shape::mix(face, c(HELP_MARK), mark);
        shares(&[
            (Stops(&rim).at(cy / f64::from(HELP - 1)), outer - ring),
            (c(HELP_RING), ring - inside),
            (face, inside),
        ])
    });
    let mut img = PartImage::new(atlas, &[State::Normal], Layout::Vertical).at_scale(d);
    img.true_size = true;
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Mode, Settings, style};
    use crate::raster::{Canvas, Rect};

    fn frame(part: Part, mode: Mode, state: State, d: i32) -> Canvas {
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

    const PARTS: [(Part, &[State]); 3] = [
        (Part::CommandBar, &[State::Normal]),
        (Part::CommandButton, &BOX_STATES),
        (Part::CommandHelp, &[State::Normal]),
    ];

    #[test]
    fn command_bar_parts_pin_per_state_in_light_and_dark_at_scale_1_and_2() {
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            for d in [1, 2] {
                for (part, states) in PARTS {
                    for &state in states {
                        let h = fnv(&frame(part, mode, state, d));
                        got.push((format!("{part:?} {state:?} {mode:?} {d}"), h));
                    }
                }
            }
        }
        crate::pins::check("glass/commandbar", &got);
    }

    #[test]
    fn the_band_runs_pale_to_blue_over_three_edge_rows() {
        let c = frame(Part::CommandBar, Mode::Light, State::Normal, 1);
        assert_eq!((c.width, c.height), (BAR_W as u32, BAR_H as u32));
        assert_eq!(c.get(1, 0), 0xFFFA_FCFD, "the top row");
        assert_eq!(c.get(1, 14), 0xFFE6_F0FA, "the gradient's last row");
        assert_eq!(c.get(1, 15), 0xFFDC_E6F4, "the flat rows start");
        assert_eq!(c.get(1, 30), 0xFFA0_AFC3, "the bottom line");
        assert_eq!(
            c.get(0, 30),
            0xFFA0_AFC3,
            "the bottom line runs under the ends"
        );
        assert!(
            c.get(0, 20) & 0xFF > c.get(1, 20) & 0xFF,
            "an end column is lighter than the band"
        );
    }

    #[test]
    fn a_box_is_clear_at_rest_and_its_corners_are_clear() {
        for d in [1, 2] {
            let rest = frame(Part::CommandButton, Mode::Light, State::Normal, d);
            assert!(
                rest.pixels.iter().all(|&p| p == 0),
                "nothing at rest at {d}"
            );
            for state in [State::Hot, State::Pressed] {
                let c = frame(Part::CommandButton, Mode::Light, state, d);
                let (r, b) = (BOX_W * d - 1, BOX_H * d - 1);
                for (x, y) in [(0, 0), (r, 0), (0, b), (r, b)] {
                    assert!(
                        c.get(x, y) >> 24 < 0x80,
                        "{state:?} at {d}: corner ({x}, {y})"
                    );
                }
                assert_eq!(
                    c.get(0, 10 * d) >> 24,
                    0xFF,
                    "{state:?} at {d}: the side line"
                );
                assert_eq!(
                    c.get(3 * d, 10 * d) >> 24,
                    0xFF,
                    "{state:?} at {d}: the face"
                );
            }
        }
    }

    #[test]
    fn a_held_box_sinks_and_a_hot_one_steps_at_the_middle() {
        let hot = frame(Part::CommandButton, Mode::Light, State::Hot, 1);
        let held = frame(Part::CommandButton, Mode::Light, State::Pressed, 1);
        let sum = |p: u32| (p & 0xFF) + (p >> 8 & 0xFF) + (p >> 16 & 0xFF);
        assert!(
            sum(held.get(3, 4)) < sum(hot.get(3, 4)),
            "a deeper upper face"
        );
        assert!(
            sum(held.get(1, 10)) < sum(hot.get(1, 10)),
            "no light line inside"
        );
        assert_eq!(held.get(3, 1), 0xFFC9_D4E4, "shade under its top");
        assert_eq!(held.get(0, 10), hot.get(0, 10), "the same outline");
        assert!(
            sum(hot.get(3, 11)) - sum(hot.get(3, 12)) > 40,
            "the face steps down at row 12"
        );
    }

    #[test]
    fn curved_edges_are_partly_covered_at_scale_2() {
        let c = frame(Part::CommandButton, Mode::Light, State::Hot, 2);
        let partial = (0..6)
            .flat_map(|y| (0..6).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let a = c.get(x, y) >> 24;
                a > 0 && a < 0xFF
            })
            .count();
        assert!(
            partial >= 2,
            "the box's corner fades in over {partial} pixels"
        );
        let help = frame(Part::CommandHelp, Mode::Light, State::Normal, 2);
        let rim = (0..32)
            .map(|x| help.get(x, 2) >> 24)
            .filter(|&a| a > 0 && a < 0xFF)
            .count();
        assert!(rim >= 2, "the disc's top is smooth");
    }

    #[test]
    fn the_help_mark_is_a_blue_disc_with_white_in_it_and_clear_corners() {
        let c = frame(Part::CommandHelp, Mode::Light, State::Normal, 1);
        assert_eq!((c.width, c.height), (HELP as u32, HELP as u32));
        assert_eq!(c.get(0, 0) >> 24, 0, "a clear corner");
        let blue = |p: u32| (p & 0xFF) > (p >> 16 & 0xFF) + 0x40;
        assert!(blue(c.get(4, 9)), "the body is blue: {:08x}", c.get(4, 9));
        let lightest = (7..9)
            .flat_map(|x| (11..13).map(move |y| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .map(|p| (p & 0xFF).min(p >> 8 & 0xFF).min(p >> 16 & 0xFF))
            .max()
            .unwrap();
        assert!(lightest > 0xD0, "the mark's dot is white: {lightest:02x}");
    }

    #[test]
    fn nine_grid_insets_double_exactly_at_scale_2() {
        let one = style(&Settings::default());
        let two = one.at_scale(2);
        for (part, _) in PARTS {
            let (a, b) = (one.part(part).unwrap(), two.part(part).unwrap());
            assert_eq!(b.scale, 2, "{part:?}");
            assert_eq!(
                (b.atlas.width, b.atlas.height),
                (2 * a.atlas.width, 2 * a.atlas.height),
                "{part:?}"
            );
            assert_eq!(a.sizing, b.sizing, "{part:?}: design insets");
        }
        let mut c = Canvas::new(80, BAR_H as u32);
        one.draw(
            &mut c,
            Rect::new(0, 0, 80, BAR_H),
            Part::CommandBar,
            State::Normal,
            1,
        );
        assert_eq!(
            c.get(40, BAR_H - 1),
            0xFFA0_AFC3,
            "a wide band keeps its bottom line"
        );
    }
}
