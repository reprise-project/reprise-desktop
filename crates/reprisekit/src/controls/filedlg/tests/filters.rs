use super::*;

#[test]
fn a_type_label_carries_its_pattern_in_parentheses() {
    assert_eq!(type_pattern("Text Documents (*.txt)"), Some("*.txt"));
    assert_eq!(type_pattern("All Files (*.*)"), Some("*.*"));
    assert_eq!(
        type_pattern("Background Files (*.bmp;*.png) "),
        Some("*.bmp;*.png")
    );
    assert_eq!(type_pattern("Programs"), None);
    assert_eq!(
        type_pattern("Notes (draft)"),
        None,
        "no wildcard, no pattern"
    );
}

#[test]
fn a_pattern_keeps_names_any_of_its_globs_match() {
    assert!(pattern_keeps("*.txt", "notes.TXT"));
    assert!(!pattern_keeps("*.txt", "bash"));
    assert!(pattern_keeps("*.txt; *.log", "boot.log"));
    assert!(
        pattern_keeps("*.*", "bash"),
        "*.* keeps a name without a dot"
    );
    assert!(pattern_keeps("*", "bash"));
    assert!(pattern_keeps("b?sh", "bash"));
    assert!(!pattern_keeps("", "bash"));
}

#[test]
fn the_picked_type_gives_its_pattern() {
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![],
    );
    assert_eq!(d.picked_pattern().as_deref(), Some("*.txt"));
    d.set_filter(1);
    assert_eq!(d.picked_pattern().as_deref(), Some("*.*"));
}

#[test]
fn the_files_of_type_list_opens_above_when_it_would_pass_the_bottom() {
    let mut d = multi(DlgMode::Open, false);
    let t = text();
    let field = d.layout(&t, 11, R, &ControlMetrics::CLASSIC).filetype.field;
    d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    let list = d.layout(&t, 11, R, &ControlMetrics::CLASSIC).filetype.list;
    assert!(
        list.bottom() <= R.bottom(),
        "the list ends inside the client"
    );
    assert_eq!(list.bottom(), field.y, "it sits right on top of the field");
    let hit = d.hit(&t, 11, R, list.x + 10, list.y + 3);
    assert_eq!(
        hit,
        Some(Hit::FileType(combo::Hit::Row(0))),
        "its first row takes the click"
    );
}

/// A typed `*` or `?` pattern is not accepted: the host applies it, which
/// filters the view and leaves the pattern in the field.
#[test]
fn a_typed_pattern_filters_instead_of_accepting() {
    let mut d = multi(DlgMode::Open, false);
    d.set_file_name("*");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Wildcard { dir, name } => assert_eq!((dir.as_str(), name.as_str()), ("", "*")),
        e => panic!("expected Wildcard, got {e:?}"),
    }
    d.apply_wildcard("*");
    assert_eq!(d.wildcard(), Some("*"));
    assert_eq!(d.file_name(), "*");
    d.set_file_name("/etc/*.conf");
    let name = match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Wildcard { dir, name } => {
            assert_eq!((dir.as_str(), name.as_str()), ("/etc/", "/etc/*.conf"));
            name
        }
        e => panic!("expected Wildcard, got {e:?}"),
    };
    assert_eq!(d.wildcard(), Some("*"), "the old filter until applied");
    assert_eq!(d.file_name(), "/etc/*.conf", "the field as typed");
    d.apply_wildcard(&name);
    assert_eq!(d.wildcard(), Some("*.conf"));
    assert_eq!(d.file_name(), "*.conf", "the field keeps the pattern alone");
    d.set_file_name("/e*/a.txt");
    assert!(
        matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::Accept(_)
        ),
        "a pattern in the folder part is a name"
    );
}

#[test]
fn picking_a_type_drops_the_typed_pattern() {
    let mut d = multi(DlgMode::Open, false);
    d.set_file_name("*.txt");
    d.apply_wildcard("*.txt");
    assert_eq!(d.wildcard(), Some("*.txt"));
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::FilterChanged(1)
    ));
    assert_eq!(d.wildcard(), None);
}

#[test]
fn a_listed_name_or_folder_mode_takes_no_pattern() {
    let mut d = multi(DlgMode::Open, false);
    d.set_listing(folder(2, None, vec![file("why?.txt")]));
    d.set_file_name("why?.txt");
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::Accept(_)
    ));
    assert_eq!(d.wildcard(), None);
    let mut d = multi(DlgMode::Folder, false);
    d.set_file_name("*");
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::Accept(_)
    ));
}

#[test]
fn many_filters_open_a_list_that_stays_inside_the_top() {
    let filters: Vec<String> = (0..40).map(|i| format!("Type {i}")).collect();
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters,
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![],
    );
    d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    let list = d
        .layout(&text(), 11, R, &ControlMetrics::CLASSIC)
        .filetype
        .list;
    assert!(
        list.y >= 0,
        "the list starts at y {} inside the client",
        list.y
    );
    assert!(list.bottom() <= R.bottom());
}

#[test]
fn an_open_filter_list_takes_clicks_over_the_choice_rows() {
    let mut d = with_extra();
    let (w, h) = d.client();
    let big = Rect::new(0, 0, w, h);
    d.click(big, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    let list = d
        .layout(&text(), 11, big, &ControlMetrics::CLASSIC)
        .filetype
        .list;
    let row1 = list.y + 1 + d.filetype.row_h(&ControlMetrics::CLASSIC) + 2;
    assert!(row1 > EXTRA_Y, "the second row lies over the extra rows");
    let hit = d.hit(&text(), 11, big, list.x + 20, row1);
    assert_eq!(hit, Some(Hit::FileType(combo::Hit::Row(1))));
    let ev = d.motion(&text(), 11, big, list.x + 20, row1);
    assert!(matches!(ev, Event::Redraw), "got {ev:?}");
    assert_eq!(d.filetype.open, Some(1));
}

#[test]
fn set_filter_picks_the_row_and_clamps() {
    let mut d = multi(DlgMode::Open, false);
    d.set_filter(1);
    assert_eq!(d.filter(), 1);
    d.set_filter(9);
    assert_eq!(d.filter(), 1, "clamped to the last row");
}

#[test]
fn picking_another_filter_says_so() {
    let mut d = multi(DlgMode::Open, false);
    d.focus = Focus::FileType;
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::FilterChanged(1)
    ));
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::Redraw | Event::None
    ));
}
