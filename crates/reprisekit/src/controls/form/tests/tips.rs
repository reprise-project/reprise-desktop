use super::*;

const LONG: &str = "HP LaserJet 4250 PostScript Printer Driver";

fn long_label_form() -> Form {
    Form::new(vec![
        Item::label_at("short", 10, 10, 120, "Model:"),
        Item::label_at("long", 10, 30, 120, LONG),
    ])
}

#[test]
fn a_label_wider_than_its_rect_tips_its_whole_text_after_a_blink() {
    let text = tr();
    let mut form = long_label_form();
    draw(
        &mut Canvas::new(200, 60),
        &text,
        &classic_paint(&Theme::default().colors, 1),
        &form,
        false,
        PX,
    );
    assert_eq!(
        *form.rendered.ellipsized.borrow(),
        vec![1],
        "only the long label is cut"
    );

    assert_eq!(form.motion(20, 15, None), Event::None);
    assert!(!form.tip_tick(), "a label that fits has no tip");
    assert_eq!(form.label_tip(), None);

    form.motion(20, 35, None);
    assert_eq!(form.label_tip(), None, "no tip before a blink");
    assert!(form.tip_tick());
    assert_eq!(form.label_tip().as_deref(), Some(LONG));
    assert!(!form.tip_tick());
    assert_eq!(
        form.motion(25, 35, None),
        Event::None,
        "moving on the label keeps it up"
    );
    assert_eq!(form.label_tip().as_deref(), Some(LONG));

    assert_eq!(
        form.motion(20, 15, None),
        Event::Redraw,
        "moving off takes it down"
    );
    assert_eq!(form.label_tip(), None);
    form.motion(20, 35, None);
    form.tip_tick();
    assert_eq!(form.leave(), Event::Redraw);
    assert_eq!(form.label_tip(), None);

    form.motion(20, 35, None);
    let ups: Vec<bool> = (0..=TIP_TICKS).map(|_| form.tip_tick()).collect();
    assert_eq!(
        ups.iter().filter(|&&b| b).count(),
        2,
        "it comes up and pops after a while"
    );
    assert_eq!(form.label_tip(), None);
}

#[test]
fn an_ellipsized_label_keeps_an_access_key_the_cut_leaves() {
    let text = tr();
    let form = Form::new(Vec::new());
    let cut = |label: &str| ellipsized(&form, 0, &text, label, 80, PX).into_owned();
    let kept = cut("&Maximum size of the Recycle Bin");
    assert!(kept.starts_with("&Max") && kept.ends_with("..."), "{kept}");
    let gone = cut("Maximum size of the Recycle &Bin");
    assert!(!gone.contains('&') && gone.ends_with("..."), "{gone}");
    let amp = cut("R&&D: maximum size of the Recycle Bin");
    assert!(amp.starts_with("R&&D") && amp.ends_with("..."), "{amp}");
    assert_eq!(cut("&Size"), "&Size", "fits");
}

#[test]
fn a_label_running_past_its_window_ends_in_an_ellipsis_at_the_edge() {
    let text = tr();
    let form = Form::new(vec![Item::label_at("wide", 10, 10, 400, LONG)]);
    let theme = Theme::default();
    let p = classic_paint(&theme.colors, 1);
    draw(&mut Canvas::new(500, 40), &text, &p, &form, false, PX);
    assert!(
        form.rendered.ellipsized.borrow().is_empty(),
        "fits in a wide window"
    );
    draw(&mut Canvas::new(120, 40), &text, &p, &form, false, PX);
    assert_eq!(
        *form.rendered.ellipsized.borrow(),
        vec![0],
        "cut at the window's edge"
    );
}

#[test]
fn a_check_or_radio_label_cut_by_the_next_control_tips_its_whole_text() {
    let text = tr();
    let mut form = Form::new(vec![
        Item::check("fits", 10, 10, "Model"),
        Item::check("long", 10, 30, LONG),
        Item::radio("pick", 10, 50, LONG, "g"),
        Item::button_at("b1", 150, 26, 60, "Go", true),
        Item::button_at("b2", 150, 46, 60, "Go", true),
    ]);
    let theme = Theme::default();
    let p = classic_paint(&theme.colors, 1);
    draw(&mut Canvas::new(300, 100), &text, &p, &form, false, PX);
    assert_eq!(
        *form.rendered.ellipsized.borrow(),
        vec![1, 2],
        "the labels running into a button are cut"
    );
    form.motion(40, 15, None);
    assert!(!form.tip_tick(), "a check box label that fits has no tip");
    for y in [35, 55] {
        form.motion(40, y, None);
        assert!(form.tip_tick(), "at {y}");
        assert_eq!(form.label_tip().as_deref(), Some(LONG), "at {y}");
    }
}

/// A check box built narrower than its label, with the form reaching further right on
/// another row, draws its label cut, and the audit reports it.
#[test]
fn the_audit_reports_a_check_box_label_cut_at_its_own_edge() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut narrow = Item::check("narrow", 10, 10, LONG);
    narrow.rect.width = 60;
    let wide = Item::label_at("wide", 10, 40, 400, "x");
    let form = Form::new(vec![narrow, wide]);
    let theme = Theme::default();
    let p = classic_paint(&theme.colors, 1);
    draw(&mut Canvas::new(500, 100), &text, &p, &form, false, 11);
    assert_eq!(
        *form.rendered.ellipsized.borrow(),
        vec![0],
        "the label draws cut"
    );
    assert!(
        matches!(form.overflows(&text, 11)[..], [("narrow", by)] if by > 0),
        "the audit reports the cut label, got {:?}",
        form.overflows(&text, 11)
    );
}

/// At 200% a box-sized check box's or radio's rect grows past where its label starts; the
/// label still runs to the form's edge. A check box built wider is still cut at its edge.
#[test]
fn a_grown_box_sized_check_or_radio_keeps_its_whole_label() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut wide = Item::check("wide", 10, 50, LONG);
    wide.rect.width = 60;
    let form = Form::new(vec![
        Item::check("check", 10, 10, "Turn on Bluetooth"),
        Item::radio("radio", 10, 30, "Turn on Bluetooth", "g"),
        wide,
    ]);
    let theme = Theme::default();
    let p = Paint {
        text: TextUnits::new(22, 11),
        ..classic_paint(&theme.colors, 1)
    };
    draw(&mut Canvas::new(800, 200), &text, &p, &form, false, 22);
    assert_eq!(
        *form.rendered.ellipsized.borrow(),
        vec![2],
        "only the check box built 60 px wide is cut"
    );
}

#[test]
fn a_long_label_ends_in_an_ellipsis_at_scale_1_and_2() {
    if !crate::text::test_tahoma() {
        return;
    }
    let text = tr();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut got = Vec::new();
    for s in [1, 2] {
        let mut form = Form::new(vec![
            Item::label_at("left", 10, 10, 120, LONG),
            Item::value_at("right", 10, 30, 120, LONG),
            Item::label_at("grey", 10, 50, 120, LONG).enabled(false),
        ]);
        let mut c = Canvas::with_scale(160, 120, s);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &form, false, PX);

        let cut = text.ellipsize(LONG, 120, false, PX);
        assert!(cut.ends_with("...") && cut.len() < LONG.len());
        let mut want = Canvas::with_scale(160, 120, s);
        want.fill_rect(want.bounds(), cs.button_face);
        let plain = Form::new(vec![
            Item::label_at("left", 10, 10, 120, &cut),
            Item::value_at("right", 10, 30, 120, &cut),
            Item::label_at("grey", 10, 50, 120, &cut).enabled(false),
        ]);
        draw(&mut want, &text, &classic_paint(cs, 1), &plain, false, PX);
        assert_eq!(c.pixels, want.pixels, "@{s}: drawn as the cut text");
        got.push((format!("labels @{s}"), fnv(&c)));

        form.motion(20, 15, None);
        form.tip_tick();
        report_tip(&mut c, &text, &form);
        assert!(draw_tip(&mut c, &text, &theme).is_some());
        assert_ne!(c.pixels, want.pixels, "@{s}: the tip shows");
        got.push((format!("tip @{s}"), fnv(&c)));
    }
    crate::pins::check("form/label_ellipsis", &got);
}

#[test]
fn a_tip_wider_than_the_window_wraps_inside_it() {
    let text = tr();
    let theme = Theme::default();
    let mut form = long_label_form();
    let mut c = Canvas::new(150, 120);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &form,
        false,
        PX,
    );
    form.motion(125, 35, None);
    form.tip_tick();
    let mut blank = Canvas::new(150, 120);
    blank.pixels.copy_from_slice(&c.pixels);
    report_tip(&mut c, &text, &form);
    draw_tip(&mut c, &text, &theme);
    let inked = |y: i32| (0..150).any(|x| c.get(x, y) != blank.get(x, y));
    let rows: Vec<i32> = (0..120).filter(|&y| inked(y)).collect();
    assert!(!rows.is_empty());
    assert!(
        rows.last().unwrap() - rows[0] > 2 * PX as i32,
        "the text runs onto more lines"
    );
    assert!(
        (0..120).all(|y| c.get(149, y) == blank.get(149, y)),
        "clear of the right edge"
    );
}

/// Draws `form` aside and hands the tip it reports to `c`, leaving `c`'s pixels as they are.
fn report_tip(c: &mut Canvas, text: &TextRenderer, form: &Form) {
    let mut aside = Canvas::with_scale(c.width, c.height, c.scale());
    let theme = Theme::default();
    draw(
        &mut aside,
        text,
        &classic_paint(&theme.colors, 1),
        form,
        false,
        PX,
    );
    if let Some(tip) = aside.take_tip() {
        c.report_tip(tip);
    }
}

#[test]
fn a_form_reports_its_label_tip_only_once_a_blink_shows_it() {
    let text = tr();
    let theme = Theme::default();
    let mut form = long_label_form();
    let drawn = |form: &Form| {
        let mut c = Canvas::new(150, 120);
        draw(&mut c, &text, &theme.paint(), form, false, PX);
        c
    };
    assert_eq!(drawn(&form).take_tip(), None, "nothing at rest");
    form.motion(125, 35, None);
    let mut c = drawn(&form);
    assert_eq!(c.take_tip(), None, "nothing before a blink");
    assert_eq!(draw_tip(&mut c, &text, &theme), None);
    form.tip_tick();
    let mut c = drawn(&form);
    let before = c.pixels.clone();
    let tip = c.take_tip().expect("the tip after a blink");
    assert_eq!(tip.at, (125, 35), "at the pointer");
    assert_eq!(tip.text, LONG, "the whole text");
    c.report_tip(tip);
    let r = draw_tip(&mut c, &text, &theme).expect("the host draws it");
    assert_ne!(c.pixels, before);
    assert!(r.width > 0 && r.height > 0);
    assert_eq!(c.take_tip(), None, "taken once");
}

fn fnv(c: &Canvas) -> u64 {
    c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, w| {
        w.to_le_bytes()
            .iter()
            .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    })
}
