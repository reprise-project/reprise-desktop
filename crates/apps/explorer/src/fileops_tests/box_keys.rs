//! The keys of the message and question boxes: Tab, Shift+Tab and the arrows move the focus
//! between the buttons, and the focus stays there for Space and Enter until the box goes.

use super::*;
use reprisefs::netshare::mount::{self, Step};
use reprisekit::controls::msgbox;
use reprisekit::textedit::keysym;

fn tab(back: bool) -> repriseapp::Event {
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym: if back {
            keysym::ISO_LEFT_TAB
        } else {
            keysym::TAB
        },
        utf8: None,
        mods: repriseapp::Mods {
            shift: back,
            ..repriseapp::Mods::default()
        },
        edit: None,
    })
}

fn space() -> repriseapp::Event {
    let edit = reprisekit::textedit::key_for(keysym::SPACE, false, false, Some(" "));
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym: keysym::SPACE,
        utf8: Some(" ".into()),
        mods: repriseapp::Mods::default(),
        edit,
    })
}

fn left() -> repriseapp::Event {
    key_event_plain(
        keysym::LEFT,
        Some(reprisekit::textedit::EditKey::Left {
            select: false,
            word: false,
        }),
    )
}

/// The box up's window.
fn win(ex: &Explorer) -> repriseapp::WinId {
    (ex.children().into_iter())
        .map(|c| c.id)
        .find(|&id| crate::dialogs::is_dialog(id))
        .expect("a box is up")
}

/// The button the box up has the focus on, as it is drawn.
fn focused(ex: &Explorer) -> Option<&'static str> {
    let f = ex.box_form().expect("a message or question box is up");
    f.focus().map(|i| f.items[i].id)
}

fn key(ex: &mut Explorer, ev: repriseapp::Event, ctx: &mut repriseapp::Ctx) {
    let w = win(ex);
    ex.event(w, ev, ctx);
}

fn host_key_question(choices: &[&str]) -> Step {
    Step::Question(mount::Question {
        message: "The identity of box is unknown.".into(),
        choices: choices.iter().map(|c| c.to_string()).collect(),
    })
}

/// The server's host-key question opens on Cancel Login; Tab moves to Log In Anyway, and Space
/// or Enter presses it: the connection goes on.
#[test]
fn tab_to_log_in_anyway_and_space_or_enter_presses_it() {
    for press in [space(), enter()] {
        let mut ctx = test_ctx();
        let (mut ex, tx) = crate::shares::tests::connecting("sftp://box/", &mut ctx);
        tx.send(host_key_question(&["Log In Anyway", "Cancel Login"]))
            .expect("the step");
        ex.poll_connection(&mut ctx);
        assert_eq!(
            focused(&ex),
            Some(msgbox::button_id(1)),
            "opens on Cancel Login"
        );
        key(&mut ex, tab(false), &mut ctx);
        assert_eq!(
            focused(&ex),
            Some(msgbox::button_id(0)),
            "Tab moves to Log In Anyway"
        );
        key(&mut ex, press, &mut ctx);
        assert!(
            ex.connecting_up(),
            "the question is answered and Connect to Server is back, connecting"
        );
    }
}

/// A three-choice question: Shift+Tab and Left walk back from the default, the focus stays
/// between the keys, and Escape still cancels.
#[test]
fn a_three_button_question_keeps_the_focus_its_keys_move_and_escape_cancels() {
    let mut ctx = test_ctx();
    let (mut ex, tx) = crate::shares::tests::connecting("sftp://box/", &mut ctx);
    tx.send(host_key_question(&["Yes", "No", "Cancel"]))
        .expect("the step");
    ex.poll_connection(&mut ctx);
    assert_eq!(focused(&ex), Some(msgbox::button_id(2)), "opens on Cancel");
    key(&mut ex, tab(true), &mut ctx);
    assert_eq!(focused(&ex), Some(msgbox::button_id(1)), "Shift+Tab: No");
    key(&mut ex, left(), &mut ctx);
    assert_eq!(focused(&ex), Some(msgbox::button_id(0)), "Left: Yes");
    key(&mut ex, right(), &mut ctx);
    assert_eq!(focused(&ex), Some(msgbox::button_id(1)), "Right: No");
    key(&mut ex, key_event_escape(), &mut ctx);
    assert!(
        ex.dialog().is_none() && ex.connection.is_none(),
        "Escape cancels the connection"
    );
}

/// A question that takes the place of another, answered or not, opens on its own default.
#[test]
fn a_new_question_opens_on_its_default_whatever_the_last_was_left_on() {
    let mut ctx = test_ctx();
    let (mut ex, tx) = crate::shares::tests::connecting("sftp://box/", &mut ctx);
    let two = ["Log In Anyway", "Cancel Login"];
    tx.send(host_key_question(&two)).expect("the step");
    ex.poll_connection(&mut ctx);
    key(&mut ex, tab(false), &mut ctx);
    assert_eq!(focused(&ex), Some(msgbox::button_id(0)));

    tx.send(host_key_question(&two)).expect("the step");
    ex.poll_connection(&mut ctx);
    assert_eq!(
        focused(&ex),
        Some(msgbox::button_id(1)),
        "a question in place of an unanswered one opens on Cancel Login"
    );

    key(&mut ex, tab(false), &mut ctx);
    key(&mut ex, enter(), &mut ctx);
    assert!(ex.connecting_up(), "answered");
    tx.send(host_key_question(&two)).expect("the step");
    ex.poll_connection(&mut ctx);
    assert_eq!(
        focused(&ex),
        Some(msgbox::button_id(1)),
        "the next question opens on Cancel Login"
    );
}

/// A message box's one button keeps the focus through Tab, and Space presses it.
#[test]
fn a_message_box_keeps_its_one_button_through_tab_and_space_closes_it() {
    let mut ctx = test_ctx();
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.message_box("Explorer", "first".into(), &mut ctx);
    ex.message_box("Explorer", "second".into(), &mut ctx);
    key(&mut ex, tab(false), &mut ctx);
    assert_eq!(focused(&ex), Some(msgbox::OK));
    key(&mut ex, space(), &mut ctx);
    assert!(
        matches!(ex.dialog(), Some(Dialog::Message { text, .. }) if text == "second"),
        "Space closes the first box and the second comes up"
    );
    assert_eq!(focused(&ex), Some(msgbox::OK));
}

/// The replace prompt: Tab to No and Space keeps the file; the next clash opens on Yes, and Enter
/// on the button an arrow moved to presses it.
#[test]
fn the_replace_prompt_keeps_the_focus_and_the_next_clash_opens_on_yes() {
    use reprisekit::controls::replacebox;
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = super::replace::paste_onto_taken_names(&mut ctx);
    let yes = focused(&ex);
    key(&mut ex, tab(false), &mut ctx);
    key(&mut ex, tab(false), &mut ctx);
    assert_eq!(focused(&ex), Some(replacebox::NO), "Tab twice: No");
    key(&mut ex, space(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace".to_string(), "b.txt".to_string()))
    );
    assert_eq!(focused(&ex), yes, "the next clash opens on Yes");
    key(&mut ex, right(), &mut ctx);
    key(&mut ex, right(), &mut ctx);
    key(&mut ex, enter(), &mut ctx);
    assert!(
        matches!(asked(&ex), Some((_, name)) if name == "Reports"),
        "No on b.txt asks about Reports"
    );
    ex.event(win(&ex), key_event_alt('n'), &mut ctx);
    ex.finish_puts(&mut ctx);
    for name in ["a.txt", "b.txt"] {
        assert_eq!(
            std::fs::read_to_string(dest.join(name)).expect("read"),
            "old",
            "No keeps {name}"
        );
    }
}

/// A job's Skip/Retry/Cancel box opens on Skip; the arrows move the focus and it stays there.
#[test]
fn a_failed_items_box_keeps_the_focus_its_arrows_move() {
    use reprisekit::controls::dlgtemplate::BUTTON_IDS;
    let mut ctx = test_ctx();
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let meter = std::sync::Arc::new(reprisefs::progress::Meter::default());
    let job = reprisefs::progress::Asked::new(meter);
    ex.ask_failed(job, "Error Copying File or Folder", "x".into(), &mut ctx);
    assert_eq!(focused(&ex), Some(BUTTON_IDS[0]), "opens on Skip");
    key(&mut ex, right(), &mut ctx);
    key(&mut ex, right(), &mut ctx);
    assert_eq!(focused(&ex), Some(BUTTON_IDS[2]), "Right twice: Cancel");
    key(&mut ex, space(), &mut ctx);
    assert!(!ex.holds_a_job(), "Cancel answers the box");
}
