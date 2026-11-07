use std::time::Duration;

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

/// `user`'s check up to its password's answer, with the clock then at the answer.
fn answered_as(user: &str) -> Run {
    let mut run = asked_as(user, PowerAction::PowerOff);
    run.worker(2, secret("Password: "));
    run.greeter(answer("pw"));
    run
}

fn after(run: &mut Run, wait: Duration) -> &mut Run {
    run.now += wait;
    run
}

fn refuse_timer(after: Duration) -> Effect {
    Effect::Arm {
        timer: Timer::AdminRefuse(CHECK),
        after,
    }
}

fn fire(run: &mut Run) -> Vec<Effect> {
    run.on(Input::Timer(Timer::AdminRefuse(CHECK)))
}

/// The refusal the effects of a correct password hold back: the worker ends, no reply goes
/// out, and the error waits for `ADMIN_REFUSE_DELAY` after the answer, 0.1 s ago.
fn assert_held(effects: &[Effect]) {
    let rest: Vec<_> = (effects.iter())
        .filter(|e| !matches!(e, Effect::Log(..)))
        .collect();
    assert_eq!(
        rest,
        [
            &Effect::Kill(CHECK),
            &refuse_timer(ADMIN_REFUSE_DELAY - Duration::from_millis(100))
        ]
    );
}

/// Fires the refusal timer a second before `ADMIN_REFUSE_DELAY` past the answer, then at it.
fn assert_refused_at_the_delay(run: &mut Run) {
    assert_eq!(
        fire(after(run, Duration::from_millis(1900))),
        [refuse_timer(Duration::from_secs(1))],
        "no reply a second before ADMIN_REFUSE_DELAY"
    );
    assert_eq!(fire(after(run, Duration::from_secs(1))), [auth_failed()]);
    assert!(run.d.conversation.is_none());
    assert_eq!(fire(run), [], "a late timer finds no conversation");
}

#[test]
fn a_non_administrator_is_refused_like_a_wrong_password() {
    // Not in the group, an account gone since PAM, and a uid other than PAM's.
    for (uid, is_member) in [(Some(1001), false), (None, false), (Some(1005), true)] {
        let mut run = answered_as("bert");
        assert_eq!(
            after(&mut run, Duration::from_millis(100)).worker(2, authed("bert", 1001)),
            wheel("bert")
        );
        let effects = run.on(member(uid, is_member));
        assert_held(&effects);
        assert_refused_at_the_delay(&mut run);
    }
}

#[test]
fn root_cannot_be_the_administrator() {
    let mut run = answered_as("root");
    let effects = after(&mut run, Duration::from_millis(100)).worker(2, authed("root", 0));
    assert!(
        !(effects.iter()).any(|e| matches!(e, Effect::LookUp { .. } | Effect::Power(_))),
        "{effects:?}"
    );
    assert_held(&effects);
    assert_refused_at_the_delay(&mut run);
}

#[test]
fn a_quick_wrong_password_is_answered_at_the_refuse_delay() {
    let mut run = answered_as("anna");
    let wrong = FromWorker::Failed {
        kind: ErrorKind::Auth,
        text: AUTH_FAILED.into(),
    };
    assert_eq!(
        after(&mut run, Duration::from_millis(500)).worker(2, wrong),
        [refuse_timer(Duration::from_millis(2500))]
    );
    assert_eq!(run.exited(2), [], "the check worker ends by itself");
    assert_eq!(
        fire(after(&mut run, Duration::from_millis(2500))),
        [auth_failed()]
    );
    assert!(run.d.conversation.is_none());
}

#[test]
fn a_slow_wrong_password_is_answered_when_it_comes() {
    let mut run = answered_as("anna");
    let wrong = FromWorker::Failed {
        kind: ErrorKind::Auth,
        text: AUTH_FAILED.into(),
    };
    let effects = after(&mut run, Duration::from_secs(4)).worker(2, wrong);
    assert!(
        matches!(effects.as_slice(), [Effect::Log(Level::Debug, _), e] if *e == auth_failed()),
        "{effects:?}"
    );
    assert!(run.d.conversation.is_none());
}

#[test]
fn a_refusal_cancelled_in_its_delay_is_never_sent() {
    let mut run = answered_as("root");
    after(&mut run, Duration::from_millis(100)).worker(2, authed("root", 0));
    assert_eq!(
        run.greeter(power_as(PowerAction::Reboot, "anna")),
        [refused(BUSY)],
        "the held refusal still holds the conversation"
    );
    assert_eq!(
        run.greeter(GreeterRequest::CancelSession),
        [Effect::Reply(GreeterReply::Success)]
    );
    assert_eq!(fire(after(&mut run, ADMIN_REFUSE_DELAY)), []);
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

#[test]
fn a_check_takes_no_second_request() {
    let mut run = asked_as("anna", PowerAction::PowerOff);
    assert_eq!(
        run.greeter(power_as(PowerAction::Reboot, "bert")),
        [refused(BUSY)]
    );
    assert_eq!(run.greeter(create("bert")), [refused(BUSY)]);
    assert_eq!(run.worker(2, authed("anna", 1000)), wheel("anna"));
    assert_eq!(run.greeter(create("bert")), [refused(BUSY)]);
}

#[test]
fn a_greeter_gone_mid_check_turns_nothing_off() {
    let mut run = asked_as("anna", PowerAction::PowerOff);
    assert_eq!(run.worker(2, authed("anna", 1000)), wheel("anna"));
    run.exited(1);
    assert!(run.d.conversation.is_none());
    assert_eq!(run.on(member(Some(1000), true)), []);
}
