//! Remote logins from the RDP front: checked against the group and PAM or the stored NT hash,
//! routed to the user's session or a new background one, then handed over to its reprise-rdp.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use log::Level;
use repriselogin::account::{nt_hash, same_hash};
use repriselogin::front::{FrontEvent, FrontRequest, NlaUser, Secret};
use repriselogin::greeter::{AuthKind, Note};
use repriselogin::machine::MachineConfig;
use repriselogin::throttle::{Attempt, Pause, Throttle};
use zeroize::Zeroizing;

use super::account::Accounts;
use super::{
    Conversation, Daemon, Effect, Job, LogindSession, NO_VT, Phase, SESSION_ENDED, START_WAIT,
    SessionWorker, Timer, Unread, WORKER_ENDED, WorkerId, World, table_entry,
};
use crate::table::SessionEntry;
use crate::vt;
use crate::worker::{FromWorker, Mode, ToWorker};

/// A refused login hears `Denied` this long after its `Authenticate`, whatever refused it.
pub const REFUSE_DELAY: Duration = Duration::from_secs(3);
/// How long after its `Authenticate` a login has to reach its session.
pub const HANDOVER_DEADLINE: Duration = Duration::from_secs(8);
/// Most logins in their group look-up or PAM check at once.
pub const MAX_REMOTE_CHECKS: usize = 4;
/// Logins held at once. The front holds at most 32 connections before their verdict and 32
/// for their hand-over, so one past this misbehaves.
pub const MAX_LOGINS: usize = 128;
const MAX_USER_NAME: usize = 32;
/// The first wait before the front's list is looked up again after a look-up failed; it
/// doubles with each failed round up to `RELIST_RETRY_MOST`.
const RELIST_RETRY: Duration = Duration::from_secs(30);
const RELIST_RETRY_MOST: Duration = Duration::from_secs(300);

const MOST_SESSIONS: &str = "the computer runs its most sessions";
pub(super) const ASKED_MORE: &str = "PAM asked for more than the password";
pub(super) const NEVER_ASKED: &str = "PAM passed the login without asking for its password";

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

/// Who asked for a look-up: a front conn, an account link, the administrator check, the
/// hand-over link of a session's uid, or the front's list of NLA users in its `generation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookUpKey {
    Conn(u64),
    Account(u64),
    Admin(WorkerId),
    Link(u32),
    Listed { uid: u32, generation: u32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Inhibit {
    #[default]
    Nothing,
    Idle,
    SleepAndIdle,
}

/// What the backend does with `reprise-login-rdp.service` and the socket holding its port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontUnit {
    /// Holds `port` from boot on, moving the socket and its front when the port changed, and
    /// starts the front.
    Start { port: u16 },
    /// Stops the front and frees the port, from boot on too.
    Stop,
}

/// The rounds of group look-ups behind `listed`: one runs at a time, and any number of requests
/// during it run one more round after it.
#[derive(Default)]
struct Listing {
    /// The current round's; answers from other rounds are dropped.
    generation: u32,
    /// Stored uids whose answer in the current round is still out.
    awaiting: HashSet<u32>,
    again: bool,
    /// A look-up of the current round failed.
    failed: bool,
    /// The next retry's wait; zero until a round fails.
    backoff: Duration,
    retrying: bool,
    /// The front has said Hello on its link.
    greeted: bool,
}

#[derive(Default)]
pub(super) struct Remote {
    pub(super) front: bool,
    pub(super) passwords: Vec<StoredPassword>,
    /// Uids of stored passwords whose users the last look-up found in the group; the front gets
    /// only theirs.
    pub(super) listed: HashSet<u32>,
    listing: Listing,
    pub(super) accounts: Accounts,
    tls_throttle: Throttle,
    /// Failed delegated NLA passwords, which the front has already proved.
    nla_throttle: Throttle,
    logins: BTreeMap<u64, Login>,
    /// Background session starts in flight, by uid.
    starting: HashMap<u32, Starting>,
    /// Start workers killed at `START_WAIT`, with their VTs, until they exit.
    stalled: HashMap<WorkerId, u16>,
    /// Clients connected to each uid's session; a uid with none has no entry.
    pub(super) viewers: HashMap<u32, u32>,
    /// Uids whose hand-over link is up.
    linked: HashSet<u32>,
    /// Uids whose lost link is reopening, until the new link first reports.
    reopening: HashSet<u32>,
    /// Background sessions with no viewer, by session id.
    pub(super) disconnects: HashMap<String, Unviewed>,
    /// Sessions logind could not describe at recovery, by id, with their uids.
    pub(super) unread: HashMap<String, u32>,
    generations: u32,
    inhibit: Inhibit,
}

impl Remote {
    fn throttle_mut(&mut self, nla: bool) -> &mut Throttle {
        match nla {
            true => &mut self.nla_throttle,
            false => &mut self.tls_throttle,
        }
    }
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
    /// Delegated under NLA, so counted by `nla_throttle`.
    nla: bool,
    /// `Denied`'s back-off as the address stood after this `Authenticate`, so it is the
    /// same whatever refuses the login and whenever.
    backoff_s: u32,
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
    /// The backend passes the conn to `uid`'s session, answering with `Forwarded`.
    Forwarding {
        uid: u32,
    },
    /// `Denied` goes out once `REFUSE_DELAY` has passed.
    Refused,
}

impl Step {
    /// The uid whose session the login is headed for, once known.
    fn uid(self) -> Option<u32> {
        match self {
            Step::Checking { uid, .. }
            | Step::Starting { uid }
            | Step::Linking { uid }
            | Step::Accepted { uid }
            | Step::Forwarding { uid } => Some(uid),
            Step::LookingUp | Step::Refused => None,
        }
    }
}

pub(super) struct Unviewed {
    /// When the last viewer left, or the session started or was found without one.
    since: Instant,
    /// The armed `Timer::Disconnected`'s; an older one is stale.
    generation: Option<u32>,
}

struct Starting {
    worker: WorkerId,
    vt: u16,
    /// PAM's name for the account.
    user: String,
}

impl Daemon {
    /// Keeps the seat0 sessions logind listed at recovery but could not describe, returning them
    /// to be read again. Until each reads or is removed, a remote login for its uid is refused.
    pub fn recover_unread(&mut self, unread: Vec<Unread>) -> Vec<Unread> {
        let kept: Vec<Unread> = unread.into_iter().filter(|u| u.seat == "seat0").collect();
        self.remote.unread = kept.iter().map(|u| (u.id.clone(), u.uid)).collect();
        kept
    }

    /// A session left unread at recovery joins the table as recovery would have taken it.
    pub(super) fn session_read(
        &mut self,
        s: LogindSession,
        seat_active: Option<&str>,
        out: &mut Vec<Effect>,
    ) {
        if self.remote.unread.remove(&s.id).is_none() || s.closing {
            return;
        }
        let Some(entry) = table_entry(&s, seat_active) else {
            return;
        };
        let (uid, user, background) = (entry.uid, entry.user.clone(), entry.background);
        self.sessions.insert(entry);
        if !self.remote.linked.contains(&uid) {
            self.open_intake(uid, user, background, out);
        }
        self.update_inhibit(out);
    }

    /// Replaces the Remote Desktop passwords, telling a linked front.
    pub fn set_remote_passwords(&mut self, passwords: Vec<StoredPassword>) -> Vec<Effect> {
        self.remote
            .listed
            .retain(|&uid| passwords.iter().any(|p| p.uid == uid));
        self.remote.passwords = passwords;
        if self.remote.front {
            vec![self.passwords_event()]
        } else {
            Vec::new()
        }
    }

    /// The NLA users: the stored passwords of the users in the group, never root's.
    fn passwords_event(&self) -> Effect {
        let users = (self.remote.passwords.iter())
            .filter(|p| p.uid != 0 && self.remote.listed.contains(&p.uid))
            .map(|p| NlaUser {
                user: p.user.clone(),
                nt_hash: p.nt_hash.clone(),
            })
            .collect();
        Effect::ToFront(FrontEvent::Passwords { users })
    }

    /// Looks every stored password's user up in the group again, for a linked front, or once
    /// more after the round in flight.
    pub(super) fn look_up_listed(&mut self, out: &mut Vec<Effect>) {
        if !self.remote.front || !self.config.remote.enabled {
            return;
        }
        let l = &mut self.remote.listing;
        if !l.awaiting.is_empty() {
            l.again = true;
            return;
        }
        l.generation = l.generation.wrapping_add(1);
        l.failed = false;
        for p in self.remote.passwords.iter().filter(|p| p.uid != 0) {
            l.awaiting.insert(p.uid);
            out.push(Effect::LookUp {
                key: LookUpKey::Listed {
                    uid: p.uid,
                    generation: l.generation,
                },
                user: p.user.clone(),
                group: self.config.remote.group.clone(),
            });
        }
    }

    pub(super) fn listed_member(
        &mut self,
        uid: u32,
        generation: u32,
        found: Option<u32>,
        member: bool,
        failed: bool,
        out: &mut Vec<Effect>,
    ) {
        let l = &mut self.remote.listing;
        if generation != l.generation || !l.awaiting.remove(&uid) {
            return;
        }
        l.failed |= failed;
        if let Some(p) = self.remote.passwords.iter().find(|p| p.uid == uid) {
            let (user, group) = (&p.user, &self.config.remote.group);
            let listed = uid != 0 && found == Some(uid) && member;
            let changed = match listed {
                true => self.remote.listed.insert(uid),
                false => self.remote.listed.remove(&uid),
            };
            if changed || failed {
                let why = match found {
                    _ if listed => format!("{user} is in {group}"),
                    _ if failed => format!("{user}'s group look-up failed"),
                    Some(f) if f == uid => format!("{user} is not in {group}"),
                    _ => format!("{user}'s account is gone or has another uid"),
                };
                let (level, what) = match listed {
                    true => (Level::Info, "takes"),
                    false => (Level::Warn, "leaves out"),
                };
                let text = format!("{why}; the front {what} their Remote Desktop password");
                out.push(Effect::Log(level, text));
            }
            if changed && self.remote.front {
                out.push(self.passwords_event());
            }
        }
        if self.remote.listing.awaiting.is_empty() {
            self.listing_answered(out);
        }
    }

    /// Runs the round asked for during the last, or arms a retry after a failed look-up.
    fn listing_answered(&mut self, out: &mut Vec<Effect>) {
        let l = &mut self.remote.listing;
        if std::mem::take(&mut l.again) {
            return self.look_up_listed(out);
        }
        if !l.failed {
            l.backoff = Duration::ZERO;
            return;
        }
        if l.retrying {
            return;
        }
        let after = l.backoff.max(RELIST_RETRY);
        l.backoff = (after * 2).min(RELIST_RETRY_MOST);
        l.retrying = true;
        out.push(Effect::Arm {
            timer: Timer::Relist,
            after,
        });
    }

    pub(super) fn relist(&mut self, out: &mut Vec<Effect>) {
        let l = &mut self.remote.listing;
        l.retrying = false;
        if l.failed && l.awaiting.is_empty() {
            self.look_up_listed(out);
        }
    }

    /// Looks the NLA users up again when a look-up of `user` disagrees with the front's list; a
    /// failed one says nothing either way.
    fn check_listed(
        &mut self,
        user: &str,
        found: Option<u32>,
        member: bool,
        failed: bool,
        out: &mut Vec<Effect>,
    ) {
        let Some(p) = self.remote.passwords.iter().find(|p| p.user == user) else {
            return;
        };
        let member = p.uid != 0 && found == Some(p.uid) && member;
        if !failed && member != self.remote.listed.contains(&p.uid) {
            self.look_up_listed(out);
        }
    }

    pub(super) fn front_linked(&mut self) {
        self.remote.front = true;
        self.remote.listing.greeted = false;
    }

    /// On starts the front and links background and members' sessions; off stops it and denies
    /// logins not handed over, keeping the links of uids with a session. Every reload while on
    /// starts it again, and every reload while off stops it again.
    pub(super) fn config_reloaded(
        &mut self,
        now: Instant,
        config: MachineConfig,
        out: &mut Vec<Effect>,
    ) {
        let was = self.config.remote.enabled;
        let ending = self.config.remote.end_disconnected_after_min;
        let regrouped = self.config.remote.group != config.remote.group;
        self.config = config;
        if self.config.remote.end_disconnected_after_min != ending {
            let ids: BTreeSet<String> = self.remote.disconnects.keys().cloned().collect();
            for id in ids {
                self.arm_unviewed(now, &id, out);
            }
        }
        match (was, self.config.remote.enabled) {
            (false, true) => {
                out.push(self.front_unit());
                self.open_intakes(out);
            }
            (true, false) => {
                out.push(Effect::Front(FrontUnit::Stop));
                let pending: Vec<_> = (self.remote.logins.iter())
                    .filter(|(_, l)| !matches!(l.step, Step::Forwarding { .. }))
                    .map(|(&conn, l)| (conn, l.backoff_s))
                    .collect();
                for (conn, backoff_s) in pending {
                    self.forget(conn, out);
                    out.push(Effect::ToFront(FrontEvent::Denied { conn, backoff_s }));
                }
                let keep: BTreeSet<u32> = self.sessions.iter().map(|e| e.uid).collect();
                self.remote.linked.retain(|uid| keep.contains(uid));
                out.push(Effect::CloseIntakes { keep });
            }
            // Moves the port, or retries a port another program held at the last start.
            (true, true) => out.push(self.front_unit()),
            // Frees a port whose socket was started by hand.
            (false, false) => out.push(Effect::Front(FrontUnit::Stop)),
        }
        if regrouped {
            let l = &mut self.remote.listing;
            l.awaiting.clear();
            l.again = false;
            if !self.remote.listed.is_empty() {
                self.remote.listed.clear();
                if self.remote.front {
                    out.push(self.passwords_event());
                }
            }
        }
        self.look_up_listed(out);
    }

    /// The front's start or stop as the settings stand.
    pub(super) fn front_unit(&self) -> Effect {
        Effect::Front(match self.config.remote.enabled {
            true => FrontUnit::Start {
                port: self.config.remote.port,
            },
            false => FrontUnit::Stop,
        })
    }

    pub(super) fn is_remote_worker(&self, worker: WorkerId) -> bool {
        self.checking_conn(worker).is_some()
            || self.starting_uid(worker).is_some()
            || self.remote.stalled.contains_key(&worker)
    }

    pub(super) fn remote_start_in_flight(&self) -> bool {
        !self.remote.starting.is_empty()
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
            // Restarting reprise-login restarts its front (PartOf) whatever the settings say.
            FrontRequest::Hello if !self.config.remote.enabled => {
                out.push(Effect::Front(FrontUnit::Stop));
            }
            FrontRequest::Hello => {
                out.push(self.passwords_event());
                if std::mem::replace(&mut self.remote.listing.greeted, true) {
                    out.push(Effect::Log(
                        Level::Warn,
                        "the front said Hello again on its link; reading the group once a link"
                            .into(),
                    ));
                } else {
                    self.look_up_listed(out);
                }
            }
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
        if self.remote.logins.len() >= MAX_LOGINS {
            return out.push(Effect::Log(
                Level::Warn,
                format!("the front holds {MAX_LOGINS} logins; ignoring conn {conn}'s"),
            ));
        }
        out.push(Effect::Arm {
            timer: Timer::Refuse(conn),
            after: REFUSE_DELAY,
        });
        out.push(Effect::Arm {
            timer: Timer::Handover(conn),
            after: HANDOVER_DEADLINE,
        });
        let checks = (self.remote.logins.values())
            .filter(|l| matches!(l.step, Step::LookingUp | Step::Checking { .. }))
            .count();
        let peer = peer.ip();
        let nla = matches!(secret, Secret::Delegated(_));
        self.remote.logins.insert(
            conn,
            Login {
                peer,
                user: user.clone(),
                secret: Some(secret),
                at: now,
                deadline: now + HANDOVER_DEADLINE,
                attempt: None,
                nla,
                backoff_s: 0,
                step: Step::LookingUp,
            },
        );
        let gate = self.gate(now, peer, nla, checks, out);
        let throttle = self.remote.throttle_mut(nla);
        let attempt = gate.is_none().then(|| throttle.failed(peer, now));
        let backoff = throttle.delay(peer, now);
        if let Some(l) = self.remote.logins.get_mut(&conn) {
            l.attempt = attempt;
            l.backoff_s = u32::try_from(backoff.as_millis().div_ceil(1000)).unwrap_or(u32::MAX);
        }
        if let Some(why) = gate {
            return self.refuse(now, conn, why, out);
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

    /// Why a login from `peer` is refused before any check, if it is. Each kind of login (`nla`
    /// or TLS) backs off and pauses on its own failures.
    fn gate(
        &mut self,
        now: Instant,
        peer: IpAddr,
        nla: bool,
        checks: usize,
        out: &mut Vec<Effect>,
    ) -> Option<&'static str> {
        if !self.config.remote.enabled {
            return Some("remote logins are off");
        }
        let throttle = self.remote.throttle_mut(nla);
        if !throttle.delay(peer, now).is_zero() {
            return Some(match nla {
                true => "the address backs off after failed NLA logins",
                false => "the address backs off after failed TLS logins",
            });
        }
        let paused = match nla {
            true => "NLA logins are paused",
            false => "TLS logins are paused",
        };
        match throttle.paused(now) {
            Pause::Off => {}
            Pause::Started => {
                out.push(Effect::Log(
                    Level::Warn,
                    format!("{paused} for 10 minutes after repeated failures"),
                ));
                return Some(paused);
            }
            Pause::Running => return Some(paused),
        }
        (checks >= MAX_REMOTE_CHECKS).then_some("too many remote logins are being checked")
    }

    pub(super) fn member(
        &mut self,
        now: Instant,
        conn: u64,
        uid: Option<u32>,
        member: bool,
        failed: bool,
        out: &mut Vec<Effect>,
    ) {
        let Some(l) = self.remote.logins.get(&conn) else {
            return;
        };
        if l.step != Step::LookingUp {
            return;
        }
        let user = l.user.clone();
        self.check_listed(&user, uid, member, failed, out);
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
                let stored =
                    (self.remote.passwords.iter()).find(|p| p.uid == uid && p.user == l.user);
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
            FromWorker::Authenticated { .. } if matches!(l.secret, Some(Secret::Password(_))) => {
                out.push(Effect::Kill(worker));
                self.refuse(now, conn, NEVER_ASKED, out);
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
    /// background session that its worker opens. Every other worker is cancelled. A background
    /// session with no client counted restarts its disconnect time.
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
            let throttle = match l.nla {
                true => &mut self.remote.nla_throttle,
                false => &mut self.remote.tls_throttle,
            };
            throttle.succeeded(l.peer, attempt, now);
            l.backoff_s = 0;
        }
        let cancel = Effect::ToWorker {
            worker,
            msg: ToWorker::Cancel,
        };
        if let Some(e) = self.sessions.by_uid(uid) {
            out.push(cancel);
            if e.background && !self.remote.viewers.contains_key(&uid) {
                let id = e.id.clone();
                self.arm_disconnected(now, id, out);
            }
            self.ask_link(uid, out);
            return self.link(conn, uid, out);
        }
        if self.remote.starting.contains_key(&uid) {
            out.push(cancel);
            return self.set_step(conn, Step::Starting { uid });
        }
        if let Some(id) = (self.remote.unread.iter()).find_map(|(id, &u)| (u == uid).then_some(id))
        {
            let why = format!("logind has not described the user's running session {id}");
            out.push(cancel);
            return self.refuse_at(Level::Warn, now, conn, &why, out);
        }
        let most = self.config.remote.max_sessions as usize;
        if self.sessions.len() + self.remote.starting.len() >= most {
            out.push(cancel);
            return self.refuse(now, conn, MOST_SESSIONS, out);
        }
        let Some(vt) = vt::pick(&self.taken_vts(), world.kernel_open, world.active_vt) else {
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
                if let Some(at) = (self.removed_early.iter()).position(|(r, _)| *r == session_id) {
                    self.removed_early.remove(at);
                    out.push(Effect::Log(
                        Level::Warn,
                        format!(
                            "{}'s background session {session_id} ended as it started",
                            s.user
                        ),
                    ));
                    for conn in self.conns(Step::Starting { uid }) {
                        self.refuse(now, conn, SESSION_ENDED, out);
                    }
                    return;
                }
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
                        interface: None,
                    });
                    self.arm_disconnected(now, session_id, out);
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

    /// Opens `uid`'s hand-over link while remote logins are on, unless it is up.
    fn ask_link(&mut self, uid: u32, out: &mut Vec<Effect>) {
        if self.config.remote.enabled {
            self.open_link(uid, out);
        }
    }

    fn open_link(&mut self, uid: u32, out: &mut Vec<Effect>) {
        if !self.remote.linked.contains(&uid) {
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
        self.refuse_at(Level::Info, now, conn, why, out);
    }

    /// [`Self::refuse`], logged at `level`.
    fn refuse_at(
        &mut self,
        level: Level,
        now: Instant,
        conn: u64,
        why: &str,
        out: &mut Vec<Effect>,
    ) {
        let Some(l) = self.remote.logins.get_mut(&conn) else {
            return;
        };
        let checking = matches!(l.step, Step::LookingUp | Step::Checking { .. });
        l.step = Step::Refused;
        l.secret = None;
        out.push(Effect::Log(
            level,
            format!("a remote login from {} was refused: {why}", l.peer),
        ));
        let waited = now.saturating_duration_since(l.at);
        if waited < REFUSE_DELAY {
            return;
        }
        if checking {
            out.push(Effect::Log(
                Level::Warn,
                format!(
                    "the check of a remote login from {} was slow: {waited:?}, past the \
                     {REFUSE_DELAY:?} a refusal waits",
                    l.peer
                ),
            ));
        }
        self.deny(conn, out);
    }

    fn deny(&mut self, conn: u64, out: &mut Vec<Effect>) {
        if let Some(Login {
            step: Step::Refused,
            backoff_s,
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
        if l.step != Step::Refused {
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

    /// Denies a login not in its session by the deadline; a session started for it keeps running for
    /// a reconnect. An unconfirmed forward is dropped, as the front gave its socket away already.
    pub(super) fn handover_deadline(&mut self, now: Instant, conn: u64, out: &mut Vec<Effect>) {
        let Some(l) = self.remote.logins.get(&conn) else {
            return;
        };
        let left = l.deadline.saturating_duration_since(now);
        if !left.is_zero() {
            return out.push(Effect::Arm {
                timer: Timer::Handover(conn),
                after: left,
            });
        }
        if let Step::Forwarding { .. } = l.step {
            out.push(Effect::Log(
                Level::Warn,
                format!(
                    "the remote login from {} was not handed to its session within \
                     {HANDOVER_DEADLINE:?}",
                    l.peer
                ),
            ));
            self.remote.logins.remove(&conn);
            return out.push(Effect::DropConn { conn });
        }
        out.push(Effect::Log(
            Level::Info,
            format!(
                "a remote login from {} did not reach its session within {HANDOVER_DEADLINE:?}",
                l.peer
            ),
        ));
        let backoff_s = l.backoff_s;
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

    /// A front that went away takes its conns along, hand-overs in flight included; session
    /// starts in flight go on.
    pub(super) fn front_lost(&mut self, out: &mut Vec<Effect>) {
        self.remote.front = false;
        self.remote.listing.greeted = false;
        let conns: Vec<_> = self.remote.logins.keys().copied().collect();
        for conn in conns {
            if let Step::Forwarding { .. } = self.remote.logins[&conn].step {
                out.push(Effect::DropConn { conn });
            }
            self.forget(conn, out);
        }
    }

    /// Forwards an accepted login to its session; one whose session ended since is denied, so the
    /// intake never opens a link for a uid the daemon no longer tracks.
    pub(super) fn front_ready(&mut self, conn: u64, user: &str, out: &mut Vec<Effect>) {
        match self.remote.logins.get_mut(&conn) {
            Some(l) if l.user == user && matches!(l.step, Step::Accepted { .. }) => {
                let Step::Accepted { uid } = l.step else {
                    unreachable!()
                };
                if self.sessions.by_uid(uid).is_none() {
                    out.push(Effect::Log(
                        Level::Info,
                        format!(
                            "a remote login from {} lost its session before its hand-over",
                            l.peer
                        ),
                    ));
                    let backoff_s = l.backoff_s;
                    self.forget(conn, out);
                    out.push(Effect::ToFront(FrontEvent::Denied { conn, backoff_s }));
                    return out.push(Effect::DropConn { conn });
                }
                l.step = Step::Forwarding { uid };
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

    /// `world` is read fresh: a session the console does not show becomes a background one.
    pub(super) fn forwarded(
        &mut self,
        now: Instant,
        world: &World,
        conn: u64,
        ok: bool,
        out: &mut Vec<Effect>,
    ) {
        let Some(&Login {
            step: Step::Forwarding { uid },
            ..
        }) = self.remote.logins.get(&conn)
        else {
            return;
        };
        let l = self.remote.logins.remove(&conn).expect("login");
        if ok {
            return self.reached_remotely(now, world, uid, out);
        }
        self.remote.linked.remove(&uid);
        out.push(Effect::Log(
            Level::Warn,
            format!(
                "the remote login from {} could not be handed to its session",
                l.peer
            ),
        ));
    }

    /// A link that opens after its session ended stays unmarked, so the next session's first
    /// login opens it again.
    pub(super) fn intake_open(&mut self, uid: u32, out: &mut Vec<Effect>) {
        if self.sessions.by_uid(uid).is_none() {
            return;
        }
        self.remote.linked.insert(uid);
        for conn in self.conns(Step::Linking { uid }) {
            self.link(conn, uid, out);
        }
    }

    /// A lost link with viewers reopens once, keeping its count meanwhile, at a login's pace if
    /// one waits. Any other loss zeroes the count and refuses the logins waiting on it; the next
    /// login for `uid` reopens it.
    pub(super) fn intake_lost(&mut self, now: Instant, uid: u32, out: &mut Vec<Effect>) {
        let reported = self.remote.linked.remove(&uid);
        let viewed = self.remote.viewers.contains_key(&uid);
        if viewed && self.sessions.by_uid(uid).is_some() && self.remote.reopening.insert(uid) {
            out.push(Effect::ReopenIntake { uid });
            if !self.conns(Step::Linking { uid }).is_empty() {
                out.push(Effect::OpenIntake { uid });
            }
            return;
        }
        self.remote.reopening.remove(&uid);
        self.set_viewers(now, uid, 0, out);
        if !reported
            && let Some(e) = self.sessions.by_uid(uid)
            && e.background
            && !self.remote.disconnects.contains_key(&e.id)
        {
            let id = e.id.clone();
            out.push(Effect::Log(
                Level::Warn,
                format!(
                    "background session {id}'s hand-over server never reported; counting no client"
                ),
            ));
            self.arm_disconnected(now, id, out);
        }
        for conn in self.conns(Step::Linking { uid }) {
            self.refuse(now, conn, "its session's hand-over link did not open", out);
        }
    }

    /// Links every background session, whose count alone starts its time; looks up the group
    /// for the others while remote logins are on.
    pub(super) fn open_intakes(&mut self, out: &mut Vec<Effect>) {
        let users: BTreeMap<u32, (String, bool)> = (self.sessions.iter())
            .filter(|e| !self.remote.linked.contains(&e.uid))
            .map(|e| (e.uid, (e.user.clone(), e.background)))
            .collect();
        for (uid, (user, background)) in users {
            self.open_intake(uid, user, background, out);
        }
    }

    /// Links `uid`'s background session, or looks up its user for `link_member`.
    fn open_intake(&mut self, uid: u32, user: String, background: bool, out: &mut Vec<Effect>) {
        if background {
            self.open_link(uid, out);
        } else if self.config.remote.enabled {
            out.push(Effect::LookUp {
                key: LookUpKey::Link(uid),
                user,
                group: self.config.remote.group.clone(),
            });
        }
    }

    /// Opens `uid`'s link while its session is there, unless the look-up says its user is not in
    /// the group. A failed look-up opens it: a lost viewer count can end a session with a client.
    pub(super) fn link_member(
        &mut self,
        uid: u32,
        found: Option<u32>,
        member: bool,
        failed: bool,
        out: &mut Vec<Effect>,
    ) {
        let outside = uid == 0 || found.is_some_and(|f| f != uid || !member);
        let Some(user) = self.sessions.by_uid(uid).map(|e| e.user.clone()) else {
            return;
        };
        self.check_listed(&user, found, member, failed, out);
        if !outside {
            self.ask_link(uid, out);
        }
    }

    /// A link's count of its session's clients; the first ends a reopen. A count of none starts
    /// a background session's time when none runs, as for one found at recovery.
    pub(super) fn viewers_reported(
        &mut self,
        now: Instant,
        uid: u32,
        count: u32,
        out: &mut Vec<Effect>,
    ) {
        self.remote.reopening.remove(&uid);
        self.set_viewers(now, uid, count, out);
        let Some(e) = self.sessions.by_uid(uid) else {
            return;
        };
        if count == 0 && e.background && !self.remote.disconnects.contains_key(&e.id) {
            let id = e.id.clone();
            self.arm_disconnected(now, id, out);
        }
    }

    /// A background session whose last viewer leaves ends `end_disconnected_after` later.
    /// A count for a uid with no session is ignored.
    pub(super) fn set_viewers(
        &mut self,
        now: Instant,
        uid: u32,
        count: u32,
        out: &mut Vec<Effect>,
    ) {
        let Some(e) = self.sessions.by_uid(uid) else {
            return;
        };
        let before = match count {
            0 => self.remote.viewers.remove(&uid),
            _ => self.remote.viewers.insert(uid, count),
        };
        if count > 0 {
            self.remote.disconnects.remove(&e.id);
            return;
        }
        if before.is_some() && e.background {
            let id = e.id.clone();
            self.arm_disconnected(now, id, out);
        }
    }

    /// Records background session `id` as unviewed from `now` and arms its `Disconnected`
    /// timer: when the session starts, is reached remotely, loses its last viewer, or is first
    /// counted with none.
    pub(super) fn arm_disconnected(&mut self, now: Instant, id: String, out: &mut Vec<Effect>) {
        let unviewed = Unviewed {
            since: now,
            generation: None,
        };
        self.remote.disconnects.insert(id.clone(), unviewed);
        self.arm_unviewed(now, &id, out);
    }

    /// Arms `id`'s `Disconnected` timer for `end_disconnected_after` past its `since` with a
    /// new generation, which stales any earlier one; ends it at once if that has passed.
    fn arm_unviewed(&mut self, now: Instant, id: &str, out: &mut Vec<Effect>) {
        let min = self.config.remote.end_disconnected_after_min;
        let Some(u) = self.remote.disconnects.get_mut(id) else {
            return;
        };
        u.generation = None;
        let Some(min) = min else {
            return;
        };
        let end = u.since + Duration::from_secs(u64::from(min) * 60);
        let after = end.saturating_duration_since(now);
        if after.is_zero() {
            return self.end_unviewed(id, out);
        }
        self.arm_after(id, after, out);
    }

    /// Arms `id`'s `Disconnected` timer `after` from now with a new generation.
    fn arm_after(&mut self, id: &str, after: Duration, out: &mut Vec<Effect>) {
        let Some(u) = self.remote.disconnects.get_mut(id) else {
            return;
        };
        self.remote.generations += 1;
        let generation = self.remote.generations;
        u.generation = Some(generation);
        out.push(Effect::Arm {
            timer: Timer::Disconnected {
                id: id.into(),
                generation,
            },
            after,
        });
    }

    /// `active_vt` is read fresh: a session the console shows is the console's even when the
    /// VT follower's report of it has not arrived.
    pub(super) fn disconnected(
        &mut self,
        id: &str,
        generation: u32,
        active_vt: u16,
        out: &mut Vec<Effect>,
    ) {
        if self.remote.disconnects.get(id).and_then(|u| u.generation) != Some(generation) {
            return;
        }
        if active_vt != 0 && self.sessions.by_id(id).is_some_and(|e| e.vt == active_vt) {
            self.remote.disconnects.remove(id);
            return self.console_on_vt(active_vt, out);
        }
        self.end_unviewed(id, out);
    }

    /// Ends unviewed background session `id`, unless a login is on its way to it: that holds
    /// it another `HANDOVER_DEADLINE`, by which the login has reached it or been refused.
    fn end_unviewed(&mut self, id: &str, out: &mut Vec<Effect>) {
        let Some(e) = self.sessions.by_id(id) else {
            self.remote.disconnects.remove(id);
            return;
        };
        if !e.background || self.remote.viewers.contains_key(&e.uid) {
            self.remote.disconnects.remove(id);
            return;
        }
        let (uid, user) = (e.uid, e.user.clone());
        if (self.remote.logins.values()).any(|l| l.step.uid() == Some(uid)) {
            return self.arm_after(id, HANDOVER_DEADLINE, out);
        }
        self.remote.disconnects.remove(id);
        out.push(Effect::Log(
            Level::Info,
            format!("ending {user}'s background session {id}: no client is connected"),
        ));
        out.push(Effect::TerminateSession(id.into()));
    }

    pub(super) fn remote_session_gone(&mut self, entry: &SessionEntry) {
        self.remote.viewers.remove(&entry.uid);
        self.remote.linked.remove(&entry.uid);
        self.remote.reopening.remove(&entry.uid);
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
