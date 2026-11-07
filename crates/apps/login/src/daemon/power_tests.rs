use repriselogin::greeter::{AuthKind, ErrorKind};
use zeroize::Zeroizing;

use super::tests::{Run, answer, authed, create, reply, secret, spawn};
use super::*;

const CHECK: WorkerId = WorkerId(2);

fn power(action: PowerAction) -> GreeterRequest {
    GreeterRequest::Power { action }
}

fn power_as(action: PowerAction, user: &str) -> GreeterRequest {
    GreeterRequest::PowerAs {
        action,
        user: user.into(),
    }
}

/// The check worker's spawn with its `AUTH_WAIT`.
fn check_spawn(user: &str) -> Vec<Effect> {
    let mut effects = spawn(CHECK.0, Mode::Check, user);
    effects.push(Effect::Arm {
        timer: Timer::Auth(CHECK),
        after: AUTH_WAIT,
    });
    effects
}

fn wheel(user: &str) -> Vec<Effect> {
    vec![Effect::LookUp {
        key: LookUpKey::Admin(CHECK),
        user: user.into(),
        group: "wheel".into(),
    }]
}

fn member(uid: Option<u32>, member: bool) -> Input {
    Input::Member {
        key: LookUpKey::Admin(CHECK),
        uid,
        member,
    }
}

fn auth_failed() -> Effect {
    Effect::Reply(GreeterReply::Error {
        kind: ErrorKind::Auth,
        text: AUTH_FAILED.into(),
    })
}

/// anna's session runs and the greeter is up; `user` asks for `action` as an administrator.
fn asked_as(user: &str, action: PowerAction) -> Run {
    let mut run = Run::anna_switched(Remembered::default());
    assert_eq!(run.greeter(power_as(action, user)), check_spawn(user));
    run
}

#[test]
fn power_with_other_sessions_asks_for_an_administrator() {
    let mut run = Run::anna_switched(Remembered::default());
    assert_eq!(
        run.greeter(power(PowerAction::PowerOff)),
        reply(GreeterReply::AdminNeeded {
            action: PowerAction::PowerOff
        })
    );

    let mut run = Run::booted();
    run.world.user_sessions = 1;
    assert_eq!(
        run.greeter(power(PowerAction::Reboot)),
        reply(GreeterReply::AdminNeeded {
            action: PowerAction::Reboot
        })
    );
}

#[test]
fn an_administrators_password_shuts_down() {
    let mut run = asked_as("anna", PowerAction::PowerOff);
    assert_eq!(
        run.worker(2, secret("Password: ")),
        reply(GreeterReply::AuthMessage {
            kind: AuthKind::Secret,
            text: "Password: ".into(),
        })
    );
    assert_eq!(
        run.greeter(answer("pw")),
        vec![Effect::ToWorker {
            worker: CHECK,
            msg: ToWorker::Answer(Some(Zeroizing::new("pw".into()))),
        }]
    );
    assert_eq!(run.worker(2, authed("anna", 1000)), wheel("anna"));
    assert_eq!(run.exited(2), vec![], "the check worker ends by itself");
    assert_eq!(
        run.on(member(Some(1000), true)),
        vec![
            Effect::Log(
                Level::Info,
                "anna turns the computer off as an administrator".into()
            ),
            Effect::Reply(GreeterReply::Success),
            Effect::Power(PowerAction::PowerOff),
        ],
        "Success goes out before the power action"
    );
    assert_eq!(
        run.on(Input::PowerDone(Ok(()))),
        reply(GreeterReply::Success)
    );
    assert!(run.d.conversation.is_none());
    assert_eq!(run.greeter(create("bert")), spawn(3, Mode::Login, "bert"));
}

#[test]
fn a_non_administrator_is_refused_like_a_wrong_password() {
    let mut run = asked_as("bert", PowerAction::PowerOff);
    let wrong = run.worker(
        2,
        FromWorker::Failed {
            kind: ErrorKind::Auth,
            text: AUTH_FAILED.into(),
        },
    );
    assert_eq!(wrong, vec![auth_failed()]);

    // Not in the group, an account gone since PAM, and a uid other than PAM's.
    for (uid, is_member) in [(Some(1001), false), (None, false), (Some(1005), true)] {
        let mut run = asked_as("bert", PowerAction::Reboot);
        assert_eq!(run.worker(2, authed("bert", 1001)), wheel("bert"));
        let effects = run.on(member(uid, is_member));
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Power(_))),
            "{effects:?}"
        );
        let replies: Vec<_> = (effects.iter())
            .filter(|e| matches!(e, Effect::Reply(_)))
            .collect();
        assert_eq!(replies, vec![&auth_failed()], "{uid:?} {is_member}");
        assert!(effects.contains(&Effect::Kill(CHECK)), "{effects:?}");
        assert!(run.d.conversation.is_none());
    }
}

#[test]
fn root_cannot_be_the_administrator() {
    let mut run = asked_as("root", PowerAction::PowerOff);
    let effects = run.worker(2, authed("root", 0));
    assert!(
        !(effects.iter()).any(|e| matches!(e, Effect::LookUp { .. } | Effect::Power(_))),
        "{effects:?}"
    );
    assert!(effects.contains(&auth_failed()), "{effects:?}");
    assert!(effects.contains(&Effect::Kill(CHECK)), "{effects:?}");
    assert!(run.d.conversation.is_none());
}

#[test]
fn power_with_no_other_session_needs_no_administrator() {
    let mut run = Run::anna_switched(Remembered::default());
    run.removed("c1", None);
    assert_eq!(
        run.greeter(power(PowerAction::Reboot)),
        vec![Effect::Power(PowerAction::Reboot)]
    );
}

#[test]
fn a_cancelled_administrator_check_turns_nothing_off() {
    let mut run = asked_as("anna", PowerAction::PowerOff);
    assert_eq!(run.worker(2, authed("anna", 1000)), wheel("anna"));
    assert_eq!(
        run.greeter(GreeterRequest::CancelSession),
        vec![Effect::Kill(CHECK), Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(run.on(member(Some(1000), true)), vec![]);
}

#[test]
fn an_administrator_check_never_starts_a_session() {
    let mut run = asked_as("anna", PowerAction::PowerOff);
    assert_eq!(run.worker(2, authed("anna", 1000)), wheel("anna"));
    let effects = run.greeter(GreeterRequest::StartSession);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Reply(GreeterReply::Error {
                kind: ErrorKind::Refused,
                ..
            })]
        ),
        "{effects:?}"
    );
}
