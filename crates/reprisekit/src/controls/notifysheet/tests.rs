use super::draw::{draw_parts, relight};
use super::glass::sheet_spec;
use super::opening::{glass_at, opening_fades};
use super::stack::{BEAD_FADE, BEAD_UNTIL};
use super::*;
use crate::access::ROOT;
use crate::access::accesskit::{Live, Role};
use crate::controls::tooltip::{NOTIFY_BODY_LINES, notify_layout};
use crate::interface::Interface;
use crate::interfaces::{theme_for, theme_with};
use crate::raster::Canvas;
use crate::shape::scale;
use crate::taskbar::Edge;
use crate::textedit::{Caret, TextEdit};
use crate::theme::Color;
use crate::theme::FontConfig;
use crate::{blue, glass};

fn text() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

fn content<'a>(app: &'a str, title: &'a str, body: &'a str) -> SheetContent<'a> {
    SheetContent {
        app,
        when: "now",
        title,
        body,
        actions: &[],
        action_icons: &[],
        reply: None,
        has_picture: false,
        has_app_icon: true,
    }
}

fn lozenge(c: &SheetContent, expanded: bool) -> SheetLayout {
    sheet_layout(
        &theme_for(Interface::Glass),
        &text(),
        SheetShape::Lozenge,
        c,
        expanded,
    )
    .unwrap()
}

fn glass(dark: bool, scale: i32) -> Theme {
    let g = glass::Settings {
        mode: if dark {
            glass::Mode::Dark
        } else {
            glass::Mode::Light
        },
        ..glass::Settings::default()
    };
    theme_with(Interface::Glass, blue::Variant::Blue, &g).scaled(scale)
}

fn fnv(c: &Canvas) -> u64 {
    [c.width, c.height]
        .iter()
        .chain(c.pixels.iter())
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
}

const REPLY: NotifyReply = NotifyReply {
    placeholder: "Write a reply",
    send: "Send",
};

#[test]
fn a_lozenge_is_360_wide_with_a_46_px_icon_slot() {
    let lay = lozenge(&content("Mail", "New message", "Hi"), false);
    assert_eq!((lay.sheet.width, lay.footprint), (360, lay.sheet));
    assert_eq!(
        lay.icon,
        Rect::new(lay.sheet.x + 14, lay.sheet.y + 14, 46, 46)
    );
    assert_eq!(lay.well, Well::Squircle);
}

#[test]
fn a_picture_puts_the_app_icon_in_an_18_px_badge() {
    let both = lozenge(
        &SheetContent {
            has_picture: true,
            has_app_icon: true,
            ..content("Chat", "Ana", "Lunch?")
        },
        false,
    );
    let b = both.badge.unwrap();
    assert_eq!((b.width, b.height), (18, 18));
    assert_eq!(
        (b.x + 9, b.y + 9),
        (both.icon.right() - 4, both.icon.bottom() - 4)
    );
    assert_eq!(both.picture, Some(both.icon));
    let only = lozenge(
        &SheetContent {
            has_picture: true,
            has_app_icon: false,
            ..content("Chat", "Ana", "Lunch?")
        },
        false,
    );
    assert_eq!(only.badge, None);
    let icon = lozenge(&content("Chat", "Ana", "Lunch?"), false);
    assert_eq!((icon.badge, icon.picture), (None, None));
}

#[test]
fn the_close_spot_takes_a_tap_before_hover_shows_it() {
    let lay = lozenge(&content("Mail", "New", "Hi"), false);
    let c = lay.parts.close;
    assert_eq!(
        c,
        Rect::new(lay.sheet.right() - 34, lay.sheet.y + 12, 20, 20)
    );
    assert_eq!(sheet_hit(&lay, c.x + 2, c.y + 2), Some(NotifyHit::Close));
}

#[test]
fn the_shadow_margin_takes_no_pointer() {
    let lay = lozenge(&content("Mail", "New", "Hi"), false);
    assert_eq!(sheet_hit(&lay, 0, 0), None);
    assert_eq!(
        sheet_hit(&lay, lay.sheet.x + 100, lay.sheet.y + 40),
        Some(NotifyHit::Body)
    );
}

#[test]
fn title_is_bold_16_body_14_and_app_12() {
    let lay = lozenge(&content("Mail", "New", "Hi"), false);
    assert_eq!((lay.app_px, lay.title_px, lay.body_px), (12, 16, 14));
}

#[test]
fn the_body_wraps_to_the_line_cap() {
    let long = "word ".repeat(400);
    assert_eq!(
        lozenge(&content("Mail", "t", &long), false).lines.len(),
        NOTIFY_BODY_LINES
    );
}

#[test]
fn a_short_sheet_is_as_tall_as_its_icon_slot_and_padding() {
    let lay = lozenge(&content("Mail", "New", ""), false);
    assert_eq!(lay.sheet.height, 14 + 46 + 14);
    let long = lozenge(&content("Mail", "New", &"word ".repeat(400)), false);
    assert!(long.sheet.height > lay.sheet.height + 9 * 14);
    assert_eq!(long.sheet.bottom(), long.parts.body.bottom() + 14);
}

#[test]
fn pills_sit_right_aligned_under_the_body_and_a_fourth_brings_more() {
    let acts = ["Open", "Later"];
    let lay = lozenge(
        &SheetContent {
            actions: &acts,
            ..content("Mail", "New", "Hi")
        },
        false,
    );
    let [a, b] = lay.parts.buttons[..] else {
        panic!("two pills: {:?}", lay.parts.buttons)
    };
    assert_eq!((a.height, b.height), (26, 26));
    assert_eq!((b.right(), a.right() + 6), (lay.sheet.right() - 14, b.x));
    assert!(a.y > lay.parts.body.bottom());
    assert_eq!(lay.sheet.bottom(), a.bottom() + 14);
    assert_eq!(
        sheet_hit(&lay, b.x + 3, b.y + 3),
        Some(NotifyHit::Button(1))
    );
    let four = ["A", "B", "C", "D"];
    let more = lozenge(
        &SheetContent {
            actions: &four,
            ..content("Mail", "New", "Hi")
        },
        false,
    );
    assert_eq!(more.parts.buttons.len(), 3);
    let m = more.parts.more.unwrap();
    assert_eq!(m.right(), more.sheet.right() - 14);
}

#[test]
fn an_icon_pill_is_sized_by_its_icon_and_draws_it() {
    let t = text();
    let acts = ["media-playback-start", "Later"];
    let red = Color::rgb(0xFA, 12, 0);
    for s in [1, 2] {
        let theme = glass(false, s);
        let lay = |icons: &[bool]| {
            let c = SheetContent {
                actions: &acts,
                action_icons: icons,
                ..content("Music", "Song", "Next up")
            };
            sheet_layout(&theme, &t, SheetShape::Lozenge, &c, false).unwrap()
        };
        let (plain, lay) = (lay(&[]), lay(&[true]));
        let [play, later] = lay.parts.buttons[..] else {
            panic!("two pills: {:?}", lay.parts.buttons)
        };
        assert_eq!(play.width, (16 + 2 * 14) * s, "the icon pill at {s}");
        assert_eq!(
            later.width, plain.parts.buttons[1].width,
            "the labelled one at {s}"
        );
        assert_eq!(later.right(), lay.sheet.right() - 14 * s);

        let side = 16 * s;
        let mut icon = Canvas::new(side as u32, side as u32);
        icon.fill_rect(icon.bounds(), red);
        let actions = [Some(icon)];
        let icons = SheetIcons {
            actions: &actions,
            ..SheetIcons::default()
        };
        let drawn = draw_sheet(&theme, &t, &lay, icons, None, None, None, None);
        let (x, y) = (
            play.x + (play.width - side) / 2,
            play.y + (play.height - side) / 2,
        );
        assert_eq!(drawn.get(x, y), red.argb(), "the icon's top-left at {s}");
        let end = drawn.get(x + side - 1, y + side - 1);
        assert_eq!(end, red.argb(), "its bottom-right at {s}");
        assert_ne!(drawn.get(x - 1, y), red.argb(), "nothing left of it at {s}");
    }
}

#[test]
fn expanded_the_reply_row_replaces_the_pills() {
    let acts = ["Reply"];
    let c = SheetContent {
        actions: &acts,
        reply: Some(REPLY),
        ..content("Chat", "Ana", "Lunch?")
    };
    let shut = lozenge(&c, false);
    assert_eq!((shut.parts.buttons.len(), shut.parts.field), (1, None));
    let open = lozenge(&c, true);
    assert!(open.parts.buttons.is_empty());
    let (f, s) = (open.parts.field.unwrap(), open.parts.send.unwrap());
    assert_eq!(s.right(), open.sheet.right() - 14);
    assert_eq!((f.x, f.right() + 6), (open.icon.right() + 12, s.x));
    assert_eq!(open.parts.send_label, "Send");
    assert_eq!(sheet_hit(&open, f.x + 4, f.y + 4), Some(NotifyHit::Field));
}

fn picture_sheet(pic: &Canvas, app: Option<&Canvas>, with_pic: bool) -> (SheetLayout, Canvas) {
    let theme = glass(false, 1);
    let t = text();
    let c = SheetContent {
        has_picture: true,
        has_app_icon: app.is_some(),
        ..content("Chat", "Ana", "Lunch?")
    };
    let lay = sheet_layout(&theme, &t, SheetShape::Lozenge, &c, false).unwrap();
    let icons = SheetIcons {
        app,
        picture: with_pic.then_some(pic),
        actions: &[],
    };
    let drawn = draw_sheet(&theme, &t, &lay, icons, None, None, None, None);
    (lay, drawn)
}

#[test]
fn a_letterboxed_wide_picture_fills_the_slot() {
    let mut pic = Canvas::new(96, 96);
    pic.fill_rect(Rect::new(0, 21, 96, 54), Color::rgb(0xE0, 0x6A, 0x2E));
    let (lay, c) = picture_sheet(&pic, None, true);
    let i = lay.icon;
    assert_eq!(
        c.get(i.x + 23, i.y + 1),
        0xFFE0_6A2E,
        "the top edge is picture"
    );
    assert_eq!(
        c.get(i.x + 23, i.bottom() - 2),
        0xFFE0_6A2E,
        "so is the foot"
    );
    assert!(c.get(i.x, i.y) >> 24 < 0xFF, "its corners are rounded");
}

#[test]
fn without_its_picture_the_app_icon_fills_the_slot_unbadged() {
    let app = swatch(Color::rgb(0x2E, 0x7D, 0xE0), 46);
    let (lay, c) = picture_sheet(&app, Some(&app), false);
    let (i, b) = (lay.icon, lay.badge.unwrap());
    assert_eq!(c.get(i.x + 23, i.y + 23), 0xFF2E_7DE0);
    assert_eq!(c.get(b.x + 9, b.y), 0xFF2E_7DE0, "no badge ring");
}

#[test]
fn a_click_in_the_reply_field_places_the_caret() {
    let (theme, t) = (theme_for(Interface::Glass), text());
    let acts = ["Reply"];
    let c = SheetContent {
        actions: &acts,
        reply: Some(REPLY),
        ..content("Chat", "Ana", "Lunch?")
    };
    let lay = lozenge(&c, true);
    let f = lay.parts.field.unwrap();
    let mut reply = ReplyField {
        edit: TextEdit::new("Sure thing", false),
        focused: false,
    };
    sheet_click(&lay, &theme, &t, &mut reply, f.x + 2);
    assert!(reply.focused);
    assert_eq!(reply.edit.caret(), 0);
    sheet_click(&lay, &theme, &t, &mut reply, f.right() - 2);
    assert_eq!(reply.edit.caret(), "Sure thing".len());
    let sure = t.measure("Sure", false, lay.body_px);
    sheet_click(&lay, &theme, &t, &mut reply, f.x + 13 + sure);
    assert_eq!(
        reply.edit.caret(),
        4,
        "14 px text from inside the rounded end"
    );
}

#[test]
fn a_balloons_parts_are_its_rects() {
    let lay = notify_layout(
        &Theme::default(),
        &text(),
        "t",
        "b",
        &["Open", "Later"],
        &[],
        None,
    );
    let p = lay.parts();
    assert_eq!(
        (p.close, &p.buttons, p.more),
        (lay.close, &lay.buttons, lay.more)
    );
}

#[test]
fn every_sheet_is_a_live_alert_naming_its_parts() {
    let t = text();
    let acts = ["Reply", "Later"];
    let lay = lozenge(
        &SheetContent {
            actions: &acts,
            ..content("Chat", "Ana", "Lunch?")
        },
        false,
    );
    let says = NotifySays {
        app: "Chat",
        title: "Ana",
        body: "Lunch?",
        actions: &acts,
        opens: false,
        urgent: false,
    };
    let mut tree = Tree::window("", (lay.width, lay.height));
    let alert = access_sheet(1, &lay, (0, 0), &says, None, &t, 12, &mut tree, ROOT);
    let n = tree.get(alert).unwrap();
    assert_eq!(n.role(), Role::Alert);
    assert_eq!(n.label(), Some("Chat: Ana. Lunch?"));
    assert_eq!(n.live(), Some(Live::Polite));
    assert_eq!(n.bounds(), Some(crate::access::bounds(lay.sheet)));
    let kids: Vec<(Role, String)> = n
        .children()
        .iter()
        .map(|&k| {
            let k = tree.get(k).unwrap();
            (
                k.role(),
                k.label().or(k.value()).unwrap_or_default().to_string(),
            )
        })
        .collect();
    let named = |r: Role, s: &str| (r, s.to_string());
    assert_eq!(
        kids,
        [
            named(Role::Label, "Ana"),
            named(Role::Label, "Lunch?"),
            named(Role::Button, "Reply"),
            named(Role::Button, "Later"),
            named(Role::Button, "Close"),
        ]
    );
    let mut tree = Tree::window("", (800, 600));
    let alert = access_sheet(1, &lay, (300, 200), &says, None, &t, 12, &mut tree, ROOT);
    let moved = Rect::new(lay.sheet.x + 300, lay.sheet.y + 200, 360, lay.sheet.height);
    assert_eq!(
        tree.get(alert).unwrap().bounds(),
        Some(crate::access::bounds(moved)),
        "at the sheet canvas's offset"
    );
}

fn swatch(c: Color, side: i32) -> Canvas {
    let mut k = Canvas::new(side as u32, side as u32);
    k.fill_rect(Rect::new(0, 0, side, side), c);
    k
}

fn pin_case(case: usize, theme: &Theme, t: &TextRenderer) -> Canvas {
    let s = theme.scale;
    let acts = ["Reply", "Later"];
    let base = content("Chat", "Ana", "Lunch at noon? The usual place.");
    let (c, expanded, hot) = match case {
        0 => (base, false, None),
        1 => (base, false, Some(NotifyHit::Close)),
        2 => (
            SheetContent {
                actions: &acts,
                has_picture: true,
                ..base
            },
            false,
            Some(NotifyHit::Button(0)),
        ),
        _ => (
            SheetContent {
                actions: &acts,
                reply: Some(REPLY),
                ..base
            },
            true,
            Some(NotifyHit::Field),
        ),
    };
    let lay = sheet_layout(theme, t, SheetShape::Lozenge, &c, expanded).unwrap();
    let app = swatch(Color::rgb(0x2E, 0x7D, 0xE0), 46 * s);
    let picture = swatch(Color::rgb(0xE0, 0x6A, 0x2E), 46 * s);
    let icons = SheetIcons {
        app: Some(&app),
        picture: c.has_picture.then_some(&picture),
        actions: &[],
    };
    let typed = TextEdit::new("Sounds good", false);
    let reply = (case == 3).then_some((&typed, Caret::Shown, None));
    draw_sheet(theme, t, &lay, icons, None, None, hot, reply)
}

const CASES: [&str; 4] = [
    "resting",
    "hot close",
    "two actions, picture with badge",
    "reply row",
];

const LOZENGE_PINS: [u64; 16] = [
    0x841f8deb53b6c29b,
    0x139b3f3dd55a2564,
    0x8ece672977df1c73,
    0x37440ad983172b73,
    0x27f9c499ad78b0b4,
    0x00fc97110915fe6c,
    0xc15a3106b27fa121,
    0x53ffc6ce7a94c962,
    0xeb8e2b0c83168d0b,
    0x6ee3b1fbd69cf289,
    0x5cfa4f92b72ef9b1,
    0x4ebec1f115eaee42,
    0xaf867fd881b60a18,
    0x89a6a152c4b44b12,
    0x32c233c4d2699634,
    0xee1ac479ac4980fe,
];

#[test]
fn lozenge_pins() {
    if !crate::text::test_tahoma() {
        return;
    }
    let t = text();
    let mut got = Vec::new();
    for (case, name) in CASES.iter().enumerate() {
        for dark in [false, true] {
            for s in [1, 2] {
                let c = pin_case(case, &glass(dark, s), &t);
                got.push((format!("{name} dark={dark} x{s}"), fnv(&c)));
            }
        }
    }
    let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
    for (i, (case, h)) in got.iter().enumerate() {
        assert_eq!(*h, LOZENGE_PINS[i], "{case}; now:\n{}", now.join("\n"));
    }
}

#[test]
fn the_sheet_canvas_is_the_layout_and_its_corners_are_clear() {
    for s in [1, 2] {
        let theme = glass(false, s);
        let t = text();
        let lay = sheet_layout(
            &theme,
            &t,
            SheetShape::Lozenge,
            &content("a", "b", "c"),
            false,
        )
        .unwrap();
        let c = draw_sheet(
            &theme,
            &t,
            &lay,
            SheetIcons::default(),
            None,
            None,
            None,
            None,
        );
        assert_eq!((c.width as i32, c.height as i32), (lay.width, lay.height));
        assert_eq!(lay.sheet.width, 360 * s);
        let corner = c.get(lay.sheet.x, lay.sheet.y);
        assert_eq!(
            corner & 0xFF_FFFF,
            0,
            "x{s}: only shadow outside the corner"
        );
        let close = lay.parts.close;
        let hot = draw_sheet(
            &theme,
            &t,
            &lay,
            SheetIcons::default(),
            None,
            None,
            Some(NotifyHit::Body),
            None,
        );
        let mid = (close.x + close.width / 2, close.y + close.height / 2);
        assert_ne!(
            c.get(mid.0, mid.1),
            hot.get(mid.0, mid.1),
            "x{s}: the close button shows on hover"
        );
    }
}

#[test]
fn classic_and_blue_have_no_sheet_layout() {
    for i in [Interface::Classic, Interface::Blue] {
        assert!(!has_sheets(&theme_for(i)));
        assert!(
            sheet_layout(
                &theme_for(i),
                &text(),
                SheetShape::Lozenge,
                &content("a", "b", "c"),
                false
            )
            .is_none()
        );
    }
    assert!(has_sheets(&theme_for(Interface::Glass)));
}

#[test]
fn the_chip_reads_plus_n_more_and_fits_its_label() {
    assert_eq!(chip_label(2), "+2 more");
    let (theme, t) = (theme_for(Interface::Glass), text());
    let (w, h) = chip_layout(&theme, &t, 12);
    assert_eq!(h, 28);
    assert!(w > t.measure("+12 more", false, 12));
    let c = draw_chip(&theme, &t, 12, false);
    assert_eq!((c.width as i32, c.height as i32), (w, h));
    assert_ne!(fnv(&c), fnv(&draw_chip(&theme, &t, 12, true)), "hot lifts");
    let (w2, h2) = chip_layout(&theme.scaled(2), &t, 12);
    assert_eq!((w2 % 2, h2), (0, 2 * h), "whole design px at scale 2");
}

#[test]
fn the_stack_grows_away_from_the_tray_on_every_edge() {
    let u = Rect::new(0, 0, 1920, 1050);
    let three = [(360, 96); 3];
    let chip = Some((90, 28));
    assert_eq!(
        place_stack(u, &three, chip, Edge::Bottom),
        (
            vec![(1552, 946), (1552, 842), (1552, 738)],
            Some((1822, 702))
        )
    );
    assert_eq!(
        place_stack(u, &three, chip, Edge::Right).0,
        vec![(1552, 946), (1552, 842), (1552, 738)]
    );
    assert_eq!(
        place_stack(u, &three, None, Edge::Left).0,
        vec![(8, 946), (8, 842), (8, 738)]
    );
    assert_eq!(
        place_stack(u, &three, chip, Edge::Top),
        (vec![(1552, 8), (1552, 112), (1552, 216)], Some((1822, 320)))
    );
}

#[test]
fn a_stack_too_tall_stays_inside_the_usable_area() {
    let u = Rect::new(0, 30, 800, 300);
    let (at, _) = place_stack(u, &[(360, 200); 3], None, Edge::Bottom);
    assert!(at.iter().all(|&(_, y)| y >= 30), "{at:?}");
}

#[test]
fn an_arriving_sheet_rises_14_px_and_fades_in() {
    let rest = Rect::new(100, 500, 360, 120);
    assert_eq!(
        pose(0.0, rest, RISE, 1, 0.95),
        (Rect::new(109, 517, 342, 114), 0.0)
    );
    assert_eq!(pose(1.0, rest, RISE, 1, 0.95), (rest, 1.0));
    assert_eq!(
        pose(1.3, rest, RISE, 1, 0.95),
        (rest, 1.0),
        "k past 1 is clamped"
    );
}

#[test]
fn under_a_top_bar_the_sheet_rises_from_above() {
    assert_eq!(
        pose(0.0, Rect::new(100, 500, 360, 120), RISE, -1, 0.95),
        (Rect::new(109, 489, 342, 114), 0.0)
    );
}

#[test]
fn the_lozenge_style_draws_the_lozenge_shape() {
    assert_eq!(glass::NotifyStyle::Lozenge.shape(), SheetShape::Lozenge);
}

fn halo(c: &SheetContent) -> SheetLayout {
    sheet_layout(
        &theme_for(Interface::Glass),
        &text(),
        SheetShape::Halo,
        c,
        false,
    )
    .unwrap()
}

#[test]
fn the_halo_style_draws_the_halo_shape() {
    assert_eq!(glass::NotifyStyle::Halo.shape(), SheetShape::Halo);
}

#[test]
fn the_halo_icon_stands_half_outside_the_sheet() {
    let lay = halo(&content("Mail", "New", "Hi"));
    assert_eq!(
        lay.icon,
        Rect::new(lay.sheet.x + 9, lay.sheet.y - 25, 50, 50)
    );
    assert_eq!(
        (lay.footprint.y, lay.footprint.bottom()),
        (lay.icon.y, lay.sheet.bottom())
    );
    assert_eq!(
        (lay.sheet.width, lay.well, lay.badge),
        (360, Well::Squircle, None)
    );
}

#[test]
fn the_breakout_takes_the_pointer() {
    let lay = halo(&content("Mail", "New", "Hi"));
    assert_eq!(
        sheet_hit(&lay, lay.icon.x + 25, lay.icon.y + 5),
        Some(NotifyHit::Body)
    );
    assert_eq!(
        sheet_hit(&lay, lay.sheet.right() - 40, lay.icon.y + 5),
        None,
        "beside the well, above the sheet"
    );
}

#[test]
fn a_halo_sender_picture_sits_beside_the_title() {
    let with = halo(&SheetContent {
        has_picture: true,
        ..content("Chat", "Ana", "Lunch?")
    });
    let p = with.picture.unwrap();
    assert_eq!(
        (p.width, p.height, with.parts.title.x),
        (28, 28, p.right() + 8)
    );
    assert_eq!((p.x, p.y), (with.sheet.x + 18, with.parts.title.y));
    assert_eq!(with.parts.body.x, with.parts.title.x);
    assert_eq!(with.badge, None, "the app icon keeps the well");
    assert_eq!(
        halo(&content("Chat", "Ana", "Lunch?")).parts.title.x,
        with.sheet.x + 18
    );
}

#[test]
fn the_halo_text_runs_full_width_under_the_app_line() {
    let lay = halo(&content("Mail", "New", "Hi"));
    let (sh, t) = (lay.sheet, lay.parts.title);
    assert_eq!(lay.app_box.x, sh.x + 69);
    assert_eq!(lay.app_box.y, sh.y + 8);
    assert_eq!((t.y, t.right()), (sh.y + 33, sh.right() - 18));
    assert_eq!(lay.parts.body.right(), sh.right() - 18);
    assert_eq!((lay.app_px, lay.title_px, lay.body_px), (12, 15, 14));
    assert_eq!(
        lay.parts.close,
        Rect::new(sh.right() - 36, sh.y + 13, 20, 20)
    );
    assert!(
        !lay.parts.close.intersects(t),
        "the title row clears the close button"
    );
    assert_eq!(sh.bottom(), lay.parts.body.bottom() + 18);
}

#[test]
fn halo_pills_span_the_sheet_under_the_body() {
    let acts = ["Open", "Later"];
    let lay = halo(&SheetContent {
        actions: &acts,
        has_picture: true,
        ..content("Mail", "New", "Hi")
    });
    let b = lay.parts.buttons[1];
    assert_eq!(b.right(), lay.sheet.right() - 18);
    assert!(lay.parts.buttons[0].y > lay.picture.unwrap().bottom());
    assert_eq!(lay.sheet.bottom(), b.bottom() + 18);
    let c = SheetContent {
        actions: &acts,
        reply: Some(REPLY),
        ..content("Chat", "Ana", "Lunch?")
    };
    let open = sheet_layout(
        &theme_for(Interface::Glass),
        &text(),
        SheetShape::Halo,
        &c,
        true,
    )
    .unwrap();
    assert_eq!(open.parts.field.unwrap().x, open.sheet.x + 18);
}

fn halo_case(case: usize, theme: &Theme, t: &TextRenderer) -> Canvas {
    let s = theme.scale;
    let acts = ["Reply", "Later"];
    let base = content("Chat", "Ana", "Lunch at noon? The usual place.");
    let (red, blue) = (Color::rgb(0xD0, 0x30, 0x30), Color::rgb(0x30, 0x60, 0xD0));
    let (c, hot, tint) = match case {
        0 => (base, None, None),
        1 => (base, Some(NotifyHit::Close), None),
        2 => (
            SheetContent {
                actions: &acts,
                ..base
            },
            Some(NotifyHit::Button(0)),
            None,
        ),
        3 => (
            SheetContent {
                has_picture: true,
                ..base
            },
            None,
            None,
        ),
        4 => (base, None, Some(red)),
        _ => (base, None, Some(blue)),
    };
    let lay = sheet_layout(theme, t, SheetShape::Halo, &c, false).unwrap();
    let app = disc(tint.unwrap_or(Color::rgb(0x80, 0x80, 0x80)), 48 * s);
    let picture = swatch(Color::rgb(0xE0, 0x6A, 0x2E), 28 * s);
    let icons = SheetIcons {
        app: Some(&app),
        picture: c.has_picture.then_some(&picture),
        actions: &[],
    };
    draw_sheet(theme, t, &lay, icons, tint, None, hot, None)
}

/// A round icon, clear outside the disc as a theme icon is.
fn disc(c: Color, side: i32) -> Canvas {
    let r = f64::from(side) * 0.42;
    let mid = f64::from(side) / 2.0;
    crate::shape::draw(side, side, 1, |x, y| {
        scale(
            crate::shape::premul(c, 255),
            crate::shape::ellipse(x, y, mid, mid, r, r),
        )
    })
}

const HALO_CASES: [&str; 6] = [
    "resting",
    "hot close",
    "two actions",
    "picture",
    "red tint",
    "blue tint",
];

const HALO_PINS: [u64; 24] = [
    0xc87baade66c86c50,
    0xc15e81e1b26ed426,
    0x08f7453f23193b67,
    0x6f1a7ab14cccb98e,
    0xd18bdae0cbfce7dd,
    0x662a91cdd0d9eb67,
    0xb089ae5e075f0bc6,
    0x26fb58f810a4bf27,
    0x8bc0bdcefce6bda7,
    0x526bde193ecf3240,
    0x2b96f63ca064b37a,
    0xfbc687a88aa59f4d,
    0x6b3812eb5fca657b,
    0xd4e27dd1ea4ef6f3,
    0xa58186a1c834bce6,
    0x0c69f2445e672382,
    0x9d409da44a21dcf5,
    0xf6c935a74d8c9579,
    0x62ff16259d512b5e,
    0x53d25fb60cd67965,
    0x2cafd025beeb7306,
    0xa1bad3ac5ac3cf29,
    0x4120aa3ac20c5e05,
    0x0c4ac6dca907400a,
];

#[test]
fn halo_pins() {
    if !crate::text::test_tahoma() {
        return;
    }
    let t = text();
    let mut got = Vec::new();
    for (case, name) in HALO_CASES.iter().enumerate() {
        for dark in [false, true] {
            for s in [1, 2] {
                let c = halo_case(case, &glass(dark, s), &t);
                got.push((format!("{name} dark={dark} x{s}"), fnv(&c)));
            }
        }
    }
    let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
    for (i, (case, h)) in got.iter().enumerate() {
        assert_eq!(*h, HALO_PINS[i], "{case}; now:\n{}", now.join("\n"));
    }
}

#[test]
fn a_halo_canvas_is_its_layout_with_the_icon_in_the_well() {
    for s in [1, 2] {
        let theme = glass(false, s);
        let t = text();
        let c = SheetContent {
            has_picture: true,
            ..content("Chat", "Ana", "Lunch?")
        };
        let lay = sheet_layout(&theme, &t, SheetShape::Halo, &c, false).unwrap();
        let blue = Color::rgb(0x2E, 0x7D, 0xE0);
        let (app, pic) = (
            swatch(blue, 50 * s),
            swatch(Color::rgb(0xE0, 0x6A, 0x2E), 28 * s),
        );
        let icons = SheetIcons {
            app: Some(&app),
            picture: Some(&pic),
            actions: &[],
        };
        let drawn = draw_sheet(&theme, &t, &lay, icons, Some(blue), None, None, None);
        assert_eq!(
            (drawn.width as i32, drawn.height as i32),
            (lay.width, lay.height)
        );
        let i = lay.icon;
        assert_eq!(drawn.get(i.x + 25 * s, i.y + 4 * s), 0xFF2E_7DE0, "x{s}");
        let p = lay.picture.unwrap();
        assert_eq!(drawn.get(p.x + 14 * s, p.y + 14 * s), 0xFFE0_6A2E, "x{s}");
        assert_ne!(drawn.get(p.x, p.y), 0xFFE0_6A2E, "x{s}: a round picture");
    }
}

#[test]
#[ignore]
fn render_halo_sheets() {
    let t = text();
    for dark in [false, true] {
        for s in [1, 2] {
            for case in 0..HALO_CASES.len() {
                let c = halo_case(case, &glass(dark, s), &t);
                let mode = if dark { "dark" } else { "light" };
                crate::raster::write_preview(
                    &on_backdrop(&c),
                    &format!("notifysheet-halo-{case}-{mode}-x{s}.png"),
                );
            }
        }
    }
}

fn on_backdrop(c: &Canvas) -> Canvas {
    let mut bg = Canvas::new(c.width, c.height);
    let w = c.width as i32;
    bg.fill_rect(
        Rect::new(0, 0, w / 2, c.height as i32),
        Color::rgb(0x3A, 0x6E, 0xA5),
    );
    bg.fill_rect(
        Rect::new(w / 2, 0, w - w / 2, c.height as i32),
        Color::rgb(0xC8, 0xD4, 0xE0),
    );
    bg.blit_canvas(0, 0, c);
    bg
}

fn droplet(c: &SheetContent, expanded: bool) -> SheetLayout {
    sheet_layout(
        &theme_for(Interface::Glass),
        &text(),
        SheetShape::Droplet,
        c,
        expanded,
    )
    .unwrap()
}

const LONG_ACTS: [&str; 2] = ["Reply", "Later"];

fn long() -> SheetContent<'static> {
    SheetContent {
        actions: &LONG_ACTS,
        ..content(
            "Chat",
            "Ana",
            "Lunch at noon?\nThe usual place by the river.\nBring the photos.",
        )
    }
}

#[test]
fn a_collapsed_droplet_is_one_68_px_row() {
    let lay = droplet(&long(), false);
    assert_eq!(lay.sheet.height, 68);
    assert_eq!(lay.lines.len(), 1, "body ellipsized");
    assert!(lay.parts.buttons.is_empty());
}

#[test]
fn hover_expands_to_the_wrapped_body_and_actions() {
    let lay = droplet(&long(), true);
    assert_eq!((lay.lines.len(), lay.parts.buttons.len()), (3, 2));
    assert!(lay.sheet.height > 68);
}

#[test]
fn the_droplet_picture_sits_at_the_right_end() {
    let lay = droplet(
        &SheetContent {
            has_picture: true,
            ..long()
        },
        false,
    );
    assert_eq!(
        lay.picture,
        Some(Rect::new(lay.sheet.right() - 64, lay.sheet.y + 10, 48, 48))
    );
}

#[test]
fn the_droplet_icon_slot_is_round_48() {
    let lay = droplet(&long(), false);
    assert_eq!(
        lay.icon,
        Rect::new(lay.sheet.x + 10, lay.sheet.y + 10, 48, 48)
    );
    assert_eq!(lay.well, Well::Round);
    assert_eq!(lay.badge, None);
}

#[test]
fn an_opening_droplet_lerps_its_height() {
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    assert_eq!(open_layout(&c, &e, 0.0).sheet.height, 68);
    assert_eq!(
        open_layout(&c, &e, 0.5).sheet.height,
        (f64::from(68 + e.sheet.height) / 2.0).round() as i32
    );
    assert!(open_layout(&c, &e, 0.0).parts.buttons.is_empty());
    assert_eq!(open_layout(&c, &e, 1.0).sheet, e.sheet);
}

#[test]
fn the_droplet_text_column_ends_short_of_the_close_and_picture() {
    let plain = droplet(&long(), false);
    assert_eq!(plain.parts.body.x, plain.sheet.x + 68);
    let close = plain.parts.close;
    assert_eq!(plain.parts.body.right(), close.x - 4);
    assert_eq!(plain.parts.title.right(), close.x - 4);
    let open = droplet(&long(), true);
    assert_eq!(open.parts.body.right(), open.sheet.right() - 34);
    assert_eq!(
        open.parts.title, plain.parts.title,
        "the head row keeps its end"
    );
    let pic = droplet(
        &SheetContent {
            has_picture: true,
            ..long()
        },
        false,
    );
    let p = pic.picture.unwrap();
    assert_eq!(pic.parts.close.right() + 4, p.x);
    assert_eq!(pic.parts.body.right(), pic.parts.close.x - 4);
}

#[test]
fn the_droplet_close_never_covers_the_picture() {
    for expanded in [false, true] {
        let lay = droplet(
            &SheetContent {
                has_picture: true,
                ..long()
            },
            expanded,
        );
        let (c, p) = (lay.parts.close, lay.picture.unwrap());
        assert!(
            c.intersect(&p).is_empty(),
            "expanded={expanded}: {c:?} on {p:?}"
        );
    }
}

#[test]
fn a_body_cut_at_the_line_cap_ends_in_an_ellipsis() {
    let long = "word ".repeat(400);
    let c = content("Mail", "t", &long);
    for lay in [lozenge(&c, false), halo(&c), droplet(&c, true)] {
        assert_eq!(lay.lines.len(), NOTIFY_BODY_LINES);
        let last = lay.lines.last().unwrap();
        assert!(last.ends_with("..."), "{:?}: {last:?}", lay.shape);
        assert!(text().measure(last, false, lay.body_px) <= lay.parts.body.width);
    }
    let fits = lozenge(&content("Mail", "t", "word word"), false);
    assert_eq!(fits.lines, ["word word"]);
}

#[test]
fn the_droplet_close_spot_moves_up_when_expanded() {
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    assert_eq!(
        c.parts.close,
        Rect::new(c.sheet.right() - 54, c.sheet.y + 24, 20, 20)
    );
    assert_eq!(
        e.parts.close,
        Rect::new(e.sheet.right() - 34, e.sheet.y + 12, 20, 20)
    );
    let x = c.parts.close;
    assert_eq!(sheet_hit(&c, x.x + 2, x.y + 2), Some(NotifyHit::Close));
}

#[test]
fn a_bodyless_droplet_centres_its_title_on_the_icon() {
    let lay = droplet(&content("Mail", "Synced", ""), false);
    let (t, i) = (lay.parts.title, lay.icon);
    assert_eq!(t.y + t.height / 2, i.y + i.height / 2);
    assert!(lay.lines.is_empty());
}

#[test]
fn the_droplet_title_is_15_px() {
    let lay = droplet(&long(), false);
    assert_eq!((lay.app_px, lay.title_px, lay.body_px), (12, 15, 14));
}

#[test]
fn an_opening_droplet_draws_its_ends_at_0_and_1() {
    let (theme, t) = (glass(false, 1), text());
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    let icons = SheetIcons::default();
    let draw = |lay: &SheetLayout| draw_sheet(&theme, &t, lay, icons, None, None, None, None);
    let open = |k: f64| draw_sheet_opening(&theme, &t, &c, &e, k, icons, None, None, None, None);
    assert_eq!(
        fnv(&open(0.0)),
        fnv(&draw(&c)),
        "t = 0 is the collapsed sheet"
    );
    assert_eq!(
        fnv(&open(1.0)),
        fnv(&draw(&e)),
        "t = 1 is the expanded sheet"
    );
    let half = open(0.5);
    assert_eq!(half.height as i32, open_layout(&c, &e, 0.5).height);
}

#[test]
fn an_opening_droplet_draws_no_part_it_has_dropped() {
    let (theme, t) = (glass(false, 1), text());
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    let open = open_layout(&c, &e, 0.5);
    let pill = e.parts.buttons[0];
    assert!(open.parts.buttons.is_empty() && pill.y < open.sheet.bottom());
    let hot = Some(NotifyHit::Body);
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    let icons = SheetIcons::default();
    let drawn = opening.draw(
        &mut kept, &theme, &t, &c, &e, 0.5, icons, None, None, hot, None,
    );
    let key = &opening.layers.as_ref().unwrap().key;
    let glass = glass_at(&kept, key, true, open.sheet, 1);
    assert_eq!((drawn.width, drawn.height), (glass.width, glass.height));
    for y in 0..drawn.height as i32 {
        for x in 0..drawn.width as i32 {
            let off_sheet = !open.sheet.contains(x, y);
            let under_pill = (pill.x..pill.right()).contains(&x) && y >= pill.y;
            if off_sheet || under_pill {
                assert_eq!(drawn.get(x, y), glass.get(x, y), "({x}, {y}) is glass");
            }
        }
    }
}

fn warm(opening: &mut SheetOpening, kept: &mut SheetGlass, c: &SheetContent, lit: bool) -> bool {
    let (theme, t) = (glass(false, 1), text());
    let (shut, open) = (droplet(c, false), droplet(c, true));
    let icons = SheetIcons::default();
    opening.warm(kept, &theme, &t, &shut, &open, icons, None, None, lit)
}

#[test]
fn a_droplet_warmed_at_rest_opens_under_any_hot_part_without_a_build() {
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    assert!(
        warm(&mut opening, &mut kept, &long(), true),
        "the first warm paints"
    );
    assert!(
        !warm(&mut opening, &mut kept, &long(), true),
        "a second finds it warm"
    );
    let (theme, t) = (glass(false, 1), text());
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    let hots = [NotifyHit::Body, NotifyHit::Close, NotifyHit::Button(0)];
    for (k, hot) in [0.1, 0.35, 0.6, 0.9]
        .into_iter()
        .zip(hots.into_iter().cycle())
    {
        let icons = SheetIcons::default();
        opening.draw(
            &mut kept,
            &theme,
            &t,
            &c,
            &e,
            k,
            icons,
            None,
            Some(hot),
            Some(hot),
            None,
        );
    }
    assert_eq!(opening.builds, 1, "hover and presses reuse the warm layers");
    let other = content("Chat", "Ben", "Running late, start without me.");
    assert!(
        warm(&mut opening, &mut kept, &other, true),
        "new content paints once"
    );
    assert!(!warm(&mut opening, &mut kept, &other, true));
    assert_eq!(opening.builds, 2);
}

fn warm_off_thread(
    opening: &mut SheetOpening,
    kept: &mut SheetGlass,
    theme: &Theme,
    c: &SheetContent,
    lit: bool,
) -> Option<()> {
    let lay = |expanded| sheet_layout(theme, &text(), SheetShape::Droplet, c, expanded);
    let (shut, open) = (lay(false).unwrap(), lay(true).unwrap());
    let job = opening.warm_job(kept, theme, &shut, &open, SheetIcons::default(), None, lit)?;
    let built =
        std::thread::spawn(move || build_warm(job, &TextRenderer::load(&FontConfig::default())))
            .join()
            .unwrap();
    opening.install(kept, built);
    Some(())
}

#[test]
fn a_droplet_warmed_on_a_worker_paints_nothing_on_the_callers_thread() {
    let (theme, t) = (glass(false, 1), text());
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    let icons = SheetIcons::default();
    let ask = |opening: &mut SheetOpening| {
        opening
            .warm_job(&kept, &theme, &c, &e, icons, None, true)
            .is_some()
    };
    assert!(ask(&mut opening));
    assert!(!ask(&mut opening), "one job out per key and rim");
    opening.forget_job();
    assert!(
        ask(&mut opening),
        "a job lost with its worker is asked again"
    );
    opening.job_failed();
    assert!(!ask(&mut opening), "a job that panicked is not asked again");
    let other = |opening: &mut SheetOpening| {
        let lay = |expanded| droplet(&content("Chat", "Ben", "Late."), expanded);
        let job = opening.warm_job(&kept, &theme, &lay(false), &lay(true), icons, None, true);
        job.is_some()
    };
    assert!(other(&mut opening), "other content is");
    let mut opening = SheetOpening::default();
    warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).unwrap();
    assert!(
        warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).is_none(),
        "warm already"
    );
    for (k, pushed, hot) in [
        (0.2, None, Some(NotifyHit::Body)),
        (0.5, Some(NotifyHit::Close), Some(NotifyHit::Close)),
        (1.0, None, Some(NotifyHit::Button(0))),
    ] {
        opening.draw(
            &mut kept, &theme, &t, &c, &e, k, icons, None, pushed, hot, None,
        );
    }
    warm_off_thread(&mut opening, &mut kept, &theme, &long(), false).unwrap();
    for k in [0.7, 0.3, 0.0] {
        opening.draw(
            &mut kept, &theme, &t, &c, &e, k, icons, None, None, None, None,
        );
    }
    assert_eq!(
        (opening.builds(), kept.builds()),
        (0, 0),
        "the worker painted the parts and both rims"
    );
}

#[test]
fn a_droplet_keeps_its_glass_at_rest_and_the_ends_of_one_rim() {
    let (theme, t) = (glass(false, 1), text());
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    let icons = SheetIcons::default();
    let frame = |opening: &mut SheetOpening, kept: &mut SheetGlass, k: f64, lit: bool| {
        let hot = lit.then_some(NotifyHit::Body);
        opening.draw(kept, &theme, &t, &c, &e, k, icons, None, None, hot, None);
        let rims: Vec<bool> = kept.kept.iter().map(|(k, _)| k.lit()).collect();
        rims
    };
    assert_eq!(frame(&mut opening, &mut kept, 0.0, false), [false]);
    warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).unwrap();
    assert_eq!(
        kept.kept.len(),
        3,
        "at rest, and both ends lit for the pointer"
    );
    assert_eq!(frame(&mut opening, &mut kept, 0.5, true), [true, true]);
    assert_eq!(frame(&mut opening, &mut kept, 1.0, true), [true]);
    warm_off_thread(&mut opening, &mut kept, &theme, &long(), false).unwrap();
    assert_eq!(
        kept.kept.len(),
        3,
        "at rest, and both ends plain for the close"
    );
    assert_eq!(frame(&mut opening, &mut kept, 0.5, false), [false, false]);
}

#[test]
fn an_app_line_ageing_keeps_a_droplets_layers_and_draws_on_top() {
    let (theme, t) = (glass(false, 2), text());
    let later = SheetContent {
        when: "1 min ago",
        ..long()
    };
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).unwrap();
    assert!(
        warm_off_thread(&mut opening, &mut kept, &theme, &later, true).is_none(),
        "no new job for an older app line"
    );
    let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &later, expanded);
    let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
    let hot = Some(NotifyHit::Body);
    let icons = SheetIcons::default();
    let aged = opening.draw(
        &mut kept, &theme, &t, &c, &e, 0.5, icons, None, None, hot, None,
    );
    let fresh = draw_sheet_opening(&theme, &t, &c, &e, 0.5, icons, None, None, hot, None);
    assert_eq!(opening.builds(), 0);
    assert!(aged == fresh);
}

#[test]
fn a_droplet_parts_layer_painted_in_the_frame_leaves_the_app_line_to_the_top_pass() {
    let (theme, t) = (glass(false, 2), text());
    let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded);
    let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
    let (icons, hot) = (SheetIcons::default(), Some(NotifyHit::Body));
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    let mut frame = |opening: &mut SheetOpening| {
        opening.draw(
            &mut kept, &theme, &t, &c, &e, 0.9, icons, None, None, hot, None,
        )
    };
    frame(&mut opening);
    let warmed = std::mem::take(&mut opening.layers.as_mut().unwrap().open_parts);
    frame(&mut opening);
    let painted = &opening.layers.as_ref().unwrap().open_parts;
    let [(key, layer)] = painted.as_slice() else {
        panic!("one layer painted in the frame");
    };
    let cached = warmed.iter().find(|(k, _)| k == key).unwrap();
    assert!(*layer == cached.1);
}

#[test]
fn a_late_warm_result_for_earlier_content_leaves_the_layers_painted_since() {
    let (theme, t) = (glass(false, 1), text());
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    let icons = SheetIcons::default();
    let job = opening
        .warm_job(&kept, &theme, &c, &e, icons, None, true)
        .unwrap();
    let other = content("Chat", "Ben", "Running late, start without me.");
    let (oc, oe) = (droplet(&other, false), droplet(&other, true));
    let hot = Some(NotifyHit::Body);
    let frame = |opening: &mut SheetOpening, kept: &mut SheetGlass, k: f64| {
        opening.draw(kept, &theme, &t, &oc, &oe, k, icons, None, None, hot, None);
    };
    frame(&mut opening, &mut kept, 0.3);
    assert_eq!(
        opening.builds(),
        1,
        "the new content is painted in the frame"
    );
    opening.install(&mut kept, build_warm(job, &t));
    frame(&mut opening, &mut kept, 0.6);
    assert_eq!(opening.builds(), 1, "the late result left the new layers");
}

#[test]
fn a_frame_from_worker_layers_matches_one_from_layers_painted_in_place() {
    let (theme, t) = (glass(false, 2), text());
    let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded);
    let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
    let icons = SheetIcons::default();
    let hot = Some(NotifyHit::Button(0));
    let (mut there, mut there_glass) = (SheetOpening::default(), SheetGlass::default());
    warm_off_thread(&mut there, &mut there_glass, &theme, &long(), true).unwrap();
    let (mut here, mut here_glass) = (SheetOpening::default(), SheetGlass::default());
    let a = there.draw(
        &mut there_glass,
        &theme,
        &t,
        &c,
        &e,
        0.6,
        icons,
        None,
        None,
        hot,
        None,
    );
    let b = here.draw(
        &mut here_glass,
        &theme,
        &t,
        &c,
        &e,
        0.6,
        icons,
        None,
        None,
        hot,
        None,
    );
    assert!(a == b);
}

#[test]
fn lit_parts_laid_on_the_unlit_layer_match_parts_drawn_lit() {
    let (theme, t) = (glass(false, 2), text());
    let look = notify_look(&theme).unwrap();
    let e = sheet_layout(&theme, &t, SheetShape::Droplet, &long(), true).unwrap();
    let icons = SheetIcons::default();
    let clear = || Canvas::new(e.width as u32, e.height as u32);
    for (pushed, hot) in [
        (None, Some(NotifyHit::Body)),
        (None, Some(NotifyHit::Button(1))),
        (Some(NotifyHit::Close), Some(NotifyHit::Close)),
    ] {
        let mut want = clear();
        draw_parts(&mut want, &theme, &t, &look, &e, icons, pushed, hot, None);
        let mut got = clear();
        draw_parts(&mut got, &theme, &t, &look, &e, icons, None, None, None);
        relight(&mut got, &t, &look, &e, &[], true, pushed, hot, 2);
        assert!(got == want, "{hot:?} pushed {pushed:?}");
    }
}

#[test]
fn an_opening_droplets_glass_stays_within_6_levels_of_the_glass_painted_at_its_height() {
    let (theme, t) = (glass(false, 1), text());
    let look = notify_look(&theme).unwrap();
    let (c, e) = (droplet(&long(), false), droplet(&long(), true));
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    let icons = SheetIcons::default();
    opening.warm(&mut kept, &theme, &t, &c, &e, icons, None, None, true);
    let key = &opening.layers.as_ref().unwrap().key;
    for k in [0.25, 0.5, 0.75] {
        let open = open_layout(&c, &e, k);
        let (got, want) = (
            glass_at(&kept, key, true, open.sheet, 1),
            (look.sheet)(&look, &sheet_spec(&open, None, true, 1), 1),
        );
        assert_eq!((got.width, got.height), (want.width, want.height));
        let worst = got
            .pixels
            .iter()
            .zip(&want.pixels)
            .flat_map(|(&a, &b)| {
                (0..4).map(move |i| ((a >> (8 * i)) & 0xFF).abs_diff((b >> (8 * i)) & 0xFF))
            })
            .max();
        assert!(worst <= Some(6), "t = {k}: off by {worst:?}");
    }
}

/// Prints what warming an opening at rest, its first frame and later ones cost at scales 1
/// and 2. Run with `--release -- --ignored --nocapture`.
#[test]
#[ignore = "timing"]
fn opening_frame_cost() {
    let t = text();
    for s in [1, 2] {
        let theme = glass(false, s);
        let lay =
            |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded).unwrap();
        let (c, e) = (lay(false), lay(true));
        let hot = Some(NotifyHit::Body);
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let at = std::time::Instant::now();
        let icons = SheetIcons::default();
        let job = opening
            .warm_job(&kept, &theme, &c, &e, icons, None, true)
            .unwrap();
        let asked = at.elapsed();
        let built = build_warm(job, &t);
        let warm = at.elapsed() - asked;
        let at = std::time::Instant::now();
        opening.install(&mut kept, built);
        let installed = at.elapsed();
        let mut frame = |k: f64| {
            let at = std::time::Instant::now();
            opening.draw(
                &mut kept,
                &theme,
                &t,
                &c,
                &e,
                k,
                SheetIcons::default(),
                None,
                None,
                hot,
                None,
            );
            at.elapsed()
        };
        let first = frame(0.05);
        let ticks: Vec<_> = (1..=9).map(|i| frame(f64::from(i) / 10.0)).collect();
        let worst = ticks.iter().max().unwrap();
        let last = frame(1.0);
        println!(
            "scale {s}: job made {asked:?}, warm on a worker {warm:?}, installed {installed:?}, first frame {first:?}, later frames up to {worst:?}, last frame {last:?}"
        );
    }
}

/// Prints a Droplet's held glass and parts at scale 2, at rest collapsed and open, each warmed
/// for its next move. Run with `--release -- --ignored --nocapture`.
#[test]
#[ignore = "memory"]
fn droplet_memory() {
    let (theme, t) = (glass(false, 2), text());
    let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded);
    let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
    let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
    let icons = SheetIcons::default();
    let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
    let end = |lay: &SheetLayout| lay.width as usize * lay.height as usize * 4;
    let hot = Some(NotifyHit::Body);
    for (k, hot) in [(0.0, None), (1.0, hot)] {
        opening.draw(
            &mut kept, &theme, &t, &c, &e, k, icons, None, None, hot, None,
        );
        warm_off_thread(&mut opening, &mut kept, &theme, &long(), k < 1.0).unwrap();
        println!(
            "at rest t = {k}: glass {:.2} MB, parts {:.2} MB",
            mb(kept.bytes()),
            mb(opening.bytes())
        );
    }
    println!(
        "one end's glass: collapsed {:.2} MB, expanded {:.2} MB",
        mb(end(&c)),
        mb(end(&e))
    );
}

#[test]
fn the_collapsed_ink_is_gone_before_the_expanded_ink_is_half_in() {
    assert_eq!(
        (opening_fades(0.0), opening_fades(1.0)),
        ((1.0, 0.0), (0.0, 1.0))
    );
    let (shut, open) = opening_fades(0.4);
    assert!(shut == 0.0 && open < 0.5);
    for i in 0..=20 {
        let (a, b) = opening_fades(f64::from(i) / 20.0);
        assert!(a + b <= 1.0, "never brighter than either end");
    }
}

fn droplet_case(case: usize, theme: &Theme, t: &TextRenderer) -> Canvas {
    let s = theme.scale;
    let base = long();
    let c = SheetContent {
        has_picture: matches!(case, 3 | 5),
        ..base
    };
    let lay = |expanded| sheet_layout(theme, t, SheetShape::Droplet, &c, expanded).unwrap();
    let app = swatch(Color::rgb(0x2E, 0x7D, 0xE0), 48 * s);
    let picture = swatch(Color::rgb(0xE0, 0x6A, 0x2E), 48 * s);
    let icons = SheetIcons {
        app: Some(&app),
        picture: c.has_picture.then_some(&picture),
        actions: &[],
    };
    match case {
        0 | 3 => draw_sheet(theme, t, &lay(false), icons, None, None, None, None),
        1 => draw_sheet(
            theme,
            t,
            &lay(true),
            icons,
            None,
            None,
            Some(NotifyHit::Body),
            None,
        ),
        2 | 5 => draw_sheet(
            theme,
            t,
            &lay(true),
            icons,
            None,
            None,
            Some(NotifyHit::Close),
            None,
        ),
        _ => draw_sheet_opening(
            theme,
            t,
            &lay(false),
            &lay(true),
            0.5,
            icons,
            None,
            None,
            Some(NotifyHit::Body),
            None,
        ),
    }
}

const DROPLET_CASES: [&str; 6] = [
    "collapsed",
    "expanded",
    "expanded, hot close",
    "picture",
    "opening t = 0.5",
    "expanded, picture, hot close",
];

const DROPLET_PINS: [u64; 24] = [
    0x425bc144996b943f,
    0x1b5e94833245fcf1,
    0xb7ffd9c5ecc872c0,
    0xb9b970e50ca4aeb3,
    0x77a28eba7566d48d,
    0x309fb736c325eb07,
    0x7dd3e1fbb7ea804d,
    0x5541d5a466e923f5,
    0xd60ae18f4120a49b,
    0x55ac48a5b7d76edf,
    0xdd25fb326db09377,
    0x8fcc436ede36624f,
    0xdf9448f5843dc0db,
    0x782d7e3e6ebd8bb2,
    0x635209b4461c9ef5,
    0x1374d93fde986aed,
    0xe2695eb474531d93,
    0x044298801479ee55,
    0x4ea1de53291eb3c7,
    0x3a2d8890877b5201,
    0x54929f8ba950da9e,
    0x7ce3739d49072a81,
    0xd9ea5a9475d49e13,
    0xdd1759115362f434,
];

#[test]
fn droplet_pins() {
    if !crate::text::test_tahoma() {
        return;
    }
    let t = text();
    let mut got = Vec::new();
    for (case, name) in DROPLET_CASES.iter().enumerate() {
        for dark in [false, true] {
            for s in [1, 2] {
                let c = droplet_case(case, &glass(dark, s), &t);
                got.push((format!("{name} dark={dark} x{s}"), fnv(&c)));
            }
        }
    }
    let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
    for (i, (case, h)) in got.iter().enumerate() {
        assert_eq!(*h, DROPLET_PINS[i], "{case}; now:\n{}", now.join("\n"));
    }
}

/// Light then dark, each at scale 1 and 2.
const BEAD_PINS: [u64; 4] = [
    0xc57c0479b1a5e635,
    0xe326070904f5e5f3,
    0xf58e8278f11aff6a,
    0x96f63b889c79bf5f,
];

#[test]
fn bead_pins() {
    let mut got = Vec::new();
    for dark in [false, true] {
        for s in [1, 2] {
            got.push(fnv(&drip_bead(&glass(dark, s)).unwrap().0));
        }
    }
    let now: Vec<String> = got.iter().map(|h| format!("{h:#018x},")).collect();
    assert_eq!(got, BEAD_PINS, "now:\n{}", now.join("\n"));
}

#[test]
fn the_bead_centres_its_round_body_in_clear_corners() {
    let (c, body) = drip_bead(&glass(false, 2)).unwrap();
    let (w, h) = (c.width as i32, c.height as i32);
    assert_eq!(body.width, 24);
    assert_eq!((body.x * 2 + body.width, body.y * 2 + body.height), (w, h));
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
        assert_eq!(c.get(x, y), 0, "({x}, {y}) is clear");
    }
    let alpha = |x: i32, y: i32| c.get(x, y) >> 24;
    let mid = body.x + body.width / 2;
    let corner = alpha(body.x, body.y);
    assert!(
        corner < alpha(mid, mid) / 2,
        "the body square's corner is off the drop"
    );
    let rim = (1..body.width / 2)
        .map(|i| alpha(body.x + i, body.y + i))
        .collect::<Vec<_>>();
    assert!(
        rim.windows(2)
            .any(|p| p[0] < p[1] && p[1] < alpha(mid, mid)),
        "the curved edge is part covered at scale 2: {rim:?}"
    );
}

#[test]
fn the_bead_shows_whole_until_the_swell_and_is_gone_a_third_in() {
    assert_eq!(drip_bead_share(0.0), 1.0);
    assert_eq!(drip_bead_share(BEAD_UNTIL), 1.0);
    let gone = BEAD_UNTIL + BEAD_FADE * (1.0 - BEAD_UNTIL);
    assert_eq!(drip_bead_share(gone), 0.0);
    let mid = drip_bead_share((BEAD_UNTIL + gone) / 2.0);
    assert!(0.0 < mid && mid < 1.0, "{mid}");
}

#[test]
fn the_bead_sits_on_the_drips_bead_then_at_its_tray_side_end() {
    let rest = Rect::new(100, 200, 360, 68);
    let (p, _) = drip_pose(0.1, rest, true);
    assert_eq!(drip_bead_at(p, true), p);
    let (p, _) = drip_pose(0.4, rest, true);
    let b = drip_bead_at(p, true);
    assert_eq!(
        (b.right(), b.height, b.width),
        (p.right(), p.height, p.height)
    );
    let (p, _) = drip_pose(0.4, rest, false);
    assert_eq!(drip_bead_at(p, false).x, p.x);
}

#[test]
#[ignore]
fn render_beads() {
    for dark in [false, true] {
        for s in [1, 2] {
            let (c, _) = drip_bead(&glass(dark, s)).unwrap();
            let mode = if dark { "dark" } else { "light" };
            crate::raster::write_preview(
                &on_backdrop(&c),
                &format!("notifysheet-bead-{mode}-x{s}.png"),
            );
        }
    }
}

#[test]
#[ignore]
fn render_droplets() {
    let t = text();
    for dark in [false, true] {
        for s in [1, 2] {
            for case in 0..DROPLET_CASES.len() {
                let c = droplet_case(case, &glass(dark, s), &t);
                let mode = if dark { "dark" } else { "light" };
                crate::raster::write_preview(
                    &on_backdrop(&c),
                    &format!("notifysheet-droplet-{case}-{mode}-x{s}.png"),
                );
            }
        }
    }
}

#[test]
#[ignore]
fn render_sheets() {
    let t = text();
    for dark in [false, true] {
        for s in [1, 2] {
            for case in 0..4 {
                let c = pin_case(case, &glass(dark, s), &t);
                let mut bg = Canvas::new(c.width, c.height);
                let w = c.width as i32;
                bg.fill_rect(
                    Rect::new(0, 0, w / 2, c.height as i32),
                    Color::rgb(0x3A, 0x6E, 0xA5),
                );
                bg.fill_rect(
                    Rect::new(w / 2, 0, w - w / 2, c.height as i32),
                    Color::rgb(0xC8, 0xD4, 0xE0),
                );
                bg.blit_canvas(0, 0, &c);
                let mode = if dark { "dark" } else { "light" };
                crate::raster::write_preview(&bg, &format!("notifysheet-{case}-{mode}-x{s}.png"));
            }
        }
    }
}

#[test]
fn a_droplet_starts_as_a_bead_at_the_tray() {
    let rest = Rect::new(1000, 900, 360, 68);
    assert_eq!(
        drip_pose(0.0, rest, true),
        (Rect::new(1320, 928, 12, 12), 0.0)
    );
    assert_eq!(
        drip_pose(0.2, rest, true),
        (Rect::new(1320, 928, 12, 12), 1.0)
    );
    assert_eq!(drip_pose(0.0, rest, false).0, Rect::new(1028, 928, 12, 12));
}

#[test]
fn a_droplet_swells_past_its_size_and_settles_at_rest() {
    let rest = Rect::new(1000, 900, 360, 68);
    assert!(drip_pose(0.8, rest, true).0.width > 360, "overshoot");
    assert_eq!(drip_pose(1.0, rest, true), (rest, 1.0));
    assert_eq!(drip_pose(1.0, rest, false), (rest, 1.0));
}

#[test]
fn the_drip_takes_twelve_fifths_of_the_open_time() {
    assert_eq!((drip_ms(250), drip_ms(0)), (600, 0));
}

#[test]
fn a_short_drip_gets_a_corner_that_fits() {
    use crate::backdrop::corner_radius;
    assert_eq!(
        (
            corner_radius(drip_corner(68)),
            corner_radius(drip_corner(30)),
            corner_radius(drip_corner(8))
        ),
        (34.0, 12.0, 6.0)
    );
}

/// A Lozenge whose app line reads `when`, expanded to its reply row with `reply`.
fn reply_sheet(theme: &Theme, when: &str, body: &str, reply: bool) -> SheetLayout {
    let c = SheetContent {
        when,
        actions: &["Open", "Mark read"],
        reply: reply.then_some(REPLY),
        ..content("Chat", "Ana", body)
    };
    sheet_layout(theme, &text(), SheetShape::Lozenge, &c, reply).unwrap()
}

#[test]
fn a_sheet_on_kept_glass_matches_one_drawn_whole() {
    let (theme, t) = (glass(false, 2), text());
    let lay = reply_sheet(&theme, "now", "Hi", false);
    let mut kept = SheetGlass::default();
    for hot in [
        None,
        Some(NotifyHit::Button(0)),
        Some(NotifyHit::Body),
        None,
    ] {
        let icons = SheetIcons::default();
        let whole = draw_sheet(&theme, &t, &lay, icons, None, None, hot, None);
        let drawn = kept.draw(&theme, &t, &lay, icons, None, None, hot, None);
        assert_eq!(fnv(&drawn), fnv(&whole), "{hot:?}");
    }
    assert_eq!(kept.builds(), 2, "one glass per rim");
}

#[test]
fn a_caret_blink_or_an_age_tick_paints_no_glass_and_a_resize_paints_one() {
    let (theme, t) = (glass(false, 2), text());
    let edit = TextEdit::new("On my way", false);
    let mut kept = SheetGlass::default();
    let mut draw = |lay: &SheetLayout, shown: bool| {
        let reply = Some((&edit, Caret::new(true, shown), None));
        kept.draw(
            &theme,
            &t,
            lay,
            SheetIcons::default(),
            None,
            None,
            None,
            reply,
        );
        kept.builds()
    };
    let now = reply_sheet(&theme, "now", "Hi", true);
    assert_eq!(draw(&now, true), 1);
    assert_eq!(draw(&now, false), 1, "a caret blink");
    let later = reply_sheet(&theme, "1 min ago", "Hi", true);
    assert_eq!(draw(&later, false), 1, "an age tick");
    let long = "A body long enough to wrap onto a second line and grow the sheet taller";
    let taller = reply_sheet(&theme, "1 min ago", long, true);
    assert!(taller.sheet.height > later.sheet.height);
    assert_eq!(draw(&taller, false), 2, "a taller sheet");
}

/// A three-sheet stack's paint cost at scale 2 for a hover move and a caret blink, every sheet
/// whole against the changed one on kept glass. Run `--release -- --ignored --nocapture`.
#[test]
#[ignore]
fn stack_paint_cost() {
    use std::time::Instant;
    let (theme, t) = (glass(false, 2), text());
    let edit = TextEdit::new("On my way", false);
    let lays = [
        reply_sheet(&theme, "now", "Hi", true),
        reply_sheet(&theme, "now", "Lunch at one?", false),
        reply_sheet(&theme, "1 min ago", "The build is green", false),
    ];
    let icons = SheetIcons::default();
    let ms = |f: &mut dyn FnMut()| {
        let start = Instant::now();
        for _ in 0..5 {
            f();
        }
        start.elapsed().as_secs_f64() * 1000.0 / 5.0
    };
    // Sheet 0 is replying; the pointer is on sheet 1 and moves to its first pill.
    let state = |hover: NotifyHit, shown: bool| {
        [
            (None, Some((&edit, Caret::new(true, shown), None))),
            (Some(hover), None),
            (None, None),
        ]
    };
    let whole = |hover: NotifyHit, shown: bool| {
        for (lay, (hot, reply)) in lays.iter().zip(state(hover, shown)) {
            draw_sheet(&theme, &t, lay, icons, None, None, hot, reply);
        }
    };
    let hover = ms(&mut || whole(NotifyHit::Button(0), true));
    let blink = ms(&mut || whole(NotifyHit::Body, false));
    println!("every sheet whole: hover {hover:.1} ms, caret blink {blink:.1} ms");
    let one = |kept: &mut SheetGlass, i: usize, hover: NotifyHit, shown: bool| {
        let (hot, reply) = state(hover, shown)[i];
        kept.draw(&theme, &t, &lays[i], icons, None, None, hot, reply);
    };
    let first = ms(&mut || {
        let mut fresh: [SheetGlass; 3] = Default::default();
        for (i, kept) in fresh.iter_mut().enumerate() {
            one(kept, i, NotifyHit::Body, true);
        }
    });
    let mut kept: [SheetGlass; 3] = Default::default();
    for (i, k) in kept.iter_mut().enumerate() {
        one(k, i, NotifyHit::Body, true);
    }
    let hover = ms(&mut || one(&mut kept[1], 1, NotifyHit::Button(0), true));
    let blink = ms(&mut || one(&mut kept[0], 0, NotifyHit::Body, false));
    println!(
        "changed sheet on kept glass: hover {hover:.1} ms, caret blink {blink:.1} ms \
         (first paint of the stack {first:.1} ms)"
    );
}
