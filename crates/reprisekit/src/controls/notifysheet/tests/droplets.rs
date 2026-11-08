use super::*;

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
    crate::pins::fonts();
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
