//! A box that comes up after another shows in a window of its own ([`Explorer::dialog_win`]).

use super::*;

fn box_win(ex: &Explorer) -> Option<repriseapp::WinId> {
    (ex.children().into_iter())
        .map(|c| c.id)
        .find(|&id| id == crate::DIALOG || id == crate::dialogs::NEXT_DIALOG)
}

#[test]
fn a_box_up_after_an_answer_is_in_the_other_window_and_a_late_event_misses_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.message_box("Explorer", "first".into(), &mut ctx);
    ex.message_box("Explorer", "second".into(), &mut ctx);
    let first = box_win(&ex).expect("a box window");

    ex.event(first, enter(), &mut ctx);
    let second = box_win(&ex).expect("the second box's window");
    assert_ne!(second, first, "the second box maps a window of its own");
    ex.event(first, enter(), &mut ctx);
    assert!(
        matches!(ex.dialog(), Some(Dialog::Message { text, .. }) if text == "second"),
        "an Enter for the first box's window leaves the second up"
    );
    ex.event(second, enter(), &mut ctx);
    assert_eq!(box_win(&ex), None);
}

#[test]
fn a_clash_asked_after_an_answer_is_in_the_other_window() {
    let mut ctx = test_ctx();
    let (_t, _dest, mut ex) = super::replace::paste_onto_taken_names(&mut ctx);
    let first = box_win(&ex).expect("the prompt's window");
    ex.event(first, key_event_alt('y'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "b.txt".to_string()))
    );
    assert_ne!(
        box_win(&ex),
        Some(first),
        "the next clash maps a window of its own"
    );
}

#[test]
fn the_picker_keeps_its_window_across_its_events() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.open_browse(false, &mut ctx);
    let win = box_win(&ex).expect("the picker's window");
    ex.event(win, right(), &mut ctx);
    assert!(matches!(ex.dialog(), Some(Dialog::Browse(_))));
    assert_eq!(box_win(&ex), Some(win));
}
