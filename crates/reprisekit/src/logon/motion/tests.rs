use super::*;
use crate::logon::{LogonKey, LogonUser, PowerAction, Prompt, text as strings};

fn users(n: usize) -> Vec<LogonUser> {
    (0..n)
        .map(|i| LogonUser {
            name: format!("user{i}"),
            real_name: String::new(),
            picture: None,
            note: None,
            stock: i,
        })
        .collect()
}

/// Glass on the tiles, moving.
fn tiles() -> Logon {
    let mut m = Logon::new(Interface::Glass, users(3), true, None);
    m.click(Part::SwitchUser);
    m.moves = Moves::Full;
    m
}

fn change(m: &mut Logon) -> Option<Change> {
    m.take_motion().map(|t| t.change)
}

#[test]
fn every_screen_change_is_noted_with_its_change() {
    let mut m = tiles();
    m.click(Part::Tile(1));
    assert_eq!(change(&mut m), Some(Change::Pick(Part::Tile(1))));
    m.key(LogonKey::Escape);
    assert_eq!(change(&mut m), Some(Change::Back(Part::Tile(1))));
    m.click(Part::OtherUser);
    assert_eq!(change(&mut m), Some(Change::Pick(Part::OtherUser)));
    m.click(Part::SwitchUser);
    assert_eq!(change(&mut m), Some(Change::Back(Part::OtherUser)));

    m.click(Part::Tile(0));
    m.take_motion();
    m.key(LogonKey::Text("abc"));
    assert_eq!(change(&mut m), None, "typing changes no screen");
    m.sent();
    assert_eq!(change(&mut m), Some(Change::Busy));
    m.starting();
    assert_eq!(change(&mut m), Some(Change::Status));
    m.fail(strings::incorrect());
    assert_eq!(change(&mut m), Some(Change::Fail));
    m.click(Part::MessageOk);
    assert_eq!(change(&mut m), Some(Change::MessageOk));
    m.sent();
    m.ask(Prompt {
        text: "Code:".into(),
        echo: true,
    });
    m.take_motion();
    m.set_busy(true);
    assert_eq!(change(&mut m), Some(Change::Busy));
    m.set_busy(false);
    assert_eq!(change(&mut m), Some(Change::Idle));
    m.admin(PowerAction::Reboot);
    assert_eq!(change(&mut m), Some(Change::Admin));
    m.key(LogonKey::Text("root"));
    m.key(LogonKey::Enter);
    m.sent();
    m.going_down();
    m.take_motion();
    m.enter();
    assert_eq!(change(&mut m), Some(Change::Enter));
}

#[test]
fn an_ask_after_the_password_is_noted() {
    let mut m = tiles();
    m.click(Part::Tile(0));
    m.sent();
    m.take_motion();
    m.ask(Prompt {
        text: "Code:".into(),
        echo: false,
    });
    assert_eq!(change(&mut m), Some(Change::Ask));
}

#[test]
fn nothing_is_noted_when_off_or_under_classic() {
    let mut off = tiles();
    off.moves = Moves::Off;
    off.click(Part::Tile(0));
    off.sent();
    off.enter();
    assert!(off.motion.is_none(), "Off notes nothing");
    let mut classic = Logon::new(Interface::Classic, users(2), true, None);
    classic.moves = Moves::Full;
    classic.key(LogonKey::Text("user0"));
    classic.sent();
    classic.fail(strings::incorrect());
    classic.enter();
    assert!(classic.motion.is_none(), "Classic notes nothing");
}

#[test]
fn a_change_not_yet_taken_keeps_the_screen_it_began_from() {
    let mut m = tiles();
    m.click(Part::Tile(2));
    m.sent();
    let t = m.take_motion().unwrap();
    assert_eq!(t.change, Change::Busy, "the last change");
    assert_eq!(t.was.model().screen, Screen::Tiles, "the first screen");
    assert!(t.was.model().motion.is_none());
}

#[test]
fn the_screen_before_keeps_no_typed_secret() {
    let mut m = tiles();
    m.click(Part::Tile(0));
    m.take_motion();
    m.key(LogonKey::Text("hunter2"));
    m.key(LogonKey::Escape);
    let t = m.motion.clone().unwrap();
    assert!(!format!("{:?}", m.motion).contains("hunter2"));
    let was = t.was.model();
    assert_eq!(was.screen, Screen::Tile(0));
    assert!(was.password.is_secret());
    assert_eq!(was.password.text.chars().count(), 7, "as many stand-ins");
    assert!(!was.password.text.contains("hunter2"));
}

#[test]
fn sent_notes_the_password_row_before_wiping_it() {
    let mut m = tiles();
    m.click(Part::Tile(0));
    m.take_motion();
    m.key(LogonKey::Text("abc"));
    m.sent();
    assert!(m.password.text.is_empty() && m.busy);
    let t = m.take_motion().unwrap();
    assert_eq!(t.was.model().password.text.chars().count(), 3);
    assert!(!t.was.model().busy);
}

#[test]
fn an_echoed_answer_is_left_out_of_the_screen_before() {
    let mut m = tiles();
    m.click(Part::Tile(0));
    m.sent();
    m.ask(Prompt {
        text: "Code:".into(),
        echo: true,
    });
    m.take_motion();
    m.key(LogonKey::Text("4711"));
    m.sent();
    let t = m.take_motion().unwrap();
    assert!(t.was.model().password.text.is_empty());
}

#[test]
fn a_starting_session_is_welcomed_until_the_screen_is_idle_again() {
    let mut m = tiles();
    m.click(Part::Tile(0));
    m.sent();
    assert_eq!(m.busy_text(), strings::verifying());
    m.starting();
    assert_eq!(m.busy_text(), strings::welcome());
    m.set_busy(false);
    m.set_busy(true);
    assert_eq!(m.busy_text(), strings::verifying());
}
