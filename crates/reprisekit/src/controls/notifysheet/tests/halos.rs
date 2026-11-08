use super::*;

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
    crate::pins::fonts();
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
