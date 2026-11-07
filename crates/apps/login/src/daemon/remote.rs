//! Remote logins from the RDP front: checked against the group and PAM or the stored NT hash,
//! routed to the user's session or a new background one, then handed over to its reprise-rdp.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::hint::black_box;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use log::Level;
use repriselogin::account::nt_hash;
use repriselogin::front::{FrontEvent, FrontRequest, NlaUser, Secret};
use repriselogin::greeter::{AuthKind, Note};
use repriselogin::throttle::{Attempt, Pause, Throttle};
use zeroize::Zeroizing;

use super::{
    Conversation, Daemon, Effect, Job, NO_VT, Phase, START_WAIT, SessionWorker, Timer,
    WORKER_ENDED, WorkerId, World,
};
use crate::table::SessionEntry;
use crate::vt;
use crate::worker::{FromWorker, Mode, ToWorker};

/// A refused login hears `Denied` this long after its `Authenticate`, whatever refused it.
pub const REFUSE_DELAY: Duration = Duration::from_secs(3);
/// How long after the TCP accept a login has to reach its session.
pub const HANDOVER_DEADLINE: Duration = Duration::from_secs(8);
/// Most logins in their group look-up or PAM check at once.
pub const MAX_REMOTE_CHECKS: usize = 4;
const MAX_USER_NAME: usize = 32;

const MOST_SESSIONS: &str = "the computer runs its most sessions";
const ASKED_MORE: &str = "PAM asked for more than the password";

/// A user's Remote Desktop password as the store keeps it: 32 uppercase hex characters.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredPassword {
    pub user: String,
    pub uid: u32,
    pub nt_hash: Zeroizing<String>,
}

impl fmt::Debug for StoredPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredPassword")
            .field("user", &self.user)
            .field("uid", &self.uid)
            .field("nt_hash", &format_args!("<redacted>"))
            .finish()
    }
}

/// Who asked for a look-up: a front conn, an account link or the administrator check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookUpKey {
    Conn(u64),
    Account(u64),
    Admin(WorkerId),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Inhibit {
    #[default]
    Nothing,
    Idle,
    SleepAndIdle,
}

#[derive(Default)]
pub(super) struct Remote {
    pub(super) front: bool,
    passwords: Vec<StoredPassword>,
    throttle: Throttle,
    logins: BTreeMap<u64, Login>,
    /// Background session starts in flight, by uid.
    starting: HashMap<u32, Starting>,
    /// Start workers killed at `START_WAIT`, with their VTs, until they exit.
    stalled: HashMap<WorkerId, u16>,
    /// Clients connected to each uid's session; a uid with none has no entry.
    viewers: HashMap<u32, u32>,
    /// Uids whose hand-over link is up.
    linked: HashSet<u32>,
    /// The generation of each armed `Timer::Disconnected`, by session id.
    disconnects: HashMap<String, u32>,
    generations: u32,
    inhibit: Inhibit,
}

struct Login {
    peer: IpAddr,
    /// The name the client gave; the hand-over must carry the same.
    user: String,
    /// Held until the hash compare (delegated) or PAM's prompt (password) takes it.
    secret: Option<Secret>,
    at: Instant,
    deadline: Instant,
    /// The failure counted from the start of the check, taken back once verified.
    attempt: Option<Attempt>,
    step: Step,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    LookingUp,
    /// A `Remote` worker checks the password, or a `RemoteNla` worker the account, of `uid`.
    Checking {
        worker: WorkerId,
        uid: u32,
    },
    /// Verified; waits for `uid`'s session start in flight.
    Starting {
        uid: u32,
    },
    /// Waits for `uid`'s hand-over link.
    Linking {
        uid: u32,
    },
    /// `Accepted` went to the front, which answers with `Ready`.
    Accepted {
        uid: u32,
    },
    Forwarding,
    /// `Denied` goes out once `REFUSE_DELAY` has passed.
    Refused {
        backoff_s: u32,
    },
}

struct Starting {
    worker: WorkerId,
    vt: u16,
    /// PAM's name for the account.
    user: String,
}

impl Daemon {
    /// Replaces the Remote Desktop passwords, telling a linked front.
    pub fn set_remote_passwords(&mut self, passwords: Vec<StoredPassword>) -> Vec<Effect> {
        self.remote.passwords = passwords;
        if self.remote.front {
            vec![self.passwords_event()]
        } else {
            Vec::new()
        }
    }

    fn passwords_event(&self) -> Effect {
        let users = (self.remote.passwords.iter())
            .map(|p| NlaUser {
                user: p.user.clone(),
                nt_hash: p.nt_hash.clone(),
            })
            .collect();
        Effect::ToFront(FrontEvent::Passwords { users })
    }

    pub(super) fn is_remote_worker(&self, worker: WorkerId) -> bool {
        self.checking_conn(worker).is_some()
            || self.starting_uid(worker).is_some()
            || self.remote.stalled.contains_key(&worker)
    }

    fn checking_conn(&self, worker: WorkerId) -> Option<u64> {
        (self.remote.logins.iter())
            .find(|(_, l)| matches!(l.step, Step::Checking { worker: w, .. } if w == worker))
            .map(|(&conn, _)| conn)
    }

    fn starting_uid(&self, worker: WorkerId) -> Option<u32> {
        (self.remote.starting.iter())
            .find(|(_, s)| s.worker == worker)
            .map(|(&uid, _)| uid)
    }

    /// Every VT a session holds or a start is about to take.
    pub(super) fn taken_vts(&self) -> BTreeSet<u16> {
        let mut taken = self.sessions.taken_vts();
        taken.extend(self.remote.starting.values().map(|s| s.vt));
        taken.extend(self.remote.stalled.values());
        if let Some(Conversation {
            phase: Phase::Starting { vt },
            ..
        }) = &self.conversation
        {
            taken.insert(*vt);
        }
        taken.extend(self.autologin.as_ref().map(|a| a.vt));
        taken
    }

    pub(super) fn notes(&self) -> HashMap<String, Note> {
        self.sessions
            .notes(|uid| self.remote.viewers.contains_key(&uid))
    }

    pub(super) fn front_request(
        &mut self,
        now: Instant,
        request: FrontRequest,
        out: &mut Vec<Effect>,
    ) {
        match request {
            FrontRequest::Hello => out.push(self.passwords_event()),
            FrontRequest::Authenticate { .. } => self.authenticate(now, request, out),
            FrontRequest::Gone { conn } => self.forget(conn, out),
            FrontRequest::Ready { conn, .. } => out.push(Effect::Log(
                Level::Warn,
                format!("the front's Ready for conn {conn} came without its socket"),
            )),
        }
    }

    fn authenticate(&mut self, now: Instant, request: FrontRequest, out: &mut Vec<Effect>) {
        let FrontRequest::Authenticate {
            conn,
            peer,
            user,
            secret,
            elapsed_ms,
        } = request
        else {
            return;
        };
        if self.remote.logins.contains_key(&conn) {
            return out.push(Effect::Log(
                Level::Warn,
                format!("the front sent a second login for conn {conn}; ignoring it"),
            ));
        }
        let left = HANDOVER_DEADLINE.saturating_sub(Duration::from_millis(elapsed_ms.into()));
        out.push(Effect::Arm {
            timer: Timer::Refuse(conn),
            after: REFUSE_DELAY,
        });
        out.push(Effect::Arm {
            timer: Timer::Handover(conn),
            after: left,
        });
        let checks = (self.remote.logins.values())
            .filter(|l| matches!(l.step, Step::LookingUp | Step::Checking { .. }))
            .count();
        let peer = peer.ip();
        self.remote.logins.insert(
            conn,
            Login {
                peer,
                user: user.clone(),
                secret: Some(secret),
                at: now,
                deadline: now + left,
                attempt: None,
                step: Step::LookingUp,
            },
        );
        if let Some(why) = self.gate(now, peer, checks, out) {
            return self.refuse(now, conn, why, out);
        }
        let attempt = self.remote.throttle.failed(peer, now);
        if let Some(l) = self.remote.logins.get_mut(&conn) {
            l.attempt = Some(attempt);
        }
        if !valid_user_name(&user) {
            return self.refuse(now, conn, "the name is no account name", out);
        }
        out.push(Effect::LookUp {
            key: LookUpKey::Conn(conn),
            user,
            group: self.config.remote.group.clone(),
        });
    }

    /// Why a login from `peer` is refused before any check, if it is.
    fn gate(
        &mut self,
        now: Instant,
        peer: IpAddr,
        checks: usize,
        out: &mut Vec<Effect>,
    ) -> Option<&'static str> {
        if !self.config.remote.enabled {
            return Some("remote logins are off");
        }
        if !self.remote.throttle.delay(peer, now).is_zero() {
            return Some("the address backs off after failed logins");
        }
        match self.remote.throttle.paused(now) {
            Pause::Off => {}
            Pause::Started => {
                out.push(Effect::Log(
                    Level::Warn,
                    "remote logins are paused for 10 minutes after repeated failures".into(),
                ));
                return Some("remote logins are paused");
            }
            Pause::Running => return Some("remote logins are paused"),
        }
        (checks >= MAX_REMOTE_CHECKS).then_some("too many remote logins are being checked")
    }

    pub(super) fn member(
        &mut self,
        now: Instant,
        conn: u64,
        uid: Option<u32>,
        member: bool,
        out: &mut Vec<Effect>,
    ) {
        if self.remote.logins.get(&conn).map(|l| l.step) != Some(Step::LookingUp) {
            return;
        }
        let uid = match uid {
            None => return self.refuse(now, conn, "no such account", out),
            Some(0) => return self.refuse(now, conn, "root never logs in remotely", out),
            Some(_) if !member => {
                let why = "the account is not in the remote logins' group";
                return self.refuse(now, conn, why, out);
            }
            Some(uid) => uid,
        };
        let l = self.remote.logins.get_mut(&conn).expect("login");
        let mode = match l.secret.take() {
            Some(Secret::Delegated(password)) => {
                let stored = self.remote.passwords.iter().find(|p| p.uid == uid);
                match stored {
                    None => {
                        let why = "the account has no Remote Desktop password";
                        return self.refuse(now, conn, why, out);
                    }
                    Some(p) if !same_hash(&nt_hash(&password), &p.nt_hash) => {
                        let why = "a wrong Remote Desktop password";
                        return self.refuse(now, conn, why, out);
                    }
                    Some(_) => Mode::RemoteNla,
                }
            }
            secret => {
                l.secret = secret;
                Mode::Remote
            }
        };
        let worker = self.next_worker();
        let l = self.remote.logins.get_mut(&conn).expect("login");
        l.step = Step::Checking { worker, uid };
        out.push(Effect::Spawn {
            worker,
            job: Job {
                mode,
                user: l.user.clone(),
                autologin_user: None,
                greeter_socket: false,
                rhost: Some(l.peer.to_string()),
            },
        });
    }

    pub(super) fn remote_worker_message(
        &mut self,
        now: Instant,
        world: &World,
        worker: WorkerId,
        msg: FromWorker,
        out: &mut Vec<Effect>,
    ) {
        if let Some(conn) = self.checking_conn(worker) {
            return self.check_message(now, world, conn, worker, msg, out);
        }
        if let Some(uid) = self.starting_uid(worker) {
            return self.start_message(now, uid, worker, msg, out);
        }
        if let FromWorker::Started { session_id, .. } = msg {
            out.push(Effect::Log(
                Level::Warn,
                format!(
                    "background session {session_id} opened after its start timed out; ending it"
                ),
            ));
            out.push(Effect::TerminateSession(session_id));
        }
    }

    fn check_message(
        &mut self,
        now: Instant,
        world: &World,
        conn: u64,
        worker: WorkerId,
        msg: FromWorker,
        out: &mut Vec<Effect>,
    ) {
        let l = self.remote.logins.get_mut(&conn).expect("login");
        let Step::Checking { uid, .. } = l.step else {
            return;
        };
        match msg {
            FromWorker::Prompt {
                kind: AuthKind::Info | AuthKind::Error,
                ..
            } => out.push(Effect::ToWorker {
                worker,
                msg: ToWorker::Answer(None),
            }),
            FromWorker::Prompt {
                kind: AuthKind::Secret,
                ..
            } if matches!(l.secret, Some(Secret::Password(_))) => {
                let Some(Secret::Password(password)) = l.secret.take() else {
                    unreachable!()
                };
                out.push(Effect::ToWorker {
                    worker,
                    msg: ToWorker::Answer(Some(password)),
                });
            }
            FromWorker::Prompt { .. } => {
                out.push(Effect::Kill(worker));
                self.refuse(now, conn, ASKED_MORE, out);
            }
            FromWorker::Authenticated { uid: got, .. } if got != uid => {
                out.push(Effect::Kill(worker));
                let why = format!("PAM's account {got} is not the looked-up {uid}");
                self.refuse(now, conn, &why, out);
            }
            FromWorker::Authenticated { user, .. } => self.verified(now, world, conn, user, out),
            FromWorker::Failed { text, .. } => self.refuse(now, conn, &text, out),
            FromWorker::Started { session_id, .. } => {
                out.push(Effect::TerminateSession(session_id));
                out.push(Effect::Kill(worker));
                let why = "the session started before it was asked to";
                self.refuse(now, conn, why, out);
            }
        }
    }

    /// Routes a verified login: to its user's session, to its start in flight, or to a new
    /// background session that its worker opens. Every other worker is cancelled.
    fn verified(
        &mut self,
        now: Instant,
        world: &World,
        conn: u64,
        user: String,
        out: &mut Vec<Effect>,
    ) {
        let l = self.remote.logins.get_mut(&conn).expect("login");
        let Step::Checking { worker, uid } = l.step else {
            return;
        };
        out.push(Effect::ToFront(FrontEvent::Verified { conn }));
        if let Some(attempt) = l.attempt.take() {
            self.remote.throttle.succeeded(l.peer, attempt, now);
        }
        let cancel = Effect::ToWorker {
            worker,
            msg: ToWorker::Cancel,
        };
        if self.sessions.by_uid(uid).is_some() {
            out.push(cancel);
            self.ask_link(uid, out);
            return self.link(conn, uid, out);
        }
        if self.remote.starting.contains_key(&uid) {
            out.push(cancel);
            return self.set_step(conn, Step::Starting { uid });
        }
        let most = self.config.remote.max_sessions as usize;
        if self.sessions.len() + self.remote.starting.len() >= most {
            out.push(cancel);
            return self.refuse(now, conn, MOST_SESSIONS, out);
        }
        let Some(vt) = vt::pick(&self.taken_vts(), world.kernel_open) else {
            out.push(cancel);
            return self.refuse(now, conn, NO_VT, out);
        };
        out.push(Effect::ToWorker {
            worker,
            msg: ToWorker::Start {
                vt,
                background: true,
            },
        });
        out.push(Effect::Arm {
            timer: Timer::Start(worker),
            after: START_WAIT,
        });
        (self.remote.starting).insert(uid, Starting { worker, vt, user });
        self.set_step(conn, Step::Starting { uid });
    }

    fn start_message(
        &mut self,
        now: Instant,
        uid: u32,
        worker: WorkerId,
        msg: FromWorker,
        out: &mut Vec<Effect>,
    ) {
        match msg {
            FromWorker::Prompt { .. } => out.push(Effect::ToWorker {
                worker,
                msg: ToWorker::Answer(None),
            }),
            FromWorker::Authenticated { .. } => {}
            FromWorker::Failed { text, .. } => self.start_failed(now, uid, &text, out),
            FromWorker::Started { session_id, .. } => {
                let s = self.remote.starting.remove(&uid).expect("start");
                self.session_workers
                    .push(SessionWorker { worker, vt: s.vt });
                if self.sessions.by_uid(uid).is_some() {
                    out.push(Effect::Log(
                        Level::Warn,
                        format!(
                            "{}'s background session {session_id} started beside a running \
                             one; ending it",
                            s.user
                        ),
                    ));
                    out.push(Effect::TerminateSession(session_id));
                } else {
                    self.sessions.insert(SessionEntry {
                        id: session_id.clone(),
                        user: s.user,
                        uid,
                        vt: s.vt,
                        started: Some(now),
                        background: true,
                    });
                    self.arm_disconnected(session_id, out);
                }
                self.ask_link(uid, out);
                for conn in self.conns(Step::Starting { uid }) {
                    self.link(conn, uid, out);
                }
            }
        }
    }

    fn start_failed(&mut self, now: Instant, uid: u32, why: &str, out: &mut Vec<Effect>) {
        let Some(s) = self.remote.starting.remove(&uid) else {
            return;
        };
        out.push(Effect::Log(
            Level::Warn,
            format!("{}'s background session did not start: {why}", s.user),
        ));
        for conn in self.conns(Step::Starting { uid }) {
            self.refuse(now, conn, why, out);
        }
    }

    pub(super) fn remote_worker_exited(
        &mut self,
        now: Instant,
        worker: WorkerId,
        out: &mut Vec<Effect>,
    ) {
        if let Some(conn) = self.checking_conn(worker) {
            return self.refuse(now, conn, WORKER_ENDED, out);
        }
        if let Some(uid) = self.starting_uid(worker) {
            return self.start_failed(now, uid, WORKER_ENDED, out);
        }
        if let Some(vt) = self.remote.stalled.remove(&worker) {
            out.push(Effect::Disallocate(vt));
        }
    }

    pub(super) fn remote_start_timed_out(
        &mut self,
        now: Instant,
        worker: WorkerId,
        out: &mut Vec<Effect>,
    ) {
        let Some(uid) = self.starting_uid(worker) else {
            return;
        };
        out.push(Effect::Kill(worker));
        let vt = self.remote.starting[&uid].vt;
        self.remote.stalled.insert(worker, vt);
        let why = format!("the session did not start within {START_WAIT:?}");
        self.start_failed(now, uid, &why, out);
    }

    /// Opens `uid`'s hand-over link unless it is up.
    fn ask_link(&mut self, uid: u32, out: &mut Vec<Effect>) {
        if self.config.remote.enabled && !self.remote.linked.contains(&uid) {
            out.push(Effect::OpenIntake { uid });
        }
    }

    /// Accepts `conn` once `uid`'s hand-over link is up.
    fn link(&mut self, conn: u64, uid: u32, out: &mut Vec<Effect>) {
        if self.remote.linked.contains(&uid) {
            out.push(Effect::ToFront(FrontEvent::Accepted { conn }));
            self.set_step(conn, Step::Accepted { uid });
        } else {
            self.set_step(conn, Step::Linking { uid });
        }
    }

    fn set_step(&mut self, conn: u64, step: Step) {
        if let Some(l) = self.remote.logins.get_mut(&conn) {
            l.step = step;
        }
    }

    fn conns(&self, step: Step) -> Vec<u64> {
        (self.remote.logins.iter())
            .filter(|(_, l)| l.step == step)
            .map(|(&conn, _)| conn)
            .collect()
    }

    /// Refuses `conn`'s login. `Denied` waits until `REFUSE_DELAY` after its `Authenticate`,
    /// so a refusal takes as long as a wrong password. The caller ends its worker.
    fn refuse(&mut self, now: Instant, conn: u64, why: &str, out: &mut Vec<Effect>) {
        let Some(l) = self.remote.logins.get_mut(&conn) else {
            return;
        };
        let backoff = self.remote.throttle.delay(l.peer, now);
        let backoff_s = u32::try_from(backoff.as_millis().div_ceil(1000)).unwrap_or(u32::MAX);
        l.step = Step::Refused { backoff_s };
        l.secret = None;
        out.push(Effect::Log(
            Level::Info,
            format!("a remote login from {} was refused: {why}", l.peer),
        ));
        let waited = now.saturating_duration_since(l.at);
        if waited >= REFUSE_DELAY {
            out.push(Effect::Log(
                Level::Warn,
                format!(
                    "the check of a remote login from {} was slow: {waited:?}, past the \
                     {REFUSE_DELAY:?} a refusal waits",
                    l.peer
                ),
            ));
            self.deny(conn, out);
        }
    }

    fn deny(&mut self, conn: u64, out: &mut Vec<Effect>) {
        if let Some(Login {
            step: Step::Refused { backoff_s },
            ..
        }) = self.remote.logins.remove(&conn)
        {
            out.push(Effect::ToFront(FrontEvent::Denied { conn, backoff_s }));
        }
    }

    /// Sends a refused login's `Denied`; a timer of an earlier conn with the same id waits on.
    pub(super) fn refuse_delay_over(&mut self, now: Instant, conn: u64, out: &mut Vec<Effect>) {
        let Some(l) = self.remote.logins.get(&conn) else {
            return;
        };
        if !matches!(l.step, Step::Refused { .. }) {
            return;
        }
        let left = REFUSE_DELAY.saturating_sub(now.saturating_duration_since(l.at));
        if !left.is_zero() {
            return out.push(Effect::Arm {
                timer: Timer::Refuse(conn),
                after: left,
            });
        }
        self.deny(conn, out);
    }

    /// Denies a login that has not reached its session by the deadline. A session started
    /// for it keeps running, and a reconnect lands in it.
    pub(super) fn handover_deadline(&mut self, now: Instant, conn: u64, out: &mut Vec<Effect>) {
        let Some(l) = self.remote.logins.get(&conn) else {
            return;
        };
        if l.step == Step::Forwarding {
            return;
        }
        let left = l.deadline.saturating_duration_since(now);
        if !left.is_zero() {
            return out.push(Effect::Arm {
                timer: Timer::Handover(conn),
                after: left,
            });
        }
        out.push(Effect::Log(
            Level::Info,
            format!(
                "a remote login from {} did not reach its session within {HANDOVER_DEADLINE:?}",
                l.peer
            ),
        ));
        let backoff_s = match l.step {
            Step::Refused { backoff_s } => backoff_s,
            _ => 0,
        };
        self.forget(conn, out);
        out.push(Effect::ToFront(FrontEvent::Denied { conn, backoff_s }));
        out.push(Effect::DropConn { conn });
    }

    /// Drops `conn`'s login, killing a worker still checking it.
    fn forget(&mut self, conn: u64, out: &mut Vec<Effect>) {
        if let Some(Login {
            step: Step::Checking { worker, .. },
            ..
        }) = self.remote.logins.remove(&conn)
        {
            out.push(Effect::Kill(worker));
        }
    }

    /// A front that went away takes its conns along; session starts in flight go on.
    pub(super) fn front_lost(&mut self, out: &mut Vec<Effect>) {
        self.remote.front = false;
        let conns: Vec<_> = self.remote.logins.keys().copied().collect();
        for conn in conns {
            self.forget(conn, out);
        }
    }

    pub(super) fn front_ready(&mut self, conn: u64, user: &str, out: &mut Vec<Effect>) {
        match self.remote.logins.get_mut(&conn) {
            Some(l) if l.user == user && matches!(l.step, Step::Accepted { .. }) => {
                let Step::Accepted { uid } = l.step else {
                    unreachable!()
                };
                l.step = Step::Forwarding;
                out.push(Effect::Forward { conn, uid });
            }
            _ => {
                out.push(Effect::Log(
                    Level::Warn,
                    format!("dropping conn {conn}: its Ready matches no accepted login"),
                ));
                self.forget(conn, out);
                out.push(Effect::DropConn { conn });
            }
        }
    }

    pub(super) fn forwarded(&mut self, conn: u64, ok: bool, out: &mut Vec<Effect>) {
        let Some(Login {
            step: Step::Forwarding,
            ..
        }) = self.remote.logins.get(&conn)
        else {
            return;
        };
        let l = self.remote.logins.remove(&conn).expect("login");
        if !ok {
            out.push(Effect::Log(
                Level::Warn,
                format!(
                    "the remote login from {} could not be handed to its session",
                    l.peer
                ),
            ));
        }
    }

    pub(super) fn intake_open(&mut self, uid: u32, out: &mut Vec<Effect>) {
        self.remote.linked.insert(uid);
        for conn in self.conns(Step::Linking { uid }) {
            self.link(conn, uid, out);
        }
    }

    /// A lost link counts as no viewers; the next login for `uid` opens it again.
    pub(super) fn intake_lost(&mut self, uid: u32, out: &mut Vec<Effect>) {
        self.remote.linked.remove(&uid);
        self.set_viewers(uid, 0, out);
    }

    /// Opens the hand-over link of every session in the table while remote logins are on.
    pub(super) fn open_intakes(&mut self, out: &mut Vec<Effect>) {
        let uids: Vec<_> = self.sessions.iter().map(|e| e.uid).collect();
        for uid in uids {
            self.ask_link(uid, out);
        }
    }

    /// A background session whose last viewer leaves ends `end_disconnected_after` later.
    pub(super) fn set_viewers(&mut self, uid: u32, count: u32, out: &mut Vec<Effect>) {
        let before = match count {
            0 => self.remote.viewers.remove(&uid),
            _ => self.remote.viewers.insert(uid, count),
        };
        let Some(e) = self.sessions.by_uid(uid) else {
            return;
        };
        if count > 0 {
            self.remote.disconnects.remove(&e.id);
            return;
        }
        if before.is_some() && e.background {
            let id = e.id.clone();
            self.arm_disconnected(id, out);
        }
    }

    /// Arms background session `id`'s `Disconnected` timer with a new generation, which
    /// stales any earlier one: when the session starts, is found or loses its last viewer.
    pub(super) fn arm_disconnected(&mut self, id: String, out: &mut Vec<Effect>) {
        let Some(min) = self.config.remote.end_disconnected_after_min else {
            return;
        };
        self.remote.generations += 1;
        let generation = self.remote.generations;
        (self.remote.disconnects).insert(id.clone(), generation);
        out.push(Effect::Arm {
            timer: Timer::Disconnected { id, generation },
            after: Duration::from_secs(u64::from(min) * 60),
        });
    }

    pub(super) fn disconnected(&mut self, id: &str, generation: u32, out: &mut Vec<Effect>) {
        if self.remote.disconnects.get(id) != Some(&generation) {
            return;
        }
        self.remote.disconnects.remove(id);
        let Some(e) = self.sessions.by_id(id) else {
            return;
        };
        if !e.background || self.remote.viewers.contains_key(&e.uid) {
            return;
        }
        out.push(Effect::Log(
            Level::Info,
            format!(
                "ending {}'s background session {id}: no client is connected",
                e.user
            ),
        ));
        out.push(Effect::TerminateSession(id.into()));
    }

    pub(super) fn remote_session_gone(&mut self, entry: &SessionEntry) {
        self.remote.viewers.remove(&entry.uid);
        self.remote.linked.remove(&entry.uid);
        self.remote.disconnects.remove(&entry.id);
    }

    /// Emits the inhibitor when the viewers or the background sessions change it.
    pub(super) fn update_inhibit(&mut self, out: &mut Vec<Effect>) {
        let want = if !self.remote.viewers.is_empty() {
            Inhibit::SleepAndIdle
        } else if self.sessions.iter().any(|e| e.background) {
            Inhibit::Idle
        } else {
            Inhibit::Nothing
        };
        if want != self.remote.inhibit {
            self.remote.inhibit = want;
            out.push(Effect::Inhibit(want));
        }
    }
}

/// A name `useradd` accepts: at most 32 bytes of `[a-z_][a-z0-9_.-]*`.
fn valid_user_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= MAX_USER_NAME
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
}

/// Compares two hashes in a time that depends on their length only.
fn same_hash(a: &str, b: &str) -> bool {
    let (a, b) = (black_box(a.as_bytes()), black_box(b.as_bytes()));
    a.len() == b.len() && black_box(a.iter().zip(b).fold(0, |d, (x, y)| d | (x ^ y))) == 0
}
