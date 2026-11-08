use super::*;

fn glass_text(pct: u32, scale: i32) -> Theme {
    let light = glass::Settings {
        mode: glass::Mode::Light,
        ..glass::Settings::default()
    };
    theme_with(Interface::Glass, blue::Variant::Blue, &light)
        .with_text_size(pct)
        .scaled(scale)
}

const CROWDED_TITLE: &str = "Your scheduled backup finished with warnings on two drives";

const CROWDED_BODY: &str = "The backup of Documents, Pictures and Music finished at 03:12. \
    Two files were skipped because another program had them open; open the log to see which.";

#[test]
fn at_200_percent_text_the_sheets_widen_by_their_text_column_and_grow_their_lines() {
    let t = text();
    let theme = glass_text(200, 1);
    let u = theme.text;
    let c = content("Backup", "Done", "");
    let lay = |shape| sheet_layout(&theme, &t, shape, &c, false).unwrap();
    let (loz, halo, drop) = (
        lay(SheetShape::Lozenge),
        lay(SheetShape::Halo),
        lay(SheetShape::Droplet),
    );
    assert_eq!(
        loz.sheet.width,
        360 - 274 + u.len(274),
        "Lozenge column 274"
    );
    assert_eq!(halo.sheet.width, 360 - 324 + u.len(324), "Halo column 324");
    assert_eq!(
        drop.sheet.width,
        360 - 258 + u.len(258),
        "Droplet column 258"
    );
    assert_eq!(
        (
            loz.app_px,
            loz.title_px,
            loz.body_px,
            loz.pill_px,
            loz.line_h
        ),
        (u.px(12), u.px(16), u.px(14), u.px(13), u.len(19))
    );
    assert_eq!((halo.title_px, drop.title_px), (u.px(15), u.px(15)));
    assert_eq!(
        loz.parts.title.y - loz.sheet.y,
        14 + u.len(16),
        "the title under the grown app line"
    );
    assert_eq!(loz.parts.title.height, u.len(20));
    let (block, drop_body) = (
        u.len(20),
        sheet_layout(&theme, &t, SheetShape::Droplet, &long(), false).unwrap(),
    );
    assert_eq!(drop.sheet.height, 68.max(block + 20), "a bodyless capsule");
    assert_eq!(
        drop_body.sheet.height,
        68.max(block + u.len(19) + 20),
        "a capsule row holding the title and one body line"
    );
    let mid = |r: Rect| r.y + r.height / 2;
    assert_eq!(
        mid(drop_body.icon),
        mid(drop_body.sheet),
        "the slot centres on the taller row"
    );
    assert_eq!(mid(drop_body.parts.close), mid(drop_body.sheet));
    assert_eq!(
        chip_layout(&theme, &t, 2).1,
        u.len(28),
        "the chip grows too"
    );
}

/// A long title, a body past the line cap and three long pills (or the reply row), at every
/// shape and scale, at the largest text size.
#[test]
fn at_the_largest_text_size_a_crowded_sheet_holds_every_part_whole() {
    let t = text();
    for shape in SHAPES {
        for s in [1, 2] {
            let theme = glass_text(200, s);
            let base = sheet_layout(
                &glass_text(100, s),
                &t,
                shape,
                &content("Backup", CROWDED_TITLE, CROWDED_BODY),
                true,
            )
            .unwrap();
            let pills = SheetContent {
                actions: &LONG_PILLS[..3],
                ..content("Backup", CROWDED_TITLE, CROWDED_BODY)
            };
            let replying = SheetContent {
                reply: Some(REPLY),
                ..pills
            };
            let shut = (shape == SheetShape::Droplet).then_some((&pills, false));
            for (c, expanded) in [(&pills, true), (&replying, true)].into_iter().chain(shut) {
                let lay = sheet_layout(&theme, &t, shape, c, expanded).unwrap();
                let at = format!(
                    "{shape:?} x{s} expanded {expanded}, reply {}",
                    c.reply.is_some()
                );
                if shape != SheetShape::Droplet || expanded {
                    assert!(lay.sheet.height > base.sheet.height, "{at}: taller");
                }
                assert!(lay.sheet.width > base.sheet.width, "{at}: wider");
                let p = &lay.parts;
                let holds = |r: Rect, bold: bool, px: u32, what: &str| {
                    let (top, h) = t.line(r, bold, px).expect("a face");
                    assert!(
                        top >= r.y && top + h <= r.bottom(),
                        "{at}: {what} line {top}+{h} inside {r:?}"
                    );
                };
                holds(lay.app_box, false, lay.app_px, "app");
                holds(p.title, true, lay.title_px, "title");
                for i in 0..lay.lines.len() as i32 {
                    let line = Rect::new(
                        p.body.x,
                        p.body.y + i * lay.line_h,
                        p.body.width,
                        lay.line_h,
                    );
                    holds(line, false, lay.body_px, "body");
                }
                let pill_rects: Vec<Rect> = p
                    .buttons
                    .iter()
                    .copied()
                    .chain(p.more)
                    .chain(p.send)
                    .collect();
                for &b in &pill_rects {
                    holds(b, true, lay.pill_px, "pill");
                }
                if let Some(f) = p.field {
                    holds(f, false, lay.body_px, "field");
                }
                if !expanded {
                    assert!(pill_rects.is_empty(), "{at}: a shut Droplet shows no pills");
                } else if c.reply.is_none() {
                    let whole: Vec<Option<String>> = LONG_PILLS[..3]
                        .iter()
                        .map(|a| Some(a.to_string()))
                        .collect();
                    assert_eq!(lay.labels, whole, "{at}");
                }
                let body = Rect::new(
                    p.body.x,
                    p.body.y,
                    p.body.width,
                    lay.lines.len() as i32 * lay.line_h,
                );
                let mut parts = vec![("title", p.title), ("body", body)];
                if shape == SheetShape::Droplet {
                    let title_end = p.title.x + t.measure(&lay.title, true, lay.title_px);
                    assert!(
                        title_end <= lay.app_box.x,
                        "{at}: the app line follows the title"
                    );
                    assert!(
                        lay.app_box.right() <= p.title.right(),
                        "{at}: on the head row"
                    );
                } else {
                    parts.push(("app", lay.app_box));
                }
                parts.extend(pill_rects.iter().map(|&b| ("pill", b)));
                parts.extend(p.field.map(|f| ("field", f)));
                for (i, (what, r)) in parts.iter().enumerate() {
                    assert!(
                        lay.sheet.union(r) == lay.sheet,
                        "{at}: {what} {r:?} inside {:?}",
                        lay.sheet
                    );
                    for (other, o) in &parts[i + 1..] {
                        assert!(
                            !r.intersects(*o),
                            "{at}: {what} {r:?} clear of {other} {o:?}"
                        );
                    }
                    if *what != "body" && *what != "pill" && *what != "field" {
                        assert!(!r.intersects(p.close), "{at}: {what} clear of the close");
                    }
                }
            }
        }
    }
}

const GROWN_PINS: [u64; 6] = [
    0xb0390d13bf2bc07d,
    0xf71993b8476645a7,
    0xa76e87019232142b,
    0xba5a2f2a3c6fea02,
    0x23e2ff31f95c0c0f,
    0x93c98c6fb07e7011,
];

/// The crowded sheet at 200% text on each shape, expanded, at scale 1 and 2.
#[test]
fn grown_text_pins() {
    crate::pins::fonts();
    let t = text();
    let mut got = Vec::new();
    for shape in SHAPES {
        for s in [1, 2] {
            let theme = glass_text(200, s);
            let c = SheetContent {
                actions: &LONG_PILLS[..3],
                ..content("Backup", CROWDED_TITLE, CROWDED_BODY)
            };
            let lay = sheet_layout(&theme, &t, shape, &c, true).unwrap();
            let hot = Some(NotifyHit::Button(2));
            let drawn = draw_sheet(
                &theme,
                &t,
                &lay,
                SheetIcons::default(),
                None,
                None,
                hot,
                None,
            );
            got.push((format!("{shape:?} x{s}"), fnv(&drawn)));
        }
    }
    let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
    for (i, (case, h)) in got.iter().enumerate() {
        assert_eq!(*h, GROWN_PINS[i], "{case}; now:\n{}", now.join("\n"));
    }
}
