use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use reprisebus::login;
use reprisekit::controls::form::{self, Control, Form};
use reprisekit::controls::remotepassword::{self, Action};
use repriselogin::account::{AccountRefusal, AccountReply, AccountRequest, RemoteAccess};
use repriselogin::frame::{read_msg, write_msg};
use zeroize::Zeroizing;

use super::Sources;
use crate::AccountsApp;
use crate::props::TAB_PASSWORD;
use crate::rdpassword::{self, CONFIRM, LOGIN, NEW};
use crate::users::tests::user;

const ANSWERED: Duration = Duration::from_secs(5);

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("reprise-accounts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A login manager on `dir`'s account socket: one request read per connection, each answered
/// with the next of `replies`. The requests come back on the receiver.
fn account_daemon(dir: &Path, replies: Vec<AccountReply>) -> (PathBuf, Receiver<AccountRequest>) {
    let path = dir.join("account.sock");
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for reply in replies {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let Ok(req) = read_msg::<AccountRequest>(&mut s) else {
                return;
            };
            let _ = tx.send(req);
            let _ = write_msg(&mut s, &reply);
        }
    });
    (path, rx)
}

/// Sources with a running login manager on `socket`.
fn running(socket: PathBuf) -> Sources {
    Sources {
        socket,
        manager: || login::Manager::Running,
        retry: Duration::ZERO,
    }
}

/// The app reading `sources`, with your account's Properties open on Password and the first
/// read landed.
fn app_with(sources: Sources, you: bool) -> AccountsApp {
    let mut a = AccountsApp::new();
    a.sources = sources;
    a.users.refresh(
        vec![user(1000, "jo", true, you), user(1001, "al", false, !you)],
        vec![None, None],
    );
    a.open_props(1000);
    a.props.as_mut().unwrap().sheet.tabs.open = TAB_PASSWORD;
    a.wake();
    a
}

fn page(a: &AccountsApp) -> &Form {
    &a.props.as_ref().expect("Properties are open").sheet.pages[TAB_PASSWORD]
}

fn lit(a: &AccountsApp, id: &str) -> bool {
    matches!(
        page(a).item(id).map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    )
}

fn status(a: &AccountsApp) -> Vec<String> {
    match page(a).item(remotepassword::STATUS).map(|i| &i.control) {
        Some(Control::Lines(l)) => l.clone(),
        _ => Vec::new(),
    }
}

fn press(a: &mut AccountsApp, id: &'static str) {
    let ev = reprisekit::controls::propsheet::Event::Page(TAB_PASSWORD, form::Event::Pressed(id));
    let out = a.props.as_mut().unwrap().event(ev, |_| Err(String::new()));
    if let crate::props::Outcome::RemotePassword(action) = out {
        a.open_rd_password(action);
    }
}

/// Type into the open dialog and press OK; the reply lands on the next wake.
fn ok(a: &mut AccountsApp, typed: &[(&'static str, &str)]) {
    let d = a.rd_password.as_mut().expect("the dialog is open");
    for (id, text) in typed {
        rdpassword::type_into(d, id, text);
    }
    let out = d.event(form::Event::Pressed(rdpassword::OK));
    a.rd_password_outcome(out);
}

fn send(a: &mut AccountsApp, typed: &[(&'static str, &str)]) {
    ok(a, typed);
    a.wake();
}

fn cancel(a: &mut AccountsApp) {
    let d = a.rd_password.as_mut().expect("the dialog is open");
    let out = d.event(form::Event::Cancel);
    a.rd_password_outcome(out);
}

const SET_NEW: [(&str, &str); 3] = [
    (LOGIN, "hunter2"),
    (NEW, "swordfish"),
    (CONFIRM, "swordfish"),
];

fn status_is(has: bool) -> AccountReply {
    answering(has, RemoteAccess::Allowed)
}

fn answering(has: bool, access: RemoteAccess) -> AccountReply {
    AccountReply::Status {
        remote_password: has,
        access,
    }
}

#[test]
fn create_sends_the_login_and_remote_passwords_and_shows_done() {
    let dir = tmp("rd-create");
    let (socket, seen) = account_daemon(&dir, vec![status_is(false), AccountReply::Done]);
    let mut a = app_with(running(socket), true);
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));
    assert_eq!(status(&a), [remotepassword::HAS_NONE]);
    assert!(lit(&a, remotepassword::CREATE));
    assert!(
        page(&a).item(remotepassword::REMOVE).is_none(),
        "nothing to remove"
    );

    press(&mut a, remotepassword::CREATE);
    assert_eq!(
        a.rd_password.as_ref().map(|d| d.title()),
        Some("Create Remote Desktop Password")
    );
    send(&mut a, &SET_NEW);
    assert_eq!(
        seen.recv_timeout(ANSWERED),
        Ok(AccountRequest::SetRemotePassword {
            login_password: Zeroizing::new("hunter2".into()),
            remote_password: Zeroizing::new("swordfish".into()),
        })
    );
    assert!(a.rd_password.is_none(), "Done closes the dialog");
    assert_eq!(status(&a), [remotepassword::HAS_ONE]);
    assert!(lit(&a, remotepassword::CHANGE) && lit(&a, remotepassword::REMOVE));
}

#[test]
fn a_wrong_login_password_is_said_in_the_dialog() {
    let dir = tmp("rd-wrong");
    let (socket, seen) = account_daemon(
        &dir,
        vec![
            status_is(true),
            AccountReply::Refused {
                reason: AccountRefusal::WrongPassword,
            },
        ],
    );
    let mut a = app_with(running(socket), true);
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));

    press(&mut a, remotepassword::REMOVE);
    send(&mut a, &[(LOGIN, "hunter3")]);
    assert_eq!(
        seen.recv_timeout(ANSWERED),
        Ok(AccountRequest::ClearRemotePassword {
            login_password: Zeroizing::new("hunter3".into()),
        })
    );
    let d = a.rd_password.as_ref().expect("the dialog stays open");
    assert_eq!(d.why(), "The password is incorrect.");
    assert_eq!(d.form.edit_text(LOGIN), "", "emptied to type again");
    assert_eq!(status(&a), [remotepassword::HAS_ONE], "still set");
}

#[test]
fn a_stopped_login_manager_is_not_asked_and_says_it_is_not_running() {
    let dir = tmp("rd-stopped");
    let (socket, seen) = account_daemon(&dir, vec![AccountReply::Done]);
    let a = app_with(
        Sources {
            manager: || login::Manager::Stopped,
            ..running(socket)
        },
        true,
    );
    assert!(seen.try_recv().is_err());
    assert!(!lit(&a, remotepassword::CREATE));
    let note = page(&a)
        .item(remotepassword::NOTE)
        .map(|i| i.control.clone());
    assert_eq!(
        note,
        Some(Control::Lines(vec![remotepassword::NO_MANAGER.into()]))
    );
}

#[test]
fn without_the_login_manager_the_password_page_has_no_section() {
    let dir = tmp("rd-missing");
    let a = app_with(
        Sources {
            manager: || login::Manager::Missing,
            ..running(dir.join("none.sock"))
        },
        true,
    );
    assert!(a.remote.found.is_some(), "read");
    assert!(page(&a).item(remotepassword::CREATE).is_none());
    assert!(page(&a).item(remotepassword::NOTE).is_none());
}

#[test]
fn another_accounts_page_says_only_that_each_user_sets_their_own() {
    let dir = tmp("rd-another");
    let (socket, _seen) = account_daemon(&dir, vec![status_is(true)]);
    let a = app_with(running(socket), false);
    let note = page(&a)
        .item(remotepassword::NOTE)
        .map(|i| i.control.clone());
    assert_eq!(
        note,
        Some(Control::GreyedLines(vec![remotepassword::NOT_YOURS.into()]))
    );
    for id in [
        remotepassword::CREATE,
        remotepassword::CHANGE,
        remotepassword::REMOVE,
    ] {
        assert!(page(&a).item(id).is_none(), "{id}");
    }
}

#[test]
fn a_silent_login_manager_is_asked_again_while_the_sheet_is_open() {
    let dir = tmp("rd-silent");
    let mut a = app_with(running(dir.join("none.sock")), true);
    assert!(!lit(&a, remotepassword::CREATE));
    assert!(a.remote.reading.is_some(), "asked again");
    let (socket, seen) = account_daemon(&dir, vec![status_is(false)]);
    a.sources.socket = socket;
    a.remote.reading = None;
    a.read_remote(Duration::ZERO);
    a.wake();
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));
    assert!(lit(&a, remotepassword::CREATE));
}

#[test]
fn a_failed_status_says_the_login_manager_is_not_answering_and_asks_again() {
    let dir = tmp("rd-status-failed");
    let failed = AccountReply::Refused {
        reason: AccountRefusal::Failed,
    };
    let (socket, seen) = account_daemon(&dir, vec![failed, status_is(false)]);
    let mut a = app_with(running(socket), true);
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));
    assert!(!lit(&a, remotepassword::CREATE));
    let note = page(&a)
        .item(remotepassword::NOTE)
        .map(|i| i.control.clone());
    assert_eq!(
        note,
        Some(Control::Lines(vec![remotepassword::SILENT.into()]))
    );
    assert!(a.remote.reading.is_some(), "asked again");
    a.wake();
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));
    assert!(lit(&a, remotepassword::CREATE));
}

#[test]
fn a_silent_login_manager_is_not_asked_again_for_another_accounts_page() {
    let dir = tmp("rd-silent-another");
    let a = app_with(running(dir.join("none.sock")), false);
    assert!(a.remote.found.is_some(), "read");
    assert!(a.remote.reading.is_none());
}

#[test]
fn outside_the_group_change_greys_and_remove_stays_lit() {
    let dir = tmp("rd-outside");
    let outside = RemoteAccess::NotInGroup {
        group: "staff".into(),
    };
    let (socket, _seen) = account_daemon(&dir, vec![answering(true, outside)]);
    let a = app_with(running(socket), true);
    assert!(!lit(&a, remotepassword::CHANGE));
    assert!(lit(&a, remotepassword::REMOVE));
    let note = page(&a)
        .item(remotepassword::NOTE)
        .map(|i| i.control.clone());
    let Some(Control::Lines(l)) = note else {
        panic!("a note says why: {note:?}");
    };
    assert!(l[0].contains("aren't in the staff group"), "{l:?}");
    assert_eq!(status(&a), [remotepassword::HAS_ONE_SHORT]);
}

#[test]
fn roots_own_page_greys_create_and_says_root_never_logs_on_remotely() {
    let dir = tmp("rd-root");
    let (socket, _seen) = account_daemon(&dir, vec![answering(false, RemoteAccess::Root)]);
    let a = app_with(running(socket), true);
    assert!(!lit(&a, remotepassword::CREATE));
    let note = page(&a)
        .item(remotepassword::NOTE)
        .map(|i| i.control.clone());
    assert_eq!(
        note,
        Some(Control::Lines(vec![remotepassword::ROOT.into()]))
    );
}

#[test]
fn the_actions_grey_while_a_request_is_out_and_its_done_still_shows() {
    let dir = tmp("rd-late-done");
    let (socket, seen) = account_daemon(&dir, vec![status_is(false), AccountReply::Done]);
    let mut a = app_with(running(socket), true);
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));
    press(&mut a, remotepassword::CREATE);
    ok(&mut a, &SET_NEW);
    cancel(&mut a);
    assert!(a.rd_password.is_none());
    assert!(!lit(&a, remotepassword::CREATE), "Create waits");
    a.wake();
    assert_eq!(status(&a), [remotepassword::HAS_ONE]);
    assert!(lit(&a, remotepassword::CHANGE));
    assert!(lit(&a, remotepassword::REMOVE));
}

#[test]
fn a_late_done_leaves_a_dialog_that_never_sent_it_open() {
    let dir = tmp("rd-late-close");
    let (socket, seen) = account_daemon(&dir, vec![status_is(false), AccountReply::Done]);
    let mut a = app_with(running(socket), true);
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));
    press(&mut a, remotepassword::CREATE);
    ok(&mut a, &SET_NEW);
    cancel(&mut a);
    a.open_rd_password(Action::Remove);
    let d = a.rd_password.as_mut().unwrap();
    rdpassword::type_into(d, LOGIN, "hunter3");
    a.wake();
    let d = a
        .rd_password
        .as_ref()
        .expect("the Remove dialog stays open");
    assert_eq!(d.why(), "");
    assert_eq!(d.form.edit_text(LOGIN), "hunter3");
    assert_eq!(status(&a), [remotepassword::HAS_ONE]);
}

#[test]
fn a_late_refusal_is_not_said_in_a_dialog_that_never_sent_it() {
    let dir = tmp("rd-late-refusal");
    let (socket, seen) = account_daemon(
        &dir,
        vec![
            status_is(true),
            AccountReply::Refused {
                reason: AccountRefusal::WrongPassword,
            },
        ],
    );
    let mut a = app_with(running(socket), true);
    assert_eq!(seen.recv_timeout(ANSWERED), Ok(AccountRequest::Status));
    press(&mut a, remotepassword::CHANGE);
    ok(&mut a, &SET_NEW);
    cancel(&mut a);
    a.open_rd_password(Action::Remove);
    let d = a.rd_password.as_mut().unwrap();
    rdpassword::type_into(d, LOGIN, "hunter3");
    a.wake();
    let d = a
        .rd_password
        .as_ref()
        .expect("the Remove dialog stays open");
    assert_eq!(d.why(), "");
    assert_eq!(d.form.edit_text(LOGIN), "hunter3");
    assert!(lit(&a, remotepassword::REMOVE), "lit once answered");
}

#[test]
fn a_done_outdates_a_read_under_way() {
    let dir = tmp("rd-outdated");
    let (socket, _seen) = account_daemon(&dir, vec![status_is(false), AccountReply::Done]);
    let mut a = app_with(running(socket), true);
    press(&mut a, remotepassword::CREATE);
    ok(&mut a, &SET_NEW);
    let (tx, rx) = std::sync::mpsc::channel();
    a.remote.reading = Some(rx);
    let stale = a.remote.found.clone().unwrap();
    a.wake();
    let _ = tx.send(stale);
    a.wake();
    assert_eq!(status(&a), [remotepassword::HAS_ONE]);
}

#[test]
fn typing_on_the_page_survives_the_section_changing() {
    let dir = tmp("rd-typed");
    let (socket, _seen) = account_daemon(&dir, vec![status_is(false), AccountReply::Done]);
    let mut a = app_with(running(socket), true);
    let p = a.props.as_mut().unwrap();
    let page = &mut p.sheet.pages[TAB_PASSWORD];
    page.focus_id("current");
    let ev = page.key(form::Key::Char('x'));
    p.event(
        reprisekit::controls::propsheet::Event::Page(TAB_PASSWORD, ev),
        |_| Err(String::new()),
    );
    press(&mut a, remotepassword::CREATE);
    send(&mut a, &SET_NEW);
    assert_eq!(status(&a), [remotepassword::HAS_ONE]);
    assert_eq!(page_text(&a, "current"), "x");
}

fn page_text<'a>(a: &'a AccountsApp, id: &str) -> &'a str {
    page(a).edit_text(id)
}
