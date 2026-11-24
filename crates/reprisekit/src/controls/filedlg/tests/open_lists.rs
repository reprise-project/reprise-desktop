use super::*;

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn with_place() -> FileDlg {
    let mut d = multi(DlgMode::Open, false);
    d.places.places = vec![Place {
        label: "My Computer".into(),
        icon: None,
        enabled: true,
        pinned: false,
    }];
    d
}

/// A left press at (`x`, `y`) as the host sends it, released.
fn press(d: &mut FileDlg, (x, y): (i32, i32)) -> Event {
    let hit = d.hit(&text(), 11, R, x, y);
    let ev = d.click_at(R, hit, Mods::default(), (x, y));
    d.release();
    ev
}

#[test]
fn a_press_beside_the_open_files_of_type_list_only_closes_it() {
    let mut d = with_place();
    let cm = ControlMetrics::CLASSIC;
    let lay = d.layout(&text(), 11, R, &cm);
    let entry = centre(d.shell_layout(R, &cm).cells[1]);
    let place = centre(lay.places_rects[0]);
    for (what, at) in [
        ("a file", entry),
        ("Open", centre(lay.accept)),
        ("Cancel", centre(lay.cancel)),
        ("Up One Level", centre(lay.toolbar_rects[TOOLBAR_UP])),
        ("a place", place),
        ("Look in", centre(lay.lookin.field)),
    ] {
        d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
        assert!(d.filetype.open.is_some());
        let ev = press(&mut d, at);
        assert!(
            matches!(ev, Event::Redraw),
            "{what} only closes the list, not {ev:?}"
        );
        assert_eq!(d.filetype.open, None, "{what} closes the list");
        assert_eq!(d.lookin.open, None, "{what} opens no other list");
        assert_eq!(names_of(&d), ["sub"], "{what} leaves the selection");
        assert_eq!(d.focus, Focus::FileType, "{what} keeps the focus");
        assert_eq!(d.places.selected, None, "{what} picks no place");
        assert!(!d.views_open, "{what} opens no menu");
    }
    press(&mut d, entry);
    assert_eq!(
        names_of(&d),
        ["a.txt"],
        "with the list closed the file is picked"
    );
}

#[test]
fn a_press_beside_the_open_look_in_list_only_closes_it() {
    let mut d = with_place();
    for (what, hit) in [
        ("a place", Some(Hit::Places(0))),
        ("Files of type", Some(Hit::FileType(combo::Hit::Drop))),
        ("Cancel", Some(Hit::Cancel)),
        ("nothing", None),
    ] {
        d.click(R, Some(Hit::LookIn(combo::Hit::Drop)), Mods::default());
        assert!(d.lookin.open.is_some());
        let ev = d.click(R, hit, Mods::default());
        assert!(
            matches!(ev, Event::Redraw),
            "{what} only closes the list, not {ev:?}"
        );
        assert_eq!(d.lookin.open, None, "{what} closes the list");
        assert_eq!(d.filetype.open, None, "{what} opens no other list");
        assert_eq!(d.places.selected, None, "{what} picks no place");
    }
}

#[test]
fn a_press_beside_an_open_extra_list_only_closes_it() {
    let mut d = with_extra();
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let combo = d.hit(&text(), 11, big, FIELD_X + 5, EXTRA_Y + 25);
    let check = d.hit(&text(), 11, big, FIELD_X + 5, EXTRA_Y + 5);
    assert!(
        matches!(check, Some(Hit::Extra(h)) if h.item == 0),
        "got {check:?}"
    );
    for (what, hit) in [
        ("a file", entry_hit(1)),
        ("Files of type", Some(Hit::FileType(combo::Hit::Drop))),
        ("the check box", check),
    ] {
        d.click(big, combo, Mods::default());
        assert!(d.extra().unwrap().popup_open(), "the encoding list drops");
        let ev = d.click(big, hit, Mods::default());
        assert!(
            matches!(ev, Event::Redraw),
            "{what} only closes the list, not {ev:?}"
        );
        assert!(!d.extra().unwrap().popup_open(), "{what} closes the list");
        assert_eq!(d.filetype.open, None, "{what} opens no other list");
        assert_eq!(names_of(&d), ["sub"], "{what} leaves the selection");
        assert_eq!(d.extra().unwrap().checked("extra-check"), Some(false));
    }
}

fn names_of(d: &FileDlg) -> Vec<&str> {
    d.selected().iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn a_place_beside_an_open_list_stays_cold() {
    let mut d = with_place();
    for open in [
        Hit::FileType(combo::Hit::Drop),
        Hit::LookIn(combo::Hit::Drop),
    ] {
        d.click(R, Some(open), Mods::default());
        d.hover(Some(Hit::Places(0)));
        assert_eq!(d.places.hot, None, "{open:?}'s list is open");
        d.click(R, None, Mods::default());
        d.hover(Some(Hit::Places(0)));
        assert_eq!(d.places.hot, Some(0), "with {open:?}'s list closed");
        d.leave();
    }
}

#[test]
fn a_cut_extra_label_under_the_open_files_of_type_list_tips_nothing() {
    const LONG: &str = "Character encoding used when the document is written back to disk";
    let mut d = multi(DlgMode::Open, false);
    let form = form::Form::new(vec![form::Item::label_at(
        "long", FIELD_X, EXTRA_Y, 60, LONG,
    )]);
    d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let t = text();
    let mut c = Canvas::new(big.width as u32, big.height as u32);
    d.draw(&mut c, &t, &Theme::default().paint(), big, 11);
    let (x, y) = (FIELD_X + 20, EXTRA_Y + 5);
    d.filetype.open = Some(0);
    d.motion(&t, 11, big, x, y);
    d.blink();
    assert_eq!(d.extra().unwrap().label_tip(), None);
    d.filetype.open = None;
    d.motion(&t, 11, big, x, y);
    d.blink();
    assert_eq!(
        d.extra().unwrap().label_tip().as_deref(),
        Some(LONG),
        "with the list closed"
    );
}
