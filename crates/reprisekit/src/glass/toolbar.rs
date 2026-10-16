//! Glass's toolbar and rebar parts: flat buttons on a flat light band, drawn from named colours.
//! A button draws nothing at rest or greyed; a split button's drop part overlaps its main part by three columns.

use crate::raster::{Canvas, Fill, Insets, Rect};
use crate::shape::{Px, Stops, centre, draw, ringed};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const STATES: [State; 6] = [
    State::Normal,
    State::Hot,
    State::Pressed,
    State::Disabled,
    State::Checked,
    State::CheckedHot,
];

/// Every frame's height, and the rows of it that keep their height at
/// the top and the bottom.
const H: i32 = 24;
const TOP: i32 = 12;
const BOTTOM: i32 = 3;

const END: i32 = 3;

/// How far into its main part a drop part's frame starts: the main
/// part's face column, its inner column and the split line.
const DROP_INTO_MAIN: i32 = 3;

const BUTTON_W: i32 = 2 * END + 1;
const DROP_W: i32 = DROP_INTO_MAIN + 2 + END;
const DROP_MAIN_INNER: i32 = 1;
const DROP_SPLIT: i32 = 2;
const DROP_INNER: i32 = 3;
/// How far left of the drop frame the box it is cut from starts, so the
/// box's rounded left end and its held left shade fall outside the frame.
const DROP_BOX_LEFT: i32 = 6;

const RADIUS: f64 = 3.0;

const fn grey(v: f64) -> Px {
    (255.0, [v, v, v])
}

const HOT_OUTLINE: Px = grey(170.0);
const HOT_SPLIT_EDGE: Px = grey(201.0);
const HOT_FACE: [(f64, Px); 6] = [
    (2.0, grey(249.0)),
    (8.0, grey(237.0)),
    (11.5, grey(237.0)),
    (11.5, grey(223.0)),
    (12.0, grey(223.0)),
    (21.0, grey(247.0)),
];
const HOT_RING: [(f64, Px); 5] = [
    (1.0, grey(253.0)),
    (11.5, grey(251.0)),
    (11.5, grey(226.0)),
    (20.0, grey(248.0)),
    (22.0, grey(251.0)),
];

const HELD_OUTLINE: Px = grey(84.0);
const HELD_SPLIT_EDGE: Px = grey(150.0);
const HELD_FACE: [(f64, Px); 4] = [
    (1.0, grey(233.0)),
    (11.5, grey(233.0)),
    (11.5, grey(228.0)),
    (22.0, grey(234.0)),
];
const HELD_TOP_SHADE: f64 = 0.32;
const HELD_TOP_FADE: f64 = 0.45;
const HELD_LEFT_SHADE: f64 = 0.20;
const HELD_LEFT_SPAN: f64 = END as f64 - 1.5;
const HELD_DROP_SHADE: f64 = 0.30;

const REBAR: Color = Color::rgb(0xF6, 0xF7, 0xF7);
const REBAR_LINE: Color = Color::rgb(0x6B, 0x6C, 0x6C);

const SEP_DARK: Color = Color::rgb(0x9D, 0x9E, 0x9E);
const SEP_LIGHT: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const SEP_W: i32 = 6;
const SEP_X: i32 = 2;
const SEP_INSET: i32 = 2;

const GRIP: [[u32; 3]; 4] = [
    [0xFFFDFCFD, 0xFFFDFCFD, 0],
    [0xFFFDFCFD, 0xFFE1E0E1, 0xFFBBBABB],
    [0, 0xFFBBBABB, 0xFFBBBABB],
    [0, 0, 0],
];

pub fn parts(scale: i32) -> [(Part, PartImage); 6] {
    let button = PartImage::new(frames(Kind::Button, scale), &STATES, Layout::Vertical)
        .sizing(Insets::new(END, TOP, END, BOTTOM))
        .at_scale(scale);
    let drop = PartImage::new(frames(Kind::Drop, scale), &STATES, Layout::Vertical)
        .sizing(Insets::new(DROP_INTO_MAIN + 1, TOP, END, BOTTOM))
        .outset(Insets::new(DROP_INTO_MAIN, 0, 0, 0))
        .at_scale(scale);
    [
        (Part::ToolbarButton, button),
        (Part::ToolbarDrop, drop),
        (Part::ToolbarSeparator, separator()),
        (Part::Rebar, rebar()),
        (Part::RebarBand, band_line()),
        (Part::RebarGripper, gripper()),
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Button,
    Drop,
}

impl Kind {
    fn width(self) -> i32 {
        match self {
            Kind::Button => BUTTON_W,
            Kind::Drop => DROP_W,
        }
    }
}

fn frames(kind: Kind, d: i32) -> Canvas {
    let w = kind.width();
    let order = [None, Some(false), Some(true), None, Some(true), Some(true)];
    let mut a = Canvas::new((w * d) as u32, (H * d * order.len() as i32) as u32);
    for (i, held) in order.into_iter().enumerate() {
        let Some(held) = held else { continue };
        let px = frame(kind, held, d);
        let f = draw(w, H, d, |x, y| px[(y as i32 * w * d + x as i32) as usize]);
        a.blit_over(0, i as i32 * H * d, &f);
    }
    a
}

/// One lit or `held` frame of `kind` at `d`, row by row in device px.
///
/// A button is a whole rounded box. A drop part is the right end of a
/// box starting [`DROP_BOX_LEFT`] columns left of it, with the main
/// part's inner column, the split line and its own inner column painted
/// across it.
fn frame(kind: Kind, held: bool, d: i32) -> Vec<Px> {
    let w = kind.width();
    let left = if kind == Kind::Drop { DROP_BOX_LEFT } else { 0 };
    let bw = w + left;
    let rings = if held { 1 } else { 2 };
    let face = ringed(bw, H, RADIUS, rings, d, |x, y, ring| match (held, ring) {
        (false, 0) => HOT_OUTLINE,
        (false, 1) => Stops(&HOT_RING).at(y),
        (false, _) => Stops(&HOT_FACE).at(y),
        (true, 0) => HELD_OUTLINE,
        (true, _) => held_face(x, y),
    });
    let (wd, hd) = (w * d, H * d);
    let mut out = Vec::with_capacity((wd * hd) as usize);
    for y in 0..hd {
        for x in 0..wd {
            let boxed = face[(y * bw * d + x + left * d) as usize];
            let own = (kind == Kind::Drop)
                .then(|| drop_column(x / d, y / d, centre(y, d), held))
                .flatten();
            out.push(own.unwrap_or(boxed));
        }
    }
    out
}

fn drop_column(col: i32, row: i32, t: f64, held: bool) -> Option<Px> {
    let edge = row == 0 || row == H - 1;
    match (col, held) {
        (DROP_SPLIT, false) if edge => Some(HOT_SPLIT_EDGE),
        (DROP_SPLIT, false) => Some(HOT_OUTLINE),
        (DROP_SPLIT, true) if edge => Some(HELD_SPLIT_EDGE),
        (DROP_SPLIT, true) => Some(HELD_OUTLINE),
        (DROP_MAIN_INNER | DROP_INNER, false) if !edge => Some(Stops(&HOT_RING).at(t)),
        (DROP_INNER, true) if !edge => {
            Some(shade(held_face(f64::from(DROP_W), t), HELD_DROP_SHADE))
        }
        _ => None,
    }
}

fn held_face(x: f64, y: f64) -> Px {
    let top = HELD_TOP_SHADE * (-HELD_TOP_FADE * (y - 1.0)).exp();
    let left = HELD_LEFT_SHADE * (1.0 - (x - 1.0) / HELD_LEFT_SPAN).clamp(0.0, 1.0).powi(2);
    shade(shade(Stops(&HELD_FACE).at(y), top), left)
}

fn shade(p: Px, k: f64) -> Px {
    let keep = 1.0 - k.clamp(0.0, 1.0);
    (p.0, p.1.map(|c| c * keep))
}

fn rebar() -> PartImage {
    let mut a = Canvas::new(1, 2);
    a.fill_rect(Rect::new(0, 0, 1, 1), REBAR);
    a.fill_rect(Rect::new(0, 1, 1, 1), REBAR_LINE);
    PartImage::new(a, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(0, 0, 0, 1))
        .outset(Insets::new(0, 0, 0, 1))
}

fn band_line() -> PartImage {
    PartImage::new(Canvas::new(1, 2), &[State::Normal], Layout::Vertical)
}

fn separator() -> PartImage {
    let h = 2 * SEP_INSET + 1;
    let mut a = Canvas::new(SEP_W as u32, h as u32);
    a.fill_rect(Rect::new(SEP_X, SEP_INSET, 1, 1), SEP_DARK);
    a.fill_rect(Rect::new(SEP_X + 1, SEP_INSET, 1, 1), SEP_LIGHT);
    PartImage::new(a, &[State::Normal], Layout::Vertical).sizing(Insets::new(
        SEP_X + 2,
        SEP_INSET,
        SEP_W - SEP_X - 2,
        SEP_INSET,
    ))
}

/// The separator's lines over the frame's glass: translucent, so they etch whatever glass lies
/// behind in either mode.
const GLASS_SEP_DARK: u32 = 0x3300_0000;
const GLASS_SEP_LIGHT: u32 = 0x8080_8080;

/// [`separator`] over the frame's glass, painted at `d`: its dark line and light line
/// [`GLASS_SEP_DARK`] and [`GLASS_SEP_LIGHT`].
pub fn glass_separator(d: i32) -> PartImage {
    let h = 2 * SEP_INSET + 1;
    let mut a = Canvas::new((SEP_W * d) as u32, (h * d) as u32);
    for (x, px) in [(SEP_X, GLASS_SEP_DARK), (SEP_X + 1, GLASS_SEP_LIGHT)] {
        for (dx, dy) in (0..d).flat_map(|dx| (0..d).map(move |dy| (dx, dy))) {
            a.put_raw(x * d + dx, SEP_INSET * d + dy, px);
        }
    }
    PartImage::new(a, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(
            SEP_X + 2,
            SEP_INSET,
            SEP_W - SEP_X - 2,
            SEP_INSET,
        ))
        .at_scale(d)
}

fn gripper() -> PartImage {
    let mut a = Canvas::new(3, GRIP.len() as u32);
    for (y, row) in GRIP.iter().enumerate() {
        for (x, &px) in row.iter().enumerate() {
            a.put_raw(x as i32, y as i32, px);
        }
    }
    PartImage::new(a, &[State::Normal], Layout::Vertical).fill(Fill::Tile)
}

#[cfg(test)]
mod tests {
    use super::H;
    use crate::controls::menubar;
    use crate::controls::rebar::{self, Band, Rebar};
    use crate::glass::{Material, Mode, Settings, dark};
    use crate::interface::Interface;
    use crate::interfaces::{theme_for, theme_with};
    use crate::look::styled::fixture::{doubles_at_paint_scale_2, part_doubles_at_scale_2};
    use crate::look::widgets::toolbar::{draw_button, draw_separator};
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, PartImage, State};
    use crate::text::TextRenderer;
    use crate::theme::{Color, FontConfig, Theme};

    const BAND: u32 = 0xFFF6F7F7;

    fn glass() -> Theme {
        theme_for(Interface::Glass)
    }

    fn dark_glass() -> Theme {
        theme_with(
            Interface::Glass,
            crate::blue::Variant::default(),
            &Settings {
                mode: Mode::Dark,
                ..Settings::default()
            },
        )
    }

    fn button(theme: &Theme, h: i32, state: State, split: Option<i32>) -> Canvas {
        let mut c = Canvas::new(72, h as u32);
        c.fill_rect(c.bounds(), Color::rgb(0xF6, 0xF7, 0xF7));
        draw_button(&mut c, &theme.paint(), Rect::new(0, 0, 72, h), split, state);
        c
    }

    /// A corner pixel the rounded outline only grazes: the band, give or
    /// take a few levels.
    #[track_caller]
    fn corner(c: &Canvas, x: i32, y: i32, what: &str) {
        let v = c.get(x, y) & 0xFF;
        assert!(v.abs_diff(0xF7) <= 4, "{what}: ({x}, {y}) is {v:02X}");
    }

    #[track_caller]
    fn pin(c: &Canvas, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(x, y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    #[test]
    fn a_lit_or_held_button_draws_its_frame_at_both_strip_heights() {
        let theme = glass();
        let lum = |px: u32| px & 0xFF;
        for h in [30, 22] {
            let hot = button(&theme, h, State::Hot, None);
            pin(&hot, 0, h / 2, 0xAAAAAA, "hot left outline");
            pin(&hot, 71, h / 2, 0xAAAAAA, "hot right outline");
            pin(&hot, 36, 0, 0xAAAAAA, "hot top outline");
            pin(&hot, 36, h - 1, 0xAAAAAA, "hot bottom outline");
            corner(&hot, 0, 0, "hot clear corner");
            corner(&hot, 71, h - 1, "hot clear corner");
            assert!(lum(hot.get(36, 11)) > lum(hot.get(36, 12)) + 8, "hot step");
            assert!(lum(hot.get(36, 2)) > lum(hot.get(36, 11)), "hot top light");
            assert!(
                lum(hot.get(1, 5)) > lum(hot.get(36, 5)),
                "hot ring over face"
            );

            let held = button(&theme, h, State::Pressed, None);
            pin(&held, 0, h / 2, 0x545454, "held left outline");
            pin(&held, 71, h / 2, 0x545454, "held right outline");
            corner(&held, 0, 0, "held clear corner");
            assert!(
                lum(held.get(36, 1)) + 50 < lum(held.get(36, 11)),
                "held top shade"
            );
            assert!(
                lum(held.get(1, 9)) + 30 < lum(held.get(36, 9)),
                "held left shade"
            );
        }
    }
    #[test]
    fn a_latched_button_draws_held_and_a_resting_one_nothing() {
        let theme = glass();
        let held = button(&theme, 30, State::Pressed, None);
        for st in [State::Checked, State::CheckedHot] {
            assert!(button(&theme, 30, st, None) == held, "{st:?}");
        }
        for st in [State::Normal, State::Disabled] {
            let c = button(&theme, 30, st, None);
            assert!(
                (0..30).all(|y| (0..72).all(|x| c.get(x, y) == BAND)),
                "{st:?}"
            );
        }
    }

    #[test]
    fn a_split_button_is_one_frame_cut_by_a_line() {
        let theme = glass();
        let hot = button(&theme, 30, State::Hot, Some(60));
        pin(&hot, 59, 15, 0xAAAAAA, "split line");
        pin(&hot, 59, 0, 0xC9C9C9, "split line in the top outline");
        pin(&hot, 59, 29, 0xC9C9C9, "split line in the bottom outline");
        pin(
            &hot,
            58,
            0,
            0xAAAAAA,
            "main part's top outline runs to the line",
        );
        pin(&hot, 58, 5, 0xFCFCFC, "main part's inner column");
        pin(&hot, 60, 5, 0xFCFCFC, "drop part's inner column");
        assert!(
            (hot.get(57, 5) & 0xFF) < 0xFC,
            "main part's face under its ring"
        );
        pin(&hot, 71, 15, 0xAAAAAA, "drop part's right outline");
        corner(&hot, 71, 0, "drop part's clear corner");
        corner(&hot, 0, 0, "main part's clear corner");
        let held = button(&theme, 30, State::Pressed, Some(60));
        pin(&held, 59, 15, 0x545454, "held split line");
        pin(&held, 59, 0, 0x969696, "held split line in the outline");
        assert!(
            (held.get(60, 5) & 0xFF) + 40 < (held.get(62, 5) & 0xFF),
            "held drop part's shaded column"
        );
    }
    #[test]
    fn a_separator_is_a_dark_and_a_light_column() {
        let theme = glass();
        for (w, big) in [(6, true), (8, false)] {
            let mut c = Canvas::new(w, 30);
            c.fill_rect(c.bounds(), Color::rgb(0xF6, 0xF7, 0xF7));
            draw_separator(&mut c, &theme.paint(), big, Rect::new(0, 0, w as i32, 30));
            for y in 0..30 {
                let line = (2..28).contains(&y);
                let (dark, light) = if line {
                    (0xFF9D9E9E, 0xFFFFFFFF)
                } else {
                    (BAND, BAND)
                };
                assert_eq!(c.get(2, y), dark, "{w} dark column y {y}");
                assert_eq!(c.get(3, y), light, "{w} light column y {y}");
                for x in (0..w as i32).filter(|x| ![2, 3].contains(x)) {
                    assert_eq!(c.get(x, y), BAND, "{w} clear ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_rebar_is_flat_and_closes_with_its_own_line() {
        let theme = glass();
        let cm = theme.look.controls();
        let rb = Rebar {
            bands: [22, 30, 22].map(|height| Band { height }).into(),
        };
        let h = rebar::height(&rb, &cm.rebar);
        assert_eq!(h, 78, "three bands and two 2 px gaps");
        assert_eq!((cm.rebar.top, cm.rebar.closing), (0, 2));
        let mut c = Canvas::new(100, (h + 2) as u32);
        c.fill_rect(c.bounds(), Color::rgb(1, 2, 3));
        rebar::draw(&mut c, &theme.paint(), &rb, Rect::new(0, 0, 100, h));
        for y in 0..h {
            assert_eq!(c.get(50, y), BAND, "y {y}");
        }
        assert_eq!(c.get(0, h), 0xFF6B6C6C, "closing line");
        assert_eq!(c.get(99, h), 0xFF6B6C6C, "closing line");
        assert_eq!(c.get(50, h + 1), 0xFF010203, "nothing past it");
    }

    #[test]
    fn the_gripper_is_a_column_of_dots() {
        let theme = glass();
        let st = theme.look.style().unwrap();
        let mut c = Canvas::new(3, 22);
        c.fill_rect(c.bounds(), Color::rgb(0xF6, 0xF7, 0xF7));
        st.draw(
            &mut c,
            Rect::new(0, 0, 3, 22),
            Part::RebarGripper,
            State::Normal,
            1,
        );
        let want = [
            [0xFDFCFD, 0xFDFCFD, 0xF6F7F7],
            [0xFDFCFD, 0xE1E0E1, 0xBBBABB],
            [0xF6F7F7, 0xBBBABB, 0xBBBABB],
            [0xF6F7F7, 0xF6F7F7, 0xF6F7F7],
        ];
        for y in 0..22 {
            for x in 0..3 {
                pin(&c, x, y, want[y as usize % 4][x as usize], "gripper");
            }
        }
    }

    #[test]
    fn a_banded_menu_bar_leaves_the_rebar_behind_it() {
        let theme = glass();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut bar = menubar::Bar::new(vec![("&File", vec![]), ("&Edit", vec![])]);
        bar.banded = true;
        let lay = bar.layout(&text, px, Rect::new(0, 0, 300, 22), &theme.look.controls());
        let mut c = Canvas::new(300, 22);
        c.fill_rect(c.bounds(), Color::rgb(0xF6, 0xF7, 0xF7));
        bar.draw(&mut c, &text, &theme, &lay);
        assert_eq!(c.get(250, 10), BAND, "past the titles");
        assert_eq!(c.get(1, 1), BAND, "before the first title");
    }

    #[test]
    fn dark_glass_draws_a_dark_band_and_light_outlines() {
        let theme = dark_glass();
        let band = dark::remap_color(Color::rgb(0xF6, 0xF7, 0xF7));
        let mut c = Canvas::new(40, 30);
        rebar::draw(
            &mut c,
            &theme.paint(),
            &Rebar {
                bands: vec![Band { height: 29 }],
            },
            Rect::new(0, 0, 40, 29),
        );
        assert_eq!(c.get(20, 10), band.argb(), "dark band");
        let lum = |px: u32| (px >> 16 & 0xFF) + (px >> 8 & 0xFF) + (px & 0xFF);
        for st in [State::Hot, State::Pressed] {
            let mut b = Canvas::new(72, 30);
            b.fill_rect(b.bounds(), band);
            draw_button(&mut b, &theme.paint(), Rect::new(0, 0, 72, 30), None, st);
            assert!(lum(b.get(0, 15)) > lum(b.get(36, 15)), "{st:?}");
            assert!(lum(b.get(36, 15)) < lum(0xFFBDBEBE), "{st:?} dark face");
        }
    }

    #[test]
    fn rebar_parts_double_at_scale_2() {
        for theme in [glass(), dark_glass()] {
            let st = theme.look.style().unwrap();
            for (part, w, h) in [
                (Part::ToolbarSeparator, 8, 22),
                (Part::Rebar, 60, 40),
                (Part::RebarBand, 60, 2),
                (Part::RebarGripper, 3, 26),
            ] {
                part_doubles_at_scale_2(st, part, w, h).unwrap();
            }
        }
        assert!(doubles_at_paint_scale_2(Interface::Glass, 6, 30, |c, p| {
            draw_separator(c, p, true, Rect::new(0, 0, 6, 30).scaled(p.s))
        }));
    }

    fn smoked_glass() -> Theme {
        theme_with(
            Interface::Glass,
            crate::blue::Variant::default(),
            &Settings {
                material: Material::Smoked,
                ..Settings::default()
            },
        )
    }

    fn part_at(theme: &Theme, part: Part, d: i32) -> PartImage {
        let st = theme.look.at_scale(d);
        st.style().unwrap().part(part).unwrap().clone()
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    #[test]
    fn toolbar_buttons_pin_at_scale_1_and_2() {
        let got: Vec<u64> = [glass(), dark_glass(), smoked_glass()]
            .iter()
            .flat_map(|t| {
                [Part::ToolbarButton, Part::ToolbarDrop]
                    .into_iter()
                    .flat_map(move |p| [1, 2].map(|d| fnv(&part_at(t, p, d).atlas)))
            })
            .collect();
        let want: [u64; 12] = [
            0x3486_84ab_6d7a_3716,
            0x782e_504f_132d_9de1,
            0x6896_4cdf_3d9b_c8b0,
            0x2e26_23e1_437d_f4ee,
            0x1361_da40_01a0_879b,
            0xe7f5_56db_16f7_ed84,
            0xa823_8cce_5bfa_3d66,
            0x442e_6ad3_1376_53d4,
            0x3486_84ab_6d7a_3716,
            0x782e_504f_132d_9de1,
            0x6896_4cdf_3d9b_c8b0,
            0x2e26_23e1_437d_f4ee,
        ];
        assert_eq!(got, want, "{got:#018x?}");
    }

    #[test]
    fn toolbar_buttons_redraw_at_scale_2() {
        for (theme, dark) in [(glass(), false), (dark_glass(), true)] {
            for part in [Part::ToolbarButton, Part::ToolbarDrop] {
                let one = part_at(&theme, part, 1);
                let two = part_at(&theme, part, 2);
                assert_eq!(two.atlas.width, one.atlas.width * 2, "{part:?}");
                assert_eq!(two.atlas.height, one.atlas.height * 2, "{part:?}");
                assert_eq!(one.sizing, two.sizing, "{part:?} design insets");
                assert_eq!(one.outset, two.outset, "{part:?} design outset");
                let fh = (H * 2) as u32;
                let w = two.atlas.width;
                let frame = |i: u32| -> Vec<u32> {
                    (i * fh..(i + 1) * fh)
                        .flat_map(|y| (0..w).map(move |x| (x, y)))
                        .map(|(x, y)| two.atlas.get(x as i32, y as i32))
                        .collect()
                };
                let [rest, hot, held, grey, checked, checked_hot] = [0, 1, 2, 3, 4, 5].map(frame);
                assert!(rest.iter().chain(&grey).all(|&p| p == 0), "{part:?} rest");
                assert_ne!(hot, held, "{part:?} lit and held");
                assert_eq!(held, checked, "{part:?} latched");
                assert_eq!(held, checked_hot, "{part:?} latched lit");
                for (i, outline) in [(1, 0xAA), (2, 0x54)] {
                    let top = i * fh;
                    let x0 = w - 6;
                    let a = |x: u32, y: u32| two.atlas.get(x as i32, y as i32) >> 24;
                    assert_eq!(a(w - 1, top), 0, "{part:?} {i} corner clear");
                    let partial = (0..6)
                        .flat_map(|dy| (x0..w).map(move |x| (x, top + dy)))
                        .filter(|&(x, y)| (1..255).contains(&a(x, y)))
                        .count();
                    assert!(partial >= 3, "{part:?} {i} partial alpha on the curve");
                    if !dark {
                        for (x, y) in (0..6).flat_map(|dy| (x0..w).map(move |x| (x, top + dy))) {
                            let px = two.atlas.get(x as i32, y as i32);
                            let v = (px & 0xFF) + (255 - (px >> 24)) * 0xF7 / 255;
                            assert!(v + 1 >= outline, "{part:?} {i} fringe at {x},{y}: {v:x}");
                        }
                    }
                }
            }
        }
    }
}
