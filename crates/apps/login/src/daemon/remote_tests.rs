use std::collections::BTreeSet;
use std::net::SocketAddr;

use repriselogin::account::nt_hash;
use repriselogin::front::{FrontEvent, FrontRequest, NlaUser, Secret};
use repriselogin::greeter::{AuthKind, ErrorKind, GreeterRequest, Note};
use repriselogin::machine::{MachineConfig, MachineRemote};
use repriselogin::throttle::PAUSE_AFTER;
use zeroize::Zeroizing;

use super::tests::{
    Run, authed, create, greeter_spawn, logind, reply, secret, spawn, started, unlock,
};
use super::*;

const ANNA: u32 = 1001;
const BERT: u32 = 1002;
const CARL: u32 = 1003;

fn config(remote: MachineRemote) -> MachineConfig {
    MachineConfig {
        remote,
        ..MachineConfig::default()
    }
}

fn enabled() -> MachineRemote {
    MachineRemote {
        enabled: true,
        ..MachineRemote::default()
    }
}

/// The front's start, which `start` emits first while remote logins are on.
fn front_start(remote: &MachineRemote) -> Vec<Effect> {
    match remote.enabled {
        true => vec![Effect::Front(FrontUnit::Start)],
        false => vec![],
    }
}

/// Nothing recovered, the greeter up as worker 1.
fn remote_run(remote: MachineRemote) -> Run {
    let mut want = front_start(&remote);
    want.extend(greeter_spawn(1));
    let mut run = Run::machine(config(remote));
    assert_eq!(run.recover(vec![]), vec![]);
    assert_eq!(run.start(), want);
    run
}

/// Anna's session `c1` on VT 7 recovered with `seat_active` on seat0.
fn anna_recovered(remote: MachineRemote, seat_active: &str, service: &str) -> Run {
    let want = front_start(&remote);
    let mut run = Run::machine(config(remote));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", service)];
    run.d.recover(run.now, found, Some(seat_active));
    assert_eq!(run.start(), want);
    run
}

fn peer(n: u8) -> SocketAddr {
    SocketAddr::from(([192, 0, 2, n], 50000))
}

fn authenticate(conn: u64, from: u8, user: &str, secret: Secret) -> Input {
    Input::Front(FrontRequest::Authenticate {
        conn,
        peer: peer(from),
        user: user.into(),
        secret,
    })
}

fn tls(conn: u64, user: &str, password: &str) -> Input {
    tls_from(conn, 1, user, password)
}

fn tls_from(conn: u64, from: u8, user: &str, password: &str) -> Input {
    authenticate(
        conn,
        from,
        user,
        Secret::Password(Zeroizing::new(password.into())),
    )
}

fn nla(conn: u64, user: &str, password: &str) -> Input {
    authenticate(
        conn,
        1,
        user,
        Secret::Delegated(Zeroizing::new(password.into())),
    )
}

/// The timers every `Authenticate` arms.
fn armed(conn: u64) -> Vec<Effect> {
    vec![
        Effect::Arm {
            timer: Timer::Refuse(conn),
            after: REFUSE_DELAY,
        },
        Effect::Arm {
            timer: Timer::Handover(conn),
            after: HANDOVER_DEADLINE,
        },
    ]
}

fn looked_up(conn: u64, user: &str) -> Vec<Effect> {
    let mut effects = armed(conn);
    effects.push(Effect::LookUp {
        key: LookUpKey::Conn(conn),
        user: user.into(),
        group: "reprise-remote".into(),
    });
    effects
}

fn member(conn: u64, uid: u32) -> Input {
    Input::Member {
        key: LookUpKey::Conn(conn),
        uid: Some(uid),
        member: true,
    }
}

fn remote_spawn(w: u32, mode: Mode, user: &str, from: u8) -> Vec<Effect> {
    vec![Effect::Spawn {
        worker: WorkerId(w),
        job: Job {
            mode,
            user: user.into(),
            autologin_user: None,
            greeter_socket: false,
            rhost: Some(peer(from).ip().to_string()),
        },
    }]
}

fn to_worker(w: u32, msg: ToWorker) -> Effect {
    Effect::ToWorker {
        worker: WorkerId(w),
        msg,
    }
}

fn password(text: &str) -> ToWorker {
    ToWorker::Answer(Some(Zeroizing::new(text.into())))
}

fn background_start(w: u32, vt: u16) -> Vec<Effect> {
    vec![
        to_worker(
            w,
            ToWorker::Start {
                vt,
                background: true,
            },
        ),
        Effect::Arm {
            timer: Timer::Start(WorkerId(w)),
            after: START_WAIT,
        },
    ]
}

fn front(event: FrontEvent) -> Effect {
    Effect::ToFront(event)
}

fn denied(conn: u64, backoff_s: u32) -> Vec<Effect> {
    vec![front(FrontEvent::Denied { conn, backoff_s })]
}

fn ready(conn: u64, user: &str) -> Input {
    Input::FrontReady {
        conn,
        user: user.into(),
    }
}

fn failed(kind: ErrorKind, text: &str) -> FromWorker {
    FromWorker::Failed {
        kind,
        text: text.into(),
    }
}

fn quiet(effects: Vec<Effect>) -> Vec<Effect> {
    (effects.into_iter())
        .filter(|e| !matches!(e, Effect::Log(..)))
        .collect()
}

fn disconnect_arm(id: &str, generation: u32, min: u32) -> Effect {
    Effect::Arm {
        timer: Timer::Disconnected {
            id: id.into(),
            generation,
        },
        after: Duration::from_secs(u64::from(min) * 60),
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
    fn after(&mut self, wait: Duration) -> &mut Run {
        self.now += wait;
        self
    }

    fn fire(&mut self, timer: Timer) -> Vec<Effect> {
        self.on(Input::Timer(timer))
    }

    /// Runs conn's TLS login for `user` through worker `w` up to `Verified`; returns what
    /// `Authenticated` gave.
    fn verify(&mut self, conn: u64, w: u32, user: &str, uid: u32) -> Vec<Effect> {
        assert_eq!(self.on(tls(conn, user, "pw")), looked_up(conn, user));
        assert_eq!(
            self.on(member(conn, uid)),
            remote_spawn(w, Mode::Remote, user, 1)
        );
        assert_eq!(
            self.worker(w, secret("Password: ")),
            [to_worker(w, password("pw"))]
        );
        self.worker(w, authed(user, uid))
    }

    /// Starts anna's background session `c2` on VT 7 for conn through worker `w`. With
    /// `end_disconnected_after` set, its first `Disconnected` timer is generation 1.
    fn anna_in_background(&mut self, conn: u64, w: u32) {
        let mut want = vec![front(FrontEvent::Verified { conn })];
        want.extend(background_start(w, 7));
        assert_eq!(self.verify(conn, w, "anna", ANNA), want);
        let mut want = vec![
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle),
        ];
        if let Some(min) = self.d.config.remote.end_disconnected_after_min {
            want.insert(0, disconnect_arm("c2", 1, min));
        }
        assert_eq!(self.worker(w, started("c2", ANNA)), want);
    }

    fn viewers(&mut self, uid: u32, count: u32) -> Vec<Effect> {
        self.on(Input::Viewers { uid, count })
    }
}

#[test]
fn a_tls_login_checks_the_group_then_pam_then_starts_a_background_session() {
    let mut run = remote_run(enabled());
    assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
    assert_eq!(
        run.on(member(7, ANNA)),
        remote_spawn(2, Mode::Remote, "anna", 1)
    );
    assert_eq!(
        run.worker(2, secret("Password: ")),
        [to_worker(2, password("pw"))]
    );
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
    assert_eq!(
        run.worker(2, started("c2", ANNA)),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle)
        ]
    );
    assert_eq!(
        run.d.sessions.by_uid(ANNA),
        Some(&SessionEntry {
            id: "c2".into(),
            user: "anna".into(),
            uid: ANNA,
            vt: 7,
            started: Some(run.now),
            background: true,
        })
    );
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [front(FrontEvent::Accepted { conn: 7 })]
    );
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
    assert_eq!(run.on(Input::Forwarded { conn: 7, ok: true }), []);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), []);
    assert_eq!(run.after(HANDOVER_DEADLINE).fire(Timer::Handover(7)), []);
}

#[test]
fn only_a_remote_start_runs_its_session_in_the_background() {
    let background = |effects: &[Effect]| {
        effects.iter().find_map(|e| match e {
            Effect::ToWorker {
                msg: ToWorker::Start { background, .. },
                ..
            } => Some(*background),
            _ => None,
        })
    };
    let mut run = remote_run(enabled());
    assert_eq!(run.greeter(create("bert")), spawn(2, Mode::Login, "bert"));
    run.worker(2, authed("bert", BERT));
    let console = run.greeter(GreeterRequest::StartSession);
    assert_eq!(background(&console), Some(false), "{console:?}");
    let remote = run.verify(7, 3, "anna", ANNA);
    assert_eq!(background(&remote), Some(true), "{remote:?}");
}

#[test]
fn a_background_start_never_picks_the_vt_on_screen() {
    let mut run = remote_run(enabled());
    run.world.active_vt = 7;
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 8));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
}

#[test]
fn a_second_prompt_from_pam_fails_the_remote_login() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    let effects = run.worker(2, secret("New password: "));
    assert_eq!(quiet(effects), [Effect::Kill(WorkerId(2))]);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    let visible = FromWorker::Prompt {
        kind: AuthKind::Visible,
        text: "login: ".into(),
    };
    assert_eq!(quiet(run.worker(2, visible)), [Effect::Kill(WorkerId(2))]);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
}

#[test]
fn a_login_for_a_user_with_a_session_hands_over_to_it_without_starting_one() {
    let mut run = anna_recovered(enabled(), "g1", Mode::Login.service());
    let effects = run.verify(7, 1, "anna", ANNA);
    assert_eq!(
        effects,
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [front(FrontEvent::Accepted { conn: 7 })]
    );
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
}

#[test]
fn a_login_for_the_console_users_session_hands_over_to_it() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", "reprise-login")];
    assert_eq!(
        run.d.recover(run.now, found, Some("c1")),
        [Effect::OpenIntake { uid: ANNA }],
        "the session on the console is neither locked nor a background one"
    );
    let mut effects = run.verify(7, 1, "anna", ANNA);
    assert_eq!(
        effects,
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    effects.extend(run.on(Input::IntakeOpen { uid: ANNA }));
    effects.extend(run.on(ready(7, "anna")));
    assert_eq!(
        effects[3..],
        [
            front(FrontEvent::Accepted { conn: 7 }),
            Effect::Forward { conn: 7, uid: ANNA },
        ]
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::UnlockSession(_) | Effect::SwitchTo(_))),
        "{effects:?}"
    );
}

#[test]
fn two_logins_for_one_user_while_their_session_starts_share_it() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(tls_from(8, 2, "anna", "pw"));
    run.on(member(7, ANNA));
    assert_eq!(
        run.on(member(8, ANNA)),
        remote_spawn(3, Mode::Remote, "anna", 2)
    );
    run.worker(2, secret("Password: "));
    run.worker(3, secret("Password: "));
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
    assert_eq!(
        run.worker(3, authed("anna", ANNA)),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel)
        ],
        "the second login waits for the first one's start"
    );
    assert_eq!(
        run.worker(2, started("c2", ANNA)),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle)
        ]
    );
    assert_eq!(
        run.on(Input::IntakeOpen { uid: ANNA }),
        [
            front(FrontEvent::Accepted { conn: 7 }),
            front(FrontEvent::Accepted { conn: 8 })
        ]
    );
    assert_eq!(
        run.on(ready(8, "anna")),
        [Effect::Forward { conn: 8, uid: ANNA }]
    );
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
}

#[test]
fn every_refusal_waits_the_refuse_delay() {
    let not_member = Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(ANNA),
        member: false,
    };
    let unknown = Input::Member {
        key: LookUpKey::Conn(7),
        uid: None,
        member: false,
    };
    let root = Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(0),
        member: true,
    };
    for answer in [not_member, unknown, root] {
        let mut run = remote_run(enabled());
        assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
        assert_eq!(quiet(run.on(answer)), []);
        assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    }
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "wrong"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    run.after(Duration::from_secs(1));
    let refused = run.worker(2, failed(ErrorKind::Auth, AUTH_FAILED));
    assert_eq!(quiet(refused), [], "no Denied before REFUSE_DELAY");
    assert_eq!(
        run.after(REFUSE_DELAY - Duration::from_secs(1))
            .fire(Timer::Refuse(7)),
        denied(7, 0)
    );
}

#[test]
fn a_refusal_decided_after_the_refuse_delay_goes_out_at_once_and_is_logged_as_slow() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "wrong"));
    run.on(member(7, ANNA));
    run.worker(2, secret("Password: "));
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), []);
    let effects = run
        .after(Duration::from_secs(1))
        .worker(2, failed(ErrorKind::Auth, AUTH_FAILED));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Log(Level::Warn, text) if text.contains("slow"))),
        "{effects:?}"
    );
    assert_eq!(quiet(effects), denied(7, 0));
}

#[test]
fn failures_back_off_the_address_and_a_pause_stops_every_login() {
    let mut run = remote_run(enabled());
    let not_member = |conn| Input::Member {
        key: LookUpKey::Conn(conn),
        uid: Some(ANNA),
        member: false,
    };
    for conn in 1..=4 {
        assert_eq!(run.on(tls(conn, "anna", "pw")), looked_up(conn, "anna"));
        run.on(not_member(conn));
    }
    assert_eq!(
        quiet(run.on(tls(5, "anna", "pw"))),
        armed(5),
        "the fifth login after four failures is refused without a look-up"
    );
    match run.after(REFUSE_DELAY).fire(Timer::Refuse(5)).as_slice() {
        [Effect::ToFront(FrontEvent::Denied { conn: 5, backoff_s })] => assert!(*backoff_s > 0),
        other => panic!("{other:?}"),
    }

    let mut run = remote_run(enabled());
    for n in 1..=PAUSE_AFTER as u8 {
        let conn = u64::from(n);
        assert_eq!(
            run.on(tls_from(conn, n, "anna", "pw")),
            looked_up(conn, "anna")
        );
        run.on(not_member(conn));
        run.after(Duration::from_secs(30));
    }
    let effects = run.on(tls_from(11, 11, "anna", "pw"));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Log(Level::Warn, text) if text.contains("paused"))),
        "{effects:?}"
    );
    assert_eq!(
        quiet(effects),
        armed(11),
        "a pause refuses an address that never failed"
    );
    assert_eq!(
        run.after(REFUSE_DELAY).fire(Timer::Refuse(11)),
        denied(11, 0)
    );
}

#[test]
fn max_sessions_refuses_a_new_session_but_not_an_existing_one() {
    let remote = MachineRemote {
        max_sessions: 1,
        ..enabled()
    };
    let mut run = anna_recovered(remote, "c1", Mode::Login.service());
    let effects = run.verify(7, 1, "carl", CARL);
    assert_eq!(
        quiet(effects),
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel)
        ]
    );
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    assert_eq!(
        run.verify(8, 2, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(2, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
}

#[test]
fn a_hand_over_not_taken_by_the_deadline_is_denied_and_the_session_kept() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    let effects = run.after(HANDOVER_DEADLINE).fire(Timer::Handover(7));
    assert_eq!(
        quiet(effects),
        [
            front(FrontEvent::Denied {
                conn: 7,
                backoff_s: 0
            }),
            Effect::DropConn { conn: 7 },
        ]
    );
    assert!(run.d.sessions.by_id("c2").is_some(), "the session stays");
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(
        run.on(ready(7, "anna")).last(),
        Some(&Effect::DropConn { conn: 7 })
    );
}

#[test]
fn a_login_whose_authenticate_comes_late_still_gets_the_whole_hand_over_deadline() {
    let mut run = remote_run(enabled());
    run.after(Duration::from_secs(20));
    assert_eq!(run.on(tls(7, "anna", "pw")), looked_up(7, "anna"));
    let early = HANDOVER_DEADLINE - Duration::from_millis(100);
    assert_eq!(
        run.after(early).fire(Timer::Handover(7)),
        [Effect::Arm {
            timer: Timer::Handover(7),
            after: Duration::from_millis(100),
        }]
    );
}

#[test]
fn nla_verifies_the_delegated_password_against_the_stored_hash() {
    let mut run = remote_run(enabled());
    assert_eq!(
        run.d
            .set_remote_passwords(vec![stored("anna", ANNA, "Remote1!")]),
        []
    );
    assert_eq!(run.on(nla(7, "anna", "Remote1!")), looked_up(7, "anna"));
    assert_eq!(
        run.on(member(7, ANNA)),
        remote_spawn(2, Mode::RemoteNla, "anna", 1),
        "a match spawns the account check, and no answer goes to it"
    );
    assert_eq!(run.on(nla(8, "anna", "remote1!")), looked_up(8, "anna"));
    assert_eq!(quiet(run.on(member(8, ANNA))), [], "a mismatch");
    assert_eq!(run.on(nla(9, "carl", "Remote1!")), looked_up(9, "carl"));
    assert_eq!(quiet(run.on(member(9, CARL))), [], "no stored hash");
    run.after(REFUSE_DELAY);
    assert_eq!(run.fire(Timer::Refuse(8)), denied(8, 0));
    assert_eq!(run.fire(Timer::Refuse(9)), denied(9, 0));
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.worker(2, authed("anna", ANNA)), want);
}

#[test]
fn nla_into_a_running_session_runs_the_account_check_first() {
    let mut run = anna_recovered(enabled(), "c1", Mode::Login.service());
    run.d
        .set_remote_passwords(vec![stored("anna", ANNA, "Remote1!")]);
    run.on(nla(7, "anna", "Remote1!"));
    assert_eq!(
        run.on(member(7, ANNA)),
        remote_spawn(1, Mode::RemoteNla, "anna", 1)
    );
    assert_eq!(
        run.worker(1, authed("anna", ANNA)),
        [
            front(FrontEvent::Verified { conn: 7 }),
            to_worker(1, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    run.on(nla(8, "anna", "Remote1!"));
    run.on(member(8, ANNA));
    let expired = run.worker(2, failed(ErrorKind::Account, "the account has expired"));
    assert_eq!(quiet(expired), [], "a refused account is never verified");
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(8)), denied(8, 0));
}

#[test]
fn remote_logins_while_disabled_are_denied() {
    let mut run = remote_run(MachineRemote::default());
    assert_eq!(quiet(run.on(tls(7, "anna", "pw"))), armed(7));
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
}

#[test]
fn more_than_max_remote_checks_at_once_are_denied() {
    let mut run = remote_run(enabled());
    for conn in 1..=MAX_REMOTE_CHECKS as u64 {
        assert_eq!(
            run.on(tls_from(conn, conn as u8, "anna", "pw")),
            looked_up(conn, "anna")
        );
    }
    assert_eq!(quiet(run.on(tls_from(9, 9, "anna", "pw"))), armed(9));
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(9)), denied(9, 0));
    run.on(member(1, ANNA));
    run.worker(2, secret("Password: "));
    run.worker(2, authed("anna", ANNA));
    assert_eq!(
        run.on(tls_from(10, 10, "anna", "pw")),
        looked_up(10, "anna"),
        "a verified login is no longer a check"
    );
}

#[test]
fn a_front_holding_max_logins_is_not_heard_until_one_ends() {
    let mut run = remote_run(enabled());
    for conn in 0..MAX_LOGINS as u64 {
        run.on(tls_from(conn, conn as u8, "Not A Name", "pw"));
    }
    assert_eq!(
        quiet(run.on(tls_from(900, 1, "anna", "pw"))),
        vec![],
        "a login past the cap is dropped unanswered"
    );
    run.after(REFUSE_DELAY).fire(Timer::Refuse(0));
    assert_eq!(quiet(run.on(tls_from(901, 1, "anna", "pw"))), armed(901));
}

#[test]
fn a_ready_for_another_user_is_dropped() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        quiet(run.on(ready(7, "bert"))),
        [Effect::DropConn { conn: 7 }]
    );
    assert_eq!(
        quiet(run.on(ready(7, "anna"))),
        [Effect::DropConn { conn: 7 }]
    );
}

#[test]
fn a_ready_for_a_conn_never_accepted_is_dropped() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    assert_eq!(
        quiet(run.on(ready(7, "anna"))),
        [Effect::DropConn { conn: 7 }]
    );
    assert_eq!(quiet(run.on(member(7, ANNA))), [], "the login is forgotten");
    assert_eq!(
        quiet(run.on(ready(9, "anna"))),
        [Effect::DropConn { conn: 9 }]
    );
}

#[test]
fn a_user_name_no_account_can_have_is_refused_without_a_look_up() {
    let long = "a".repeat(33);
    for (conn, name) in [
        (1, "Anna"),
        (2, "1anna"),
        (3, "an na"),
        (4, ""),
        (5, long.as_str()),
    ] {
        let mut run = remote_run(enabled());
        assert_eq!(
            quiet(run.on(tls(conn, name, "pw"))),
            armed(conn),
            "{name:?}"
        );
        assert_eq!(
            run.after(REFUSE_DELAY).fire(Timer::Refuse(conn)),
            denied(conn, 0)
        );
    }
    let mut run = remote_run(enabled());
    let name = format!("_{}", "a.b-c_9".repeat(5))[..32].to_string();
    assert_eq!(run.on(tls(1, &name, "pw")), looked_up(1, &name));
}

#[test]
fn a_lost_front_kills_unstarted_checks_and_forgets_its_conns() {
    let mut run = remote_run(enabled());
    run.on(Input::FrontLinked);
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.verify(7, 2, "anna", ANNA), want);
    run.on(tls_from(8, 2, "bert", "pw"));
    run.on(member(8, BERT));
    run.on(tls_from(9, 3, "carl", "pw"));
    assert_eq!(run.on(Input::FrontLost), [Effect::Kill(WorkerId(3))]);
    assert_eq!(
        run.worker(2, started("c2", ANNA)),
        [
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle)
        ],
        "a start in flight goes on"
    );
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(quiet(run.on(member(9, CARL))), []);
    run.after(HANDOVER_DEADLINE);
    for conn in [7, 8, 9] {
        assert_eq!(run.fire(Timer::Refuse(conn)), []);
        assert_eq!(run.fire(Timer::Handover(conn)), []);
    }
    run.on(Input::FrontLinked);
    assert_eq!(
        run.on(tls(7, "carl", "pw")),
        looked_up(7, "carl"),
        "a new front's conn 7 is a new login"
    );
}

#[test]
fn a_lost_front_drops_the_hand_overs_in_flight() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
    assert_eq!(run.on(Input::FrontLost), [Effect::DropConn { conn: 7 }]);
    assert_eq!(
        run.on(Input::Forwarded { conn: 7, ok: true }),
        [],
        "a late result is forgotten"
    );
}

#[test]
fn a_gone_conn_kills_its_worker() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    assert_eq!(
        run.on(Input::Front(FrontRequest::Gone { conn: 7 })),
        [Effect::Kill(WorkerId(2))]
    );
    assert_eq!(run.worker(2, secret("Password: ")), []);
    assert_eq!(run.exited(2), []);
    run.after(HANDOVER_DEADLINE);
    assert_eq!(run.fire(Timer::Refuse(7)), []);
    assert_eq!(run.fire(Timer::Handover(7)), []);
}

#[test]
fn a_front_that_links_while_remote_logins_are_off_is_stopped() {
    let mut run = remote_run(MachineRemote::default());
    assert_eq!(run.on(Input::FrontLinked), []);
    assert_eq!(
        run.on(Input::Front(FrontRequest::Hello)),
        [Effect::Front(FrontUnit::Stop)],
        "the unit restarts with reprise-login whatever the settings say"
    );
}

#[test]
fn the_front_hears_the_nla_users_on_hello() {
    let mut run = remote_run(enabled());
    let anna = stored("anna", ANNA, "Remote1!");
    assert_eq!(run.d.set_remote_passwords(vec![anna.clone()]), []);
    assert_eq!(run.on(Input::FrontLinked), []);
    let nla_user = |p: &StoredPassword| NlaUser {
        user: p.user.clone(),
        nt_hash: p.nt_hash.clone(),
    };
    assert_eq!(
        run.on(Input::Front(FrontRequest::Hello)),
        [front(FrontEvent::Passwords {
            users: vec![nla_user(&anna)]
        })]
    );
    let bert = stored("bert", BERT, "Another1!");
    assert_eq!(
        run.d.set_remote_passwords(vec![anna.clone(), bert.clone()]),
        [front(FrontEvent::Passwords {
            users: vec![nla_user(&anna), nla_user(&bert)]
        })]
    );
    let shown = format!("{anna:?}");
    assert!(!shown.contains(&anna.nt_hash[..8]), "{shown}");
}

#[test]
fn a_background_session_ending_leaves_the_console_alone() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    assert_eq!(
        run.removed("c2", Some("g1")),
        [Effect::Inhibit(Inhibit::Nothing)],
        "no greeter, switch or notice"
    );
    assert_eq!(run.exited(2), [Effect::Disallocate(7)]);
    let mut run = anna_recovered(enabled(), "g1", Mode::Remote.service());
    assert_eq!(
        run.removed("c1", Some("g1")),
        [Effect::Disallocate(7), Effect::Inhibit(Inhibit::Nothing)]
    );
}

#[test]
fn a_background_session_switched_to_on_the_console_becomes_the_consoles() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    assert_eq!(
        quiet(run.on(Input::ConsoleOnVt(7))),
        [Effect::Inhibit(Inhibit::Nothing)]
    );
    assert_eq!(
        run.d.sessions.by_id("c2").map(|e| e.background),
        Some(false)
    );
    let disconnected = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.fire(disconnected), [], "someone uses it at the console");
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [Effect::Inhibit(Inhibit::Nothing)],
        "no Disconnected timer"
    );
    let effects = run.removed("c2", None);
    assert!(
        matches!(
            effects.as_slice(),
            [
                Effect::SwitchTo(GREETER_VT),
                Effect::Reply(GreeterReply::Greeting(_))
            ]
        ),
        "the login screen comes back to the console: {effects:?}"
    );
}

#[test]
fn the_inhibitor_follows_viewers_and_background_sessions() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(run.viewers(ANNA, 2), []);
    assert_eq!(run.viewers(ANNA, 0), [Effect::Inhibit(Inhibit::Idle)]);
    assert_eq!(
        run.removed("c2", Some("g1")),
        [Effect::Inhibit(Inhibit::Nothing)]
    );
    let mut run = anna_recovered(enabled(), "c1", Mode::Login.service());
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)],
        "viewers of the console's session hold off sleep too"
    );
    assert_eq!(run.viewers(ANNA, 0), [Effect::Inhibit(Inhibit::Nothing)]);
}

#[test]
fn a_disconnected_background_session_ends_after_end_disconnected_after() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let disconnected = |id: &str, generation| Timer::Disconnected {
        id: id.into(),
        generation,
    };
    let arm = |generation| disconnect_arm("c2", generation, 30);
    let mut run = remote_run(remote.clone());
    run.anna_in_background(7, 2);
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [arm(2), Effect::Inhibit(Inhibit::Idle)]
    );
    assert_eq!(
        quiet(run.fire(disconnected("c2", 2))),
        [Effect::TerminateSession("c2".into())]
    );

    let mut run = remote_run(remote.clone());
    run.anna_in_background(7, 2);
    run.viewers(ANNA, 1);
    run.viewers(ANNA, 0);
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [arm(3), Effect::Inhibit(Inhibit::Idle)]
    );
    assert_eq!(run.fire(disconnected("c2", 1)), [], "a stale generation");
    assert_eq!(run.fire(disconnected("c2", 2)), [], "a stale generation");
    assert_eq!(
        quiet(run.fire(disconnected("c2", 3))),
        [Effect::TerminateSession("c2".into())]
    );

    let mut run = remote_run(remote.clone());
    run.anna_in_background(7, 2);
    assert_eq!(
        quiet(run.fire(disconnected("c2", 1))),
        [Effect::TerminateSession("c2".into())],
        "a session no client ever reached ends too"
    );

    let mut run = Run::machine(config(remote.clone()));
    let found = vec![logind(
        "c1",
        "anna",
        ANNA,
        7,
        "user",
        "reprise-login-remote",
    )];
    assert_eq!(
        run.d.recover(run.now, found, Some("g1")),
        [
            disconnect_arm("c1", 1, 30),
            Effect::OpenIntake { uid: ANNA },
            Effect::Inhibit(Inhibit::Idle),
        ]
    );
    assert_eq!(
        quiet(run.fire(disconnected("c1", 1))),
        [Effect::TerminateSession("c1".into())],
        "a recovered background session with no client ends"
    );

    let mut run = anna_recovered(remote, "c1", Mode::Login.service());
    run.viewers(ANNA, 1);
    assert_eq!(
        run.viewers(ANNA, 0),
        [Effect::Inhibit(Inhibit::Nothing)],
        "a console session arms nothing"
    );
}

fn ending_after(min: Option<u32>) -> Input {
    Input::ConfigReloaded(config(MachineRemote {
        end_disconnected_after_min: min,
        ..enabled()
    }))
}

/// Anna's background session `c2`, started at the run's start with no client, its
/// `Disconnected` timer armed for 30 minutes as generation 1.
fn anna_unviewed_for_30() -> Run {
    let mut run = remote_run(MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    });
    run.anna_in_background(7, 2);
    run
}

#[test]
fn never_cancels_an_armed_disconnected_timer_and_a_value_re_arms_it_from_the_disconnect() {
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        run.after(Duration::from_secs(600)).on(ending_after(None)),
        []
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.after(Duration::from_secs(1200)).fire(gen1), []);
    assert_eq!(
        run.after(Duration::from_secs(60))
            .on(ending_after(Some(40))),
        [Effect::Arm {
            timer: Timer::Disconnected {
                id: "c2".into(),
                generation: 2
            },
            after: Duration::from_secs(40 * 60 - 1860),
        }],
        "40 minutes from the disconnect, not from the reload"
    );
}

#[test]
fn a_shorter_end_disconnected_after_ends_a_session_past_it_at_once() {
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        quiet(
            run.after(Duration::from_secs(600))
                .on(ending_after(Some(5)))
        ),
        [Effect::TerminateSession("c2".into())]
    );
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        run.after(Duration::from_secs(60)).on(ending_after(Some(5))),
        [Effect::Arm {
            timer: Timer::Disconnected {
                id: "c2".into(),
                generation: 2
            },
            after: Duration::from_secs(240),
        }]
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.fire(gen1), [], "the old timer is stale");
}

#[test]
fn a_longer_end_disconnected_after_re_arms_from_the_disconnect() {
    let mut run = anna_unviewed_for_30();
    assert_eq!(
        run.after(Duration::from_secs(600))
            .on(ending_after(Some(60))),
        [Effect::Arm {
            timer: Timer::Disconnected {
                id: "c2".into(),
                generation: 2
            },
            after: Duration::from_secs(50 * 60),
        }]
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.after(Duration::from_secs(1200)).fire(gen1), []);
    let gen2 = Timer::Disconnected {
        id: "c2".into(),
        generation: 2,
    };
    assert_eq!(
        quiet(run.after(Duration::from_secs(1800)).fire(gen2)),
        [Effect::TerminateSession("c2".into())]
    );
    assert_eq!(
        run.after(Duration::from_secs(1)).on(ending_after(Some(60))),
        [],
        "an unchanged value re-arms nothing"
    );
}

#[test]
fn resuming_a_background_session_at_the_login_screen_makes_it_the_consoles() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    run.viewers(ANNA, 1);
    run.viewers(ANNA, 0);
    assert_eq!(run.greeter(create("anna")), spawn(3, Mode::Reauth, "anna"));
    assert_eq!(
        run.worker(3, authed("anna", ANNA)),
        reply(GreeterReply::Success)
    );
    assert_eq!(run.greeter(GreeterRequest::StartSession), unlock("c2", 1));
    let effects = run.on(Input::Unlocked { id: "c2".into() });
    assert_eq!(effects[0], Effect::SwitchTo(7));
    assert_eq!(effects.last(), Some(&Effect::Inhibit(Inhibit::Nothing)));
    for generation in [1, 2] {
        let disconnected = Timer::Disconnected {
            id: "c2".into(),
            generation,
        };
        assert_eq!(run.fire(disconnected), []);
    }
}

#[test]
fn greeting_notes_a_session_with_remote_viewers_as_remote() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    let notes = |run: &mut Run| -> Vec<_> { run.greeting().users.iter().map(|u| u.note).collect() };
    assert_eq!(notes(&mut run), [Some(Note::LoggedOn), None]);
    run.viewers(ANNA, 1);
    assert_eq!(notes(&mut run), [Some(Note::Remote), None]);
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(
        notes(&mut run),
        [Some(Note::Remote), None],
        "while it reopens"
    );
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(notes(&mut run), [Some(Note::LoggedOn), None]);
}

#[test]
fn the_login_screen_hears_each_change_of_its_notes() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    assert_eq!(run.greeting().users[0].note, Some(Note::LoggedOn));
    let greeted = |effects: Vec<Effect>| {
        effects.into_iter().find_map(|e| match e {
            Effect::Reply(GreeterReply::Greeting(g)) => Some(g.users[0].note),
            _ => None,
        })
    };
    assert_eq!(greeted(run.viewers(ANNA, 1)), Some(Some(Note::Remote)));
    assert_eq!(
        greeted(run.viewers(ANNA, 2)),
        None,
        "the notes stay the same"
    );
    assert_eq!(greeted(run.viewers(ANNA, 0)), Some(Some(Note::LoggedOn)));
}

#[test]
fn recovery_marks_sessions_off_the_seat_background_and_opens_their_intakes() {
    let mut run = Run::machine(config(enabled()));
    let found = vec![
        logind("c1", "anna", ANNA, 7, "user", "reprise-login-remote"),
        logind("c3", "bert", BERT, 8, "user", "reprise-login"),
        logind("c4", "carl", CARL, 9, "user", "reprise-login"),
    ];
    assert_eq!(
        run.d.recover(run.now, found, Some("c3")),
        [
            Effect::LockSession("c4".into()),
            Effect::OpenIntake { uid: ANNA },
            Effect::OpenIntake { uid: BERT },
            Effect::OpenIntake { uid: CARL },
            Effect::Inhibit(Inhibit::Idle),
        ]
    );
    let background = |id| run.d.sessions.by_id(id).map(|e| e.background);
    assert_eq!(background("c1"), Some(true));
    assert_eq!(background("c3"), Some(false));
    assert_eq!(
        background("c4"),
        Some(false),
        "a console session off the seat is no background one"
    );
}

#[test]
fn a_recovered_switched_away_console_session_holds_no_inhibitor_and_never_ends_unviewed() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = Run::machine(config(remote));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", "reprise-login")];
    assert_eq!(
        run.d.recover(run.now, found, Some("g1")),
        [
            Effect::LockSession("c1".into()),
            Effect::OpenIntake { uid: ANNA }
        ]
    );
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(
        run.viewers(ANNA, 0),
        [Effect::Inhibit(Inhibit::Nothing)],
        "no Disconnected timer"
    );
}

#[test]
fn intake_links_open_only_while_remote_logins_are_enabled() {
    let mut run = Run::machine(config(MachineRemote::default()));
    let found = vec![logind(
        "c1",
        "anna",
        ANNA,
        7,
        "user",
        "reprise-login-remote-nla",
    )];
    assert_eq!(
        run.d.recover(run.now, found, Some("g1")),
        [Effect::Inhibit(Inhibit::Idle)]
    );
}

#[test]
fn recovery_leaves_a_background_sessions_console_lock_to_its_compositor() {
    for service in ["reprise-login-remote", "reprise-login-remote-nla"] {
        let mut run = Run::machine(config(enabled()));
        let found = vec![logind("c1", "anna", ANNA, 7, "user", service)];
        let effects = run.d.recover(run.now, found, Some("g1"));
        assert!(
            !effects.contains(&Effect::LockSession("c1".into())),
            "{service}: a logind lock would lock its connected client out too: {effects:?}"
        );
    }
}

#[test]
fn a_lost_intake_link_reopens_and_keeps_its_viewers_until_the_new_link_reports() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [Effect::ReopenIntake { uid: ANNA }]
    );
    assert_eq!(
        run.fire(Timer::Disconnected {
            id: "c2".into(),
            generation: 1
        }),
        [],
        "the client is still counted"
    );
    assert_eq!(run.on(Input::IntakeOpen { uid: ANNA }), []);
    assert_eq!(run.viewers(ANNA, 1), []);
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [Effect::ReopenIntake { uid: ANNA }],
        "a link lost after its reopen opens again"
    );
}

#[test]
fn a_reopen_that_gives_up_zeroes_its_viewers_and_the_next_login_opens_it() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote);
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.viewers(ANNA, 1);
    run.on(Input::IntakeLost { uid: ANNA });
    assert_eq!(
        run.on(Input::IntakeLost { uid: ANNA }),
        [disconnect_arm("c2", 2, 30), Effect::Inhibit(Inhibit::Idle)]
    );
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            Effect::OpenIntake { uid: ANNA },
        ]
    );
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        run.verify(8, 3, "anna", ANNA),
        [
            front(FrontEvent::Verified { conn: 8 }),
            to_worker(3, ToWorker::Cancel),
            front(FrontEvent::Accepted { conn: 8 }),
        ],
        "an open link takes the login at once"
    );
}

#[test]
fn a_lost_link_of_a_uid_with_no_session_is_not_reopened() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.removed("c2", Some("g1"));
    assert_eq!(run.on(Input::IntakeLost { uid: ANNA }), []);
}

#[test]
fn a_background_session_removed_before_its_start_reports_leaves_no_entry() {
    let mut run = remote_run(enabled());
    let mut want = vec![front(FrontEvent::Verified { conn: 7 })];
    want.extend(background_start(2, 7));
    assert_eq!(run.verify(7, 2, "anna", ANNA), want);
    assert_eq!(run.removed("c2", Some("g1")), []);
    assert_eq!(
        quiet(run.worker(2, started("c2", ANNA))),
        [],
        "no intake link and no inhibitor"
    );
    assert_eq!(run.d.sessions.by_uid(ANNA), None);
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
    assert_eq!(run.exited(2), [Effect::Disallocate(7)]);
}

#[test]
fn every_refusal_from_one_address_carries_the_back_off_of_its_authenticate() {
    let backoff = |wrong_password: bool| {
        let mut run = remote_run(enabled());
        for conn in 1..=3 {
            run.on(tls(conn, "anna", "pw"));
            run.on(Input::Member {
                key: LookUpKey::Conn(conn),
                uid: Some(ANNA),
                member: false,
            });
        }
        run.on(tls(4, "anna", "pw"));
        if wrong_password {
            run.on(member(4, ANNA));
            run.worker(2, secret("Password: "));
            run.after(Duration::from_millis(1500));
            run.worker(2, failed(ErrorKind::Auth, AUTH_FAILED));
            run.after(REFUSE_DELAY - Duration::from_millis(1500));
        } else {
            run.after(Duration::from_millis(50));
            run.on(Input::Member {
                key: LookUpKey::Conn(4),
                uid: Some(ANNA),
                member: false,
            });
            run.after(REFUSE_DELAY - Duration::from_millis(50));
        }
        match run.fire(Timer::Refuse(4)).as_slice() {
            [Effect::ToFront(FrontEvent::Denied { conn: 4, backoff_s })] => *backoff_s,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(backoff(false), 2, "the fourth failure backs off 2 s");
    assert_eq!(backoff(true), 2, "a wrong password tells the same");
}

#[test]
fn pam_passing_a_tls_login_without_asking_its_password_refuses_it() {
    let mut run = remote_run(enabled());
    run.on(tls(7, "anna", "pw"));
    run.on(member(7, ANNA));
    assert_eq!(
        quiet(run.worker(2, authed("anna", ANNA))),
        [Effect::Kill(WorkerId(2))]
    );
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(7)), denied(7, 0));
}

#[test]
fn a_forward_never_confirmed_expires_at_the_hand_over_deadline() {
    let mut run = remote_run(enabled());
    run.anna_in_background(7, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    assert_eq!(
        run.on(ready(7, "anna")),
        [Effect::Forward { conn: 7, uid: ANNA }]
    );
    let effects = run.after(HANDOVER_DEADLINE).fire(Timer::Handover(7));
    assert_eq!(quiet(effects), [Effect::DropConn { conn: 7 }]);
    assert_eq!(run.on(Input::Forwarded { conn: 7, ok: true }), []);
    assert_eq!(
        run.on(tls(7, "anna", "pw")),
        looked_up(7, "anna"),
        "conn 7 is free again"
    );
}

#[test]
fn viewers_for_a_uid_with_no_session_are_ignored() {
    let mut run = remote_run(enabled());
    assert_eq!(run.viewers(ANNA, 1), []);
    run.anna_in_background(7, 2);
}

#[test]
fn a_reused_conn_id_re_arms_the_earlier_conns_timers() {
    let mut run = remote_run(enabled());
    let not_member = || Input::Member {
        key: LookUpKey::Conn(7),
        uid: Some(ANNA),
        member: false,
    };
    run.on(tls(7, "anna", "pw"));
    run.on(not_member());
    run.on(Input::FrontLost);
    run.on(Input::FrontLinked);
    run.after(Duration::from_secs(2));
    run.on(tls(7, "anna", "pw"));
    run.on(not_member());
    assert_eq!(
        run.after(Duration::from_secs(1)).fire(Timer::Refuse(7)),
        [Effect::Arm {
            timer: Timer::Refuse(7),
            after: REFUSE_DELAY - Duration::from_secs(1),
        }],
        "the first conn's Refuse at +3 s"
    );
    assert_eq!(
        run.after(Duration::from_secs(5)).fire(Timer::Handover(7)),
        [Effect::Arm {
            timer: Timer::Handover(7),
            after: Duration::from_secs(2),
        }],
        "the first conn's deadline at +8 s"
    );
}

#[test]
fn the_front_starts_with_the_daemon_when_remote_logins_are_on() {
    let mut run = Run::machine(config(enabled()));
    run.recover(vec![]);
    let effects = run.start();
    assert_eq!(effects[0], Effect::Front(FrontUnit::Start), "{effects:?}");
    assert_eq!(effects[1..], greeter_spawn(1));
    let mut run = Run::machine(config(MachineRemote::default()));
    run.recover(vec![]);
    let effects = run.start();
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Front(_))),
        "{effects:?}"
    );
}

#[test]
fn turning_remote_on_starts_the_front_and_opens_every_sessions_intake() {
    let mut run = anna_recovered(MachineRemote::default(), "c1", Mode::Login.service());
    assert_eq!(
        run.on(Input::ConfigReloaded(config(enabled()))),
        [
            Effect::Front(FrontUnit::Start),
            Effect::OpenIntake { uid: ANNA }
        ]
    );
    assert_eq!(run.on(Input::ConfigReloaded(config(enabled()))), []);
}

#[test]
fn turning_remote_off_stops_the_front_and_denies_pending_logins_but_keeps_connected_clients() {
    let remote = MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    };
    let mut run = remote_run(remote.clone());
    run.anna_in_background(1, 2);
    run.on(Input::IntakeOpen { uid: ANNA });
    run.on(ready(1, "anna"));
    run.on(Input::Forwarded { conn: 1, ok: true });
    assert_eq!(
        run.viewers(ANNA, 1),
        [Effect::Inhibit(Inhibit::SleepAndIdle)]
    );
    assert_eq!(run.on(tls_from(7, 2, "bert", "pw")), looked_up(7, "bert"));
    run.on(tls_from(8, 3, "carl", "pw"));
    assert_eq!(
        run.on(member(8, CARL)),
        remote_spawn(3, Mode::Remote, "carl", 3)
    );
    run.on(tls_from(9, 4, "dora", "pw"));
    let unknown = Input::Member {
        key: LookUpKey::Conn(9),
        uid: None,
        member: false,
    };
    assert_eq!(quiet(run.on(unknown)), []);

    let off = MachineRemote {
        enabled: false,
        ..remote
    };
    let effects = run.on(Input::ConfigReloaded(config(off)));
    assert_eq!(
        quiet(effects),
        [
            Effect::Front(FrontUnit::Stop),
            front(FrontEvent::Denied {
                conn: 7,
                backoff_s: 0
            }),
            Effect::Kill(WorkerId(3)),
            front(FrontEvent::Denied {
                conn: 8,
                backoff_s: 0
            }),
            front(FrontEvent::Denied {
                conn: 9,
                backoff_s: 0
            }),
            Effect::CloseIntakes {
                keep: BTreeSet::from([ANNA])
            },
        ],
        "anna's client keeps the inhibitor and arms no Disconnected timer"
    );
    let gen1 = Timer::Disconnected {
        id: "c2".into(),
        generation: 1,
    };
    assert_eq!(run.fire(gen1), []);
    assert_eq!(run.on(member(7, BERT)), [], "a denied login is forgotten");
    assert_eq!(run.after(REFUSE_DELAY).fire(Timer::Refuse(9)), []);
    assert_eq!(quiet(run.on(tls(10, "anna", "pw"))), armed(10));
    assert_eq!(
        run.after(REFUSE_DELAY).fire(Timer::Refuse(10)),
        denied(10, 0),
        "logins stay off"
    );
    assert_eq!(
        run.viewers(ANNA, 0),
        [disconnect_arm("c2", 2, 30), Effect::Inhibit(Inhibit::Idle)],
        "the kept link still reports"
    );
}

#[test]
fn a_port_change_restarts_the_front() {
    let mut run = remote_run(enabled());
    let moved = MachineRemote {
        port: 3390,
        ..enabled()
    };
    assert_eq!(
        run.on(Input::ConfigReloaded(config(moved.clone()))),
        [Effect::Front(FrontUnit::Restart)]
    );
    let more = MachineRemote {
        max_sessions: 2,
        ..moved
    };
    assert_eq!(
        run.on(Input::ConfigReloaded(config(more))),
        [],
        "other remote settings apply to the next login"
    );
    let mut run = remote_run(MachineRemote::default());
    let moved_off = MachineRemote {
        port: 3390,
        ..MachineRemote::default()
    };
    assert_eq!(run.on(Input::ConfigReloaded(config(moved_off))), []);
}
