//! Glass's popup menus and menu bars, drawn from shapes: an opaque menu with an icon gutter, lit
//! rows and check boxes, and a bar with hot and open title frames.

use std::borrow::Cow;

use super::Mode;
use super::dark::remap_color;
use super::glyph::{Mark, Piece};
use super::panel::{self, Fade, LightBox};
use crate::raster::{Canvas, Insets};
use crate::shape::{Px, Stops, centre, draw, ellipse, mix, premul, radial, ringed, scale};
use crate::style::{DropShadow, Layout, MenuLook, Part, PartImage, State};
use crate::theme::Color;

const LOOK: MenuLook = MenuLook {
    frame: Color::rgb(0x94, 0x95, 0x95),
    ring: Some(Color::rgb(0xF4, 0xF5, 0xF5)),
    face: None,
    row: 22,
    separator: 8,
    gutter: 26,
    gutter_fill: Color::rgb(0xEF, 0xF0, 0xF0),
    divider: (Color::rgb(0xE0, 0xE2, 0xE2), Color::rgb(0xFF, 0xFF, 0xFF)),
    rule: (Color::rgb(0xDE, 0xDF, 0xDF), Color::rgb(0xFF, 0xFF, 0xFF)),
    text_x: 32,
    icon_x: 3,
    right_pad: 39,
    shortcut_end: 20,
    arrow_inset: 13,
    arrow: Color::rgb(0x27, 0x27, 0x27),
    greyed: super::GREYED_CHOICE_TEXT,
    check_at: (7, 5),
    radio_at: (7, 7),
    shadow: None,
};

/// The Start panel's Shut down options: a thin frame on a flat face, a light gutter, short rows
/// and no shadow.
const OPTIONS: MenuLook = MenuLook {
    frame: Color::rgb(0xB4, 0xB6, 0xBA),
    ring: None,
    face: Some(Color::rgb(0xF3, 0xF4, 0xF5)),
    row: 21,
    separator: 6,
    gutter: 24,
    gutter_fill: Color::rgb(0xE9, 0xEA, 0xEC),
    divider: (Color::rgb(0xE0, 0xE1, 0xE3), Color::rgb(0xFA, 0xFA, 0xFB)),
    rule: (Color::rgb(0xD9, 0xDA, 0xDD), Color::rgb(0xFF, 0xFF, 0xFF)),
    text_x: 31,
    right_pad: 22,
    shortcut_end: 16,
    shadow: None,
    ..LOOK
};

pub fn look(mode: Mode, shadow: Option<&'static DropShadow>) -> MenuLook {
    in_mode(mode, MenuLook { shadow, ..LOOK })
}

pub fn options_look(mode: Mode) -> MenuLook {
    in_mode(mode, OPTIONS)
}

fn in_mode(mode: Mode, light: MenuLook) -> MenuLook {
    match mode {
        Mode::Light => light,
        Mode::Dark => MenuLook {
            frame: remap_color(light.frame),
            ring: light.ring.map(remap_color),
            face: light.face.map(remap_color),
            gutter_fill: remap_color(light.gutter_fill),
            divider: (remap_color(light.divider.0), remap_color(light.divider.1)),
            rule: (remap_color(light.rule.0), remap_color(light.rule.1)),
            arrow: remap_color(light.arrow),
            greyed: remap_color(light.greyed),
            ..light
        },
    }
}

const fn ink(v: u32, a: u8) -> (Color, u8) {
    (Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8), a)
}

const LIT: LightBox = LightBox {
    outline: ink(0x93BCFA, 186),
    line: Some((ink(0xFCFCFF, 153), ink(0xECF4FF, 153))),
    fill: (ink(0xF5F9FF, 102), ink(0xD6E5FF, 102)),
    radius: 3.0,
    fade: Fade::Ring,
};

const LIT_GREYED: LightBox = LightBox {
    outline: ink(0xC8C6C6, 186),
    line: Some((ink(0xF5F7F7, 186), ink(0xE1E0E1, 186))),
    fill: (ink(0xF2F3F3, 171), ink(0xDBDDDD, 171)),
    radius: 3.0,
    fade: Fade::Ring,
};

const CHECK: LightBox = LightBox {
    outline: ink(0xABCAF7, 255),
    line: Some((ink(0xF6F8FA, 255), ink(0xF0F4FA, 255))),
    fill: (ink(0xECF1F7, 255), ink(0xECF1F7, 255)),
    radius: 3.0,
    fade: Fade::Ring,
};

const CHECK_LIT: LightBox = LightBox {
    outline: ink(0x9BC1F9, 255),
    line: Some((ink(0xF8FAFD, 255), ink(0xEFF5FD, 255))),
    fill: (ink(0xECF2FA, 255), ink(0xE4ECFA, 255)),
    radius: 3.0,
    fade: Fade::Ring,
};

const BOX_W: i32 = 10;
const BOX_GRID: Insets = Insets::new(4, 0, 4, 0);

fn boxes(boxes: &[(&LightBox, State)], w: i32, h: i32, d: i32) -> PartImage {
    let mut a = Canvas::new((w * d) as u32, (h * d * boxes.len() as i32) as u32);
    for (i, (b, _)) in boxes.iter().enumerate() {
        a.blit_over(0, i as i32 * h * d, &panel::light_box(b, w, h, d));
    }
    let states: Vec<State> = boxes.iter().map(|b| b.1).collect();
    PartImage::new(a, &states, Layout::Vertical).at_scale(d)
}

pub fn parts(d: i32) -> [(Part, PartImage); 6] {
    let row = LOOK.row;
    let lit = boxes(
        &[(&LIT, State::Hot), (&LIT_GREYED, State::Disabled)],
        BOX_W,
        row,
        d,
    )
    .sizing(BOX_GRID);
    let check = boxes(
        &[(&CHECK, State::Normal), (&CHECK_LIT, State::Hot)],
        row,
        row,
        d,
    );
    let one = |c: Canvas, states: &[State]| PartImage::new(c, states, Layout::Vertical).at_scale(d);
    let mut titles = Canvas::new((TITLE_W * d) as u32, (2 * TITLE_H * d) as u32);
    titles.blit_over(0, 0, &title(&TITLE_HOT, d));
    titles.blit_over(0, TITLE_H * d, &title(&TITLE_OPEN, d));
    [
        (Part::MenuItem, lit),
        (Part::MenuCheck, check),
        (Part::MenuCheckMark, one(tick(d), &[State::Normal])),
        (Part::MenuRadioMark, one(bullet(d), &[State::Normal])),
        (
            Part::MenuBar,
            one(bar(d), &[State::Normal]).sizing(Insets::new(1, 0, 1, 2)),
        ),
        (
            Part::MenuBarItem,
            one(titles, &[State::Hot, State::Pressed]).sizing(Insets::new(4, 4, 4, 4)),
        ),
    ]
}

fn opaque(v: u32) -> Px {
    premul(ink(v, 0xFF).0, 0xFF)
}

fn rows<const N: usize>(stops: &[(f64, u32); N]) -> [(f64, Px); N] {
    stops.map(|(t, v)| (t, opaque(v)))
}

const BAR_W: i32 = 3;
const BAR_H: i32 = 20;
const BAR_FACE: [(f64, u32); 5] = [
    (0.0, 0xFEFDFF),
    (2.0, 0xF6F7FC),
    (6.5, 0xE2E6F4),
    (6.5, 0xD1D7EB),
    (17.0, 0xE0E4F6),
];
const BAR_LINE: u32 = 0xB4B9CA;
const BAR_UNDER: u32 = 0xEFEEEF;

fn bar(d: i32) -> Canvas {
    let face = rows(&BAR_FACE);
    draw(BAR_W, BAR_H, d, |x, y| {
        let (x, y) = (x as i32 / d, y as i32);
        match y / d {
            18 => opaque(BAR_LINE),
            19 => opaque(BAR_UNDER),
            _ => {
                let mid = Stops(&face).at(centre(y, d));
                if x == 1 {
                    mid
                } else {
                    mix(mid, opaque(0xFFFFFF), 0.5)
                }
            }
        }
    })
}

struct Title {
    outline: [(f64, u32); 5],
    line: [(f64, u32); 5],
    fill: [(f64, u32); 5],
    shadow: f64,
}

const TITLE_W: i32 = 9;
const TITLE_H: i32 = 18;
const TITLE_R: f64 = 3.0;

const TITLE_HOT: Title = Title {
    outline: [
        (0.0, 0xAEAFAF),
        (3.0, 0xA5A6AC),
        (6.0, 0x9196A3),
        (15.0, 0x999DA9),
        (17.0, 0x7C808C),
    ],
    line: [
        (1.0, 0xFDFDFE),
        (8.5, 0xE9EDF7),
        (8.5, 0xD4D6DF),
        (15.0, 0xE8EBF4),
        (16.0, 0xEEF0F9),
    ],
    fill: [
        (2.0, 0xF6F8FA),
        (6.0, 0xD6DAE8),
        (8.5, 0xD1D7E6),
        (8.5, 0xC2C7D8),
        (15.0, 0xDFE2F3),
    ],
    shadow: 0.0,
};

const TITLE_OPEN: Title = Title {
    outline: [
        (0.0, 0x565657),
        (3.0, 0x515255),
        (6.0, 0x484951),
        (15.0, 0x4C4D53),
        (17.0, 0x3E4045),
    ],
    line: [
        (1.0, 0x9D9DA0),
        (3.0, 0xC6C8CE),
        (5.5, 0xCBCFDA),
        (5.5, 0xC0C5D8),
        (16.0, 0xD4D8E8),
    ],
    fill: [
        (2.0, 0xB4B6B9),
        (3.0, 0xC6C8CE),
        (5.5, 0xCBCFDA),
        (5.5, 0xC0C5D8),
        (15.0, 0xD5D9E9),
    ],
    shadow: 0.25,
};

fn title(t: &Title, d: i32) -> Canvas {
    let (outline, line, fill) = (rows(&t.outline), rows(&t.line), rows(&t.fill));
    let px = ringed(TITLE_W, TITLE_H, TITLE_R, 2, d, |x, y, ring| match ring {
        0 => Stops(&outline).at(y),
        1 if y < 1.5 || x < 1.5 => mix(Stops(&line).at(y), Stops(&outline).at(y), t.shadow),
        1 => Stops(&line).at(y),
        _ => Stops(&fill).at(y),
    });
    draw(TITLE_W, TITLE_H, d, |x, y| {
        px[(y as i32 * TITLE_W * d + x as i32) as usize]
    })
}

const TICK_INK: u32 = 0x0D0C9D;
const TICK: Mark = Mark {
    w: 9,
    h: 11,
    pieces: Cow::Borrowed(&[Piece::Line(
        Cow::Borrowed(&[(0.7, 5.4), (3.3, 8.6), (8.3, 0.6)]),
        1.9,
    )]),
};
const BULLET: i32 = 8;
const BULLET_BODY: [(f64, u32); 3] = [(0.0, 0xC2C1E8), (0.55, 0x6D6CB1), (1.0, 0x1D1A6E)];
const BULLET_SPOT: (f64, f64) = (4.8, 2.4);
const BULLET_REACH: f64 = 5.6;

fn tick(d: i32) -> Canvas {
    let ink = opaque(TICK_INK);
    TICK.canvas(d, |_, _| ink)
}

fn bullet(d: i32) -> Canvas {
    let k = f64::from(d);
    let body = rows(&BULLET_BODY);
    let (r, (sx, sy)) = (f64::from(BULLET) / 2.0 * k, BULLET_SPOT);
    let reach = BULLET_REACH * k;
    draw(BULLET, BULLET, d, |x, y| {
        let c = radial(x, y, sx * k, sy * k, reach, reach, &Stops(&body));
        scale(c, ellipse(x, y, r, r, r, r))
    })
}

#[cfg(test)]
mod tests {
    use crate::controls::menu::{MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
    use crate::controls::menubar::{self, Layout, Title};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::metrics::MenuTitles;
    use crate::raster::{Canvas, Rect};
    use crate::text::TextRenderer;
    use crate::theme::{Color, FontConfig, Theme};

    fn rgb(v: u32) -> u32 {
        0xFF00_0000 | v
    }

    fn near(what: &str, got: u32, want: u32, tol: i32) {
        let d = [16, 8, 0]
            .iter()
            .map(|sh| ((got >> sh) & 0xFF) as i32 - ((want >> sh) & 0xFF) as i32)
            .map(i32::abs)
            .max()
            .unwrap_or(0);
        assert!(d <= tol, "{what}: {got:08X}, want {want:06X} within {tol}");
    }

    fn desktop(theme: &Theme, keys: &[MenuKey]) -> (Canvas, PopupMenu) {
        desktop_under(theme, keys, None)
    }

    fn desktop_under(
        theme: &Theme,
        keys: &[MenuKey],
        pointer: Option<(i32, i32)>,
    ) -> (Canvas, PopupMenu) {
        let text = TextRenderer::load(&FontConfig::default());
        let mut view = MenuTree::new();
        view.push(MenuItem::item("&Large icons").radio(true));
        view.push(MenuItem::item("&Medium icons").radio(true).checked(true));
        view.push(MenuItem::item("&Small icons").radio(true));
        view.push(MenuItem::Separator);
        view.push(MenuItem::item("A&lign icons to grid").checked(true));
        let mut desk = MenuTree::new();
        desk.push_sub(MenuItem::item("&View").submenu(true), view);
        desk.push(MenuItem::item("R&efresh"));
        desk.push(MenuItem::Separator);
        desk.push(MenuItem::item("&Paste").enabled(false));
        desk.push(MenuItem::item("&Undo Delete").shortcut(Some("Ctrl+Z".into())));
        let mut c = Canvas::new(500, 300);
        c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        let cx = MenuCtx {
            text: &text,
            theme,
            px: theme.fonts.size_px,
            within: c.bounds(),
        };
        let mut chain = PopupMenu::open(&desk, (20, 20), &cx, false);
        for &k in keys {
            chain.key(&desk, k, &cx);
        }
        if let Some((x, y)) = pointer {
            chain.motion(&desk, x, y, &cx);
        }
        chain.draw(&mut c, theme, &text, false);
        (c, chain)
    }

    #[test]
    #[ignore]
    fn render_glass_menu_preview() {
        let theme = theme_for(Interface::Glass);
        let walk = [MenuKey::Down, MenuKey::Right, MenuKey::Up];
        let (c, _) = desktop(&theme, &walk);
        crate::raster::write_preview(&c, "glass-menu-preview.png");
    }

    #[test]
    fn a_menu_has_a_gutter_and_rows_22_px_apart() {
        let theme = theme_for(Interface::Glass);
        let (c, chain) = desktop(&theme, &[]);
        let l = &chain.levels()[0].layout;
        let pitch: Vec<i32> = l
            .rows
            .iter()
            .map(|r| (r.y, r.height))
            .map(|p| p.1)
            .collect();
        assert_eq!(pitch, [22, 22, 8, 22, 22], "rows 22 px, a separator 8");
        assert_eq!(l.rows[0].y, 3, "the first row 3 px down");
        let (x, y) = (20, 20);
        assert_eq!(c.get(x, y + 40), rgb(0x949595), "the frame");
        assert_eq!(c.get(x + 1, y + 40), rgb(0xF4F5F5), "the ring");
        assert_eq!(
            c.get(x + 2, y + 40),
            rgb(0xF4F5F5),
            "the ring's second column"
        );
        assert_eq!(
            c.get(x + 3, y + 2),
            rgb(0xF4F5F5),
            "the ring above the gutter"
        );
        for gx in [3, 28] {
            assert_eq!(c.get(x + gx, y + 3), rgb(0xEFF0F0), "the gutter at {gx}");
        }
        assert_eq!(c.get(x + 29, y + 30), rgb(0xE0E2E2), "the divider");
        assert_eq!(
            c.get(x + 30, y + 30),
            rgb(0xFFFFFF),
            "the divider's light column"
        );
        assert_eq!(c.get(x + 31, y + 30), rgb(0xEFEEEF), "the face");
        let sep = y + l.rows[2].y;
        assert_eq!(
            c.get(x + 31, sep + 4),
            rgb(0xDEDFDF),
            "the separator's line"
        );
        assert_eq!(
            c.get(x + 31, sep + 5),
            rgb(0xFFFFFF),
            "the separator's light line"
        );
        assert_eq!(c.get(x + 31, sep + 3), rgb(0xEFEEEF), "above the line");
        assert_eq!(
            c.get(x + 28, sep + 4),
            rgb(0xEFF0F0),
            "the gutter under the line"
        );
        let right = x + l.width - 4;
        assert_eq!(
            c.get(right, sep + 4),
            rgb(0xDEDFDF),
            "the line to the row's end"
        );
    }

    #[test]
    fn a_lit_row_is_a_light_box_over_gutter_and_face() {
        let theme = theme_for(Interface::Glass);
        let (c, chain) = desktop(&theme, &[MenuKey::Down, MenuKey::Down]);
        let r = chain.levels()[0].layout.rows[1];
        let (x, y) = (20 + r.x, 20 + r.y);
        near("the corner", c.get(x, y), 0xEDEFF0, 3);
        near("the corner's neighbour", c.get(x + 1, y), 0xCCDCF4, 4);
        assert_eq!(c.get(x + 10, y), rgb(0xABC9F6), "the top outline");
        assert_eq!(c.get(x, y + 10), rgb(0xABC9F6), "the left outline");
        let face = x + r.width - 30;
        near("the fill's top", c.get(face, y + 2), 0xF1F3F6, 2);
        near("the fill's middle", c.get(face, y + 11), 0xEBEFF6, 2);
        near("the fill's foot", c.get(face, y + 19), 0xE5EBF6, 2);
        near("over the gutter", c.get(x + 10, y + 11), 0xECF0F7, 2);
        near("over the divider", c.get(x + 26, y + 11), 0xE3E7ED, 2);
        near("over its light column", c.get(x + 27, y + 11), 0xF5F8FF, 2);
        assert_eq!(c.get(face, y - 1), rgb(0xEFEEEF), "the row above stays");

        let paste = 20 + 3 + 22 + 22 + 8 + 11;
        let (c, chain) = desktop_under(&theme, &[], Some((60, paste)));
        let l = &chain.levels()[0].layout;
        assert_eq!(chain.hovered(), Some(3), "the pointer lights a greyed row");
        let r = l.rows[3];
        let (x, y) = (20 + r.x, 20 + r.y);
        near("the grey outline", c.get(x + 10, y), 0xD3D2D2, 2);
        near("the grey fill", c.get(x + 40, y + 2), 0xF2F1F2, 2);
        let ink = (y..y + r.height)
            .flat_map(|py| (x + l.text_x..x + 80).map(move |px| (px, py)))
            .map(|(px, py)| c.get(px, py) & 0xFF_FFFF)
            .min_by_key(|v| (v >> 8) & 0xFF)
            .unwrap();
        assert_eq!(ink, 0x6B6B6B, "the greyed label, flat");
    }

    #[test]
    fn checks_and_bullets_sit_in_a_box_in_the_gutter() {
        let theme = theme_for(Interface::Glass);
        let (c, chain) = desktop(&theme, &[MenuKey::Down, MenuKey::Right]);
        let sub = &chain.levels()[1];
        let at = |row: usize| {
            let r = sub.layout.rows[row];
            (sub.origin.0 + r.x, sub.origin.1 + r.y)
        };
        let (x, y) = at(1);
        assert_eq!(c.get(x + 10, y), rgb(0xABCAF7), "the box's outline");
        assert_eq!(c.get(x + 3, y + 11), rgb(0xECF1F7), "the box's fill");
        let navy = |v: u32| {
            let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
            b > r + 0x30 && b > g + 0x30 && r < 0x60
        };
        assert!(navy(c.get(x + 7 + 1, y + 7 + 5)), "the bullet's navy rim");
        let lit = c.get(x + 7 + 5, y + 7 + 2);
        assert!((lit >> 16) & 0xFF > 0x90, "its light spot: {lit:08X}");
        assert_eq!(
            c.get(x + 22, y + 11),
            rgb(0xEFF0F0),
            "the gutter past the box"
        );
        let (x, y) = at(4);
        assert_eq!(c.get(x + 7 + 7, y + 5 + 1), rgb(0x0D0C9D), "the tick");
        assert_eq!(
            c.get(x + 7 + 2, y + 5 + 7),
            rgb(0x0D0C9D),
            "the tick's short stroke"
        );
        let (_, unpicked_y) = at(2);
        assert_eq!(
            c.get(x + 10, unpicked_y),
            rgb(0xEFF0F0),
            "no box on an unpicked row"
        );

        let (c, chain) = desktop(&theme, &[MenuKey::Down, MenuKey::Right, MenuKey::Up]);
        let sub = &chain.levels()[1];
        assert_eq!(sub.hovered, Some(4));
        let r = sub.layout.rows[4];
        let (x, y) = (sub.origin.0 + r.x, sub.origin.1 + r.y);
        assert_eq!(c.get(x + 10, y), rgb(0x9BC1F9), "the lit box's outline");
        near("the lit box's fill", c.get(x + 4, y + 3), 0xECF2FA, 1);
    }

    #[test]
    fn the_arrow_and_the_submenu_sit_where_glass_puts_them() {
        let theme = theme_for(Interface::Glass);
        let (c, chain) = desktop(&theme, &[MenuKey::Down, MenuKey::Right]);
        let root = &chain.levels()[0];
        let r = root.layout.rows[0];
        let (end, y) = (20 + r.right(), 20 + r.y);
        let tip = (y..y + r.height)
            .filter(|&py| c.get(end - 13, py) == rgb(0x272727))
            .collect::<Vec<_>>();
        assert_eq!(
            tip,
            (y + 7..y + 14).collect::<Vec<_>>(),
            "the arrow's first column"
        );
        assert_eq!(c.get(end - 10, y + 10), rgb(0x272727), "its point");
        assert_ne!(c.get(end - 14, y + 10), rgb(0x272727), "nothing left of it");
        let sub = &chain.levels()[1];
        assert_eq!(sub.origin.0, root.rect().right() - 6, "overlapping by 6");
        assert_eq!(sub.origin.1 + sub.layout.rows[0].y, y, "level with View");
        let hit = chain.hit(sub.origin.0 + 40, sub.origin.1 + 3 + 22 + 21);
        assert_eq!(hit, Some((1, Some(1))), "a row's last pixel row is its own");
    }

    #[test]
    fn a_menu_casts_a_soft_shadow_right_and_below() {
        let theme = theme_for(Interface::Glass);
        let (c, chain) = desktop(&theme, &[]);
        let m = chain.levels()[0].rect();
        let mid = m.y + m.height / 2;
        let out = [0x8D8D8D, 0xABABAB, 0xD4D4D4, 0xF2F2F2, 0xFDFDFD];
        for (i, &v) in out.iter().enumerate() {
            assert_eq!(c.get(m.right() + i as i32, mid), rgb(v), "right, {i} out");
            assert_eq!(
                c.get(m.x + 40, m.bottom() + i as i32),
                rgb(v),
                "below, {i} out"
            );
        }
        assert_eq!(
            c.get(m.right(), m.y + 3),
            rgb(0xFFFFFF),
            "none at the top corner"
        );
        assert_eq!(
            c.get(m.x + 3, m.bottom()),
            rgb(0xFFFFFF),
            "none at the left corner"
        );
    }

    fn fnv(c: &Canvas) -> u64 {
        let words = [c.width, c.height]
            .into_iter()
            .chain((0..c.height as i32).flat_map(|y| {
                (0..c.width as i32)
                    .map(move |x| (x, y))
                    .map(|(x, y)| c.get(x, y))
            }));
        words.fold(0xcbf2_9ce4_8422_2325, |h, w| {
            w.to_le_bytes()
                .iter()
                .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
        })
    }

    #[test]
    fn the_bar_titles_and_marks_are_drawn_at_each_scale() {
        use crate::style::{Part, State};
        let (one, two) = (super::parts(1), super::parts(2));
        for ((p, a), (q, b)) in one.iter().zip(two.iter()) {
            assert_eq!(p, q);
            assert_eq!(b.scale, 2, "{p:?}");
            assert_eq!(
                (b.atlas.width, b.atlas.height),
                (2 * a.atlas.width, 2 * a.atlas.height),
                "{p:?}"
            );
            assert_eq!(a.sizing, b.sizing, "{p:?}: the nine-grid in design px");
        }
        let part = |parts: &[(Part, crate::style::PartImage)], p: Part| {
            parts
                .iter()
                .find(|q| q.0 == p)
                .map(|q| q.1.clone())
                .unwrap()
        };
        let titles = part(&two, Part::MenuBarItem);
        let (hot, open) = (titles.frame(State::Hot), titles.frame(State::Pressed));
        for f in [hot, open] {
            assert!(
                titles.atlas.get(f.x, f.y) >> 24 < 8,
                "a title's corner clear"
            );
        }
        let frame = |f: crate::raster::Rect| titles.atlas.copy_rect(f);
        assert_ne!(fnv(&frame(hot)), fnv(&frame(open)), "hot and open differ");
        for p in [Part::MenuBarItem, Part::MenuCheckMark, Part::MenuRadioMark] {
            let img = part(&two, p);
            let soft = (0..img.atlas.height as i32)
                .flat_map(|y| (0..img.atlas.width as i32).map(move |x| (x, y)))
                .map(|(x, y)| img.atlas.get(x, y) >> 24)
                .any(|a| a > 0 && a < 0xFF);
            assert!(soft, "{p:?}: soft edges at 2");
        }
        let tick = part(&one, Part::MenuCheckMark);
        let solid = (0..11)
            .flat_map(|y| (0..9).map(move |x| (x, y)))
            .filter(|&(x, y)| tick.atlas.get(x, y) == rgb(0x0D0C9D))
            .count();
        assert!(solid >= 8, "the tick's strokes whole at 1: {solid}");
    }

    #[test]
    fn menu_marks_pin_at_scale_1_and_2() {
        let got: Vec<u64> = [1, 2]
            .into_iter()
            .flat_map(|d| {
                super::parts(d)
                    .into_iter()
                    .skip(2)
                    .map(|(_, img)| fnv(&img.atlas))
            })
            .collect();
        let want: [u64; 8] = [
            0xf296ce72ada92351,
            0xfb031f0607ce72c5,
            0x8fb9f523a087de68,
            0xcf4eddf6cdc207ae,
            0xe204a1a40ddf7695,
            0xa09f4c420159ce4c,
            0xf5d87b878e5c790b,
            0xfb913e06d0d9bc23,
        ];
        assert_eq!(got, want, "{got:#x?}");
    }

    fn options_theme(mode: super::Mode, scale: i32) -> Theme {
        let s = super::super::Settings {
            mode,
            ..super::super::Settings::default()
        };
        let theme =
            crate::interfaces::theme_with(Interface::Glass, crate::blue::Variant::default(), &s);
        theme.scaled(scale).for_options_menu()
    }

    /// The Shut down options menu with its labels left blank, so its pixels hold no text,
    /// on white with Restart lit when `lit`.
    fn options(theme: &Theme, lit: bool) -> (Canvas, PopupMenu) {
        let text = TextRenderer::load(&FontConfig::default());
        let mut tree = MenuTree::new();
        for item in [
            MenuItem::item(""),
            MenuItem::item(""),
            MenuItem::Separator,
            MenuItem::item(""),
            MenuItem::Separator,
            MenuItem::item(""),
            MenuItem::item(""),
        ] {
            tree.push(item);
        }
        let s = theme.scale;
        let mut c = Canvas::new(200 * s as u32, 200 * s as u32);
        c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        let cx = MenuCtx {
            text: &text,
            theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 200, 200),
        };
        let mut chain = PopupMenu::open(&tree, (10, 10), &cx, false);
        if lit {
            for _ in 0..3 {
                chain.key(&tree, MenuKey::Down, &cx);
            }
        }
        chain.draw(&mut c, theme, &text, false);
        (c, chain)
    }

    #[test]
    fn the_options_menu_has_a_thin_frame_short_rows_and_no_shadow() {
        use super::Mode;
        let theme = options_theme(Mode::Light, 1);
        let (c, chain) = options(&theme, false);
        let l = &chain.levels()[0].layout;
        let rows: Vec<i32> = l.rows.iter().map(|r| r.height).collect();
        assert_eq!(rows, [21, 21, 6, 21, 6, 21, 21]);
        let (x, y) = (10, 10);
        assert_eq!(c.get(x, y + 30), rgb(0xB4B6BA), "the frame");
        assert_eq!(c.get(x + 1, y + 30), rgb(0xF3F4F5), "no ring: the face");
        assert_eq!(c.get(x + 3, y + 30), rgb(0xE9EAEC), "the gutter");
        assert_eq!(c.get(x + 27, y + 30), rgb(0xE0E1E3), "the divider");
        assert_eq!(c.get(x + 40, y + 30), rgb(0xF3F4F5), "the face");
        let sep = y + l.rows[2].y;
        assert_eq!(
            c.get(x + 40, sep + 3),
            rgb(0xD9DADD),
            "the separator's line"
        );
        assert_eq!(c.get(x + 40, sep + 4), rgb(0xFFFFFF), "its light line");
        let (right, bottom) = (x + l.width, y + l.height);
        assert_eq!(
            c.get(right, y + 20),
            rgb(0xFFFFFF),
            "no shadow at the right"
        );
        assert_eq!(c.get(x + 20, bottom), rgb(0xFFFFFF), "no shadow below");

        let plain = theme_for(Interface::Glass);
        assert!(plain.look.style().unwrap().menu.unwrap().shadow.is_some());
        let (_, menu) = options(&plain, false);
        assert!(
            l.width < menu.levels()[0].layout.width,
            "narrower than the other menus"
        );
        let dark = options_theme(Mode::Dark, 1);
        let (d, _) = options(&dark, false);
        assert_ne!(d.get(x + 40, y + 30), c.get(x + 40, y + 30), "a dark face");
    }

    #[test]
    fn the_options_menu_pins_at_scale_1_and_2() {
        use super::Mode;
        let got: Vec<u64> = [Mode::Light, Mode::Dark]
            .into_iter()
            .flat_map(|m| [1, 2].map(move |s| (m, s)))
            .flat_map(|(m, s)| {
                let theme = options_theme(m, s);
                [false, true].map(|lit| fnv(&options(&theme, lit).0))
            })
            .collect();
        let want: [u64; 8] = [
            0x0b893eee676d22b1,
            0x8fd533b96b4ffb1d,
            0x139ee9da3301412d,
            0x989f08c9e4bfb7e5,
            0x1e7fef25e74581f3,
            0x1170d223d3014d13,
            0x14d9f0f92d98b6dd,
            0xe9fa2e0e06523c12,
        ];
        assert_eq!(got, want, "{got:#x?}");
        let (one, _) = options(&options_theme(Mode::Light, 1), false);
        let (two, _) = options(&options_theme(Mode::Light, 2), false);
        assert_eq!(two.get(20, 60), one.get(10, 30), "the frame doubles");
        assert_eq!(two.get(21, 60), one.get(10, 30), "the frame doubles");
        assert_eq!(two.get(22, 60), one.get(11, 30), "then the face");
    }

    /// A title frame `h` design px tall at `d`, 30 wide.
    fn title_at(h: i32, state: crate::style::State, d: i32) -> Canvas {
        let theme = theme_for(Interface::Glass).scaled(d);
        let st = theme.look.style().unwrap();
        let mut c = Canvas::new((30 * d) as u32, (h * d) as u32);
        let r = Rect::new(0, 0, 30 * d, h * d);
        st.draw(&mut c, r, crate::style::Part::MenuBarItem, state, d);
        c
    }

    fn rows_of(c: &Canvas, ys: std::ops::Range<i32>) -> Vec<u32> {
        ys.flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .collect()
    }

    #[test]
    fn a_grown_title_frame_keeps_its_rounded_ends() {
        use crate::style::State;
        let grown = |pct| theme_for(Interface::Glass).with_text_size(pct).controls();
        for pct in [150, 200] {
            let h = grown(pct).menubar.plain.frame_h;
            assert!(h > 18, "{pct}%: the frame grows");
            for d in [1, 2] {
                for state in [State::Hot, State::Pressed] {
                    let (native, tall) = (title_at(18, state, d), title_at(h, state, d));
                    let ends = 4 * d;
                    let foot = |c: &Canvas| rows_of(c, c.height as i32 - ends..c.height as i32);
                    assert_eq!(
                        rows_of(&tall, 0..ends),
                        rows_of(&native, 0..ends),
                        "{pct}% {state:?} at {d}: top"
                    );
                    assert_eq!(foot(&tall), foot(&native), "{pct}% {state:?} at {d}: foot");
                }
            }
        }
    }

    #[test]
    fn a_grown_bar_keeps_one_line_row_under_its_face() {
        let theme = theme_for(Interface::Glass);
        let st = theme.look.style().unwrap();
        let mut c = Canvas::new(40, 40);
        st.draw(
            &mut c,
            Rect::new(0, 0, 40, 40),
            crate::style::Part::MenuBar,
            crate::style::State::Normal,
            1,
        );
        assert_eq!(c.get(20, 38), rgb(0xB4B9CA), "the line");
        assert_eq!(c.get(20, 39), rgb(0xEFEEEF), "the face under it");
        assert_ne!(c.get(20, 37), rgb(0xB4B9CA), "one line row");
    }

    #[test]
    fn grown_title_frames_pin_at_150_and_200_percent() {
        use crate::style::State;
        let mut got = Vec::new();
        for pct in [150, 200] {
            let h = theme_for(Interface::Glass)
                .with_text_size(pct)
                .controls()
                .menubar
                .plain
                .frame_h;
            for d in [1, 2] {
                for state in [State::Hot, State::Pressed] {
                    got.push((format!("{pct} {d} {state:?}"), fnv(&title_at(h, state, d))));
                }
            }
        }
        crate::pins::check("glass/menu_titles_grown", &got);
    }

    #[test]
    fn the_bar_lights_hot_and_open_titles_in_frames() {
        let theme = theme_for(Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let titles = vec![Title::new("&File"), Title::new("&Edit")];
        let lay = Layout::compute(
            &text,
            px,
            Rect::new(0, 0, 200, 40),
            &titles,
            &MenuTitles::CLASSIC,
        );
        let paint = |open, hot, active| {
            let mut c = Canvas::new(200, 40);
            let p = crate::look::Paint {
                active,
                ..theme.paint()
            };
            menubar::draw(
                &mut c, &text, &p, &lay, &titles, open, hot, false, false, px,
            );
            c
        };
        let rest = paint(None, None, true);
        assert_eq!(rest.get(100, 0), rgb(0xFEFDFF), "the bar's top");
        assert_eq!(rest.get(100, 17), rgb(0xE0E4F6), "the bar's foot");
        assert_eq!(rest.get(100, 18), rgb(0xB4B9CA), "the line");
        assert_eq!(rest.get(100, 19), rgb(0xEFEEEF), "the face under it");
        near("the lighter end column", rest.get(0, 2), 0xFAFBFD, 1);
        let f = lay.frames[1];
        let hot = paint(None, Some(1), true);
        let open = paint(Some(1), None, true);
        let grey = |v: u32| (v >> 8) & 0xFF;
        let (side, fill) = (hot.get(f.x, f.y + 8), hot.get(f.x + 4, f.y + 8));
        assert!(
            grey(side) < grey(fill),
            "the hot frame's side darker than its fill"
        );
        assert!(
            grey(open.get(f.x, f.y + 8)) + 0x30 < grey(side),
            "the open frame's side darker still"
        );
        assert_eq!(
            hot.get(f.x + 4, f.y + 2),
            rgb(0xF6F8FA),
            "the hot fill's top"
        );
        assert_eq!(
            hot.get(f.x + 4, f.bottom() - 1),
            rgb(0x7C808C),
            "the hot frame's foot"
        );
        assert_eq!(
            open.get(f.x + 4, f.y + 2),
            rgb(0xB4B6B9),
            "the open fill's top"
        );
        let ink = |c: &Canvas| {
            (f.y..f.bottom())
                .flat_map(|y| (f.x + 5..f.right() - 4).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == rgb(0))
                .collect::<Vec<_>>()
        };
        assert!(!ink(&hot).is_empty(), "black text");
        assert_eq!(ink(&hot), ink(&open), "the text stays put");
        let inactive = paint(None, Some(1), false);
        assert_eq!(
            inactive.get(f.x + 4, f.y + 2),
            rest.get(f.x + 4, f.y + 2),
            "no hot title"
        );
        assert!(ink(&inactive).is_empty(), "greyed titles");
    }
}
