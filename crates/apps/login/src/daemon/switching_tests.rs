use repriselogin::Interface;
use repriselogin::front::FrontEvent;
use repriselogin::machine::{MachineConfig, MachineRemote};
use repriselogin::session::SessionRequest;

use super::remote_tests::{
    ANNA, disconnect_arm, enabled, front, quiet, ready, remote_run, to_worker,
};
use super::tests::{Run, authed, create, greeter_spawn, logind, reply, spawn, unlock};
use super::*;

/// Anna on VT 7 holds the seat; bert's session is on VT 8.
fn anna_and_bert() -> Run {
    let mut run = Run::machine(MachineConfig::default());
    run.recover(vec![
        logind("c1", "anna", 1000, 7, "user", Mode::Login.service()),
        logind("c3", "bert", 1001, 8, "user", Mode::Login.service()),
    ]);
    assert_eq!(run.start(), []);
    run
}

fn hello(run: &mut Run, id: &str, interface: Interface, seat_active: &str) -> Vec<Effect> {
    run.on(Input::Session {
        id: id.into(),
        request: SessionRequest::Hello { interface },
        seat_active: Some(seat_active.into()),
    })
}

fn remember(interface: Interface) -> Vec<Effect> {
    vec![Effect::Remember(Remembered {
        last_user: None,
        last_interface: Some(interface),
    })]
}

fn with_end_after(min: u32) -> MachineRemote {
    MachineRemote {
        end_disconnected_after_min: Some(min),
        ..enabled()
    }
}

/// Anna logs in at the console as `c2` on VT 7 and switches user; the login screen comes back
/// as worker 3.
fn anna_switched_away(remote: MachineRemote) -> Run {
    let mut run = remote_run(remote);
    run.log_in(2, "anna", ANNA, "c2", 7);
    run.exited(1);
    let effects = run.session("c2", SessionRequest::SwitchUser);
    assert_eq!(effects, greeter_spawn(3));
    run
}

/// A remote login for anna through check worker `w` reaches her running session: the check
/// ends, and the hand-over goes to her uid.
fn hand_over_to_anna(run: &mut Run, conn: u64, w: u32) {
    assert_eq!(
        run.verify(conn, w, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn }),
            to_worker(w, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [front(FrontEvent::Accepted { conn })]
    );
    assert_eq!(
        run.on(ready(conn, "anna")),
        [Effect::Forward { conn, uid: ANNA }]
    );
}

fn background(run: &Run, id: &str) -> Option<bool> {
    run.d.sessions.by_id(id).map(|e| e.background)
}

#[test]
fn a_hello_from_a_session_off_the_seat_leaves_the_remembered_interface() {
    let mut run = anna_and_bert();
    assert_eq!(
        hello(&mut run, "c1", Interface::Classic, "c1"),
        remember(Interface::Classic)
    );
    assert_eq!(hello(&mut run, "c3", Interface::Blue, "c1"), []);
    assert_eq!(run.d.remembered.last_interface, Some(Interface::Classic));
}

#[test]
fn a_hello_from_the_session_on_the_seat_is_remembered() {
    let mut run = anna_and_bert();
    assert_eq!(
        hello(&mut run, "c1", Interface::Glass, "c1"),
        remember(Interface::Glass)
    );
    assert_eq!(hello(&mut run, "c1", Interface::Glass, "c1"), []);
}

#[test]
fn the_session_brought_to_the_console_has_its_interface_remembered() {
    let mut run = anna_and_bert();
    hello(&mut run, "c1", Interface::Classic, "c1");
    assert_eq!(hello(&mut run, "c3", Interface::Blue, "c1"), []);
    assert_eq!(run.on(Input::ConsoleOnVt(8)), remember(Interface::Blue));
    assert_eq!(
        run.on(Input::ConsoleOnVt(8)),
        [],
        "Blue is remembered already"
    );
}

#[test]
fn a_switch_user_then_remote_login_resumes_the_same_session() {
    let mut run = anna_switched_away(enabled());
    hand_over_to_anna(&mut run, 7, 4);
    assert_eq!(run.d.sessions.len(), 1);
    assert_eq!(
        run.d.sessions.by_uid(ANNA).map(|e| e.id.as_str()),
        Some("c2")
    );
}

#[test]
fn a_remote_login_reaching_a_session_off_the_seat_makes_it_a_background_session() {
    let mut run = anna_switched_away(with_end_after(30));
    hand_over_to_anna(&mut run, 7, 4);
    run.world.active_vt = vt::GREETER_VT;
    assert_eq!(
        run.on(Input::Forwarded { conn: 7, ok: true }),
        [
            Effect::Log(
                Level::Info,
                "anna's session c2 is in the background with a remote client".into()
            ),
            disconnect_arm("c2", 1, 30),
            Effect::Inhibit(Inhibit::Idle),
        ]
    );
    assert_eq!(background(&run, "c2"), Some(true));
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(
        run.viewers(ANNA, 0),
        [disconnect_arm("c2", 2, 30), Effect::Inhibit(Inhibit::Idle)]
    );
}

#[test]
fn a_session_reached_remotely_whose_client_never_shows_ends_after_its_disconnect_time() {
    let mut run = anna_switched_away(with_end_after(30));
    hand_over_to_anna(&mut run, 7, 4);
    run.world.active_vt = vt::GREETER_VT;
    let effects = quiet(run.on(Input::Forwarded { conn: 7, ok: true }));
    assert_eq!(
        effects,
        [disconnect_arm("c2", 1, 30), Effect::Inhibit(Inhibit::Idle)]
    );
    let disconnected = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    let effects = quiet(run.after(Duration::from_secs(31 * 60)).fire(disconnected));
    assert_eq!(effects, [Effect::TerminateSession("c2".into())]);
}

#[test]
fn a_hand_over_forwarded_after_the_session_reached_the_console_leaves_it_a_console_session() {
    let mut run = anna_switched_away(with_end_after(30));
    assert_eq!(run.greeter(create("anna")), spawn(4, Mode::Reauth, "anna"));
    assert_eq!(
        run.worker(4, authed("anna", ANNA)),
        reply(GreeterReply::Success)
    );
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c2", 1));
    hand_over_to_anna(&mut run, 7, 5);
    let effects = run.on(Input::Unlocked { id: "c2".into() });
    assert_eq!(effects.first(), Some(&Effect::SwitchTo(7)), "{effects:?}");
    run.world.active_vt = 7;
    assert_eq!(quiet(run.on(Input::Forwarded { conn: 7, ok: true })), []);
    assert_eq!(background(&run, "c2"), Some(false));
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(
        run.viewers(ANNA, 0),
        [Effect::Inhibit(Inhibit::Nothing)],
        "no Disconnected timer for the session on the monitor"
    );
}

#[test]
fn a_hand_over_forwarded_before_the_console_shows_the_session_does_not_end_it() {
    let mut run = anna_switched_away(with_end_after(30));
    assert_eq!(run.greeter(create("anna")), spawn(4, Mode::Reauth, "anna"));
    assert_eq!(
        run.worker(4, authed("anna", ANNA)),
        reply(GreeterReply::Success)
    );
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c2", 1));
    hand_over_to_anna(&mut run, 7, 5);
    let effects = run.on(Input::Unlocked { id: "c2".into() });
    assert_eq!(effects.first(), Some(&Effect::SwitchTo(7)), "{effects:?}");
    run.world.active_vt = vt::GREETER_VT;
    assert_eq!(
        quiet(run.on(Input::Forwarded { conn: 7, ok: true })),
        [disconnect_arm("c2", 1, 30), Effect::Inhibit(Inhibit::Idle)]
    );
    run.world.active_vt = 7;
    assert_eq!(
        quiet(run.on(Input::ConsoleOnVt(7))),
        [Effect::Inhibit(Inhibit::Nothing)]
    );
    assert_eq!(background(&run, "c2"), Some(false));
    let disconnected = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    let effects = run.after(Duration::from_secs(31 * 60)).fire(disconnected);
    assert_eq!(effects, [], "the session on the monitor is not ended");
}

#[test]
fn a_disconnect_time_running_out_while_the_console_shows_the_session_does_not_end_it() {
    let mut run = anna_switched_away(with_end_after(30));
    hand_over_to_anna(&mut run, 7, 4);
    run.world.active_vt = vt::GREETER_VT;
    quiet(run.on(Input::Forwarded { conn: 7, ok: true }));
    assert_eq!(background(&run, "c2"), Some(true));
    run.world.active_vt = 7;
    let disconnected = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    let effects = quiet(run.after(Duration::from_secs(31 * 60)).fire(disconnected));
    assert!(
        !effects.contains(&Effect::TerminateSession("c2".into())),
        "{effects:?}"
    );
    assert_eq!(background(&run, "c2"), Some(false));
}

#[test]
fn a_disconnect_time_running_out_with_no_active_vt_ends_a_session_on_vt_0() {
    let mut run = anna_switched_away(with_end_after(30));
    hand_over_to_anna(&mut run, 7, 4);
    run.world.active_vt = vt::GREETER_VT;
    quiet(run.on(Input::Forwarded { conn: 7, ok: true }));
    let mut e = run.d.sessions.by_id("c2").cloned().expect("anna's session");
    e.vt = 0;
    run.d.sessions.insert(e);
    run.world.active_vt = 0;
    let disconnected = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    let effects = quiet(run.after(Duration::from_secs(31 * 60)).fire(disconnected));
    assert!(
        effects.contains(&Effect::TerminateSession("c2".into())),
        "{effects:?}"
    );
}

#[test]
fn a_session_switched_away_with_no_remote_client_is_not_ended() {
    let mut run = anna_switched_away(with_end_after(1));
    run.viewers(ANNA, 1);
    let effects = run.viewers(ANNA, 0);
    let armed = |e: &Effect| {
        matches!(
            e,
            Effect::Arm {
                timer: Timer::Disconnected { .. },
                ..
            }
        )
    };
    assert!(!effects.iter().any(armed), "{effects:?}");
    assert_eq!(background(&run, "c2"), Some(false));
}

#[test]
fn a_session_brought_to_the_console_keeps_no_disconnect_entry() {
    let mut run = anna_switched_away(with_end_after(30));
    hand_over_to_anna(&mut run, 7, 4);
    run.world.active_vt = vt::GREETER_VT;
    quiet(run.on(Input::Forwarded { conn: 7, ok: true }));
    run.world.active_vt = 7;
    quiet(run.on(Input::ConsoleOnVt(7)));
    assert_eq!(background(&run, "c2"), Some(false));
    let reload = Input::ConfigReloaded(MachineConfig {
        remote: with_end_after(60),
        ..MachineConfig::default()
    });
    assert_eq!(quiet(run.on(reload)), [], "a console session arms nothing");
}

fn orphaned(run: &mut Run, id: &str, uid: u32) -> Vec<Effect> {
    run.on(Input::SessionOrphaned { id: id.into(), uid })
}

#[test]
fn a_session_whose_worker_died_while_it_runs_is_terminated() {
    let mut run = anna_and_bert();
    assert_eq!(
        orphaned(&mut run, "c1", 1000),
        [
            Effect::Log(
                Level::Warn,
                "anna's session c1 lost its login worker; ending it".into()
            ),
            Effect::TerminateSession("c1".into()),
        ]
    );
}

#[test]
fn an_orphan_report_with_another_uid_terminates_nothing() {
    let mut run = anna_and_bert();
    assert_eq!(orphaned(&mut run, "c1", 1001), []);
}

#[test]
fn an_orphan_report_for_an_unknown_session_terminates_nothing() {
    let mut run = anna_and_bert();
    assert_eq!(orphaned(&mut run, "c9", 1000), []);
}

#[test]
fn recovery_locks_no_session_off_the_seat() {
    let found = vec![
        logind("c1", "anna", ANNA, 7, "user", "reprise-login-remote"),
        logind("c2", "bert", 1002, 8, "user", "reprise-login-remote-nla"),
        logind("c4", "carl", 1003, 9, "user", Mode::Login.service()),
    ];
    for seat_active in [Some("g1"), None] {
        let mut run = Run::machine(MachineConfig {
            remote: enabled(),
            ..MachineConfig::default()
        });
        let effects = run.d.recover(run.now, found.clone(), seat_active);
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::LockSession(_))),
            "each compositor locks its own console when its link drops: {effects:?}"
        );
    }
}
