use repriselogin::account::{AccountRefusal, AccountReply, AccountRequest, nt_hash};
use repriselogin::front::{FrontEvent, NlaUser};
use repriselogin::greeter::{AuthKind, ErrorKind};
use zeroize::Zeroizing;

use super::tests::{Run, authed, greeter_spawn, secret, spawn};
use super::*;

const ANNA: u32 = 1001;
const BERT: u32 = 1002;
const LOGIN_PW: &str = "anna-login-pw";

/// Nothing recovered, the greeter up as worker 1 and the front linked.
fn run() -> Run {
    let mut run = Run::machine(MachineConfig::default());
    assert_eq!(run.recover(vec![]), vec![]);
    assert_eq!(run.start(), greeter_spawn(1));
    assert_eq!(run.on(Input::FrontLinked), vec![]);
    run
}

fn set(login: &str, remote: &str) -> AccountRequest {
    AccountRequest::SetRemotePassword {
        login_password: Zeroizing::new(login.into()),
        remote_password: Zeroizing::new(remote.into()),
    }
}

fn clear(login: &str) -> AccountRequest {
    AccountRequest::ClearRemotePassword {
        login_password: Zeroizing::new(login.into()),
    }
}

fn stored(user: &str, uid: u32, password: &str) -> StoredPassword {
    StoredPassword {
        user: user.into(),
        uid,
        nt_hash: nt_hash(password),
    }
}

impl Run {
    /// Account link `link`'s request from `user`, its logs dropped.
    fn account(&mut self, link: u64, user: &str, uid: u32, request: AccountRequest) -> Vec<Effect> {
        let input = Input::Account {
            link,
            uid,
            user: user.into(),
            request,
        };
        acts(self.on(input))
    }

    fn quiet(&mut self, input: Input) -> Vec<Effect> {
        acts(self.on(input))
    }
}

/// `effects` without their logs.
fn acts(effects: Vec<Effect>) -> Vec<Effect> {
    (effects.into_iter())
        .filter(|e| !matches!(e, Effect::Log(..)))
        .collect()
}

fn answered(link: u64, reply: AccountReply) -> Vec<Effect> {
    vec![Effect::AccountReply { link, reply }]
}

fn refused(link: u64, reason: AccountRefusal) -> Vec<Effect> {
    answered(link, AccountReply::Refused { reason })
}

fn looked_up(link: u64, user: &str) -> Vec<Effect> {
    vec![Effect::LookUp {
        key: LookUpKey::Account(link),
        user: user.into(),
        group: "reprise-remote".into(),
    }]
}

fn member(link: u64, uid: Option<u32>, member: bool) -> Input {
    Input::Member {
        key: LookUpKey::Account(link),
        uid,
        member,
    }
}

fn answer(w: u32, password: &str) -> Vec<Effect> {
    vec![Effect::ToWorker {
        worker: WorkerId(w),
        msg: ToWorker::Answer(Some(Zeroizing::new(password.into()))),
    }]
}

fn status(run: &mut Run, uid: u32) -> Vec<Effect> {
    run.account(99, "anna", uid, AccountRequest::Status)
}

/// Takes `user`'s request on `link` through its look-up to check worker `w`'s password prompt.
fn checking(run: &mut Run, link: u64, user: &str, uid: u32, request: AccountRequest, w: u32) {
    let login = match &request {
        AccountRequest::SetRemotePassword { login_password, .. }
        | AccountRequest::ClearRemotePassword { login_password } => login_password.to_string(),
        AccountRequest::Status => unreachable!("a status runs no check"),
    };
    assert_eq!(run.account(link, user, uid, request), looked_up(link, user));
    assert_eq!(
        run.quiet(member(link, Some(uid), true)),
        spawn(w, Mode::Check, user)
    );
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(w), secret("Password: "))),
        answer(w, &login)
    );
}

#[test]
fn setting_a_remote_password_checks_the_login_password_first() {
    let mut run = run();
    assert_eq!(
        status(&mut run, ANNA),
        answered(
            99,
            AccountReply::Status {
                remote_password: false
            }
        )
    );
    assert_eq!(
        run.account(1, "anna", ANNA, set(LOGIN_PW, "new-remote-pw")),
        looked_up(1, "anna")
    );
    assert_eq!(
        run.quiet(member(1, Some(ANNA), true)),
        spawn(2, Mode::Check, "anna")
    );
    let info = FromWorker::Prompt {
        kind: AuthKind::Info,
        text: "Hello".into(),
    };
    assert_eq!(
        run.worker(2, info),
        vec![Effect::ToWorker {
            worker: WorkerId(2),
            msg: ToWorker::Answer(None),
        }]
    );
    assert_eq!(run.worker(2, secret("Password: ")), answer(2, LOGIN_PW));
    let want = stored("anna", ANNA, "new-remote-pw");
    assert_eq!(*want.nt_hash, *nt_hash("new-remote-pw"));
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(2), authed("anna", ANNA))),
        vec![Effect::StorePasswords(vec![want.clone()])]
    );
    let mut done = answered(1, AccountReply::Done);
    done.push(Effect::ToFront(FrontEvent::Passwords {
        users: vec![NlaUser {
            user: "anna".into(),
            nt_hash: want.nt_hash.clone(),
        }],
    }));
    assert_eq!(run.quiet(Input::Stored { ok: true }), done);
    assert_eq!(run.exited(2), vec![], "the check's exit after its success");
    assert_eq!(
        status(&mut run, ANNA),
        answered(
            99,
            AccountReply::Status {
                remote_password: true
            }
        )
    );
}

#[test]
fn a_remote_password_equal_to_the_login_password_is_refused() {
    let mut run = run();
    assert_eq!(
        run.account(1, "anna", ANNA, set(LOGIN_PW, LOGIN_PW)),
        refused(1, AccountRefusal::SameAsLogin)
    );
    assert_eq!(
        run.account(2, "anna", ANNA, set(LOGIN_PW, "new-remote-pw")),
        looked_up(2, "anna"),
        "a refused request holds nothing"
    );
}

#[test]
fn a_short_remote_password_is_refused() {
    let mut run = run();
    assert_eq!(
        run.account(1, "anna", ANNA, set(LOGIN_PW, "seven77")),
        refused(1, AccountRefusal::TooShort)
    );
    assert_eq!(
        run.account(2, "anna", ANNA, set(LOGIN_PW, "ééééééé")),
        refused(2, AccountRefusal::TooShort),
        "seven characters in fourteen bytes"
    );
    assert_eq!(
        run.account(3, "anna", ANNA, set(LOGIN_PW, "eight888")),
        looked_up(3, "anna")
    );
}

#[test]
fn a_wrong_login_password_stores_nothing() {
    let mut run = run();
    checking(&mut run, 1, "anna", ANNA, set("wrong", "new-remote-pw"), 2);
    let failed = FromWorker::Failed {
        kind: ErrorKind::Auth,
        text: AUTH_FAILED.into(),
    };
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(2), failed)),
        refused(1, AccountRefusal::WrongPassword)
    );
    assert_eq!(run.exited(2), vec![]);
    assert_eq!(
        status(&mut run, ANNA),
        answered(
            99,
            AccountReply::Status {
                remote_password: false
            }
        )
    );
}

#[test]
fn a_second_request_while_one_runs_is_busy() {
    let mut run = run();
    assert_eq!(
        run.account(1, "anna", ANNA, set(LOGIN_PW, "new-remote-pw")),
        looked_up(1, "anna")
    );
    assert_eq!(
        run.account(2, "anna", ANNA, clear(LOGIN_PW)),
        refused(2, AccountRefusal::Busy)
    );
    assert_eq!(
        run.account(3, "bert", BERT, clear(LOGIN_PW)),
        looked_up(3, "bert"),
        "another uid is not held up"
    );
    assert_eq!(
        run.quiet(member(1, Some(ANNA), false)),
        refused(1, AccountRefusal::NotRemoteUser)
    );
    assert_eq!(
        run.account(4, "anna", ANNA, clear(LOGIN_PW)),
        looked_up(4, "anna"),
        "an answered request frees its uid"
    );
}

#[test]
fn only_a_remote_user_other_than_root_gets_a_remote_password() {
    let mut run = run();
    for (link, uid, member_of) in [(1, Some(ANNA), false), (2, None, false), (3, Some(0), true)] {
        assert_eq!(
            run.account(link, "anna", ANNA, set(LOGIN_PW, "new-remote-pw")),
            looked_up(link, "anna")
        );
        assert_eq!(
            run.quiet(member(link, uid, member_of)),
            refused(link, AccountRefusal::NotRemoteUser)
        );
    }
}

#[test]
fn root_is_refused_a_clear_in_the_group_or_out_of_it() {
    let mut run = run();
    for (link, member_of) in [(1, true), (2, false)] {
        assert_eq!(
            run.account(link, "root", 0, clear(LOGIN_PW)),
            looked_up(link, "root")
        );
        assert_eq!(
            run.quiet(member(link, Some(0), member_of)),
            refused(link, AccountRefusal::NotRemoteUser)
        );
    }
}

#[test]
fn a_user_outside_the_remote_group_can_clear_their_remote_password() {
    let mut run = run();
    let bert = stored("bert", BERT, "bert-remote");
    let anna = stored("anna", ANNA, "anna-remote");
    assert_eq!(
        run.d.set_remote_passwords(vec![anna, bert.clone()]).len(),
        1
    );
    assert_eq!(
        run.account(1, "anna", ANNA, set(LOGIN_PW, "new-remote-pw")),
        looked_up(1, "anna")
    );
    assert_eq!(
        run.quiet(member(1, Some(ANNA), false)),
        refused(1, AccountRefusal::NotRemoteUser)
    );
    assert_eq!(
        run.account(2, "anna", ANNA, clear(LOGIN_PW)),
        looked_up(2, "anna")
    );
    assert_eq!(
        run.quiet(member(2, Some(ANNA), false)),
        spawn(2, Mode::Check, "anna")
    );
    assert_eq!(run.worker(2, secret("Password: ")), answer(2, LOGIN_PW));
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(2), authed("anna", ANNA))),
        vec![Effect::StorePasswords(vec![bert.clone()])]
    );
    let mut done = answered(2, AccountReply::Done);
    done.push(Effect::ToFront(FrontEvent::Passwords {
        users: vec![NlaUser {
            user: "bert".into(),
            nt_hash: bert.nt_hash.clone(),
        }],
    }));
    assert_eq!(run.quiet(Input::Stored { ok: true }), done);
}

#[test]
fn a_group_look_up_naming_another_uid_fails() {
    let mut run = run();
    for (link, request, member_of) in [
        (1, set(LOGIN_PW, "new-remote-pw"), true),
        (2, clear(LOGIN_PW), false),
    ] {
        assert_eq!(
            run.account(link, "anna", ANNA, request),
            looked_up(link, "anna")
        );
        assert_eq!(
            run.quiet(member(link, Some(BERT), member_of)),
            refused(link, AccountRefusal::Failed)
        );
    }
}

#[test]
fn a_check_that_never_asks_or_names_another_uid_fails() {
    let mut run = run();
    assert_eq!(
        run.account(1, "anna", ANNA, clear(LOGIN_PW)),
        looked_up(1, "anna")
    );
    assert_eq!(
        run.quiet(member(1, Some(ANNA), true)),
        spawn(2, Mode::Check, "anna")
    );
    let mut killed = vec![Effect::Kill(WorkerId(2))];
    killed.extend(refused(1, AccountRefusal::Failed));
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(2), authed("anna", ANNA))),
        killed
    );

    checking(&mut run, 3, "anna", ANNA, clear(LOGIN_PW), 3);
    let mut killed = vec![Effect::Kill(WorkerId(3))];
    killed.extend(refused(3, AccountRefusal::Failed));
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(3), authed("bert", BERT))),
        killed
    );

    checking(&mut run, 4, "anna", ANNA, clear(LOGIN_PW), 4);
    assert_eq!(
        run.quiet(Input::WorkerExited(WorkerId(4))),
        refused(4, AccountRefusal::Failed)
    );
}

#[test]
fn stores_go_one_at_a_time_and_each_builds_on_the_last() {
    let mut run = run();
    let bert_old = stored("bert", BERT, "bert-old-pw");
    assert_eq!(run.d.set_remote_passwords(vec![bert_old.clone()]).len(), 1);
    checking(&mut run, 1, "anna", ANNA, set(LOGIN_PW, "anna-remote"), 2);
    checking(&mut run, 2, "bert", BERT, clear(LOGIN_PW), 3);
    let anna = stored("anna", ANNA, "anna-remote");
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(2), authed("anna", ANNA))),
        vec![Effect::StorePasswords(vec![bert_old.clone(), anna.clone()])]
    );
    assert_eq!(
        run.quiet(Input::Worker(WorkerId(3), authed("bert", BERT))),
        vec![],
        "a second store waits for the first"
    );
    let ok = run.quiet(Input::Stored { ok: true });
    assert_eq!(
        ok[0],
        Effect::AccountReply {
            link: 1,
            reply: AccountReply::Done
        }
    );
    assert_eq!(
        ok.last(),
        Some(&Effect::StorePasswords(vec![anna.clone()])),
        "{ok:?}"
    );
    assert_eq!(
        run.quiet(Input::Stored { ok: false }),
        refused(2, AccountRefusal::Failed)
    );
    assert_eq!(
        run.account(3, "bert", BERT, AccountRequest::Status),
        answered(
            3,
            AccountReply::Status {
                remote_password: true
            }
        ),
        "a failed store keeps the passwords"
    );
}
