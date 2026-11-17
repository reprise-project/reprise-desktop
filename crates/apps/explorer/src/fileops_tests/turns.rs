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

fn ask_failed(ex: &mut Explorer, ctx: &mut repriseapp::Ctx) {
    let meter = std::sync::Arc::new(reprisefs::progress::Meter::default());
    let job = reprisefs::progress::Asked::new(meter);
    ex.ask_failed(job, "Error Copying File or Folder", "x".into(), ctx);
}

fn connect_up(ex: &Explorer) -> bool {
    matches!(ex.dialog(), Some(Dialog::Connect(_)))
}

/// Connect to Server only shows a connection under way, so a job's question comes up in its
/// place at once, and Connect to Server comes back after it.
#[test]
fn a_jobs_question_comes_up_over_a_busy_connect_to_server_which_returns_after_it() {
    let mut ctx = test_ctx();
    let (mut ex, _steps) = crate::shares::tests::connecting("sftp://nas", &mut ctx);
    let connect = box_win(&ex).expect("Connect to Server's window");
    ask_failed(&mut ex, &mut ctx);
    assert!(ex.holds_a_job(), "the failure box is up");
    let failure = box_win(&ex).expect("the failure box's window");
    assert_ne!(failure, connect, "the failure box maps a window of its own");

    ex.event(failure, enter(), &mut ctx);
    assert!(connect_up(&ex), "Connect to Server is back");
    assert_ne!(box_win(&ex), Some(failure));
}

#[test]
fn a_pastes_replace_prompt_comes_up_over_a_busy_connect_to_server() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir(d).expect("mkdir");
        std::fs::write(d.join("a.txt"), "a").expect("write");
    }
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    let _steps = crate::shares::tests::connect_in(&mut ex, "sftp://nas", &mut ctx);
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![src.join("a.txt")],
        },
        &mut ctx,
    );
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string()))
    );

    let prompt = box_win(&ex).expect("the prompt's window");
    ex.event(prompt, key_event_escape(), &mut ctx);
    assert!(connect_up(&ex), "Connect to Server is back");
}

#[test]
fn a_jobs_question_waits_behind_connect_to_server_while_an_address_is_typed() {
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Computer, &mut ctx);
    ex.open_connect(&mut ctx);
    ask_failed(&mut ex, &mut ctx);
    assert!(connect_up(&ex), "Connect to Server stays up");

    let connect = box_win(&ex).expect("Connect to Server's window");
    ex.event(connect, key_event_escape(), &mut ctx);
    assert!(ex.holds_a_job(), "the failure box is up after it");
}

/// Connect to Server with an address typed, `box_up` raised over it, and Connect pressed.
fn press_connect_with(
    box_up: fn(&mut Explorer, &mut repriseapp::Ctx),
    ctx: &mut repriseapp::Ctx,
) -> (Explorer, repriseapp::WinId) {
    let mut ex = crate::tests::test_explorer(Place::Computer, ctx);
    ex.last_server = "sftp://nas".into();
    ex.open_connect(ctx);
    box_up(&mut ex, ctx);
    let connect = box_win(&ex).expect("Connect to Server's window");
    ex.event(connect, enter(), ctx);
    (ex, connect)
}

#[test]
fn a_jobs_question_raised_while_an_address_is_typed_comes_up_when_connect_is_pressed() {
    let mut ctx = test_ctx();
    let (mut ex, connect) = press_connect_with(ask_failed, &mut ctx);
    assert!(ex.holds_a_job(), "the failure box is up");
    let failure = box_win(&ex).expect("the failure box's window");
    assert_ne!(failure, connect, "the failure box maps a window of its own");

    ex.event(failure, enter(), &mut ctx);
    assert!(ex.connecting_up(), "Connect to Server is back, connecting");
    assert_ne!(box_win(&ex), Some(failure));
}

#[test]
fn a_message_raised_while_an_address_is_typed_waits_behind_the_busy_connect_to_server() {
    let mut ctx = test_ctx();
    let (ex, _) = press_connect_with(
        |ex, ctx| drop(ex.message_box("Explorer", "plain".into(), ctx)),
        &mut ctx,
    );
    assert!(ex.connecting_up(), "Connect to Server stays up, connecting");
    assert_eq!(ex.boxes_waiting(), 1);
}

/// The password box and the server's question each give way to Connect to Server, connecting.
#[test]
fn a_jobs_question_raised_over_a_connections_box_comes_up_when_it_is_answered() {
    use reprisefs::netshare::mount::{self, Step};
    use reprisekit::controls::msgbox;
    let question = || {
        Step::Question(mount::Question {
            message: "The identity of nas is unknown.".into(),
            choices: vec!["Log In Anyway".into(), "Cancel Login".into()],
        })
    };
    for step in [Step::Login(crate::shares::tests::login(false)), question()] {
        let mut ctx = test_ctx();
        let (mut ex, steps) = crate::shares::tests::connecting("sftp://nas", &mut ctx);
        steps.send(step).expect("the step");
        ex.poll_connection(&mut ctx);
        if let Some(Dialog::Login(b)) = ex.dialogs.up_mut() {
            b.form
                .show_edit_text(reprisekit::controls::netconnect::PASSWORD, "secret");
        }
        ask_failed(&mut ex, &mut ctx);
        assert!(
            !ex.holds_a_job(),
            "the failure box waits behind the connection's box"
        );

        let asked = box_win(&ex).expect("the connection's box window");
        let answer = match ex.box_form() {
            // The question's Enter is Cancel Login: the click is on Log In Anyway.
            Some(f) => {
                let r = f.item(msgbox::button_id(0)).expect("Log In Anyway").rect;
                repriseapp::Event::Pointer(repriseapp::PointerInput::Press {
                    x: r.x + r.width / 2,
                    y: r.y + r.height / 2,
                    button: crate::BTN_LEFT,
                    time: 0,
                    mods: repriseapp::Mods::default(),
                })
            }
            None => enter(),
        };
        ex.event(asked, answer, &mut ctx);
        assert!(ex.holds_a_job(), "the failure box is up after the answer");
        let failure = box_win(&ex).expect("the failure box's window");
        assert_ne!(failure, asked, "the failure box maps a window of its own");

        ex.event(failure, enter(), &mut ctx);
        assert!(ex.connecting_up(), "Connect to Server is back, connecting");
    }
}
