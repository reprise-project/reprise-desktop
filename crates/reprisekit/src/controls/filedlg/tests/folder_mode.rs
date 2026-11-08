use super::*;

#[test]
fn folder_mode_has_no_files_of_type_row() {
    let mut d = multi(DlgMode::Folder, false);
    let t = text();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    d.draw(&mut c, &t, &classic_paint(cs, 1), R, 11);
    for y in FILETYPE_Y..FILETYPE_Y + FILETYPE_H {
        for x in ROW_LABEL_X..FIELD_X + field_w(CLIENT.0) {
            assert_eq!(
                c.get(x, y),
                cs.button_face.argb(),
                "({x}, {y}) is bare dialog face"
            );
        }
    }
    assert!(
        !matches!(
            d.hit(&t, 11, R, FIELD_X + 20, FILETYPE_Y + 10),
            Some(Hit::FileType(_))
        ),
        "no Files of type combo to hit"
    );
    d.focus = Focus::FileName;
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Accept, "Tab goes from File name to Select");
    d.key(R, form::Key::Tab { back: true }, None, Mods::default());
    assert_eq!(d.focus, Focus::FileName, "and back");
}

#[test]
fn folder_mode_accepts_the_selected_subfolder_from_the_button() {
    let mut d = multi(DlgMode::Folder, false);
    d.click(R, entry_hit(0), Mods::default());
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["sub"]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn a_keyboard_pick_replaces_a_clicked_folders_name_before_enter() {
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(0), Mods::default());
    assert_eq!(d.file_name(), "sub");
    d.key(R, form::Key::End, None, Mods::default());
    assert_eq!(d.file_name(), "c.txt");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["c.txt"]),
        e => panic!("expected Accept of c.txt, got {e:?}"),
    }
    d.click(R, entry_hit(0), Mods::default());
    d.key(R, form::Key::Char('b'), Some('b'), Mods::default());
    assert_eq!(d.file_name(), "b.txt", "a typed letter picks too");
}

#[test]
fn folder_mode_enter_on_a_subfolder_opens_it() {
    let mut d = multi(DlgMode::Folder, false);
    d.click(R, entry_hit(0), Mods::default());
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
}

#[test]
fn folder_mode_with_nothing_picked_accepts_the_open_folder() {
    let mut d = multi(DlgMode::Folder, false);
    d.shell.selection.set_only(None);
    d.filename.set_selected("");
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["."]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn folder_mode_accepts_the_open_folder_right_after_listing() {
    let mut d = multi(DlgMode::Folder, false);
    assert_eq!(
        d.shell.selection.focus, None,
        "a folder listing starts with nothing selected"
    );
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["."]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn folder_mode_enter_then_select_picks_the_entered_folder() {
    let mut d = multi(DlgMode::Folder, false);
    d.click(R, entry_hit(0), Mods::default());
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
    d.set_listing(folder(2, Some(1), vec![]));
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["."]),
        e => panic!("expected Accept(.), got {e:?}"),
    }
}
