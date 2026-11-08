use super::*;
use crate::textedit::keysym;

fn press(d: &mut FileDlg, sym: u32, ctrl: bool, shift: bool, alt: bool) -> Option<Event> {
    d.shortcut(R, sym, ctrl, shift, alt)
}

fn picked(d: &FileDlg) -> Vec<&str> {
    d.selected().iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn refresh_new_folder_and_the_alt_arrows_work_from_any_focus() {
    for focus in [Focus::FileName, Focus::Shell, Focus::Places] {
        let mut d = multi(DlgMode::Open, false);
        d.focus = focus;
        assert!(matches!(
            press(&mut d, keysym::F5, false, false, false),
            Some(Event::Refresh)
        ));
        let r = 'r' as u32;
        assert!(matches!(
            press(&mut d, r, true, false, false),
            Some(Event::Refresh)
        ));
        let n = 'N' as u32;
        assert!(matches!(
            press(&mut d, n, true, true, false),
            Some(Event::CreateFolder(name)) if name == "New Folder"
        ));
        assert!(matches!(
            press(&mut d, keysym::UP, false, false, true),
            Some(Event::None)
        ));
    }
    let mut d = multi(DlgMode::Open, false);
    d.set_listing(folder(2, Some(1), vec![file("x.txt")]));
    assert!(matches!(
        press(&mut d, keysym::UP, false, false, true),
        Some(Event::NeedListing(1))
    ));
    assert!(matches!(
        press(&mut d, keysym::LEFT, false, false, true),
        Some(Event::NeedListing(1))
    ));
    d.set_listing(folder(1, None, vec![file("a.txt")]));
    assert!(matches!(
        press(&mut d, keysym::RIGHT, false, false, true),
        Some(Event::NeedListing(2))
    ));
}

#[test]
fn the_list_keys_leave_file_name_its_editing_keys() {
    let mut d = multi(DlgMode::Open, true);
    d.focus = Focus::FileName;
    d.set_file_name("draft.txt");
    for (sym, ctrl) in [
        ('a' as u32, true),
        (keysym::F2, false),
        (keysym::DELETE, false),
    ] {
        assert!(press(&mut d, sym, ctrl, false, false).is_none(), "{sym:#x}");
    }
    d.key(
        R,
        form::Key::Edit(EditKey::SelectAll),
        None,
        Mods::default(),
    );
    assert_eq!(
        d.filename.selected_text(),
        "draft.txt",
        "Ctrl+A selects the field's text"
    );
    d.key(
        R,
        form::Key::Edit(EditKey::Delete { word: false }),
        None,
        Mods::default(),
    );
    assert_eq!(d.file_name(), "");
}

#[test]
fn ctrl_a_selects_every_entry_only_when_the_dialog_picks_several() {
    let mut d = multi(DlgMode::Open, true);
    d.focus = Focus::Shell;
    assert!(matches!(
        press(&mut d, 'a' as u32, true, false, false),
        Some(Event::Redraw)
    ));
    assert_eq!(picked(&d), ["sub", "a.txt", "b.txt", "c.txt"]);
    assert_eq!(d.file_name(), "\"a.txt\" \"b.txt\" \"c.txt\"");

    let mut d = multi(DlgMode::Open, false);
    d.focus = Focus::Shell;
    assert!(matches!(
        press(&mut d, 'a' as u32, true, false, false),
        Some(Event::None)
    ));
    assert_eq!(picked(&d), ["sub"]);
}

#[test]
fn delete_hands_the_selection_to_the_host() {
    let mut d = multi(DlgMode::Open, true);
    d.focus = Focus::Shell;
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(2), CTRL);
    match press(&mut d, keysym::DELETE, false, false, false) {
        Some(Event::Delete { names, erase }) => {
            assert_eq!(names, ["a.txt", "b.txt"]);
            assert!(!erase, "Delete goes to the Recycle Bin");
        }
        other => panic!("Delete hands the names over, got {other:?}"),
    }
    assert!(
        matches!(
            press(&mut d, keysym::DELETE, false, true, false),
            Some(Event::Delete { erase: true, .. })
        ),
        "Shift+Delete deletes for good"
    );
    d.shell.selection.clear();
    assert!(matches!(
        press(&mut d, keysym::KP_DELETE, false, false, false),
        Some(Event::None)
    ));
}

#[test]
fn the_menus_delete_clicked_with_shift_held_deletes_for_good() {
    const DELETE_ROW: usize = 2;
    let delete_row = Some(Hit::Menu {
        level: 0,
        row: Some(DELETE_ROW),
    });
    for (mods, erase) in [(Mods::default(), false), (SHIFT, true)] {
        let mut d = multi(DlgMode::Open, false);
        d.click(R, entry_hit(1), Mods::default());
        d.right_click(R, entry_hit(1), (100, 100));
        match d.click(R, delete_row, mods) {
            Event::Delete { names, erase: got } => {
                assert_eq!(names, ["a.txt"]);
                assert_eq!(got, erase, "{mods:?}");
            }
            other => panic!("the menu's Delete hands the name over, got {other:?}"),
        }
    }
}

#[test]
fn f2_renames_the_selected_entry_in_place() {
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(1), Mods::default());
    assert!(matches!(
        press(&mut d, keysym::F2, false, false, false),
        Some(Event::Redraw)
    ));
    let (i, ed) = d.shell.edit.as_ref().expect("the box is open");
    assert_eq!(
        (*i, ed.selected_text()),
        (1, "a"),
        "the name but its extension is selected"
    );
    assert!(d.focused_edit().is_some_and(|e| e.text == "a.txt"));
    assert!(
        press(&mut d, keysym::F5, false, false, false).is_none(),
        "the box takes F5"
    );
    let typed = |d: &mut FileDlg, k| d.key(R, k, None, Mods::default());
    typed(&mut d, form::Key::Edit(EditKey::Insert("n".into())));
    assert!(d.paste("ote/s"), "a paste goes into the box");
    match typed(&mut d, form::Key::Enter) {
        Event::Rename { from, to } => {
            assert_eq!((from.as_str(), to.as_str()), ("a.txt", "notes.txt"))
        }
        other => panic!("Enter renames, got {other:?}"),
    }
    assert!(d.shell.edit.is_none());

    press(&mut d, keysym::F2, false, false, false);
    typed(&mut d, form::Key::Edit(EditKey::Insert("z".into())));
    assert!(matches!(typed(&mut d, form::Key::Escape), Event::Redraw));
    assert!(d.shell.edit.is_none(), "Escape keeps the old name");
    press(&mut d, keysym::F2, false, false, false);
    assert!(
        matches!(typed(&mut d, form::Key::Tab { back: false }), Event::Redraw),
        "the same name is no rename"
    );

    d.click(R, entry_hit(2), CTRL);
    d.shell.selection.set = [1, 2].into();
    assert!(
        matches!(
            press(&mut d, keysym::F2, false, false, false),
            Some(Event::None)
        ),
        "one entry at a time"
    );
}

#[test]
fn a_press_outside_the_rename_box_renames() {
    let text = text();
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(1), Mods::default());
    d.shortcut(R, keysym::F2, false, false, false);
    if let Some((_, ed)) = &mut d.shell.edit {
        ed.select_all();
        ed.key(EditKey::Insert("b2.txt".into()));
    }
    let cm = ControlMetrics::CLASSIC;
    let lay = d.layout(&text, 11, R, &cm);
    let b = shellview::rename_rect(&d.shell, &lay.shell, 1, &text, 11).unwrap();
    let inside = d.hit(&text, 11, R, b.x + 3, b.y + b.height / 2);
    assert!(matches!(inside, Some(Hit::Rename(0))), "{inside:?}");
    assert!(matches!(d.click(R, inside, Mods::default()), Event::Redraw));
    assert!(d.shell.edit.is_some(), "a press in the box moves its caret");
    match d.click(R, Some(Hit::Places(0)), Mods::default()) {
        Event::Rename { to, .. } => assert_eq!(to, "b2.txt"),
        other => panic!("a press elsewhere renames, got {other:?}"),
    }
}

#[test]
fn a_new_folder_opens_in_the_rename_box_once_listed() {
    let mut d = multi(DlgMode::Open, false);
    d.expect("New Folder", true);
    assert!(d.shell.edit.is_none(), "not listed yet");
    d.set_listing(folder(
        1,
        None,
        vec![dir("New Folder"), dir("sub"), file("a.txt")],
    ));
    assert_eq!(picked(&d), ["New Folder"]);
    assert_eq!(d.focus, Focus::Shell);
    let (_, ed) = d.shell.edit.as_ref().expect("the box is open");
    assert_eq!(ed.selected_text(), "New Folder", "a folder's whole name");

    d.shell.edit = None;
    d.expect("gone", false);
    d.set_listing(folder(2, None, vec![dir("gone")]));
    assert!(d.expected.is_none(), "another folder forgets it");
    assert_eq!(picked(&d), ["gone"], "as the first entry, not as expected");
}

#[test]
fn a_refused_name_comes_back_in_the_box() {
    let mut d = multi(DlgMode::Open, false);
    assert!(d.resume_rename("a.txt", "b.txt"));
    let (i, ed) = d.shell.edit.as_ref().unwrap();
    assert_eq!(
        (*i, ed.text.as_str(), ed.selected_text()),
        (1, "b.txt", "b.txt")
    );
    assert!(!d.resume_rename("missing", "x"));
}
