use super::*;

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
    crate::pins::fonts();
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
fn the_lozenge_style_draws_the_lozenge_shape() {
    assert_eq!(glass::NotifyStyle::Lozenge.shape(), SheetShape::Lozenge);
}
