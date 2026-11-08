use super::*;

#[test]
fn alt_letters_focus_the_fields_and_press_the_accept_button() {
    let mut d = sample();
    let alt = |d: &mut FileDlg, c| d.key(R, form::Key::Access(c), None, Mods::default());
    assert!(matches!(alt(&mut d, 'i'), Event::Redraw));
    assert_eq!(d.focus, Focus::LookIn);
    alt(&mut d, 'T');
    assert_eq!(d.focus, Focus::FileType);
    d.filetype.open = Some(0);
    alt(&mut d, 'n');
    assert_eq!(d.focus, Focus::FileName);
    assert_eq!(d.filetype.open, None, "a key closes the dropped list");
    assert!(matches!(alt(&mut d, 'c'), Event::None), "Cancel has no key");
    d.filename.text = "a.txt".into();
    match alt(&mut d, 'o') {
        Event::Accept(names) => assert_eq!(names, ["a.txt"]),
        other => panic!("Alt+O accepts, got {other:?}"),
    }
    assert_eq!(d.focus, Focus::Accept);
}

#[test]
fn the_accept_button_takes_a_key_the_labels_leave_free() {
    let with = |label: &str| {
        let mut d = sample();
        d.fields.accept_label = label.into();
        d.accept_label().into_owned()
    };
    assert_eq!(with("Open"), "&Open");
    assert_eq!(with("Save"), "&Save");
    assert_eq!(with("Import"), "I&mport", "i is Look in's");
    assert_eq!(with("E&xport"), "E&xport", "the caller's own key stays");
}

#[test]
fn alt_underlines_the_labels_and_the_accept_button() {
    let d = sample();
    let t = text();
    let theme = Theme::default();
    let cs = &theme.colors;
    let shot = |keys: bool| {
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        let p = Paint {
            access_keys: keys,
            ..classic_paint(cs, 1)
        };
        d.draw(&mut c, &t, &p, R, 11);
        c
    };
    let (off, on) = (shot(false), shot(true));
    let lay = d.layout(&t, 11, R, &d.drawn.0.get());
    let ink = cs.button_text.argb();
    // Each label's key is underlined one px under the baseline, and only with Alt.
    for (r, label) in [
        (lay.lookin_label, lookin_label()),
        (lay.filename_label, filename_label()),
        (lay.filetype_label, filetype_label()),
    ] {
        let (plain, m) = split_mnemonic(&label);
        let base = t.baseline(r, false, 11).unwrap();
        let (dx, w) = t.span(&plain, m.unwrap(), false, 11);
        for x in r.x + dx..r.x + dx + w {
            assert_eq!(on.get(x, base + 1), ink, "{label}: underlined at x {x}");
            assert_ne!(off.get(x, base + 1), ink, "{label}: bare at x {x}");
        }
    }
    let differs = |r: Rect| {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| on.get(x, y) != off.get(x, y)))
    };
    assert!(differs(lay.accept), "Open's O is underlined");
    assert!(!differs(lay.cancel), "Cancel has no key");
    assert!(!differs(lay.shell.frame), "the view is untouched");
}

#[test]
fn arrow_keys_step_the_listing_before_any_draw() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
    d.focus = Focus::Shell;
    d.shell.selection.set_only(Some(0));
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::Redraw
    ));
    assert_eq!(d.shell.selection.single(), Some(1));
}

#[test]
fn escape_cancels() {
    let mut d = sample();
    assert!(matches!(
        d.key(R, form::Key::Escape, None, Mods::default()),
        Event::Cancel
    ));
}

/// `combo_key` has no Escape arm, so Escape must not reach it.
#[test]
fn escape_closes_an_open_combo_instead_of_the_dialog() {
    let mut d = sample();
    d.lookin.open = Some(0);
    let ev = d.key(R, form::Key::Escape, None, Mods::default());
    assert!(matches!(ev, Event::Redraw), "got {ev:?}");
    assert!(d.lookin.open.is_none(), "the list closed");
}

/// `activate` would accept the File name field and end the dialog.
#[test]
fn enter_commits_an_open_combo_instead_of_accepting() {
    let mut d = sample();
    d.filename.set_selected("a.txt");
    d.lookin.open = Some(0);
    let ev = d.key(R, form::Key::Enter, None, Mods::default());
    assert!(
        !matches!(ev, Event::Accept(_) | Event::Cancel),
        "the dialog must not end while a list is dropped: {ev:?}"
    );
    assert!(d.lookin.open.is_none(), "the list closed");
}

#[test]
fn tab_wraps_the_focus_order() {
    let mut d = sample();
    assert_eq!(d.focus, Focus::FileName);
    d.key(R, form::Key::Tab { back: true }, None, Mods::default());
    assert_eq!(d.focus, Focus::LookIn);
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::FileName);
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::FileType);
}
