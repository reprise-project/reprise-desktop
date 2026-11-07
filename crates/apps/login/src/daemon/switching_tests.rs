use repriselogin::front::FrontEvent;
use repriselogin::machine::MachineRemote;
use repriselogin::session::SessionRequest;

use super::remote_tests::{
    ANNA, disconnect_arm, enabled, front, quiet, ready, remote_run, to_worker,
};
use super::tests::{Run, authed, create, greeter_spawn, reply, spawn, unlock};
use super::*;

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
        [disconnect_arm("c2", 1, 30), Effect::Inhibit(Inhibit::Idle)]
    );
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
fn a_session_switched_away_with_no_remote_client_is_not_ended() {
    let mut run = anna_switched_away(with_end_after(1));
    assert_eq!(background(&run, "c2"), Some(false));
    let disconnected = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    let effects = run.after(Duration::from_secs(120)).fire(disconnected);
    assert_eq!(effects, []);
    assert!(run.d.sessions.by_id("c2").is_some());
}
