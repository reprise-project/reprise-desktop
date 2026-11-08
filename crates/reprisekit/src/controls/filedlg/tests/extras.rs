use super::*;

#[test]
fn a_tight_extra_grows_to_show_up_to_four_list_rows() {
    let w = field_w(CLIENT.0);
    let combo_of = |n: usize| {
        let names: Vec<String> = (0..n).map(|i| format!("row {i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        form::Form::new(vec![form::Item::combo("c", FIELD_X, EXTRA_Y, w, &refs, n)])
    };
    for (n, shown) in [(2, 2), (3, 3), (9, 4)] {
        let mut d = multi(DlgMode::Open, false);
        d.set_extra(combo_of(n), 29, &ControlMetrics::CLASSIC);
        let it = d.extra().unwrap().item("c").unwrap();
        let form::Control::Combo(c) = &it.control else {
            panic!("combo")
        };
        assert_eq!(c.rows, shown, "{n} options show {shown} rows");
        let list = combo::layout(c, it.rect, &ControlMetrics::CLASSIC).list;
        assert_eq!(
            d.client().1,
            list.bottom(),
            "the client grows to the list's bottom"
        );
    }
}

#[test]
fn extra_rows_grow_the_client_and_leave_the_rest_in_place() {
    let d = with_extra();
    assert_eq!(d.client(), (CLIENT.0, CLIENT.1 + EXTRA_H));
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let t = text();
    let base = multi(DlgMode::Open, false).layout(&t, 11, R, &ControlMetrics::CLASSIC);
    let grown = d.layout(&t, 11, big, &ControlMetrics::CLASSIC);
    assert_eq!(grown.filename, base.filename, "File name field unchanged");
    assert_eq!(grown.accept, base.accept, "accept button unchanged");
    assert_eq!(grown.cancel, base.cancel, "Cancel unchanged");
    assert_eq!(grown.shell.frame, base.shell.frame, "shell view unchanged");
}

#[test]
fn a_click_on_an_extra_row_reaches_it() {
    let mut d = with_extra();
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let hit = d.hit(&text(), 11, big, FIELD_X + 5, EXTRA_Y + 5);
    assert!(matches!(hit, Some(Hit::Extra(_))), "got {hit:?}");
    d.click(big, hit, Mods::default());
    assert_eq!(d.extra().unwrap().checked("extra-check"), Some(true));
}

#[test]
fn an_extra_rows_key_wins_and_the_labels_pick_around_it() {
    let mut d = multi(DlgMode::Open, false);
    let form = form::Form::new(vec![
        form::Item::check("subfolders", FIELD_X, EXTRA_Y, "&Include subfolders"),
        form::Item::check("names", FIELD_X, EXTRA_Y + 20, "&Name files"),
    ]);
    d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
    d.fields.accept_label = "Load".into();
    let alt = |d: &mut FileDlg, c| d.key(R, form::Key::Access(c), None, Mods::default());
    alt(&mut d, 'i');
    assert_eq!(d.extra().unwrap().checked("subfolders"), Some(true));
    assert_eq!(d.focus, Focus::Extra);
    alt(&mut d, 'n');
    assert_eq!(d.extra().unwrap().checked("names"), Some(true));
    let keys = d.labels();
    assert_eq!(keys.lookin, "&Look in:");
    assert_eq!(keys.filename, "&File name:");
    assert_eq!(keys.filetype, "Files of &type:");
    assert_eq!(d.accept_label(), "L&oad", "l is Look in's now");
    alt(&mut d, 'l');
    assert_eq!(d.focus, Focus::LookIn);
    alt(&mut d, 'f');
    assert_eq!(d.focus, Focus::FileName);
}

/// The extra rows are drawn as a shifted copy, yet their hit test
/// still reads the look they were drawn in: after a Blue draw, the
/// check box's label 16 px in hits it.
#[test]
fn a_blue_extra_check_box_is_hit_on_its_label_16_px_in() {
    let d = with_extra();
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let mut c = Canvas::new(CLIENT.0 as u32, (CLIENT.1 + EXTRA_H) as u32);
    d.draw(&mut c, &text(), &theme.paint(), big, 11);
    let hit = d.hit(&text(), 11, big, FIELD_X + 16, EXTRA_Y + 5);
    assert!(
        matches!(hit, Some(Hit::Extra(h)) if h.item == 0),
        "the label 16 px in hits the check box, got {hit:?}"
    );
}

#[test]
fn a_cut_extra_label_tips_its_whole_text_after_a_blink() {
    const LONG: &str = "Character encoding used when the document is written back to disk";
    let mut d = multi(DlgMode::Open, false);
    let form = form::Form::new(vec![form::Item::label_at(
        "long", FIELD_X, EXTRA_Y, 60, LONG,
    )]);
    d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let theme = Theme::default();
    let t = text();
    let draw = |d: &FileDlg| {
        let mut c = Canvas::new(big.width as u32, big.height as u32);
        d.draw(&mut c, &t, &theme.paint(), big, 11);
        c.take_tip().map(|tip| (tip.text, tip.at))
    };
    assert_eq!(draw(&d), None);
    let (x, y) = (FIELD_X + 20, EXTRA_Y + 5);
    assert!(matches!(d.motion(&t, 11, big, x, y), Event::None));
    assert!(
        matches!(d.blink(), Event::Redraw),
        "the blink brings the tip up"
    );
    assert_eq!(d.extra().unwrap().label_tip().as_deref(), Some(LONG));
    assert_eq!(
        draw(&d),
        Some((LONG.to_string(), (x, y))),
        "the drawn rows report the tip at the pointer"
    );
    assert!(matches!(d.leave(), Event::Redraw), "leaving takes it down");
    assert_eq!(d.extra().unwrap().label_tip(), None);
}

#[test]
fn tab_walks_into_the_extra_rows_after_files_of_type() {
    let mut d = with_extra();
    d.focus = Focus::FileType;
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Extra);
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Extra, "second extra control");
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Accept);
}

#[test]
fn extra_rows_pixels() {
    let d = with_extra();
    let (w, h) = d.client();
    let t = text();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut c = Canvas::new(w as u32, h as u32);
    d.draw(&mut c, &t, &classic_paint(cs, 1), Rect::new(0, 0, w, h), 11);
    let mut base = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
    multi(DlgMode::Open, false).draw(&mut base, &t, &classic_paint(cs, 1), R, 11);
    for y in 0..CLIENT.1 {
        for x in 0..CLIENT.0 {
            let grip = x >= CLIENT.0 - 16 && y >= CLIENT.1 - 16;
            if (y >= EXTRA_Y - 1 && x >= FIELD_X - 1) || grip {
                continue;
            }
            assert_eq!(
                c.get(x, y),
                base.get(x, y),
                "pixel ({x}, {y}) outside the extra rows unchanged"
            );
        }
    }
    assert_eq!(
        c.get(FIELD_X, EXTRA_Y),
        cs.button_shadow.argb(),
        "check box edge at (FIELD_X, EXTRA_Y)"
    );
    assert_eq!(
        c.get(FIELD_X + 6, EXTRA_Y + 6),
        cs.window.argb(),
        "check box well"
    );
    assert_eq!(
        c.get(FIELD_X, EXTRA_Y + 20),
        cs.button_shadow.argb(),
        "combo edge 20 px lower"
    );
}
