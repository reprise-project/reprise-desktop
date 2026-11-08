use super::*;

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
