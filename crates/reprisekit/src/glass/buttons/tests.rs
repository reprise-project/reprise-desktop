use super::super::twice::twice_over;
use super::*;
use crate::frame::{Chrome, FrameGeometry, FrameState, Glow};
use crate::glass::panel::{self, DENSE_CEILING};
use crate::glass::{Material, tint};
use crate::raster::Rect;
use crate::style::Part;
use crate::text::TextRenderer;

fn parts(s: &Settings) -> [(&'static str, PartImage); 4] {
    parts_at(s, 1)
}

fn parts_at(s: &Settings, n: i32) -> [(&'static str, PartImage); 4] {
    [
        ("group", caption_group(s, n)),
        ("maximized group", max_caption_group(s, n)),
        ("dialog close", dialog_close(s, n)),
        ("help strip", help_strip(s, n)),
    ]
}

fn fnv(img: &PartImage) -> u64 {
    img.atlas
        .pixels
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
}

fn at(img: &PartImage, state: State, n: usize, x: i32, y: i32) -> u32 {
    let f = img.frame_nth(state, n);
    img.atlas.get(f.x + x, f.y + y)
}

fn over_grey(px: u32) -> [i32; 3] {
    let a = (px >> 24) as i32;
    [16, 8, 0].map(|sh| ((px >> sh) & 0xFF) as i32 + ((255 - a) * 128 + 127) / 255)
}

fn grey_of((a, p): Px) -> [f64; 3] {
    p.map(|v| v + (255.0 - a) * 128.0 / 255.0)
}

#[test]
fn every_part_has_its_size_and_frames() {
    let s = Settings::default();
    let lit = [State::Hot, State::Pressed, State::InactiveHot];
    let mut group = vec![State::Normal, State::Inactive];
    let mut help = group.clone();
    for st in lit {
        group.extend([st; 3]);
        help.extend([st; 2]);
    }
    let [(_, g), (_, m), (_, d), (_, h)] = parts(&s);
    assert_eq!(
        (g.atlas.width, g.atlas.height, &g.states),
        (105, 11 * 20, &group)
    );
    assert_eq!(
        (m.atlas.width, m.atlas.height, &m.states),
        (105, 11 * 19, &group)
    );
    let close = vec![
        State::Normal,
        State::Inactive,
        State::Hot,
        State::Pressed,
        State::InactiveHot,
    ];
    assert_eq!(
        (d.atlas.width, d.atlas.height, &d.states),
        (49, 5 * 20, &close)
    );
    assert_eq!(
        (h.atlas.width, h.atlas.height, &h.states),
        (78, 8 * 20, &help)
    );
}

#[test]
fn sky_keeps_its_strips() {
    let want = [
        0x5B0D_EBB1_B8EE_73C9_u64,
        0x9198_41FE_7005_B819,
        0x77BA_F165_400B_3AB4,
        0xDCAD_E5D2_11B7_3583,
    ];
    let s = Settings::default();
    let p = parts(&s);
    let got: Vec<u64> = p.iter().map(|(_, img)| fnv(img)).collect();
    assert_eq!(got, want, "the atlases: {got:#018X?}");
    let g = &p[0].1;
    for (st, n, x, y, want, what) in [
        (State::Normal, 0, 0, 10, 0xD0C0_C5C9, "the halo"),
        (State::Normal, 0, 1, 10, 0xD925_2A2F, "the outline"),
        (State::Normal, 0, 5, 2, 0xC9B0_B9C0, "minimize's upper band"),
        (State::Normal, 0, 5, 12, 0x9B50_5D6B, "its lower band"),
        (State::Normal, 0, 65, 14, 0xE4AE_513E, "close's red"),
        (State::Inactive, 0, 5, 10, 0x5843_4B53, "an inactive body"),
        (State::Hot, 0, 15, 17, 0xFF10_FFFB, "minimize's spot"),
        (State::Normal, 0, 15, 12, 0xFFDA_DADA, "minimize's glyph"),
        (State::Normal, 0, 10, 10, 0xFF53_5666, "and its outline"),
    ] {
        assert_eq!(at(g, st, n, x, y), want, "{st:?} {n} ({x}, {y}): {what}");
    }
    let h = &p[3].1;
    let top = at(h, State::Normal, 0, 14, 4);
    assert!(
        top >> 24 == 0xFF && top & 0xFF >= 0xB0,
        "the ?'s light top on row 4: {top:08X}"
    );
    let edge = at(h, State::Normal, 0, 14, 3);
    assert!(edge & 0xFF < 0x90, "its dark outline on row 3: {edge:08X}");
}

const WITHIN: i32 = 12;

#[track_caller]
fn near(got: u32, want: [i32; 3], within: i32, what: &str) {
    let got = over_grey(got);
    let off = (0..3).map(|i| (got[i] - want[i]).abs()).max().unwrap();
    assert!(off <= within, "{what}: {got:?} is {want:?} within {within}");
}

#[test]
fn each_group_state_has_its_key_pixel() {
    let s = Settings::default();
    let g = caption_group(&s, 1);
    let group = |st, n, x, y| at(&g, st, n, x, y);
    near(
        group(State::Normal, 0, 65, 14),
        [185, 93, 75],
        WITHIN,
        "close at rest is red",
    );
    near(
        group(State::Inactive, 0, 65, 14),
        [158, 164, 170],
        WITHIN,
        "an inactive window's close is grey glass",
    );
    near(
        group(State::Hot, 2, 65, 14),
        [201, 78, 27],
        WITHIN,
        "close lit, brighter",
    );
    near(
        group(State::Pressed, 2, 65, 14),
        [108, 48, 9],
        WITHIN,
        "held, close darkens",
    );
    near(
        group(State::InactiveHot, 2, 65, 14),
        [201, 78, 27],
        WITHIN,
        "an inactive window's close lights red too",
    );
    near(
        group(State::Normal, 0, 15, 17),
        [199, 206, 215],
        WITHIN,
        "minimize's foot at rest",
    );
    near(
        group(State::Hot, 0, 15, 17),
        [11, 254, 251],
        WITHIN,
        "minimize's cyan spot",
    );
    near(
        group(State::Hot, 1, 43, 17),
        [10, 254, 251],
        WITHIN,
        "maximize's",
    );
    near(
        group(State::Hot, 2, 78, 17),
        [245, 232, 119],
        WITHIN,
        "close lit, its foot yellow",
    );
    near(
        group(State::Hot, 0, 5, 5),
        [154, 196, 224],
        WITHIN,
        "minimize lit, pale blue",
    );
    near(
        group(State::Pressed, 0, 5, 5),
        [116, 154, 179],
        WITHIN,
        "held, darker",
    );
    let m = max_caption_group(&s, 1);
    near(
        at(&m, State::Normal, 0, 65, 13),
        [183, 92, 73],
        WITHIN,
        "maximized, close's red",
    );
    near(
        at(&m, State::Hot, 1, 43, 15),
        [47, 216, 229],
        WITHIN,
        "restore glows cyan",
    );
    assert_eq!(
        at(&m, State::Normal, 0, 40, 8) >> 24,
        at(&m, State::Normal, 0, 33, 8) >> 24,
        "restore's front pane is the body's glass"
    );
    assert_eq!(
        at(&m, State::Normal, 0, 37, 6),
        0xFFFB_FBFB,
        "restore's white frame"
    );
    assert_eq!(
        at(&m, State::Normal, 0, 40, 3),
        0xFFFF_FFFF,
        "restore's top"
    );
    assert_eq!(at(&m, State::Normal, 0, 40, 2), 0xFF53_5666, "its outline");
    for (st, n, want, what) in [
        (State::Normal, 0, 0x9745_5362, "at rest"),
        (State::Hot, 1, 0xF529_85B9, "lit"),
        (State::Pressed, 1, 0xFD15_6D82, "held"),
    ] {
        assert_eq!(at(&m, st, n, 40, 9), want, "restore's hollow {what}");
    }
    let d = dialog_close(&s, 1);
    near(
        at(&d, State::Normal, 0, 0, 10),
        [221, 226, 229],
        WITHIN,
        "the dialog close's own halo",
    );
    let [r, g, b] = over_grey(at(&d, State::Normal, 0, 1, 10));
    assert!(
        r <= 90 && r > g + 30 && r > b + 30,
        "and its outline, dark red over the close box's red: ({r}, {g}, {b})"
    );
    near(
        at(&d, State::Normal, 0, 10, 14),
        [184, 91, 72],
        WITHIN,
        "its red body",
    );
    let h = help_strip(&s, 1);
    near(
        at(&h, State::Normal, 0, 6, 14),
        [138, 148, 163],
        WITHIN,
        "a grey glass ? box",
    );
    near(
        at(&h, State::Hot, 0, 6, 14),
        [44, 137, 185],
        WITHIN,
        "the ? blue under the pointer",
    );
    near(
        at(&h, State::Pressed, 0, 6, 14),
        [19, 90, 121],
        WITHIN,
        "and darker held",
    );
    let [r, g, b] = over_grey(at(&h, State::Normal, 0, 29, 10));
    assert!(
        r > g + 50 && r > b + 50,
        "the divider beside close lies over its red: ({r}, {g}, {b})"
    );
}

#[test]
fn a_lit_cells_glow_bleeds_into_its_neighbours() {
    let g = caption_group(&Settings::default(), 1);
    let moved = |st, n, x, y| {
        let (a, b) = (
            over_grey(at(&g, State::Normal, 0, x, y)),
            over_grey(at(&g, st, n, x, y)),
        );
        (0..3).map(|i| (a[i] - b[i]).abs()).max().unwrap()
    };
    let near_divider = moved(State::Hot, 0, 31, 5);
    let further = moved(State::Hot, 0, 35, 5);
    assert!(near_divider > 40, "2 px past the divider: {near_divider}");
    assert!(
        further > 10 && further < near_divider,
        "6 px past it, less: {further}"
    );
    assert!(
        moved(State::Hot, 0, 31, 0) > 20,
        "up through the light line's row"
    );
    assert_eq!(
        moved(State::Hot, 0, 65, 10),
        0,
        "close, 37 px on, keeps its look"
    );
    let [r, _, b] = over_grey(at(&g, State::Hot, 2, 54, 5));
    let [r0, _, b0] = over_grey(at(&g, State::Normal, 0, 54, 5));
    assert!(
        r - b > r0 - b0 + 30,
        "maximize's edge warms beside lit close"
    );
    assert!(
        moved(State::Pressed, 1, 27, 5) > 30,
        "maximize held glows into minimize"
    );
    assert_eq!(moved(State::Hot, 0, 41, 15), 0, "13 px past minimize");
    assert_eq!(moved(State::Hot, 2, 44, 15), 0, "13 px short of close");
}

#[test]
fn a_lit_end_cell_keeps_the_strips_dark_outline_and_round_corner() {
    let g = caption_group(&Settings::default(), 1);
    let d = dialog_close(&Settings::default(), 1);
    let h = help_strip(&Settings::default(), 1);
    let lum = |px: u32| over_grey(px).iter().sum::<i32>();
    for (img, st, n, x, corner, what) in [
        (&g, State::Hot, 0, 1, 2, "minimize lit"),
        (&g, State::Pressed, 0, 1, 2, "minimize held"),
        (
            &g,
            State::InactiveHot,
            0,
            1,
            2,
            "minimize lit on another window",
        ),
        (&g, State::Hot, 2, 103, 102, "close lit"),
        (&g, State::Pressed, 2, 103, 102, "close held"),
        (&d, State::Hot, 0, 1, 2, "the dialog close lit, left end"),
        (&d, State::Hot, 0, 47, 46, "and right end"),
        (&h, State::Hot, 0, 1, 2, "the ? lit"),
        (&h, State::Hot, 1, 76, 75, "the ? strip's close lit"),
    ] {
        let px = over_grey(at(img, st, n, x, 10));
        assert!(
            px.iter().all(|&v| v <= 90),
            "{what}: the outline at ({x}, 10) is {px:?}, no channel above 90"
        );
        let (o, l) = (
            lum(at(img, st, n, corner, 16)),
            lum(at(img, st, n, corner, 18)),
        );
        assert!(
            l - o > 150,
            "{what}: the corner's outline ({corner}, 16) at {o} is darker than its halo at {l}"
        );
    }
}

fn settings() -> Vec<Settings> {
    let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
    [SWATCHES[0].color, ruby]
        .into_iter()
        .flat_map(|color| {
            Material::ALL.into_iter().flat_map(move |material| {
                [42, 75, 100].map(|intensity| Settings {
                    color,
                    material,
                    intensity,
                    ..Settings::default()
                })
            })
        })
        .collect()
}

#[test]
fn the_bodies_follow_the_glass_and_closes_red_lets_it_through() {
    let sky = Settings::default();
    let from = caption_group(&sky, 1);
    let mut capped = 0;
    for s in settings() {
        let to = caption_group(&s, 1);
        for (surface, state) in [
            (Surface::Active, State::Normal),
            (Surface::Inactive, State::Inactive),
        ] {
            let (a_sky, p_sky) = tint::plain(&sky, surface);
            let (a_s, p_s) = tint::plain(&s, surface);
            let moved: [f64; 3] =
                std::array::from_fn(|i| grey_of((a_s, p_s))[i] - grey_of((a_sky, p_sky))[i]);
            let what = format!(
                "{state:?}, {:?} {:?} at {}",
                s.color, s.material, s.intensity
            );
            for (x, y) in [(5, 2), (5, 12), (33, 14)] {
                let (a0, c0) = tint::split(at(&from, state, 0, x, y));
                let (a1, c1) = tint::split(at(&to, state, 0, x, y));
                let added = a0 - a_sky;
                assert!(
                    a_sky + added < DENSE_CEILING,
                    "Sky is not capped at ({x}, {y})"
                );
                if a_s + added > DENSE_CEILING {
                    capped += 1;
                    let want = a_s.max(DENSE_CEILING).round();
                    assert!(
                        (a1 - want).abs() <= 1.0,
                        "{what} ({x}, {y}): alpha {a1} stops at {want}"
                    );
                    panel::sheen_share(
                        &format!("{what} ({x}, {y})"),
                        std::array::from_fn(|i| c0[i] - p_sky[i]),
                        std::array::from_fn(|i| c1[i] - p_s[i]),
                    );
                    continue;
                }
                let (g0, g1) = (grey_of((a0, c0)), grey_of((a1, c1)));
                for i in 0..3 {
                    assert!(
                        (g1[i] - g0[i] - moved[i]).abs() <= 1.5,
                        "{what} ({x}, {y}) channel {i}: moves {} as the plain glass's {}",
                        g1[i] - g0[i],
                        moved[i]
                    );
                }
            }
            if surface == Surface::Inactive {
                continue;
            }
            let red = banded(&CLOSE_RED, 12, 12.0);
            let p = Paint::new(&s, surface);
            let body = |p: &Paint| {
                grey_of(p.glass.at(
                    banded(&SHEEN[0].map(level), 12, 12.0).0,
                    banded(&DEEPEN[0].map(level), 12, 12.0).0,
                ))
            };
            let (b0, b1) = (body(&Paint::new(&sky, surface)), body(&p));
            let (g0, g1) = (
                over_grey(at(&from, state, 0, 65, 12)),
                over_grey(at(&to, state, 0, 65, 12)),
            );
            let keep = 1.0 - red.0 / 255.0;
            for i in 0..3 {
                let want = (b1[i] - b0[i]) * keep;
                let got = f64::from(g1[i] - g0[i]);
                assert!(
                    (got - want).abs() <= 1.5,
                    "{what}: close channel {i} moves {got}, {keep:.2} of the body's {}",
                    b1[i] - b0[i]
                );
            }
        }
    }
    assert!(capped > 0, "intensity 100 takes some bands to the ceiling");
}

#[test]
fn no_setting_clips_a_bodys_colour() {
    let mut clipped = Vec::new();
    for s in panel::every_setting() {
        for surface in [Surface::Active, Surface::Inactive] {
            let p = Paint::new(&s, surface);
            let worst = (0..ROWS)
                .flat_map(|r| {
                    [Glyph::Minimize, Glyph::Close]
                        .into_iter()
                        .flat_map(move |g| [false, true].map(|lit| (g, r, lit)))
                })
                .map(|(g, r, lit)| panel::clip(p.body(g, r, f64::from(r), lit)))
                .fold(0.0, f64::max);
            if worst > 1e-9 {
                clipped.push(format!(
                    "{surface:?}, {:?} {:?} {}: {worst:.1}",
                    s.color, s.material, s.intensity
                ));
            }
        }
    }
    assert!(clipped.is_empty(), "{clipped:#?}");
}

#[test]
fn smoked_reads_darker_molten_lighter_and_close_stays_red() {
    let lum = |px: u32| over_grey(px).iter().sum::<i32>();
    let of = |material| {
        caption_group(
            &Settings {
                material,
                ..Settings::default()
            },
            1,
        )
    };
    let (f, sm, mo) = (
        of(Material::Frosted),
        of(Material::Smoked),
        of(Material::Molten),
    );
    for (x, y) in [(5, 2), (5, 12), (65, 4), (65, 12)] {
        let (a, b, c) = (
            lum(at(&f, State::Normal, 0, x, y)),
            lum(at(&sm, State::Normal, 0, x, y)),
            lum(at(&mo, State::Normal, 0, x, y)),
        );
        assert!(
            b < a && a < c,
            "({x}, {y}): Smoked {b} < Frosted {a} < Molten {c}"
        );
    }
    for img in [&f, &sm, &mo] {
        let [r, g, b] = over_grey(at(img, State::Normal, 0, 65, 12));
        assert!(r > g + 60 && r > b + 60, "close stays red: ({r}, {g}, {b})");
    }
}

#[test]
fn a_glow_takes_the_glass_colours_hue_and_close_keeps_its_own() {
    let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap();
    let red = Settings {
        color: ruby.color,
        intensity: ruby.intensity,
        ..Settings::default()
    };
    let frost = SWATCHES.iter().find(|w| w.name == "frost").unwrap();
    let white = Settings {
        color: frost.color,
        intensity: frost.intensity,
        ..Settings::default()
    };
    let spot = |s: &Settings| over_grey(at(&caption_group(s, 1), State::Hot, 0, 15, 16));
    let [r, g, b] = spot(&Settings::default());
    assert!(
        g > r + 100 && b > r + 100,
        "Sky's spot is cyan: ({r}, {g}, {b})"
    );
    let [r, g, b] = spot(&red);
    assert!(r > g + 100 && r > b + 60, "Ruby's is red: ({r}, {g}, {b})");
    let [r, g, b] = spot(&white);
    assert!(
        r.abs_diff(g) < 12 && g.abs_diff(b) < 12,
        "Frost's is white: ({r}, {g}, {b})"
    );
    let close = |s: &Settings| at(&caption_group(s, 1), State::Hot, 2, 80, 17);

    let [r, g, b] = over_grey(close(&red));
    assert!(
        r > b + 100 && g > b,
        "close's spot stays orange on Ruby: ({r}, {g}, {b})"
    );
}

#[test]
fn with_transparency_off_every_part_is_opaque() {
    for s in settings() {
        let s = Settings {
            transparency: false,
            ..s
        };
        for n in [1, 2] {
            for (name, img) in parts_at(&s, n) {
                assert!(
                    img.atlas.pixels.iter().all(|&px| px >> 24 == 0xFF),
                    "{name} at {n}, {:?} {:?} at {}",
                    s.color,
                    s.material,
                    s.intensity
                );
            }
        }
    }
}

const PARTS: [Part; 4] = [
    Part::CaptionGroup,
    Part::MaxCaptionGroup,
    Part::CloseButton,
    Part::HelpButton,
];

#[test]
fn the_style_at_scale_2_paints_the_caption_buttons_at_2() {
    for frame in crate::glass::Frame::ALL {
        let style = crate::glass::style(&Settings {
            frame,
            ..Settings::default()
        });
        let two = style.at_scale(2);
        for part in PARTS {
            assert_eq!(two.part(part).unwrap().scale, 2, "{frame:?} {part:?}");
            assert_eq!(style.part(part).unwrap().scale, 1, "{frame:?} {part:?}");
        }
    }
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass).scaled(2);
    let scaled = theme.look.style().unwrap();
    assert_eq!(
        scaled.part(Part::CaptionGroup).unwrap().scale,
        2,
        "a scaled theme's"
    );
}

const CORNER2: i32 = 13;

type StripFrames = (
    &'static str,
    &'static Strip,
    bool,
    &'static [(State, Option<usize>)],
);
const STRIPS: [StripFrames; 4] = [
    ("group", &GROUP, false, &GROUP_FRAMES),
    ("maximized group", &MAX_GROUP, true, &GROUP_FRAMES),
    ("dialog close", &DIALOG_CLOSE, false, &CLOSE_FRAMES),
    ("help strip", &HELP, false, &HELP_FRAMES),
];

#[test]
fn a_strip_at_scale_2_is_its_scale_1_design_twice_over() {
    let s = Settings::default();
    for (name, layout, maximized, list) in STRIPS {
        let w = layout.width;
        let upright = |x: i32| {
            [0, 1, w - 2, w - 1].contains(&x)
                || layout.dividers.contains(&x)
                || layout.cells.iter().any(|&(x0, x1, _)| x == x0 || x == x1)
        };
        let skip = i32::from(maximized);
        let kind = |x: i32, y: i32| {
            let across = if upright(x) { x + 1 } else { 0 };
            let down = match y + skip {
                r @ (0 | 17..) => r,
                1..=8 => 1,
                _ => 9,
            };
            (across, down)
        };
        let bare = |n: i32| {
            let mut shape = Shape::of(layout, 0, n);
            shape.glyphs.fill([0.0; 3]);
            frames_of_shape(&s, list, maximized, &shape)
        };
        let (bare1, bare2) = (bare(1), bare(2));
        let (_, full2) = (
            frames(&s, layout, list, maximized, (0, 1)),
            frames(&s, layout, list, maximized, (0, 2)),
        );
        let shape = Shape::of(layout, 0, 2);
        let h = ROWS - skip;
        for (i, &(st, lit)) in list.iter().enumerate() {
            let (top1, top2) = (i as i32 * h, 2 * i as i32 * h);
            for y in 0..2 * h {
                for x in 0..2 * w {
                    let what = format!("{name} {st:?} {lit:?} ({x}, {y})");
                    let corner = (x < CORNER2 || x >= 2 * w - CORNER2) && y >= 2 * h - CORNER2;
                    let spot = lit.is_some_and(|c| {
                        let (x0, x1, _) = layout.cells[c];
                        (x0..=x1).contains(&(x / 2)) && y / 2 + skip >= 13
                    });
                    if !corner && !spot && !(lit.is_some() && upright(x / 2)) {
                        twice_over(
                            (&bare1.atlas, top1),
                            (&bare2.atlas, top2),
                            (w, h),
                            (x, y),
                            (false, &kind),
                            &what,
                        );
                    }
                    let [ink, grown, _] = shape.glyph(x, y + 2 * skip);
                    let got = full2.atlas.get(x, top2 + y);
                    let under = bare2.atlas.get(x, top2 + y);
                    if ink == 1.0 {
                        let (r, g, b) = ((got >> 16) & 0xFF, (got >> 8) & 0xFF, got & 0xFF);
                        assert!(r == g && g == b && r >= 0xD0, "{what}: white, {got:08X}");
                    } else if grown == 0.0 {
                        assert_eq!(got, under, "{what}: clear of the glyphs");
                    }
                }
            }
        }
    }
}

#[test]
fn the_spot_at_scale_2_shades_every_row() {
    let g = caption_group(&Settings::default(), 2);
    for (st, n) in [State::Hot, State::Pressed]
        .into_iter()
        .flat_map(|st| (0..3).map(move |n| (st, n)))
    {
        let (x0, x1, _) = GROUP.cells[n];
        let x = x0 + x1;
        let column: Vec<u32> = (28..34).map(|y| at(&g, st, n, x, y)).collect();
        for (i, pair) in column.windows(2).enumerate() {
            assert_ne!(
                pair[0],
                pair[1],
                "{st:?} {n}: device rows {} and {} at x {x}",
                28 + i,
                29 + i
            );
        }
    }
}

#[test]
fn a_strips_corners_at_scale_2_are_round_and_antialiased() {
    let s = Settings::default();
    let r = RADIUS * 2.0;
    for (name, img) in parts_at(&s, 2) {
        let fh = img.frame(State::Normal).height;
        let w = img.atlas.width as i32;
        let maximized = name == "maximized group";
        for (k, surface) in [Surface::Active, Surface::Inactive].into_iter().enumerate() {
            let p = Paint::new(&s, surface);
            let state = [State::Normal, State::Inactive][k];
            let skip = i32::from(maximized);
            let band = |x: i32, y: i32| {
                let (cx, cy) = (centre(x, 2), centre(y + 2 * skip, 2));
                let at = p.under.at(
                    f64::from(RIGHT) + f64::from(w / 2 - 1) - cx,
                    cy + 1.0 - f64::from(skip),
                    maximized,
                );
                p.finish.finish(pack(at))
            };
            let mut partial = std::collections::BTreeSet::new();
            for y in fh - CORNER2..fh {
                for x in 0..CORNER2 {
                    let px = at(&img, state, 0, x, y);
                    let foot = f64::from(fh);
                    let (dx, dy) = (r - (f64::from(x) + 0.5), f64::from(y) + 0.5 - (foot - r));
                    let what = format!("{name} {state:?} ({x}, {y}): {px:08X}");
                    let on_band = px == band(x, y);
                    if dx <= 0.0 || dy <= 0.0 {
                        continue;
                    }
                    let dist = dx.hypot(dy);
                    if dist >= r + 0.71 {
                        assert!(on_band, "{what}: the band outside the curve");
                    } else if dist <= r - 0.71 {
                        assert!(!on_band, "{what}: the halo inside the curve");
                    } else {
                        partial.insert(px);
                    }
                    let mirror = w - 1 - x;
                    assert_eq!(
                        at(&img, state, 0, mirror, y) == band(mirror, y),
                        on_band,
                        "{what}: both corners cut alike"
                    );
                }
            }
            assert!(
                partial.len() >= 8,
                "{name} {state:?}: the curve's edge in many levels, not a step: {}",
                partial.len()
            );
        }
    }
}

#[test]
fn the_strips_at_scale_2_keep_their_pixels_at_sky() {
    for (frame, want) in [
        (
            crate::glass::Frame::Rounded,
            [
                0x03FA_4D81_FD95_CB22_u64,
                0x0F64_8F8D_EAF3_34A4,
                0xD30D_F78D_D893_E09D,
                0xCB73_D236_06B4_771C,
            ],
        ),
        (
            crate::glass::Frame::Square,
            [
                0x3CB9_4E7C_833C_9E95,
                0x09A4_701B_EFAE_49DD,
                0xCEF0_604D_008A_9CC5,
                0x3B1E_08D2_F681_B798,
            ],
        ),
    ] {
        let style = crate::glass::style(&Settings {
            frame,
            ..Settings::default()
        })
        .at_scale(2);
        let got = PARTS.map(|part| fnv(style.part(part).unwrap()));
        assert_eq!(
            got, want,
            "{frame:?}: group, maximized group, close, ?, {got:#018X?}"
        );
    }
}

#[test]
fn a_glass_dialog_draws_the_help_strip_from_its_help_box_to_its_close_box() {
    use crate::pointer::Interaction::{self, Hot, Idle, Pressed};
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let style = theme.look.style().unwrap();
    let text = TextRenderer::load(&theme.fonts);
    let g = FrameGeometry::layout(&theme.metrics, 300, 60, false, Chrome::DIALOG);
    let help = g.help_box();
    let strip = Rect::new(help.x, help.y, 78, 20);
    assert_eq!(
        strip.right(),
        g.buttons[2].right(),
        "the strip ends with close"
    );
    let draw = |active: bool, ptr: Interaction, hot: bool, pressed: bool| {
        let close = crate::frame::CaptionButton::Close;
        let st = FrameState {
            title: "Run",
            active,
            maximized: false,
            pressed: pressed.then_some(close),
            glow: Glow::lit(hot.then_some(close)),
            icon: None,
            tabbed: false,
        };
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        crate::look::draw_dialog_frame(&mut c, &theme, &text, &g, &st, Some(ptr));
        c
    };
    let frame = |state: State, n: usize| {
        let mut c = Canvas::new(78, 20);
        style.draw_nth(
            &mut c,
            Rect::new(0, 0, 78, 20),
            Part::HelpButton,
            state,
            n,
            1,
        );
        c
    };
    for (active, ptr, hot, pressed, state, n) in [
        (true, Idle, false, false, State::Normal, 0),
        (false, Idle, false, false, State::Inactive, 0),
        (true, Hot, false, false, State::Hot, 0),
        (true, Idle, true, false, State::Hot, 1),
        (true, Pressed, false, false, State::Pressed, 0),
        (true, Idle, false, true, State::Pressed, 1),
        (false, Hot, false, false, State::InactiveHot, 0),
        (false, Idle, true, false, State::InactiveHot, 1),
    ] {
        let c = draw(active, ptr, hot, pressed);
        let want = frame(state, n);
        for y in 0..20 {
            for x in 0..78 {
                assert_eq!(
                    c.get(strip.x + x, strip.y + y),
                    want.get(x, y),
                    "{state:?} {n}: ({x}, {y}) of the strip"
                );
            }
        }
    }
}

#[test]
fn a_half_glowing_close_box_mixes_the_help_strip_half_way() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let style = theme.look.style().unwrap();
    let text = TextRenderer::load(&theme.fonts);
    let g = FrameGeometry::layout(&theme.metrics, 300, 60, false, Chrome::DIALOG);
    let help = g.help_box();
    let strip = Rect::new(help.x, help.y, 78, 20);
    let st = FrameState {
        title: "Run",
        active: true,
        maximized: false,
        pressed: None,
        glow: Glow([0.0, 0.0, 0.5]),
        icon: None,
        tabbed: false,
    };
    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    let idle = Some(crate::pointer::Interaction::Idle);
    crate::look::draw_dialog_frame(&mut c, &theme, &text, &g, &st, idle);
    let frame = |state: State, n: usize| {
        let mut c = Canvas::new(78, 20);
        let r = Rect::new(0, 0, 78, 20);
        style.draw_nth(&mut c, r, Part::HelpButton, state, n, 1);
        c
    };
    let mut want = frame(State::Normal, 0);
    want.mix(&frame(State::Hot, 1), want.bounds(), 0.5);
    for y in 0..20 {
        for x in 0..78 {
            assert_eq!(
                c.get(strip.x + x, strip.y + y),
                want.get(x, y),
                "({x}, {y}): the strip half way from rest to close lit"
            );
        }
    }
}

#[test]
fn a_glass_dialog_without_a_close_box_draws_the_help_cell_alone() {
    use crate::frame::ButtonSet;
    use crate::pointer::Interaction;
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let style = theme.look.style().unwrap();
    let text = TextRenderer::load(&theme.fonts);
    let chrome = Chrome {
        buttons: ButtonSet::None,
        ..Chrome::DIALOG
    };
    let g = FrameGeometry::layout(&theme.metrics, 300, 60, false, chrome);
    let with_close = FrameGeometry::layout(&theme.metrics, 300, 60, false, Chrome::DIALOG);
    let help = g.help_box();
    assert_eq!(help, with_close.help_box(), "the ? box stays where it was");
    let st = FrameState {
        title: "Run",
        active: true,
        maximized: false,
        pressed: None,
        glow: Glow::lit(None),
        icon: None,
        tabbed: false,
    };
    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    crate::look::draw_dialog_frame(&mut c, &theme, &text, &g, &st, Some(Interaction::Hot));
    let mut strip = Canvas::new(78, 20);
    style.draw(
        &mut strip,
        Rect::new(0, 0, 78, 20),
        Part::HelpButton,
        State::Hot,
        1,
    );
    let mut plain = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    crate::look::draw_dialog_frame(&mut plain, &theme, &text, &g, &st, None);
    for y in 0..20 {
        for x in 0..30 {
            assert_eq!(
                c.get(help.x + x, help.y + y),
                strip.get(x, y),
                "({x}, {y}) is the lit ? cell's"
            );
        }
        for x in help.right()..g.total.0 {
            assert_eq!(
                c.get(x, help.y + y),
                plain.get(x, help.y + y),
                "({x}, {y}): no close box right of it"
            );
        }
    }
}

#[test]
fn a_glass_dialog_at_scale_2_draws_the_help_strip_painted_at_2() {
    use crate::frame::ButtonSet;
    use crate::pointer::Interaction;
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let text = TextRenderer::load(&theme.fonts);
    let two = theme.scaled(2);
    let style = two.look.style().unwrap();
    assert_eq!(style.part(Part::HelpButton).unwrap().scale, 2);
    let g = FrameGeometry::layout(&theme.metrics, 300, 60, false, Chrome::DIALOG).scaled(2);
    let help = g.help_box();
    assert_eq!(
        help.x + 156,
        g.buttons[2].right(),
        "the strip runs 156 px to close's end"
    );
    let st = FrameState {
        title: "Run",
        active: true,
        maximized: false,
        pressed: None,
        glow: Glow::lit(None),
        icon: None,
        tabbed: false,
    };
    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    crate::look::draw_dialog_frame(&mut c, &two, &text, &g, &st, Some(Interaction::Hot));
    let mut want = Canvas::new(156, 40);
    style.draw(
        &mut want,
        Rect::new(0, 0, 156, 40),
        Part::HelpButton,
        State::Hot,
        2,
    );
    for y in 0..40 {
        for x in 0..156 {
            assert_eq!(
                c.get(help.x + x, help.y + y),
                want.get(x, y),
                "({x}, {y}) is the strip's pixel at 2"
            );
        }
    }
    let chrome = Chrome {
        buttons: ButtonSet::None,
        ..Chrome::DIALOG
    };
    let g = FrameGeometry::layout(&theme.metrics, 300, 60, false, chrome).scaled(2);
    let help = g.help_box();
    let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    crate::look::draw_dialog_frame(&mut c, &two, &text, &g, &st, Some(Interaction::Hot));
    for y in 0..40 {
        for x in 0..2 * super::super::CAPTION_GROUP.widths[0] {
            assert_eq!(
                c.get(help.x + x, help.y + y),
                want.get(x, y),
                "({x}, {y}) is the lit ? cell's at 2"
            );
        }
    }
}

/// A Glass style for `frame` at scale `n`, its buttons built for a strip grown by `grow` rows.
fn grown_style(
    frame: crate::glass::Frame,
    n: i32,
    grow: i32,
) -> std::sync::Arc<crate::style::Style> {
    let st = crate::glass::style(&Settings {
        frame,
        ..Settings::default()
    })
    .at_scale(n);
    let side = st.metrics.caption_group.unwrap().restored.height + grow;
    st.at_button_side(side).expect("a grown side builds")
}

const FRAMES: [crate::glass::Frame; 2] =
    [crate::glass::Frame::Rounded, crate::glass::Frame::Square];

/// Rows a strip grows by at 150% and 200% text size.
const GROWN: [i32; 2] = [7, 13];

const GROWN_PARTS: [Part; 5] = [
    Part::CaptionGroup,
    Part::MaxCaptionGroup,
    Part::CloseButton,
    Part::HelpButton,
    Part::MinCloseGroup,
];

#[test]
fn grown_strips_are_as_big_as_the_frame_lays_them_out() {
    for frame in FRAMES {
        let base = crate::glass::style(&Settings {
            frame,
            ..Settings::default()
        });
        let g = base.metrics.caption_group.unwrap();
        for grow in GROWN {
            let want = g.grown(grow);
            let [min, max, close] = want.widths;
            let (strip, _) = want.layout(400, false);
            for n in [1, 2] {
                let st = grown_style(frame, n, grow);
                let size = |part| {
                    let p = st.part(part).unwrap();
                    let f = p.frame(State::Normal);
                    (f.width / p.scale, f.height / p.scale)
                };
                let what = format!("{frame:?} +{grow} at {n}");
                let tall = want.restored.height;
                assert_eq!(size(Part::CaptionGroup), (strip.width, tall), "{what}");
                assert_eq!(
                    size(Part::MaxCaptionGroup),
                    (strip.width, want.maximized.height),
                    "{what}"
                );
                assert_eq!(size(Part::CloseButton), (close, tall), "{what}");
                let joined = min + close - want.divider;
                let help = if st.joined_help { joined } else { min };
                assert_eq!(size(Part::HelpButton), (help, tall), "{what}");
                assert_eq!(size(Part::MinCloseGroup), (joined, tall), "{what}");
                assert_eq!(strip.width, min + max + close - 2 * want.divider);
            }
        }
    }
}

#[test]
fn grown_strips_pin_at_scale_1_and_2() {
    let mut got = Vec::new();
    for frame in FRAMES {
        for grow in GROWN {
            for n in [1, 2] {
                let st = grown_style(frame, n, grow);
                for part in GROWN_PARTS {
                    let key = format!("{frame:?} {part:?} +{grow} {n}");
                    got.push((key, fnv(st.part(part).unwrap())));
                }
            }
        }
    }
    crate::pins::check("glass/caption_buttons_grown", &got);
}

#[test]
fn a_grown_strip_draws_each_state_apart() {
    for frame in FRAMES {
        for grow in GROWN {
            let st = grown_style(frame, 1, grow);
            let img = st.part(Part::CaptionGroup).unwrap();
            let frames: Vec<Vec<u32>> = GROUP_FRAMES
                .iter()
                .enumerate()
                .map(|(i, &(st, _))| {
                    let nth = GROUP_FRAMES[..i].iter().filter(|f| f.0 == st).count();
                    let f = img.frame_nth(st, nth);
                    (f.y..f.bottom())
                        .flat_map(|y| (f.x..f.right()).map(move |x| (x, y)))
                        .map(|(x, y)| img.atlas.get(x, y))
                        .collect()
                })
                .collect();
            for i in 0..frames.len() {
                for j in 0..i {
                    assert_ne!(
                        frames[i], frames[j],
                        "{frame:?} +{grow}: frames {j} and {i}"
                    );
                }
            }
        }
    }
}

/// The bounding box of `img`'s white glyph pixels in its Normal frame's columns `x0..x1`.
fn glyph_box(img: &PartImage, x0: i32, x1: i32) -> (i32, i32) {
    let f = img.frame(State::Normal);
    let white = |x: i32, y: i32| {
        let p = img.atlas.get(f.x + x, f.y + y);
        p >> 24 == 0xFF
            && (p & 0xFF) >= 0xD0
            && ((p >> 8) & 0xFF) >= 0xD0
            && ((p >> 16) & 0xFF) >= 0xD0
    };
    let pts: Vec<(i32, i32)> = (0..f.height)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| white(x, y))
        .collect();
    let span = |v: Vec<i32>| v.iter().max().unwrap() - v.iter().min().unwrap() + 1;
    (
        span(pts.iter().map(|p| p.0).collect()),
        span(pts.iter().map(|p| p.1).collect()),
    )
}

#[test]
fn a_grown_glyph_grows_with_its_cell() {
    let s = Settings::default();
    let base = caption_group(&s, 1);
    let mins = |img: &PartImage, laid: &Laid| glyph_box(img, laid.cells[0].0, laid.cells[0].1);
    assert_eq!(mins(&base, &Laid::of(&GROUP, 0)), (10, 3));
    for (grow, bar) in [(7, (14, 4)), (13, (18, 5))] {
        let img = frames(&s, &GROUP, &GROUP_FRAMES, false, (grow, 1));
        assert_eq!(mins(&img, &Laid::of(&GROUP, grow)), bar, "+{grow}");
    }
}

#[test]
fn a_grown_strip_is_drawn_afresh_at_scale_2() {
    let s = Settings::default();
    for grow in GROWN {
        let img = frames(&s, &GROUP, &GROUP_FRAMES, false, (grow, 2));
        let f = img.frame(State::Normal);
        let (w, h) = (f.width, f.height);
        let corner = |x0: i32| {
            (h - 2 * CORNER2..h).step_by(2).any(|y| {
                (x0..x0 + 2 * CORNER2).step_by(2).any(|x| {
                    let px = |dx, dy| img.atlas.get(f.x + x + dx, f.y + y + dy);
                    px(0, 0) != px(1, 0) || px(0, 0) != px(0, 1) || px(0, 0) != px(1, 1)
                })
            })
        };
        assert!(
            corner(0) && corner(w - 2 * CORNER2),
            "+{grow}: smooth corners at 2"
        );
    }
}
