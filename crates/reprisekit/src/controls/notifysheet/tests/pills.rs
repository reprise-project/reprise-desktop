use super::*;

fn pill_rows(lay: &SheetLayout) -> (Vec<Rect>, usize) {
    let p = &lay.parts;
    let all: Vec<Rect> = p.buttons.iter().copied().chain(p.more).collect();
    let mut ys: Vec<i32> = all.iter().map(|b| b.y).collect();
    ys.dedup();
    (all, ys.len())
}

#[test]
fn long_pills_show_whole_on_wrapped_rows_on_every_shape_and_scale() {
    let t = text();
    for shape in SHAPES {
        for s in [1, 2] {
            let theme = glass(false, s);
            let (mut rows_before, mut h_before) = (0, 0);
            for n in 1..=LONG_PILLS.len() {
                let c = SheetContent {
                    actions: &LONG_PILLS[..n],
                    ..content("Drives", "Low Disk Space", "Body")
                };
                let lay = sheet_layout(&theme, &t, shape, &c, true).unwrap();
                let at = format!("{shape:?} x{s}, {n} pills");
                let shown = n.min(NOTIFY_BUTTONS);
                let whole: Vec<Option<String>> = LONG_PILLS[..shown]
                    .iter()
                    .map(|a| Some(a.to_string()))
                    .collect();
                assert_eq!(lay.labels, whole, "{at}");
                assert_eq!(lay.parts.more.is_some(), n > NOTIFY_BUTTONS, "{at}");
                let (all, rows) = pill_rows(&lay);
                let inner = lay.sheet.inset(10 * s);
                for (k, b) in all.iter().enumerate() {
                    assert!(
                        b.x >= inner.x
                            && b.right() <= inner.right()
                            && b.bottom() <= inner.bottom(),
                        "{at}: pill {k} {b:?} inside {:?}",
                        lay.sheet
                    );
                    assert!(
                        all[k + 1..].iter().all(|o| !b.intersects(*o)),
                        "{at}: pill {k} overlaps a later one"
                    );
                }
                assert!(rows >= rows_before, "{at}: {rows} rows");
                if rows > rows_before {
                    assert!(
                        lay.sheet.height > h_before,
                        "{at}: a new row grows the sheet"
                    );
                }
                (rows_before, h_before) = (rows, lay.sheet.height);
            }
            assert!(rows_before >= 2, "{shape:?} x{s}: the long set wraps");
        }
    }
}

/// A pill wider than the text column reaches left under the icon to the sheet's padding.
#[test]
fn a_pill_wider_than_the_text_column_shows_whole_out_to_the_padding() {
    let t = text();
    let sentence = "Add to the Listening Queue for Tonight, please, and again tomorrow";
    for shape in SHAPES {
        for s in [1, 2] {
            // The shortest prefix whose pill overruns the lozenge's 274 px text column.
            let px = 13 * s as u32;
            let label = (1..=sentence.len())
                .map(|n| &sentence[..n])
                .find(|l| t.measure(l, true, px) / s + 2 * 14 > 274)
                .expect("the sentence overruns the column");
            let c = SheetContent {
                actions: &[label],
                ..content("Music", "Now playing", "A song")
            };
            let lay = sheet_layout(&glass(false, s), &t, shape, &c, true).unwrap();
            assert_eq!(lay.labels, [Some(label.to_string())], "{shape:?} x{s}");
            let pill = lay.parts.buttons[0];
            assert!(pill.x >= lay.sheet.x + 14 * s, "{shape:?} x{s}: {pill:?}");
            assert!(
                pill.y >= lay.icon.bottom(),
                "{shape:?} x{s}: under the icon"
            );
            if shape == SheetShape::Lozenge {
                assert!(
                    pill.x < lay.parts.body.x,
                    "{shape:?} x{s}: wider than the text"
                );
            }
        }
    }
}

#[test]
fn a_reply_takes_the_first_pill_row_and_the_sheet_keeps_its_height() {
    let t = text();
    let theme = glass(false, 1);
    let pills = SheetContent {
        actions: &LONG_PILLS[..3],
        ..content("Chat", "Ana", "Lunch?")
    };
    let replying = SheetContent {
        reply: Some(REPLY),
        ..pills
    };
    for shape in SHAPES {
        let row = sheet_layout(&theme, &t, shape, &pills, true).unwrap();
        let lay = sheet_layout(&theme, &t, shape, &replying, true).unwrap();
        assert_eq!(pill_rows(&row).1, 2, "{shape:?}");
        assert_eq!(lay.sheet.height, row.sheet.height, "{shape:?}");
        assert_eq!(
            lay.parts.send.map(|b| b.y),
            Some(row.parts.buttons[0].y),
            "{shape:?}"
        );
    }
}

const WRAPPED_PILL_PINS: [u64; 6] = [
    0xc0119631d1d3265a,
    0xba4fff44421ea53f,
    0xa727912c2a5a2fef,
    0xc23c9f348c48263d,
    0xfcc91249b767bee4,
    0xee67f01cba1aa78e,
];

/// Three long pills on two rows on each shape, expanded, at scale 1 and 2.
#[test]
fn wrapped_pill_rows_pins() {
    crate::pins::fonts();
    let t = text();
    let mut got = Vec::new();
    for shape in SHAPES {
        for s in [1, 2] {
            let theme = glass(false, s);
            let c = SheetContent {
                actions: &LONG_PILLS[..3],
                ..content(
                    "Drives",
                    "Low Disk Space",
                    "The disk of / has only 812 MB free of 20.0 GB.",
                )
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
        assert_eq!(*h, WRAPPED_PILL_PINS[i], "{case}; now:\n{}", now.join("\n"));
    }
}
