use std::collections::BTreeSet;
use std::net::SocketAddr;

use repriselogin::account::nt_hash;
use repriselogin::front::{FrontEvent, FrontRequest, NlaUser, Secret};
use repriselogin::greeter::{AuthKind, ErrorKind, GreeterRequest, Note};
use repriselogin::machine::{MachineConfig, MachineRemote};
use repriselogin::throttle::PAUSE_AFTER;
use zeroize::Zeroizing;

use super::tests::{
    Run, authed, closing, create, greeter_spawn, logind, reply, secret, spawn, started, unlock,
};
use super::*;

mod background;
mod links;
mod logins;
mod passwords;
mod port;
mod recovery;

pub(super) const ANNA: u32 = 1001;
const BERT: u32 = 1002;
const CARL: u32 = 1003;

fn config(remote: MachineRemote) -> MachineConfig {
    MachineConfig {
        remote,
        ..MachineConfig::default()
    }
}

pub(super) fn enabled() -> MachineRemote {
    MachineRemote {
        enabled: true,
        ..MachineRemote::default()
    }
}

/// Nothing recovered, the greeter up as worker 1.
pub(super) fn remote_run(remote: MachineRemote) -> Run {
    let want = greeter_spawn(1);
    let mut run = Run::machine(config(remote));
    assert_eq!(run.recover(vec![]), vec![]);
    assert_eq!(run.start(), want);
    run
}

/// Anna's session `c1` on VT 7 recovered with `seat_active` on seat0.
fn anna_recovered(remote: MachineRemote, seat_active: &str, service: &str) -> Run {
    let mut run = Run::machine(config(remote));
    let found = vec![logind("c1", "anna", ANNA, 7, "user", service)];
    run.d.recover(found, Some(seat_active));
    assert_eq!(run.start(), []);
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
    nla_from(conn, 1, user, password)
}

fn nla_from(conn: u64, from: u8, user: &str, password: &str) -> Input {
    authenticate(
        conn,
        from,
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

fn link_look_up(uid: u32, user: &str) -> Effect {
    Effect::LookUp {
        key: LookUpKey::Link(uid),
        user: user.into(),
        group: "reprise-remote".into(),
    }
}

fn link_member(uid: u32, found: Option<u32>, member: bool) -> Input {
    Input::Member {
        key: LookUpKey::Link(uid),
        uid: found,
        member,
        failed: false,
    }
}

fn member(conn: u64, uid: u32) -> Input {
    Input::Member {
        key: LookUpKey::Conn(conn),
        uid: Some(uid),
        member: true,
        failed: false,
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

pub(super) fn to_worker(w: u32, msg: ToWorker) -> Effect {
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

pub(super) fn front(event: FrontEvent) -> Effect {
    Effect::ToFront(event)
}

fn denied(conn: u64, backoff_s: u32) -> Vec<Effect> {
    vec![front(FrontEvent::Denied { conn, backoff_s })]
}

pub(super) fn ready(conn: u64, user: &str) -> Input {
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

pub(super) fn quiet(effects: Vec<Effect>) -> Vec<Effect> {
    (effects.into_iter())
        .filter(|e| !matches!(e, Effect::Log(..)))
        .collect()
}

pub(super) fn disconnect_arm(id: &str, generation: u32, min: u32) -> Effect {
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
    pub(super) fn after(&mut self, wait: Duration) -> &mut Run {
        self.now += wait;
        self
    }

    pub(super) fn fire(&mut self, timer: Timer) -> Vec<Effect> {
        self.on(Input::Timer(timer))
    }

    /// Runs conn's TLS login for `user` through worker `w` up to `Verified`; returns what
    /// `Authenticated` gave.
    pub(super) fn verify(&mut self, conn: u64, w: u32, user: &str, uid: u32) -> Vec<Effect> {
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

    fn gone(&mut self, conn: u64) {
        assert_eq!(self.on(Input::Front(FrontRequest::Gone { conn })), []);
    }

    pub(super) fn viewers(&mut self, uid: u32, count: u32) -> Vec<Effect> {
        self.on(Input::Viewers { uid, count })
    }
}

fn logs(effects: &[Effect]) -> Vec<&str> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::Log(_, text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// A reload's start of the front while remote logins stay on at port 3389.
pub(super) fn front_on() -> Effect {
    Effect::Front(FrontUnit::Start { port: 3389 })
}

/// Anna's background session `c2`, started at the run's start for a login that then went,
/// with no client, its `Disconnected` timer armed for 30 minutes as generation 1.
fn anna_unviewed_for_30() -> Run {
    let mut run = remote_run(MachineRemote {
        end_disconnected_after_min: Some(30),
        ..enabled()
    });
    run.anna_in_background(7, 2);
    run.gone(7);
    run
}
