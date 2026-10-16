//! Glass's push button, group box, tab strip and page parts, drawn from shapes at the style's
//! scale, the same in every glass colour.

use crate::raster::{Canvas, Insets};
use crate::shape::{CLEAR, Px, Stops, centre, draw, over, premul, ringed, scale, stroke};
use crate::style::{GroupLabel, Layout, Part, PartImage, State};
use crate::theme::Color;

const BUTTON_W: i32 = 13;
const BUTTON_H: i32 = 23;
const BOX_W: i32 = BUTTON_W - 2;
const BOX_H: i32 = BUTTON_H - 2;
const BUTTON_RADIUS: f64 = 2.5;

/// The face's two halves in design rows: upper shades over rows 2 to 9, lower from row 10 to the
/// last face row.
const UPPER: (f64, f64) = (2.0, 9.0);
const LOWER_TOP: f64 = 10.0;
const STEP: f64 = 9.5;
const BUTTON_LAST: f64 = 18.0;

type Shade = (Color, Color);

const fn flat(c: Color) -> Shade {
    (c, c)
}

struct Inks {
    border: Color,
    line_top: Color,
    line_upper: Shade,
    line_lower: Shade,
    line_bottom: Color,
    upper: Shade,
    lower: Shade,
}

const GREY_UPPER: Shade = (Color::rgb(0xF2, 0xF1, 0xF2), Color::rgb(0xEA, 0xE9, 0xEA));
const GREY_LOWER: Shade = (Color::rgb(0xDC, 0xDB, 0xDC), Color::rgb(0xCD, 0xCC, 0xCD));
const BLUE_UPPER: Shade = (Color::rgb(0xE9, 0xF5, 0xFD), Color::rgb(0xD8, 0xEE, 0xFC));
const BLUE_LOWER: Shade = (Color::rgb(0xBC, 0xE2, 0xFD), Color::rgb(0xA4, 0xD4, 0xF5));
const TAB_LINE: Color = Color::rgb(0x86, 0x89, 0x93);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

const NORMAL: Inks = Inks {
    border: Color::rgb(0x6D, 0x6D, 0x6D),
    line_top: Color::rgb(0xFC, 0xFB, 0xFC),
    line_upper: (Color::rgb(0xFC, 0xFB, 0xFC), Color::rgb(0xF9, 0xFA, 0xFA)),
    line_lower: (Color::rgb(0xF5, 0xF6, 0xF6), Color::rgb(0xF2, 0xF3, 0xF3)),
    line_bottom: Color::rgb(0xF2, 0xF3, 0xF3),
    upper: GREY_UPPER,
    lower: GREY_LOWER,
};

const HOT: Inks = Inks {
    border: Color::rgb(0x3B, 0x77, 0xAD),
    line_top: Color::rgb(0xFA, 0xFD, 0xFD),
    line_upper: (Color::rgb(0xFA, 0xFD, 0xFD), Color::rgb(0xF5, 0xFA, 0xFE)),
    line_lower: (Color::rgb(0xEE, 0xF7, 0xFE), Color::rgb(0xE8, 0xF4, 0xFC)),
    line_bottom: Color::rgb(0xE7, 0xF4, 0xFC),
    upper: BLUE_UPPER,
    lower: BLUE_LOWER,
};

const PRESSED: Inks = Inks {
    border: Color::rgb(0x2B, 0x5B, 0x87),
    line_top: Color::rgb(0x9B, 0xAC, 0xB8),
    line_upper: (Color::rgb(0xA8, 0xB9, 0xC5), Color::rgb(0x97, 0xB6, 0xCA)),
    line_lower: (Color::rgb(0x75, 0xA4, 0xC4), Color::rgb(0x51, 0x8B, 0xB3)),
    line_bottom: Color::rgb(0x64, 0xAA, 0xD9),
    upper: (Color::rgb(0xE4, 0xF2, 0xFC), Color::rgb(0xC2, 0xE2, 0xF6)),
    lower: (Color::rgb(0x95, 0xCB, 0xEF), Color::rgb(0x69, 0xAD, 0xDB)),
};

const DISABLED: Inks = Inks {
    border: Color::rgb(0xAB, 0xAF, 0xB3),
    line_top: Color::rgb(0xFC, 0xFB, 0xFC),
    line_upper: flat(Color::rgb(0xFC, 0xFB, 0xFC)),
    line_lower: flat(Color::rgb(0xFC, 0xFB, 0xFC)),
    line_bottom: Color::rgb(0xFC, 0xFB, 0xFC),
    upper: flat(Color::rgb(0xF3, 0xF4, 0xF4)),
    lower: flat(Color::rgb(0xF3, 0xF4, 0xF4)),
};

const DEFAULT: Inks = Inks {
    border: Color::rgb(0x3B, 0x76, 0xAD),
    line_top: Color::rgb(0x43, 0xCE, 0xFB),
    line_upper: (Color::rgb(0x43, 0xCD, 0xFB), Color::rgb(0x41, 0xCB, 0xF9)),
    line_lower: (Color::rgb(0x3D, 0xC7, 0xF5), Color::rgb(0x39, 0xC3, 0xF1)),
    line_bottom: Color::rgb(0x38, 0xC3, 0xF0),
    upper: GREY_UPPER,
    lower: GREY_LOWER,
};

const TAB_CLOSED: Inks = Inks {
    border: TAB_LINE,
    ..NORMAL
};

const BUTTON_STATES: [(State, &Inks); 6] = [
    (State::Normal, &NORMAL),
    (State::Hot, &HOT),
    (State::Pressed, &PRESSED),
    (State::Disabled, &DISABLED),
    (State::Default, &DEFAULT),
    (State::Focused, &DEFAULT),
];

const BUTTON_SIZING: Insets = Insets::new(6, 4, 6, 4);

const BUTTON_CONTENT: Insets = Insets::new(4, 1, 4, 0);

const GROUP: i32 = 7;
const GROUP_SIZING: Insets = Insets::new(3, 3, 3, 3);
const GROUP_LINE: Color = Color::rgb(0xD4, 0xDD, 0xE4);
struct Outline {
    runs: [(i32, i32, i32, i32); 4],
    cuts: &'static [[(f64, f64); 2]],
}

const GROUP_DARK: Outline = Outline {
    runs: [(2, 0, 3, 1), (2, 5, 3, 1), (0, 2, 1, 2), (6, 2, 1, 2)],
    cuts: &[
        [(2.5, 0.5), (0.5, 2.5)],
        [(4.5, 0.5), (6.5, 2.5)],
        [(0.5, 3.5), (2.5, 5.5)],
        [(6.5, 3.5), (4.5, 5.5)],
    ],
};

const GROUP_LIGHT: Outline = Outline {
    runs: [(2, 1, 3, 1), (2, 6, 3, 1), (1, 2, 1, 2), (5, 2, 1, 2)],
    cuts: &[[(0.5, 4.5), (2.5, 6.5)], [(6.5, 4.5), (4.5, 6.5)]],
};

const TAB_W: i32 = 7;
const TAB_H: i32 = 18;
const TAB_LAST: f64 = 17.0;

const TAB_STATES: [State; 3] = [State::Normal, State::Hot, State::Checked];

const TAB_CONTENT: Insets = Insets::new(1, 2, 0, 0);

const PAGE_W: i32 = 5;
const PAGE_H: i32 = 4;
const PAGE_BORDER: i32 = 3;

pub const GROUP_LABEL: GroupLabel = GroupLabel {
    nudge: (-1, -1),
    before: 2,
    after: 2,
    rows: 2,
};

pub fn parts(d: i32) -> [(Part, PartImage); 4] {
    let button = PartImage::new(
        stack(
            BUTTON_W,
            BUTTON_H,
            d,
            BUTTON_STATES.map(|(_, i)| button(i, d)),
        ),
        &BUTTON_STATES.map(|(s, _)| s),
        Layout::Vertical,
    )
    .sizing(BUTTON_SIZING)
    .content(BUTTON_CONTENT)
    .at_scale(d);
    let mut group = PartImage::new(
        stack(GROUP, GROUP, d, [group_box(d), group_box(d)]),
        &[State::Normal, State::Disabled],
        Layout::Vertical,
    )
    .sizing(GROUP_SIZING)
    .at_scale(d);
    group.border_only = true;
    let tab = PartImage::new(
        stack(
            TAB_W,
            TAB_H,
            d,
            [tab(Some(&TAB_CLOSED), d), tab(Some(&HOT), d), tab(None, d)],
        ),
        &TAB_STATES,
        Layout::Vertical,
    )
    .sizing(Insets::new(3, 1, 3, 0))
    .content(TAB_CONTENT)
    .at_scale(d);
    let pane = PartImage::new(page(d), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(1, 1, 3, 2))
        .at_scale(d);
    [
        (Part::PushButton, button),
        (Part::GroupBox, group),
        (Part::TabItem, tab),
        (Part::TabPane, pane),
    ]
}

fn solid(c: Color) -> Px {
    premul(c, 255)
}

fn shade(s: Shade, t: f64) -> Px {
    Stops(&[(0.0, solid(s.0)), (1.0, solid(s.1))]).at(t)
}

fn halves(upper: Shade, lower: Shade, y: f64, last: f64) -> Px {
    if y < STEP {
        shade(upper, (y - UPPER.0) / (UPPER.1 - UPPER.0))
    } else {
        shade(lower, (y - LOWER_TOP) / (last - LOWER_TOP))
    }
}

fn button(i: &Inks, d: i32) -> Canvas {
    let px = ringed(BOX_W, BOX_H, BUTTON_RADIUS, 2, d, |_, y, ring| match ring {
        0 => solid(i.border),
        1 if y < 1.5 => solid(i.line_top),
        1 if y > BUTTON_LAST + 0.5 => solid(i.line_bottom),
        1 => halves(i.line_upper, i.line_lower, y, BUTTON_LAST),
        _ => halves(i.upper, i.lower, y, BUTTON_LAST),
    });
    let (bw, bh) = (BOX_W * d, BOX_H * d);
    draw(BUTTON_W, BUTTON_H, d, |x, y| {
        let (x, y) = (x as i32 - d, y as i32 - d);
        if (0..bw).contains(&x) && (0..bh).contains(&y) {
            px[(y * bw + x) as usize]
        } else {
            CLEAR
        }
    })
}

fn group_box(d: i32) -> Canvas {
    draw(GROUP, GROUP, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        let dark = if GROUP_LIGHT.run(col, row) {
            f64::from(u8::from(GROUP_DARK.run(col, row)))
        } else {
            GROUP_DARK.cover(x, y, d)
        };
        over(
            scale(solid(GROUP_LINE), dark),
            scale(solid(WHITE), GROUP_LIGHT.cover(x, y, d)),
        )
    })
}

impl Outline {
    fn run(&self, col: i32, row: i32) -> bool {
        self.runs
            .iter()
            .any(|&(x, y, w, h)| (x..x + w).contains(&col) && (y..y + h).contains(&row))
    }

    fn cover(&self, x: f64, y: f64, d: i32) -> f64 {
        if self.run(x as i32 / d, y as i32 / d) {
            return 1.0;
        }
        let k = f64::from(d);
        self.cuts
            .iter()
            .map(|c| stroke(x, y, &c.map(|(px, py)| (px * k, py * k)), k))
            .fold(0.0, f64::max)
    }
}

fn tab(inks: Option<&Inks>, d: i32) -> Canvas {
    draw(TAB_W, TAB_H, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        let edge = row == 0 || col == 0 || col == TAB_W - 1;
        let Some(i) = inks else {
            return solid(if edge { TAB_LINE } else { WHITE });
        };
        let v = centre(y as i32, d);
        if edge {
            solid(i.border)
        } else if row == 1 {
            solid(i.line_top)
        } else if col == 1 || col == TAB_W - 2 {
            halves(i.line_upper, i.line_lower, v, TAB_LAST)
        } else {
            halves(i.upper, i.lower, v, TAB_LAST)
        }
    })
}

fn page(d: i32) -> Canvas {
    draw(PAGE_W, PAGE_H, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        let border = col < PAGE_BORDER && row < PAGE_BORDER && (col, row) != (1, 1);
        solid(if border { TAB_LINE } else { WHITE })
    })
}

fn stack<const N: usize>(w: i32, h: i32, d: i32, frames: [Canvas; N]) -> Canvas {
    let mut a = Canvas::new((w * d) as u32, (h * d * N as i32) as u32);
    for (i, f) in frames.iter().enumerate() {
        a.blit_over(0, i as i32 * h * d, f);
    }
    a
}

#[cfg(test)]
mod tests {
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::styled::fixture::doubles;
    use crate::look::widgets::{draw_button, draw_group_box, draw_page, draw_tab};
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, PartImage, State};
    use crate::text::TextRenderer;
    use crate::theme::{Color, Theme};

    const FACE: Color = Color::rgb(0xEF, 0xEE, 0xEF);
    const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

    fn glass() -> Theme {
        theme_for(Interface::Glass)
    }

    fn filled(w: i32, h: i32, s: i32, bg: Color) -> Canvas {
        let mut c = Canvas::with_scale(w as u32, h as u32, s);
        c.fill_rect(Rect::new(0, 0, w, h), bg);
        c
    }

    #[track_caller]
    fn assert_pixels(c: &Canvas, what: &str, (dx, dy): (i32, i32), pins: &[((i32, i32), u32)]) {
        for &((x, y), v) in pins {
            assert_eq!(
                c.get(dx + x, dy + y),
                0xFF00_0000 | v,
                "{what} at ({x}, {y}) is {v:06X}"
            );
        }
    }

    type Flags = (bool, Interaction, bool, bool);

    const NORMAL: Flags = (false, Interaction::Idle, true, false);
    const HOT: Flags = (false, Interaction::Hot, true, false);
    const DEFAULT: Flags = (true, Interaction::Idle, true, false);
    const PRESSED: Flags = (false, Interaction::Pressed, true, false);
    const DISABLED: Flags = (false, Interaction::Idle, false, false);
    const FOCUSED: Flags = (false, Interaction::Idle, true, true);

    fn button_at(size: (i32, i32), s: i32, label: &str, flags: Flags) -> Canvas {
        let theme = glass();
        let text = TextRenderer::load(&theme.fonts);
        let mut c = filled(size.0, size.1, s, FACE);
        let (default, ptr, enabled, focus) = flags;
        draw_button(
            &mut c,
            &text,
            &theme.paint(),
            Rect::new(0, 0, size.0, size.1),
            label,
            default,
            ptr,
            enabled,
            focus,
            theme.fonts.size_px,
        );
        c
    }

    fn button(flags: Flags) -> Canvas {
        button_at((75, 23), 1, "", flags)
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    fn part(d: i32, p: Part) -> PartImage {
        super::parts(d)
            .into_iter()
            .find(|(q, _)| *q == p)
            .unwrap()
            .1
    }

    const PARTS: [Part; 4] = [
        Part::PushButton,
        Part::GroupBox,
        Part::TabItem,
        Part::TabPane,
    ];

    #[test]
    fn sheet_parts_pin_at_scale_1_and_2() {
        let want: [(Part, [u64; 4]); 4] = [
            (
                Part::PushButton,
                [
                    0x4c9c_346e_30dc_c503,
                    0x85a4_b986_230e_e620,
                    0x6db4_b471_b795_c6f2,
                    0xc92d_0807_3f3b_2fb0,
                ],
            ),
            (
                Part::GroupBox,
                [
                    0x1dbf_d782_3232_c55c,
                    0xf60b_a2c0_bf67_6ee7,
                    0xb0f0_f233_6e6a_6874,
                    0x38a3_9e39_4b4f_998f,
                ],
            ),
            (
                Part::TabItem,
                [
                    0x79c6_6841_b0ce_7937,
                    0xb3a9_7c7c_5ff2_2317,
                    0x2d3f_0a5c_ad28_33c4,
                    0x462d_db85_f6c1_32d7,
                ],
            ),
            (
                Part::TabPane,
                [
                    0xdbab_63d3_7ac9_cba4,
                    0xcd7c_807e_b581_ea47,
                    0x516e_2d86_a26b_9584,
                    0x11cf_a3e4_b92b_5307,
                ],
            ),
        ];
        let got = PARTS.map(|p| {
            let light = |d| fnv(&part(d, p).atlas);
            let dark = |d| fnv(&crate::glass::dark::remap_part(part(d, p)).atlas);
            (p, [light(1), light(2), dark(1), dark(2)])
        });
        assert_eq!(got, want, "{got:#x?}");
    }

    #[test]
    fn every_glass_colour_and_material_shares_the_sheet_parts() {
        use crate::glass::{Material, Settings};
        for material in Material::ALL {
            let s = Settings {
                material,
                ..Settings::default()
            };
            let st = crate::glass::style(&s).at_scale(2);
            for p in PARTS {
                assert!(
                    st.part(p).unwrap().atlas == part(2, p).atlas,
                    "{material:?} draws {p:?} from the sheet's own painter"
                );
            }
        }
    }

    #[test]
    fn at_scale_2_each_part_is_twice_its_design_size_with_the_same_grid() {
        for p in PARTS {
            let (one, two) = (part(1, p), part(2, p));
            assert_eq!(
                (two.atlas.width, two.atlas.height),
                (2 * one.atlas.width, 2 * one.atlas.height),
                "{p:?} at 2 is twice its size"
            );
            assert_eq!((one.scale, two.scale), (1, 2), "{p:?} knows its scale");
            assert_eq!(two.sizing, one.sizing, "{p:?}'s grid is in design px");
            assert_eq!(two.content, one.content, "{p:?}'s content is in design px");
            assert_eq!(two.states, one.states, "{p:?} has the same states");
        }
    }

    fn frames(p: Part, d: i32) -> Vec<(State, Vec<u32>)> {
        let img = part(d, p);
        img.states
            .iter()
            .map(|&s| {
                let f = img.frame(s);
                let px = (f.y..f.bottom())
                    .flat_map(|y| (f.x..f.right()).map(move |x| (x, y)))
                    .map(|(x, y)| img.atlas.get(x, y))
                    .collect();
                (s, px)
            })
            .collect()
    }

    #[test]
    fn every_button_and_tab_state_draws_its_own_frame_but_focused_shares_the_default() {
        for d in [1, 2] {
            for p in [Part::PushButton, Part::TabItem] {
                let all = frames(p, d);
                for (i, (a, pa)) in all.iter().enumerate() {
                    for (b, pb) in &all[i + 1..] {
                        let alias = (*a, *b) == (State::Default, State::Focused);
                        assert_eq!(pa == pb, alias, "{p:?} {a:?} and {b:?} at {d}");
                    }
                }
            }
        }
    }

    fn alpha(px: u32) -> u32 {
        px >> 24
    }

    #[test]
    fn the_button_has_clear_corners_and_soft_curves_at_scale_2() {
        for (s, px) in frames(Part::PushButton, 1) {
            let at = |x: i32, y: i32| px[(y * 13 + x) as usize];
            for (x, y) in [(0, 0), (6, 0), (0, 11), (12, 11), (6, 22), (12, 22)] {
                assert_eq!(
                    alpha(at(x, y)),
                    0,
                    "{s:?}: the margin at ({x}, {y}) is clear"
                );
            }
            for (x, y) in [(1, 1), (11, 1), (1, 21), (11, 21)] {
                assert!(alpha(at(x, y)) < 0x40, "{s:?}: ({x}, {y}) is all but clear");
            }
            assert_eq!(alpha(at(6, 1)), 0xFF, "{s:?}: the border's top is solid");
        }
        for (s, px) in frames(Part::PushButton, 2) {
            let soft = (2..8)
                .flat_map(|y| (2..8).map(move |x| (x, y)))
                .any(|(x, y)| (1..0xFF).contains(&alpha(px[(y * 26 + x) as usize])));
            assert!(soft, "{s:?}: the corner's curve has partial alpha at 2");
        }
    }

    #[test]
    fn the_group_box_corner_is_soft_at_scale_2_and_its_middles_solid() {
        let img = part(1, Part::GroupBox);
        let at = |x: i32, y: i32| img.atlas.get(x, y);
        assert_eq!(at(3, 0), 0xFFD4_DDE4, "the top line's middle");
        assert_eq!(at(3, 1), 0xFFFF_FFFF, "the white line under it");
        assert_eq!(at(0, 3), 0xFFD4_DDE4, "the left line's middle");
        assert_eq!(at(1, 3), 0xFFFF_FFFF, "the white line inside it");
        assert_eq!(at(3, 5), 0xFFD4_DDE4, "the bottom line's middle");
        assert_eq!(at(3, 6), 0xFFFF_FFFF, "the white line under it");
        assert_eq!(alpha(at(0, 0)), 0, "the corner outside the cut is clear");
        assert_eq!(alpha(at(3, 3)), 0, "the middle is clear");
        let two = part(2, Part::GroupBox);
        let soft = (0..6)
            .flat_map(|y| (0..6).map(move |x| (x, y)))
            .any(|(x, y)| (1..0xFF).contains(&alpha(two.atlas.get(x, y))));
        assert!(soft, "the corner cut has partial alpha at 2");
    }

    fn luma(px: u32) -> u32 {
        let c = |s: u32| (px >> s) & 0xFF;
        (299 * c(16) + 587 * c(8) + 114 * c(0)) / 1000
    }

    #[test]
    fn a_button_over_white_has_no_pixel_darker_than_its_border() {
        let border = [0x6D6D6D, 0x3B77AD, 0x2B5B87, 0xABAFB3, 0x3B76AD, 0x3B76AD];
        for d in [1, 2] {
            for ((s, px), b) in frames(Part::PushButton, d).into_iter().zip(border) {
                let floor = luma(b);
                for p in px {
                    let a = alpha(p);
                    let on_white = |sh: u32| ((p >> sh) & 0xFF) + (0xFF - a);
                    let c = (on_white(16) << 16) | (on_white(8) << 8) | on_white(0);
                    assert!(
                        luma(c) + 1 >= floor,
                        "{s:?} at {d}: {c:06X} is darker than the border {b:06X}"
                    );
                }
            }
        }
    }

    #[test]
    fn glass_has_a_push_button_group_box_tab_and_page_part() {
        let st = crate::glass::style(&crate::glass::Settings::default());
        let p = st.part(Part::PushButton).expect("Glass has a push button");
        assert_eq!(
            p.states,
            [
                State::Normal,
                State::Hot,
                State::Pressed,
                State::Disabled,
                State::Default,
                State::Focused
            ],
            "six push button frames"
        );
        assert_eq!(
            (p.atlas.width, p.atlas.height),
            (13, 6 * 23),
            "13x23 frames"
        );
        for part in [Part::GroupBox, Part::TabItem, Part::TabPane] {
            assert!(st.part(part).is_some(), "Glass has {part:?}");
        }
    }

    const BUTTON_PINS: [((i32, i32), [u32; 5]); 11] = [
        ((0, 0), [0xEFEEEF, 0xEFEEEF, 0xEFEEEF, 0xEFEEEF, 0xEFEEEF]),
        ((2, 3), [0xFAF9FA, 0xF7FBFC, 0xA6B8C4, 0xFBFAFB, 0x43CCFA]),
        ((10, 1), [0x6D6D6D, 0x3B77AD, 0x2B5B87, 0xABAFB3, 0x3B76AD]),
        ((1, 10), [0x6D6D6D, 0x3B77AD, 0x2B5B87, 0xABAFB3, 0x3B76AD]),
        ((73, 10), [0x6D6D6D, 0x3B77AD, 0x2B5B87, 0xABAFB3, 0x3B76AD]),
        ((10, 2), [0xFCFBFC, 0xFAFDFD, 0x9BACB8, 0xFCFBFC, 0x43CEFB]),
        ((10, 4), [0xF1F0F1, 0xE7F4FD, 0xDFF0FB, 0xF3F4F4, 0xF1F0F1]),
        ((10, 11), [0xDCDBDC, 0xBCE2FD, 0x95CBEF, 0xF3F4F4, 0xDCDBDC]),
        ((10, 20), [0xF2F3F3, 0xE7F4FC, 0x64AAD9, 0xFCFBFC, 0x38C3F0]),
        ((10, 21), [0x6D6D6D, 0x3B77AD, 0x2B5B87, 0xABAFB3, 0x3B76AD]),
        ((72, 20), [0xB4B4B4, 0x96B9D7, 0x4985B3, 0xD6D7DA, 0x399FD1]),
    ];

    #[test]
    fn every_glass_button_state_paints_its_own_frame() {
        let st = crate::glass::style(&crate::glass::Settings::default());
        let states = [
            State::Normal,
            State::Hot,
            State::Pressed,
            State::Disabled,
            State::Default,
        ];
        for (i, state) in states.into_iter().enumerate() {
            let mut c = filled(75, 23, 1, FACE);
            st.draw(&mut c, Rect::new(0, 0, 75, 23), Part::PushButton, state, 1);
            let pins: Vec<_> = BUTTON_PINS.iter().map(|&(at, v)| (at, v[i])).collect();
            assert_pixels(&c, &format!("the {state:?} button"), (0, 0), &pins);
        }
    }

    #[test]
    fn the_button_drawer_picks_each_frame_from_its_flags() {
        for (flags, i, what) in [
            (NORMAL, 0, "at rest"),
            (HOT, 1, "under the pointer"),
            (
                (true, Interaction::Hot, true, false),
                1,
                "the default under the pointer",
            ),
            (PRESSED, 2, "held"),
            (DISABLED, 3, "greyed"),
            (DEFAULT, 4, "the default"),
        ] {
            let pins: Vec<_> = BUTTON_PINS.iter().map(|&(at, v)| (at, v[i])).collect();
            assert_pixels(&button(flags), &format!("a button {what}"), (0, 0), &pins);
        }
    }

    #[test]
    fn an_inactive_windows_default_button_draws_plain() {
        let theme = glass();
        let text = TextRenderer::load(&theme.fonts);
        let inactive = {
            let mut c = filled(75, 23, 1, FACE);
            let p = crate::look::Paint {
                active: false,
                ..theme.paint()
            };
            draw_button(
                &mut c,
                &text,
                &p,
                Rect::new(0, 0, 75, 23),
                "",
                true,
                Interaction::Idle,
                true,
                false,
                theme.fonts.size_px,
            );
            c
        };
        let pins: Vec<_> = BUTTON_PINS.iter().map(|&(at, v)| (at, v[0])).collect();
        assert_pixels(&inactive, "an inactive default", (0, 0), &pins);
    }

    #[test]
    fn a_default_glass_button_on_a_white_page_blends_its_corners_with_it() {
        let st = crate::glass::style(&crate::glass::Settings::default());
        let on = |bg| {
            let mut c = filled(75, 23, 1, bg);
            st.draw(
                &mut c,
                Rect::new(0, 0, 75, 23),
                Part::PushButton,
                State::Default,
                1,
            );
            c
        };
        let (white, face) = (on(WHITE), on(FACE));
        assert_pixels(
            &white,
            "the default button on white",
            (0, 0),
            &[((0, 0), 0xFFFFFF), ((10, 1), 0x3B76AD), ((10, 2), 0x43CEFB)],
        );
        for (x, y) in [
            (1, 1),
            (2, 1),
            (1, 2),
            (1, 20),
            (2, 21),
            (72, 1),
            (73, 2),
            (73, 20),
            (72, 21),
        ] {
            let (w, f) = (white.get(x, y), face.get(x, y));
            for sh in [0, 8, 16] {
                assert!(
                    (w >> sh) & 0xFF >= (f >> sh) & 0xFF,
                    "({x}, {y}) on white ({w:08X}) is no darker than on the face ({f:08X})"
                );
            }
        }
        assert!(
            white.get(2, 1) != face.get(2, 1),
            "the corner's curve lets the page through"
        );
    }

    #[test]
    fn a_wide_or_tall_glass_button_keeps_its_corners_and_stretches_its_face() {
        let base = button(NORMAL);
        let wide = button_at((120, 23), 1, "", NORMAL);
        for (x, y) in [(1, 1), (2, 1), (2, 3), (1, 21)] {
            assert_eq!(
                wide.get(x, y),
                base.get(x, y),
                "the left corner at ({x}, {y})"
            );
            assert_eq!(
                wide.get(119 - x, y),
                base.get(74 - x, y),
                "the right corner"
            );
        }
        assert_pixels(
            &wide,
            "a 120x23 button",
            (0, 0),
            &[
                ((60, 1), 0x6D6D6D),
                ((118, 10), 0x6D6D6D),
                ((117, 20), 0xB4B4B4),
                ((60, 4), 0xF1F0F1),
                ((60, 11), 0xDCDBDC),
            ],
        );
        let tall = button_at((75, 40), 1, "", NORMAL);
        assert_eq!(
            tall.get(2, 1),
            base.get(2, 1),
            "the top corner keeps its pixels"
        );
        assert_pixels(
            &tall,
            "a 75x40 button",
            (0, 0),
            &[
                ((1, 20), 0x6D6D6D),
                ((72, 37), 0xB4B4B4),
                ((10, 38), 0x6D6D6D),
                ((10, 37), 0xF2F3F3),
                ((10, 4), 0xF1F0F1),
                ((10, 36), 0xCDCCCD),
            ],
        );
        let (top, bottom) = (tall.get(10, 10) & 0xFF, tall.get(10, 30) & 0xFF);
        assert!(
            top > bottom,
            "the face's upper half ({top:02X}) is lighter than its lower ({bottom:02X})"
        );
    }

    #[test]
    fn a_focused_glass_button_draws_the_default_frame_and_a_dotted_ring_3_px_in() {
        let c = button(FOCUSED);
        let base = button(DEFAULT);
        for (x, y, dot) in [
            (10, 1, false),
            (3, 3, false),
            (4, 3, true),
            (5, 3, false),
            (3, 4, true),
            (71, 4, true),
            (70, 19, true),
            (4, 10, false),
        ] {
            let want = if dot {
                base.get(x, y) ^ 0x00EF_EEEF
            } else {
                base.get(x, y)
            };
            assert_eq!(c.get(x, y), want, "the focused button at ({x}, {y})");
        }
        assert!(
            button((true, Interaction::Idle, true, true)) == c,
            "a focused default button draws the same pixels"
        );
    }

    fn inked(c: &Canvas, ink: Color) -> Vec<(i32, i32)> {
        (0..c.height as i32)
            .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink.argb())
            .collect()
    }

    #[test]
    fn a_glass_button_label_is_black_with_its_baseline_on_row_15() {
        if !crate::text::test_tahoma() {
            return;
        }
        let c = button_at((75, 23), 1, "Cancel", NORMAL);
        let rows: Vec<i32> = inked(&c, Color::rgb(0, 0, 0))
            .into_iter()
            .map(|(_, y)| y)
            .collect();
        assert_eq!(rows.iter().max(), Some(&15), "the label's last row is 15");
        let held = button_at((75, 23), 1, "Cancel", PRESSED);
        assert_eq!(
            inked(&held, Color::rgb(0, 0, 0)),
            inked(&c, Color::rgb(0, 0, 0)),
            "a held button's label stays put"
        );
    }

    #[test]
    fn a_glass_button_on_a_scale_2_canvas_doubles_every_pixel() {
        for flags in [NORMAL, HOT, DEFAULT, PRESSED, DISABLED, FOCUSED] {
            assert!(
                doubles(&button(flags), &button_at((75, 23), 2, "", flags)),
                "{flags:?} at scale 2 is its scale-1 pixels, each 2x2"
            );
        }
    }

    fn group_box(g: Rect, s: i32, bg: Color) -> (Canvas, i32) {
        let theme = glass();
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let (w, h) = (g.right() + 10, g.bottom() + 10);
        let mut c = filled(w, h, s, bg);
        let base = theme.paint();
        let p = if bg == WHITE {
            let page = Rect::new(0, 0, w, h);
            draw_page(&mut c, &base, page);
            base.on_page(page)
        } else {
            base
        };
        let drawn = text.measure("Choices", false, px);
        let lr = Rect::new(g.x + 10, g.y - 5, drawn, 13);
        draw_group_box(&mut c, &text, &p, g, lr, "&Choices", px);
        (c, drawn)
    }

    #[test]
    fn a_glass_group_box_is_a_d5dfe5_line_cut_at_the_corners_with_a_white_line_beside_it() {
        let g = Rect::new(5, 10, 100, 60);
        let (c, _) = group_box(g, 1, FACE);
        let (r, b) = (g.width - 1, g.height - 1);
        assert_pixels(
            &c,
            "the group box",
            (g.x, g.y),
            &[
                ((0, 0), 0xEFEEEF),
                ((0, 2), 0xD4DDE4),
                ((0, 30), 0xD4DDE4),
                ((1, 30), 0xFFFFFF),
                ((60, 0), 0xD4DDE4),
                ((60, 1), 0xFFFFFF),
                ((r, 30), 0xD4DDE4),
                ((r - 1, 30), 0xFFFFFF),
                ((60, b - 1), 0xD4DDE4),
                ((60, b), 0xFFFFFF),
                ((r, b), 0xEFEEEF),
                ((50, 30), 0xEFEEEF),
            ],
        );
    }

    #[test]
    fn a_glass_group_box_label_breaks_both_top_lines_2_px_either_side() {
        let g = Rect::new(5, 10, 150, 60);
        for (bg, back) in [(FACE, 0xEFEEEF), (WHITE, 0xFFFFFF)] {
            let (c, drawn) = group_box(g, 1, bg);
            let end = 9 + drawn + 2;
            assert_pixels(
                &c,
                "the group box's top lines",
                (g.x, g.y),
                &[
                    ((6, 0), 0xD4DDE4),
                    ((6, 1), 0xFFFFFF),
                    ((7, 0), back),
                    ((7, 1), back),
                    ((end - 1, 0), back),
                    ((end - 1, 1), back),
                    ((end, 0), 0xD4DDE4),
                    ((end, 1), 0xFFFFFF),
                ],
            );
        }
    }

    #[test]
    fn a_glass_group_box_label_sits_9_px_in_with_its_baseline_4_px_under_the_line() {
        if !crate::text::test_tahoma() {
            return;
        }
        let g = Rect::new(5, 20, 150, 60);
        let (c, _) = group_box(g, 1, FACE);
        let ink = inked(&c, Color::rgb(0, 0, 0));
        let left = ink.iter().map(|&(x, _)| x).min();
        let glyphs: Vec<i32> = ink.iter().map(|&(_, y)| y).collect();
        let base = glyphs.iter().filter(|&&y| y < g.y + 6).max();
        assert_eq!(left, Some(g.x + 9), "the label's ink starts 9 px in");
        assert_eq!(
            base,
            Some(&(g.y + 4)),
            "its baseline is 4 px under the line"
        );
    }

    #[test]
    fn a_glass_group_box_on_a_scale_2_canvas_doubles_every_pixel() {
        let st = crate::glass::style(&crate::glass::Settings::default());
        let draw = |s| {
            let mut c = filled(60, 40, s, FACE);
            st.draw(
                &mut c,
                Rect::new(5, 5, 50, 30),
                Part::GroupBox,
                State::Normal,
                1,
            );
            c
        };
        assert!(doubles(&draw(1), &draw(2)), "each pixel covers 2x2");
    }

    fn device(c: &Canvas) -> impl Fn(i32, i32) -> u32 {
        let (w, _) = c.device_size();
        let bytes = c.to_argb_be();
        move |x, y| {
            let i = 4 * (y as usize * w as usize + x as usize);
            u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap())
        }
    }

    #[test]
    fn a_glass_group_box_at_scale_2_moves_its_label_and_breaks_both_lines() {
        let theme = glass();
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let g = Rect::new(5, 20, 150, 60);
        let (c, drawn) = group_box(g, 2, FACE);
        let end = 9 + drawn + 2;
        let at = device(&c);
        for (x, y, rgb, what) in [
            (6, 0, 0xD4DDE4, "the line before the break"),
            (6, 1, 0xFFFFFF, "the white line before the break"),
            (7, 0, 0xEFEEEF, "the break's first column, top row"),
            (7, 1, 0xEFEEEF, "the break's first column, second row"),
            (end - 1, 1, 0xEFEEEF, "the break's last column"),
            (end, 0, 0xD4DDE4, "the line after the break"),
            (end, 1, 0xFFFFFF, "the white line after the break"),
        ] {
            for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (dx, dy) = (2 * (g.x + x) + ox, 2 * (g.y + y) + oy);
                assert_eq!(at(dx, dy), 0xFF00_0000 | rgb, "{what}: {rgb:06X}");
            }
        }
        let mut want = filled(g.right() + 10, g.bottom() + 10, 2, FACE);
        let lr = Rect::new(g.x + 10 - 1, g.y - 5 - 1, drawn, 13);
        crate::classic::widgets::draw_label(
            &mut want,
            &text,
            lr,
            "&Choices",
            false,
            Color::rgb(0, 0, 0),
            crate::text::Align::Left,
            px,
            1,
        );
        let black = |c: &Canvas| {
            let (w, h) = c.device_size();
            let at = device(c);
            (0..h as i32)
                .flat_map(|y| (0..w as i32).map(move |x| (x, y)))
                .filter(|&(x, y)| at(x, y) == 0xFF00_0000)
                .collect::<Vec<_>>()
        };
        let got = black(&c);
        assert!(!got.is_empty(), "the label inks black");
        assert_eq!(got, black(&want), "the label sits a pixel up and left");
    }

    #[test]
    fn a_glass_group_box_with_no_label_keeps_its_lines_whole() {
        let theme = glass();
        let text = TextRenderer::load(&theme.fonts);
        let g = Rect::new(5, 10, 100, 60);
        let mut c = filled(120, 80, 1, FACE);
        let lr = Rect::new(g.x + 10, g.y - 5, 0, 13);
        draw_group_box(
            &mut c,
            &text,
            &theme.paint(),
            g,
            lr,
            "",
            theme.fonts.size_px,
        );
        assert_pixels(
            &c,
            "the unlabelled group box",
            (g.x, g.y),
            &[((8, 0), 0xD4DDE4), ((8, 1), 0xFFFFFF), ((12, 0), 0xD4DDE4)],
        );
    }

    #[test]
    fn a_glass_group_title_rides_a_pixel_higher_at_its_own_x() {
        use crate::controls::form::{self, Form, Item};
        let theme = glass();
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let mut got = filled(120, 40, 1, FACE);
        let f = Form::new(vec![Item::group_title("t", 10, 12, 80, "Choices")]);
        form::draw(&mut got, &text, &theme.paint(), &f, false, px);
        let mut want = filled(120, 40, 1, FACE);
        let r = Rect::new(10, 11, 80, crate::classic::widgets::LINE_H);
        crate::classic::widgets::draw_label(
            &mut want,
            &text,
            r,
            "Choices",
            false,
            Color::rgb(0, 0, 0),
            crate::text::Align::Left,
            px,
            1,
        );
        assert!(got == want, "the title is drawn at x 10, a row above y 12");
    }

    fn tab_frame(state: State) -> Canvas {
        let st = crate::glass::style(&crate::glass::Settings::default());
        let mut c = filled(56, 22, 1, FACE);
        st.draw(&mut c, Rect::new(2, 2, 52, 18), Part::TabItem, state, 1);
        c
    }

    #[test]
    fn a_closed_glass_tab_is_a_square_898c95_outline_round_a_grey_fade() {
        assert_pixels(
            &tab_frame(State::Normal),
            "the closed tab",
            (2, 2),
            &[
                ((0, 0), 0x868993),
                ((20, 0), 0x868993),
                ((51, 0), 0x868993),
                ((0, 10), 0x868993),
                ((51, 17), 0x868993),
                ((1, 1), 0xFCFBFC),
                ((20, 1), 0xFCFBFC),
                ((20, 2), 0xF2F1F2),
                ((20, 10), 0xDCDBDC),
                ((20, 17), 0xCDCCCD),
                ((1, 17), 0xF2F3F3),
                ((50, 10), 0xF5F6F6),
            ],
        );
    }

    #[test]
    fn a_hot_glass_tab_is_outlined_3c7fb1_round_pale_blue() {
        assert_pixels(
            &tab_frame(State::Hot),
            "the hot tab",
            (2, 2),
            &[
                ((0, 0), 0x3B77AD),
                ((20, 0), 0x3B77AD),
                ((51, 10), 0x3B77AD),
                ((1, 1), 0xFAFDFD),
                ((20, 2), 0xE9F5FD),
                ((20, 10), 0xBCE2FD),
                ((20, 17), 0xA4D4F5),
                ((50, 17), 0xE8F4FC),
            ],
        );
    }

    fn strip_hot(s: i32, hot: Option<usize>) -> Canvas {
        let theme = glass();
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let p = theme.paint();
        let mut c = filled(130, 60, s, FACE);
        let page = Rect::new(0, 20, 120, 35);
        draw_page(&mut c, &p, page);
        draw_tab(
            &mut c,
            &text,
            &p,
            Rect::new(54, 2, 50, 18),
            "",
            false,
            hot == Some(1),
            page,
            px,
        );
        draw_tab(
            &mut c,
            &text,
            &p,
            Rect::new(0, 0, 56, 20),
            "",
            true,
            hot == Some(0),
            page,
            px,
        );
        c
    }

    fn strip(s: i32) -> Canvas {
        strip_hot(s, None)
    }

    #[test]
    fn a_closed_glass_tab_under_the_pointer_is_outlined_3c7fb1_and_the_open_one_stays_white() {
        assert_pixels(
            &strip_hot(1, Some(1)),
            "the hot closed tab",
            (54, 2),
            &[
                ((6, 0), 0x3B77AD),
                ((49, 8), 0x3B77AD),
                ((20, 10), 0xBCE2FD),
            ],
        );
        assert!(
            strip_hot(1, Some(0)) == strip(1),
            "the open tab under the pointer draws as it does at rest"
        );
    }

    #[test]
    fn the_open_glass_tab_is_white_and_runs_into_the_white_page() {
        assert_pixels(
            &strip(1),
            "the strip",
            (0, 0),
            &[
                ((0, 0), 0x868993),
                ((55, 0), 0x868993),
                ((10, 1), 0xFFFFFF),
                ((10, 19), 0xFFFFFF),
                ((10, 20), 0xFFFFFF),
                ((0, 20), 0x868993),
                ((55, 20), 0x868993),
                ((56, 3), 0xFCFBFC),
                ((60, 2), 0x868993),
                ((60, 20), 0x868993),
                ((60, 21), 0xFFFFFF),
            ],
        );
    }

    #[test]
    fn a_glass_page_is_white_inside_a_898c95_border_with_white_right_and_below() {
        assert_pixels(
            &strip(1),
            "the page",
            (0, 20),
            &[
                ((0, 10), 0x868993),
                ((1, 10), 0xFFFFFF),
                ((60, 15), 0xFFFFFF),
                ((116, 10), 0xFFFFFF),
                ((117, 10), 0x868993),
                ((118, 10), 0xFFFFFF),
                ((119, 10), 0xFFFFFF),
                ((117, 0), 0x868993),
                ((118, 0), 0xFFFFFF),
                ((60, 32), 0xFFFFFF),
                ((60, 33), 0x868993),
                ((60, 34), 0xFFFFFF),
                ((0, 34), 0xFFFFFF),
                ((120, 10), 0xEFEEEF),
            ],
        );
    }

    #[test]
    fn a_glass_tab_strip_on_a_scale_2_canvas_doubles_every_pixel() {
        assert!(doubles(&strip(1), &strip(2)), "each pixel covers 2x2");
    }
}
