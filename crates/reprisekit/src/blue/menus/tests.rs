use crate::blue::Variant;
use crate::controls::menu::{
    MenuItem, MenuLayout, MenuStyle, draw_menu_at, draw_menu_shadow, place_submenu_beside,
};
use crate::interface::Interface;
use crate::interfaces::{blue_start_menus, theme_for};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::{Color, FontConfig, Theme};

const AT: (i32, i32) = (10, 10);
const W: i32 = 320;
const H: i32 = 150;

#[derive(Clone, Copy, Debug)]
enum Lit {
    Rest,
    Row,
    FolderOpen,
    Greyed,
}

const STATES: [Lit; 4] = [Lit::Rest, Lit::Row, Lit::FolderOpen, Lit::Greyed];

fn rgb(c: Color) -> u32 {
    0xFF00_0000 | (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)
}

fn channels(px: u32) -> [u32; 3] {
    [(px >> 16) & 0xFF, (px >> 8) & 0xFF, px & 0xFF]
}

fn bright(px: u32) -> u32 {
    channels(px).iter().sum()
}

/// Two folders, a program and a greyed row, their labels blank so the pixels hold no text.
fn rows() -> Vec<MenuItem> {
    vec![
        MenuItem::entry("").submenu(true),
        MenuItem::entry("").submenu(true),
        MenuItem::entry(""),
        MenuItem::entry("").enabled(false),
    ]
}

fn laid_out(theme: &Theme, text: &TextRenderer, items: &[MenuItem]) -> MenuLayout {
    let style = MenuStyle::context_for(theme, items).fit(text, items, theme.fonts.size_px, W);
    MenuLayout::compute_styled(&style, items, usize::MAX)
}

/// One menu with its shadow at `at` (design px), as the compositor draws a level.
fn draw_level(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    at: (i32, i32),
    layout: &MenuLayout,
    items: &[MenuItem],
    hovered: Option<usize>,
) {
    let d = theme.scale;
    let l = layout.scaled(d);
    let shadow = draw_menu_shadow(theme, l.width, l.height, l.height + 5 * d);
    c.blit_over(at.0 * d, at.1 * d, &shadow);
    draw_menu_at(
        c,
        (at.0 * d, at.1 * d),
        theme,
        text,
        &l,
        items,
        hovered,
        "",
        false,
    );
}

/// The Start menu's menu in `v` at `d` over white, at `lit`; with its layout.
fn scene(v: Variant, d: i32, lit: Lit) -> (Canvas, MenuLayout) {
    let base = theme_for(Interface::Classic);
    let one = blue_start_menus(&base, v);
    let theme = blue_start_menus(&base.scaled(d), v);
    let text = TextRenderer::load(&FontConfig::default());
    let items = rows();
    let layout = laid_out(&one, &text, &items);
    let mut c = Canvas::new((W * d) as u32, (H * d) as u32);
    c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    let hovered = match lit {
        Lit::Rest => None,
        Lit::Row => Some(2),
        Lit::FolderOpen => Some(0),
        Lit::Greyed => Some(3),
    };
    draw_level(&mut c, &theme, &text, AT, &layout, &items, hovered);
    if let Lit::FolderOpen = lit {
        let child_items = vec![MenuItem::entry(""), MenuItem::entry("")];
        let child = laid_out(&one, &text, &child_items);
        let parent = Rect::new(AT.0, AT.1, layout.width, layout.height);
        let row = layout.rows[0];
        let row = Rect::new(AT.0 + row.x, AT.1 + row.y, row.width, row.height);
        let at = place_submenu_beside(
            parent,
            row,
            (child.width, child.height),
            Rect::new(0, 0, W, H),
            child.inset,
        );
        draw_level(&mut c, &theme, &text, at, &child, &child_items, None);
    }
    (c, layout)
}

fn fnv(c: &Canvas) -> u64 {
    let words = [c.width, c.height]
        .into_iter()
        .chain((0..c.height as i32).flat_map(|y| (0..c.width as i32).map(move |x| c.get(x, y))));
    words.fold(0xcbf2_9ce4_8422_2325, |h, w| {
        w.to_le_bytes()
            .iter()
            .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    })
}

#[test]
fn the_start_menus_pin_per_state_in_two_schemes_at_scale_1_and_2() {
    let got: Vec<u64> = [Variant::Blue, Variant::Olive]
        .into_iter()
        .flat_map(|v| [1, 2].map(move |d| (v, d)))
        .flat_map(|(v, d)| STATES.map(|lit| fnv(&scene(v, d, lit).0)))
        .collect();
    let want: [u64; 16] = [
        0x1ff33e6fb0e58ecd,
        0xc9b0fe585706e33d,
        0x27c85b08f8d3ee98,
        0x4daa609cdb7a7bcd,
        0xd25ab4f064a40ec5,
        0x6ed6131dfedb9105,
        0xa87760c824f7d88d,
        0x8ad72cf2d8679cc5,
        0xbe8e306d76d66569,
        0x2aae9e27876600c9,
        0xd4bd0e02e8ed2660,
        0xd46fd250a0ecb1d9,
        0x812f6c093054ab0d,
        0xde83a85be2c1a5cd,
        0x1ab8edbe26915f5d,
        0xb36d6df0cdc4854d,
    ];
    assert_eq!(got, want, "{got:#x?}");
}

#[test]
fn the_menu_is_white_in_a_frame_with_a_strip_down_the_left_and_a_soft_shadow() {
    let s = Variant::Blue.scheme();
    let (c, l) = scene(Variant::Blue, 1, Lit::Rest);
    let (x, y) = AT;
    let mid = y + l.height / 2;
    let edge = rgb(s.start_menu.edge);
    let white = rgb(Color::rgb(0xFF, 0xFF, 0xFF));
    assert_eq!(c.get(x, mid), edge, "the frame's left line");
    assert_eq!(c.get(x + l.width / 2, y), edge, "its top line");
    assert_eq!(c.get(x + l.width - 1, mid), edge, "its right line");
    assert_eq!(
        c.get(x + l.width / 2, y + l.height - 1),
        edge,
        "its bottom line"
    );
    let (sheen, strip) = (c.get(x + 1, mid), c.get(x + 3, mid));
    for (px, what) in [(sheen, "the sheen"), (strip, "the strip")] {
        assert!(px != white && px != edge, "{what}: {px:#x}");
    }
    assert!(
        bright(sheen) > bright(strip),
        "the sheen is lighter than the strip"
    );
    assert!(
        channels(strip)[2] > channels(strip)[0],
        "the strip is the band's blue: {strip:#x}"
    );
    assert_eq!(c.get(x + 6, mid), white, "the face starts past the strip");
    assert_eq!(l.rows[0].x, 6, "rows start past the strip");

    let (right, bottom) = (x + l.width, y + l.height);
    let out: Vec<u32> = (0..6).map(|i| bright(c.get(right + i, mid))).collect();
    assert!(
        out.windows(2).all(|w| w[0] < w[1]),
        "fades out to the right: {out:?}"
    );
    assert!(out[0] < 3 * 160, "dark next to the menu: {out:?}");
    assert_eq!(out[5], 3 * 255, "gone 5 px out");
    let below: Vec<u32> = (0..6)
        .map(|i| bright(c.get(x + l.width / 2, bottom + i)))
        .collect();
    assert!(
        below.windows(2).all(|w| w[0] < w[1]),
        "fades out below: {below:?}"
    );
    assert_eq!(c.get(right, y), white, "none at the top corner");
    assert_eq!(c.get(x, bottom), white, "none at the bottom-left corner");
}

#[test]
fn a_lit_row_is_the_selection_colour_with_a_light_arrow_and_a_greyed_one_paler() {
    let s = Variant::Blue.scheme();
    let (c, l) = scene(Variant::Blue, 1, Lit::FolderOpen);
    let (x, y) = AT;
    let r = l.rows[0];
    let hot = rgb(s.system.menu_hilight);
    assert_eq!(c.get(x + r.x + 2, y + r.y + 2), hot, "the lit row's fill");
    let arrow_x = x + r.right() - super::look(s).arrow_inset;
    let arrow_y = y + r.y + (r.height - 7) / 2 + 3;
    assert_eq!(
        c.get(arrow_x, arrow_y),
        rgb(s.system.hilight_text),
        "a light arrow"
    );
    let rest = l.rows[1];
    assert_eq!(
        c.get(x + rest.right() - 9, y + rest.y + (rest.height - 7) / 2 + 3),
        rgb(s.system.menu_text),
        "a dark arrow at rest"
    );
    let child_x = x + l.width;
    assert_eq!(
        c.get(child_x, y + r.y + 2),
        rgb(s.start_menu.edge),
        "the folder's menu opens in the same frame beside it, level with its row"
    );

    let (g, _) = scene(Variant::Blue, 1, Lit::Greyed);
    let r = l.rows[3];
    let greyed = g.get(x + r.x + 2, y + r.y + 2);
    assert!(
        greyed != hot && bright(greyed) > bright(hot),
        "a paler lit greyed row"
    );
    assert!(greyed != rgb(Color::rgb(0xFF, 0xFF, 0xFF)), "but lit");
}

#[test]
fn the_frame_doubles_at_scale_2_and_its_nine_grid_keeps_its_insets() {
    let s = Variant::Blue.scheme();
    for (one, two) in super::parts(s, 1).into_iter().zip(super::parts(s, 2)) {
        assert_eq!(one.0, two.0);
        assert_eq!(two.1.scale, 2, "{:?}", one.0);
        assert_eq!(
            (two.1.atlas.width, two.1.atlas.height),
            (2 * one.1.atlas.width, 2 * one.1.atlas.height),
            "{:?}",
            one.0
        );
        assert_eq!(
            one.1.sizing, two.1.sizing,
            "{:?}: insets in design px",
            one.0
        );
    }
    let (c1, l) = scene(Variant::Blue, 1, Lit::Rest);
    let (c2, _) = scene(Variant::Blue, 2, Lit::Rest);
    let (x, mid) = (AT.0, AT.1 + l.height / 2);
    for k in 0..2 {
        let at2 = |dx: i32| c2.get(2 * (x + dx) + k, 2 * mid);
        assert_eq!(at2(0), c1.get(x, mid), "the frame's line doubles");
        assert_eq!(at2(6), c1.get(x + 6, mid), "then the face");
        for dx in 1..6 {
            let (a, b) = (channels(at2(dx)), channels(c1.get(x + dx, mid)));
            let off = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap();
            assert!(
                off <= 6,
                "strip column {dx} drawn afresh, close to scale 1: {a:?} {b:?}"
            );
        }
    }
}

#[test]
fn the_schemes_draw_their_own_frames() {
    let at = |v: Variant| {
        let (c, l) = scene(v, 1, Lit::Rest);
        (
            c.get(AT.0, AT.1 + l.height / 2),
            c.get(AT.0 + 3, AT.1 + l.height / 2),
        )
    };
    let blue = at(Variant::Blue);
    for v in [Variant::Olive, Variant::Silver] {
        let other = at(v);
        assert_ne!(other.0, blue.0, "{v:?}: frame");
        assert_ne!(other.1, blue.1, "{v:?}: strip");
    }
}
