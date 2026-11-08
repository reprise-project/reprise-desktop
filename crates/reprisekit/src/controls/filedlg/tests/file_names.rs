use super::*;

#[test]
fn ctrl_and_shift_clicks_select_several_files_when_multiple() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(3), CTRL);
    assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\"");
    d.click(R, entry_hit(2), SHIFT);
    assert_eq!(
        d.file_name(),
        "\"b.txt\" \"a.txt\"",
        "shift ranges from the anchor"
    );
}

#[test]
fn several_names_start_at_the_focused_file() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(3), Mods::default());
    d.click(R, entry_hit(1), CTRL);
    d.click(R, entry_hit(2), CTRL);
    assert_eq!(d.file_name(), "\"b.txt\" \"c.txt\" \"a.txt\"");
    d.click(R, entry_hit(1), CTRL);
    assert_eq!(
        d.file_name(),
        "\"b.txt\" \"c.txt\"",
        "a file clicked out of the pick starts the walk"
    );
}

#[test]
fn without_multiple_ctrl_click_replaces_the_selection() {
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(3), CTRL);
    assert_eq!(d.file_name(), "c.txt");
}

#[test]
fn a_folder_in_a_multi_selection_stays_out_of_the_field() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(0), Mods::default());
    d.click(R, entry_hit(1), CTRL);
    d.click(R, entry_hit(2), CTRL);
    assert_eq!(d.file_name(), "\"b.txt\" \"a.txt\"");
}

#[test]
fn keyboard_multi_selection() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.key(R, form::Key::Down, None, SHIFT);
    assert_eq!(d.file_name(), "\"b.txt\" \"a.txt\"");
    d.key(R, form::Key::Down, None, CTRL);
    d.key(R, form::Key::Space, None, CTRL);
    assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\" \"b.txt\"");
}

#[test]
fn accept_carries_every_selected_name() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(2), CTRL);
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["b.txt", "a.txt"]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn quoted_names_split_and_a_plain_name_stays_whole() {
    assert_eq!(
        split_names("\"a b.txt\" \"c.txt\""),
        vec!["a b.txt", "c.txt"]
    );
    assert_eq!(split_names("a b.txt"), vec!["a b.txt"]);
    assert_eq!(split_names("  "), Vec::<String>::new());
    assert_eq!(
        split_names("x \"y\""),
        vec!["x \"y\""],
        "quotes split only from the start"
    );
}

fn save_dlg() -> FileDlg {
    let mut d = multi(DlgMode::Save, false);
    d.set_file_name("report.pdf");
    d
}

#[test]
fn a_paste_goes_into_file_name_only_while_it_has_the_focus() {
    let mut d = save_dlg();
    d.focus = Focus::FileName;
    d.filename.select_all();
    assert!(d.paste("notes.txt\nsecond line"));
    assert_eq!(
        d.file_name(),
        "notes.txt",
        "its first line replaces the selection"
    );
    d.focus = Focus::Shell;
    assert!(!d.paste("other"));
    assert_eq!(d.file_name(), "notes.txt");
}

#[test]
fn entering_a_clicked_folder_puts_the_suggested_name_back() {
    let mut d = save_dlg();
    d.click(R, entry_hit(0), Mods::default());
    assert_eq!(d.file_name(), "sub", "a click shows the folder");
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
    assert_eq!(d.file_name(), "report.pdf", "the suggested name is back");
}

#[test]
fn entering_a_typed_folder_name_empties_the_field() {
    let mut d = multi(DlgMode::Open, false);
    d.shell.selection.set_only(Some(0));
    d.filename.set_selected("");
    d.typed.clear();
    for c in "sub".chars() {
        d.key(
            R,
            form::Key::Edit(crate::textedit::EditKey::Insert(c.to_string())),
            None,
            Mods::default(),
        );
    }
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
    assert_eq!(d.file_name(), "");
}

#[test]
fn ctrl_click_off_leaves_the_remaining_file_in_the_field() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(3), CTRL);
    d.click(R, entry_hit(3), CTRL);
    assert_eq!(d.file_name(), "a.txt");
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn a_name_with_quotes_is_one_name_outside_multi_open() {
    let mut d = save_dlg();
    d.set_file_name("The \"Best\" Recipe.html");
    d.shell.selection.set_only(None);
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["The \"Best\" Recipe.html"]),
        e => panic!("expected Accept, got {e:?}"),
    }
    d.set_file_name(" spaced ");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec![" spaced "]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn double_clicking_a_folder_enters_it_in_every_mode() {
    for mode in [DlgMode::Open, DlgMode::Save, DlgMode::Folder] {
        let mut d = multi(mode, false);
        d.click(R, entry_hit(0), Mods::default());
        assert!(
            matches!(d.double_click(R, entry_hit(0)), Event::NeedListing(900)),
            "{mode:?}"
        );
    }
}

#[test]
fn double_clicking_a_file_accepts_it() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(2), Mods::default());
    match d.double_click(R, entry_hit(2)) {
        Event::Accept(names) => assert_eq!(names, vec!["b.txt"]),
        e => panic!("expected Accept, got {e:?}"),
    }
    let mut f = multi(DlgMode::Folder, false);
    assert!(
        matches!(f.double_click(R, entry_hit(2)), Event::Redraw | Event::None),
        "no file pick in folder mode"
    );
}

#[test]
fn space_on_the_focused_accept_button_accepts_and_is_text_in_the_name_field() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
    d.set_file_name("a.txt");
    let space = || form::Key::Edit(EditKey::Insert(" ".into()));
    d.focus = Focus::FileName;
    assert!(matches!(
        d.key(R, space(), Some(' '), Mods::default()),
        Event::Redraw
    ));
    assert_eq!(
        d.file_name(),
        " ",
        "Space in the name field is text, typed over the selected name"
    );
    d.set_file_name("a.txt");
    d.focus = Focus::Accept;
    match d.key(R, space(), Some(' '), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        other => panic!("expected Accept([a.txt]), got {other:?}"),
    }
    match d.key(R, form::Key::Space, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        other => panic!("expected Accept([a.txt]) from a flat Space, got {other:?}"),
    }
}

#[test]
fn enter_on_a_file_accepts_its_name() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
    d.set_file_name("a.txt");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        other => panic!("expected Accept, got {other:?}"),
    }
}
