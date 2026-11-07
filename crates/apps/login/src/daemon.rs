//! The login manager's state machine: inputs from the greeter, the workers, the sessions and
//! logind go in, effects for the backends come out. It does no I/O and reads no clock.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use log::Level;
use repriselogin::front::{FrontEvent, FrontRequest};
use repriselogin::greeter::{
    ErrorKind, GreeterReply, GreeterRequest, Greeting, Keyboard, Notice, PowerAction, UserEntry,
};
use repriselogin::machine::MachineConfig;
use repriselogin::session::SessionRequest;
use repriselogin::{GREETER_USER, Interface, frame};

use crate::greeterruns::{AfterGreeter, GreeterRuns, MAX_FAILURES};
use crate::remembered::Remembered;
use crate::table::{Route, SessionEntry, Sessions};
use crate::vt::{self, GREETER_VT, TEXT_LOGIN_VT};
use crate::worker::{AUTH_FAILED, FromWorker, Mode, ToWorker};

mod remote;

pub use remote::{
    HANDOVER_DEADLINE, Inhibit, LookUpKey, MAX_REMOTE_CHECKS, REFUSE_DELAY, StoredPassword,
};

/// A session that ends sooner than this after its start brings the greeter back with a notice.
pub const EARLY_END: Duration = Duration::from_secs(10);
/// How long a resumed session gets to drop its lock before the switch happens anyway.
pub const UNLOCK_WAIT: Duration = Duration::from_secs(1);
/// How long a worker gets from `Start` to `Started` before its login fails.
pub const START_WAIT: Duration = Duration::from_secs(30);
/// How long a login waits for the greeter's next request before it is cancelled.
pub const AUTH_WAIT: Duration = Duration::from_secs(5 * 60);
/// How many times an orphaned greeter's `TerminateSession` is tried before the login screen
/// comes up beside it.
pub const TERMINATE_TRIES: u32 = 4;
/// The wait before the first retry, doubled for each later one.
pub const TERMINATE_RETRY: Duration = Duration::from_secs(1);
/// How long a greeter told to end gets before its session is killed.
pub const KILL_WAIT: Duration = Duration::from_secs(5);
/// Most users one `Greeting` lists; fewer when their entries would not fit one frame.
pub const MAX_USERS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorkerId(pub u32);

/// What a worker is spawned for: `reprise-login --worker <mode>`, told the rest in `Begin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub mode: Mode,
    pub user: String,
    /// The configured autologin user, for an `Autologin` job only.
    pub autologin_user: Option<String>,
    /// `[login] greeter_socket`, for a `Greeter` job only.
    pub greeter_socket: bool,
    /// The client's address, for a remote job only.
    pub rhost: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Timer {
    /// The wait for the resumed session's `LockedHint` to go false; the number tells one
    /// resume from the next.
    UnlockAck(u32),
    /// The wait for a worker's `Started` after its `Start`.
    Start(WorkerId),
    /// The `AUTH_WAIT` of the conversation of login worker `.0`.
    Auth(WorkerId),
    /// The next try at ending orphaned greeter session `.0`.
    Terminate(String),
    /// `KILL_WAIT` after greeter worker `.0` was told to end.
    GreeterKill(WorkerId),
    /// `REFUSE_DELAY` after front conn `.0`'s `Authenticate`.
    Refuse(u64),
    /// `HANDOVER_DEADLINE` after front conn `.0`'s TCP accept.
    Handover(u64),
    /// `end_disconnected_after` since background session `id` lost its last viewer.
    Disconnected { id: String, generation: u32 },
}

#[derive(Debug, PartialEq)]
pub enum Input {
    /// A request from greeter worker `.0`'s connection; another greeter's is ignored.
    GreeterRequest(WorkerId, GreeterRequest),
    GreeterGone(WorkerId),
    /// Every message a worker sent before it exited arrives before its `WorkerExited`.
    Worker(WorkerId, FromWorker),
    /// The worker process exited, whatever its status: a greeter that was not told to leave
    /// counts as failed either way.
    WorkerExited(WorkerId),
    /// A request on session `id`'s link; `seat_active` is seat0's active session when a
    /// `SwitchUser` came, `None` with a `Hello`.
    Session {
        id: String,
        request: SessionRequest,
        seat_active: Option<String>,
    },
    /// logind removed session `id`; `seat_active` is seat0's active session after that.
    SessionRemoved {
        id: String,
        seat_active: Option<String>,
    },
    /// logind's `TerminateSession(id)` failed; `seat_active` is seat0's active session.
    TerminateFailed {
        id: String,
        seat_active: Option<String>,
    },
    /// logind reports session `id`'s `LockedHint` false after an `UnlockSession`.
    Unlocked {
        id: String,
    },
    /// logind's `SwitchTo(vt)` failed.
    SwitchFailed {
        vt: u16,
    },
    /// The console moved to the login screen's VT.
    ConsoleOnGreeterVt,
    /// The console moved to VT `.0`, not the login screen's.
    ConsoleOnVt(u16),
    PowerDone(Result<(), String>),
    Timer(Timer),
    FrontLinked,
    FrontLost,
    /// A request on the front link; its `Ready` arrives as `FrontReady`.
    Front(FrontRequest),
    /// The front's `Ready` for `conn`, its socket held by the backend; `user` is the hand-over's.
    FrontReady {
        conn: u64,
        user: String,
    },
    /// The answer to `Effect::LookUp`: the account's uid, and whether it is in the group.
    Member {
        key: LookUpKey,
        uid: Option<u32>,
        member: bool,
    },
    /// The link to `uid`'s hand-over socket is up.
    IntakeOpen {
        uid: u32,
    },
    IntakeLost {
        uid: u32,
    },
    /// The clients connected to `uid`'s session, as its reprise-rdp reports them.
    Viewers {
        uid: u32,
        count: u32,
    },
    /// The backend passed conn's socket on (`ok`), or failed to.
    Forwarded {
        conn: u64,
        ok: bool,
    },
}

#[derive(Debug, PartialEq)]
pub enum Effect {
    Reply(GreeterReply),
    Spawn {
        worker: WorkerId,
        job: Job,
    },
    ToWorker {
        worker: WorkerId,
        msg: ToWorker,
    },
    /// Closes the worker's socket and sends it SIGTERM: before `Started` the worker unwinds
    /// its PAM handle, after it the worker passes the signal on to the session.
    Kill(WorkerId),
    SwitchTo(u16),
    Power(PowerAction),
    TerminateSession(String),
    /// Stops session `.0`'s scope through systemd, which also ends a closing session that
    /// `TerminateSession` leaves running.
    StopSession(String),
    /// logind's `UnlockSession`, answered by `Input::Unlocked`.
    UnlockSession(String),
    LockSession(String),
    /// SIGKILLs every process of session `.0`.
    KillSession(String),
    Arm {
        timer: Timer,
        after: Duration,
    },
    Remember(Remembered),
    Disallocate(u16),
    Log(log::Level, String),
    ToFront(FrontEvent),
    /// Looks up `user`'s uid and whether it is a member of `group`, answered by `Input::Member`.
    LookUp {
        key: LookUpKey,
        user: String,
        group: String,
    },
    /// Opens the link to `uid`'s hand-over socket, answered by `IntakeOpen` or `IntakeLost`.
    OpenIntake {
        uid: u32,
    },
    /// Passes conn's socket and hand-over to `uid`'s session, answered by `Forwarded`.
    Forward {
        conn: u64,
        uid: u32,
    },
    /// Closes conn's socket if the backend holds it.
    DropConn {
        conn: u64,
    },
    /// The logind block inhibitor reprise-login holds from now on.
    Inhibit(Inhibit),
}

/// A session as logind's `ListSessions` and session properties describe it.
#[derive(Clone, Debug, PartialEq)]
pub struct LogindSession {
    pub id: String,
    pub user: String,
    pub uid: u32,
    pub vt: u16,
    pub seat: String,
    pub class: String,
    pub service: String,
    /// logind's `State` is `closing`: its leader is gone and its scope may still run.
    pub closing: bool,
}

/// What the backend reads fresh for each `Hello`, each start and each power request.
#[derive(Clone, Debug, PartialEq)]
pub struct World {
    /// The accounts the login screen lists, as `users::listed` picks them.
    pub users: Vec<UserEntry>,
    pub keyboard: Keyboard,
    /// `VT_GETSTATE`'s mask of open VTs 1-15.
    pub kernel_open: u16,
    /// The VT the console shows, `VT_GETSTATE`'s `v_active`; 0 when it can't be read.
    pub active_vt: u16,
    /// logind's sessions of class `user` on any seat, text and ssh logins included.
    pub user_sessions: u32,
    /// Every session logind lists, closing ones included.
    pub login_sessions: Vec<LogindSession>,
}

const BUSY: &str = "another login is in progress";
const NOT_ASKED: &str = "no question is waiting for an answer";
const NOT_AUTHENTICATED: &str = "the login is not authenticated";
const STARTING: &str = "the session is already starting";
const OTHERS_LOGGED_ON: &str = "other users are logged on";
const NO_VT: &str = "no free VT for a session";
const NO_SESSION: &str = "the account has no session to resume";
const WORKER_ENDED: &str = "the login ended unexpectedly";
const SESSION_ENDED: &str = "the session ended";
const STARTED_EARLY: &str = "the session started before it was asked to";
const AUTH_TIMED_OUT: &str = "the login waited too long and was cancelled";
/// What the greeter hears of a failed power call; logind's own error is only logged.
const POWER_FAILED: &str = "the computer could not be turned off or restarted";

struct Greeter {
    worker: WorkerId,
    /// Its worker reported `Started`.
    started: bool,
    /// Its logind session, from `Started`.
    session: Option<String>,
    /// Set by the final `Success`: the greeter exits by itself, and that exit counts no failure.
    leaving: bool,
    /// The greeter is wanted again once the leaving one has gone.
    relaunch: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// `asked` while a prompt waits for the greeter's answer.
    Authenticating { asked: bool },
    /// PAM's `uid` for the account.
    Authenticated { uid: u32 },
    /// `Start` went to the worker, which opens the session on `vt`.
    Starting { vt: u16 },
    /// `UnlockSession` went out; `generation` names its `Timer::UnlockAck`.
    Unlocking { generation: u32 },
}

struct Conversation {
    /// `None` once the worker has exited or was killed.
    worker: Option<WorkerId>,
    /// The greeter that asked for it; only that one hears how it ends.
    greeter: Option<WorkerId>,
    /// The entered name, then PAM's from `Authenticated`.
    user: String,
    route: Route,
    phase: Phase,
    /// When the greeter last asked for or answered the conversation.
    heard: Instant,
}

struct Autologin {
    worker: WorkerId,
    user: String,
    vt: u16,
}

/// A worker whose session runs; its exit frees the VT.
struct SessionWorker {
    worker: WorkerId,
    vt: u16,
}

/// A worker killed because its start timed out; it lives on until it exits.
struct Stalled {
    worker: WorkerId,
    vt: u16,
}

/// A greeter session of an earlier daemon, terminated and not yet removed.
struct Orphan {
    id: String,
    /// The `TerminateSession` calls that failed.
    failed: u32,
}

/// What follows the removal of the last orphaned greeter.
#[derive(Clone, Copy, PartialEq)]
enum AfterOrphans {
    Nothing,
    /// The orphan was showing the login screen: with no session, the autologin session or a
    /// new greeter follows it; else a new greeter, unless one of the sessions holds the seat.
    Replace,
    /// The login screen was asked for while an orphan still held VT 1.
    Greeter,
}

pub struct Daemon {
    config: MachineConfig,
    remembered: Remembered,
    sessions: Sessions,
    runs: GreeterRuns,
    greeter: Option<Greeter>,
    conversation: Option<Conversation>,
    autologin: Option<Autologin>,
    session_workers: Vec<SessionWorker>,
    stalled: Vec<Stalled>,
    /// Greeter sessions of an earlier daemon, terminated and not yet removed. No greeter
    /// starts while one is left.
    orphans: Vec<Orphan>,
    after_orphans: AfterOrphans,
    /// seat0 showed no session when `recover` ran.
    seat_empty: bool,
    /// Removals of unknown sessions while a start, console or remote, is in flight, with
    /// seat0's active session.
    removed_early: Vec<(String, Option<String>)>,
    notice: Option<Notice>,
    /// The running greeter came forward: it gets a fresh Greeting once the input is handled.
    regreet: bool,
    next_worker: u32,
    unlocks: u32,
    remote: remote::Remote,
}

impl Daemon {
    pub fn new(config: MachineConfig, remembered: Remembered) -> Daemon {
        Daemon {
            config,
            remembered,
            sessions: Sessions::default(),
            runs: GreeterRuns::default(),
            greeter: None,
            conversation: None,
            autologin: None,
            session_workers: Vec::new(),
            stalled: Vec::new(),
            orphans: Vec::new(),
            after_orphans: AfterOrphans::Nothing,
            seat_empty: false,
            removed_early: Vec::new(),
            notice: None,
            regreet: false,
            next_worker: 1,
            unlocks: 0,
            remote: remote::Remote::default(),
        }
    }

    /// The session table's logind ids, each with its uid.
    pub fn session_uids(&self) -> HashMap<String, u32> {
        (self.sessions.iter())
            .map(|e| (e.id.clone(), e.uid))
            .collect()
    }

    /// The VT of `worker` when its start timed out and it has not exited yet.
    pub fn stalled_vt(&self, worker: WorkerId) -> Option<u16> {
        (self.stalled.iter())
            .find(|s| s.worker == worker)
            .map(|s| s.vt)
    }

    /// Rebuilds the table from logind after a start, ends orphaned greeters and locks every
    /// session but `seat_active`: an earlier daemon may have unlocked one and died. A remote
    /// login's session other than `seat_active` counts as a background one.
    pub fn recover(
        &mut self,
        _now: Instant,
        found: Vec<LogindSession>,
        seat_active: Option<&str>,
    ) -> Vec<Effect> {
        let mut out = Vec::new();
        self.seat_empty = seat_active.is_none();
        for s in found {
            if s.seat == "seat0" && s.class == "user" && user_service(&s.service) {
                let off_seat = seat_active != Some(&s.id);
                if off_seat {
                    out.push(Effect::LockSession(s.id.clone()));
                }
                let background = off_seat && remote_service(&s.service);
                self.sessions.insert(SessionEntry {
                    id: s.id.clone(),
                    user: s.user,
                    uid: s.uid,
                    vt: s.vt,
                    started: None,
                    background,
                });
                if background {
                    self.arm_disconnected(s.id, &mut out);
                }
            } else if s.class == "greeter" && s.service == Mode::Greeter.service() {
                self.orphans.push(Orphan {
                    id: s.id.clone(),
                    failed: 0,
                });
                out.push(Effect::TerminateSession(s.id));
            }
        }
        self.open_intakes(&mut out);
        self.update_inhibit(&mut out);
        out
    }

    /// Starts the autologin session or the greeter; with sessions recovered, only the greeter
    /// and only when seat0 shows none. An orphaned greeter was showing the login screen, so
    /// that choice waits until it is gone.
    pub fn start(&mut self, _now: Instant, world: &World) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.orphans.is_empty() {
            self.after_orphans = AfterOrphans::Replace;
            return out;
        }
        if !self.sessions.is_empty() {
            if self.seat_empty {
                self.bring_greeter(&mut out);
            }
            return out;
        }
        self.autologin_or_greeter(world, &mut out);
        out
    }

    /// With no session running: the autologin session if one is configured and can start,
    /// else the greeter.
    fn autologin_or_greeter(&mut self, world: &World, out: &mut Vec<Effect>) {
        if let Some(user) = self.config.login.autologin_user.clone() {
            let listed = world.users.iter().any(|u| u.name == user);
            let vt = vt::pick(&self.taken_vts(), world.kernel_open, world.active_vt);
            match (listed, vt) {
                (true, Some(vt)) => {
                    let mut ends = Vec::new();
                    // PAM's uid comes after the spawn, so this matches by name; it only locks.
                    if let Some((id, _)) = self.take_leftovers(world, &user, None, &mut ends) {
                        out.extend(ends);
                        out.push(Effect::LockSession(id));
                    } else {
                        let worker = self.next_worker();
                        out.push(Effect::Spawn {
                            worker,
                            job: self.job(Mode::Autologin, &user),
                        });
                        out.extend(ends);
                        send_start(worker, vt, out);
                        self.autologin = Some(Autologin { worker, user, vt });
                        return;
                    }
                }
                (true, None) => out.push(Effect::Log(
                    Level::Warn,
                    format!("autologin of {user}: {NO_VT}"),
                )),
                (false, _) => out.push(Effect::Log(
                    Level::Info,
                    format!(
                        "autologin_user {user:?} is not an account the login screen lists; \
                     showing the login screen"
                    ),
                )),
            }
        }
        self.spawn_greeter(out);
    }

    pub fn handle(&mut self, now: Instant, world: &World, input: Input) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::GreeterRequest(w, request) if self.is_greeter(w) => {
                self.greeter_request(now, world, request, &mut out)
            }
            Input::GreeterGone(w) if self.is_greeter(w) => self.greeter_gone(&mut out),
            Input::GreeterRequest(..) | Input::GreeterGone(_) => {}
            Input::Worker(worker, msg) if self.is_remote_worker(worker) => {
                self.remote_worker_message(now, world, worker, msg, &mut out)
            }
            Input::Worker(worker, msg) => self.worker_message(now, worker, msg, &mut out),
            Input::WorkerExited(worker) if self.is_remote_worker(worker) => {
                self.remote_worker_exited(now, worker, &mut out)
            }
            Input::WorkerExited(worker) => self.worker_exited(worker, &mut out),
            Input::Session {
                id,
                request,
                seat_active,
            } => self.session_request(&id, request, seat_active, &mut out),
            Input::SessionRemoved { id, seat_active } => {
                self.session_removed(now, world, &id, seat_active, &mut out)
            }
            Input::Unlocked { id } => {
                let resuming = matches!(&self.conversation, Some(Conversation {
                    route: Route::Resume { id: resumed, .. },
                    phase: Phase::Unlocking { .. },
                    ..
                }) if *resumed == id);
                if resuming {
                    self.finish_resume(false, &mut out);
                }
            }
            Input::SwitchFailed { vt } => self.switch_failed(vt, &mut out),
            Input::ConsoleOnGreeterVt => self.console_on_greeter_vt(&mut out),
            Input::ConsoleOnVt(vt) => self.console_on_vt(vt, &mut out),
            Input::Timer(Timer::UnlockAck(generation)) => {
                let current = matches!(&self.conversation, Some(Conversation {
                    phase: Phase::Unlocking { generation: g },
                    ..
                }) if *g == generation);
                if current {
                    self.finish_resume(true, &mut out);
                }
            }
            Input::Timer(Timer::Start(worker)) if self.is_remote_worker(worker) => {
                self.remote_start_timed_out(now, worker, &mut out)
            }
            Input::Timer(Timer::Start(worker)) => self.start_timed_out(worker, &mut out),
            Input::Timer(Timer::Auth(worker)) => self.auth_wait(now, worker, &mut out),
            Input::TerminateFailed { id, seat_active } => {
                self.terminate_failed(world, id, seat_active, &mut out)
            }
            Input::Timer(Timer::GreeterKill(worker)) => {
                self.greeter_outlived_kill(worker, &mut out)
            }
            Input::Timer(Timer::Terminate(id)) => {
                if self.orphans.iter().any(|o| o.id == id) {
                    out.push(Effect::TerminateSession(id));
                }
            }
            Input::PowerDone(Ok(())) => out.push(Effect::Reply(GreeterReply::Success)),
            Input::PowerDone(Err(text)) => {
                out.push(Effect::Log(
                    Level::Warn,
                    format!("turning off or restarting: {text}"),
                ));
                out.push(Effect::Reply(error(ErrorKind::Failed, POWER_FAILED.into())));
            }
            Input::Timer(Timer::Refuse(conn)) => self.refuse_delay_over(now, conn, &mut out),
            Input::Timer(Timer::Handover(conn)) => self.handover_deadline(now, conn, &mut out),
            Input::Timer(Timer::Disconnected { id, generation }) => {
                self.disconnected(&id, generation, &mut out)
            }
            Input::FrontLinked => self.remote.front = true,
            Input::FrontLost => self.front_lost(&mut out),
            Input::Front(request) => self.front_request(now, request, &mut out),
            Input::FrontReady { conn, user } => self.front_ready(conn, &user, &mut out),
            Input::Member {
                key: LookUpKey::Conn(conn),
                uid,
                member,
            } => self.member(now, conn, uid, member, &mut out),
            Input::Member { .. } => {}
            Input::IntakeOpen { uid } => self.intake_open(uid, &mut out),
            Input::IntakeLost { uid } => self.intake_lost(uid, &mut out),
            Input::Viewers { uid, count } => self.set_viewers(uid, count, &mut out),
            Input::Forwarded { conn, ok } => self.forwarded(conn, ok, &mut out),
        }
        self.update_inhibit(&mut out);
        if !self.start_in_flight() && !self.remote_start_in_flight() {
            self.removed_early.clear();
        }
        if std::mem::take(&mut self.regreet) {
            let greeting = self.greeting(world);
            out.push(Effect::Reply(GreeterReply::Greeting(greeting)));
        }
        out
    }

    /// A job for `mode` and `user`; an autologin job names the configured user apart from
    /// `user`, for the worker to check.
    fn job(&self, mode: Mode, user: &str) -> Job {
        Job {
            mode,
            user: user.into(),
            autologin_user: (mode == Mode::Autologin)
                .then(|| self.config.login.autologin_user.clone())
                .flatten(),
            greeter_socket: mode == Mode::Greeter && self.config.login.greeter_socket,
            rhost: None,
        }
    }

    fn next_worker(&mut self) -> WorkerId {
        let id = WorkerId(self.next_worker);
        self.next_worker += 1;
        id
    }

    /// Whether `worker` is the running greeter's; nobody answers another greeter's input.
    fn is_greeter(&self, worker: WorkerId) -> bool {
        self.greeter.as_ref().is_some_and(|g| g.worker == worker)
    }

    fn greeter_request(
        &mut self,
        now: Instant,
        world: &World,
        request: GreeterRequest,
        out: &mut Vec<Effect>,
    ) {
        match request {
            GreeterRequest::Hello => {
                let greeting = self.greeting(world);
                out.push(Effect::Reply(GreeterReply::Greeting(greeting)));
            }
            GreeterRequest::CreateSession { user } => {
                if self.conversation.is_some() {
                    return out.push(refused(BUSY));
                }
                let route = self.sessions.route(&user);
                let mode = match route {
                    Route::Start => Mode::Login,
                    Route::Resume { .. } => Mode::Reauth,
                };
                let worker = self.next_worker();
                out.push(Effect::Spawn {
                    worker,
                    job: self.job(mode, &user),
                });
                out.push(Effect::Arm {
                    timer: Timer::Auth(worker),
                    after: AUTH_WAIT,
                });
                self.conversation = Some(Conversation {
                    worker: Some(worker),
                    greeter: self.greeter.as_ref().map(|g| g.worker),
                    user,
                    route,
                    phase: Phase::Authenticating { asked: false },
                    heard: now,
                });
            }
            GreeterRequest::PostAuthResponse { response } => match &mut self.conversation {
                Some(Conversation {
                    worker: Some(worker),
                    phase: phase @ Phase::Authenticating { asked: true },
                    heard,
                    ..
                }) => {
                    *phase = Phase::Authenticating { asked: false };
                    *heard = now;
                    out.push(Effect::ToWorker {
                        worker: *worker,
                        msg: ToWorker::Answer(response),
                    });
                }
                _ => out.push(refused(NOT_ASKED)),
            },
            GreeterRequest::StartSession => self.start_session(world, out),
            GreeterRequest::CancelSession => match self.conversation.as_ref().map(|c| c.phase) {
                Some(Phase::Starting { .. } | Phase::Unlocking { .. }) => {
                    out.push(refused(STARTING))
                }
                _ => {
                    self.drop_conversation(out);
                    out.push(Effect::Reply(GreeterReply::Success));
                }
            },
            GreeterRequest::Power { action } => {
                if self.sessions.is_empty() && world.user_sessions == 0 {
                    out.push(Effect::Power(action));
                } else {
                    out.push(refused(OTHERS_LOGGED_ON));
                }
            }
            GreeterRequest::PowerAs { .. } => out.push(refused(OTHERS_LOGGED_ON)),
        }
    }

    /// The configured interface, else the last one used; users capped to fit one frame.
    fn greeting(&mut self, world: &World) -> Greeting {
        let notes = self.notes();
        let mut greeting = Greeting {
            interface: (self.config.login.interface)
                .or(self.remembered.last_interface)
                .unwrap_or(Interface::Classic),
            show_users: self.config.login.show_users,
            users: (world.users.iter().take(MAX_USERS))
                .map(|u| UserEntry {
                    note: notes.get(&u.name).copied(),
                    ..u.clone()
                })
                .collect(),
            last_user: self.remembered.last_user.clone(),
            keyboard: world.keyboard.clone(),
            notice: None,
        };
        while !greeting.users.is_empty()
            && frame::encode(&GreeterReply::Greeting(greeting.clone())).is_err()
        {
            greeting.users.truncate(greeting.users.len() / 2);
        }
        greeting.notice = self.notice.take();
        greeting
    }

    fn start_session(&mut self, world: &World, out: &mut Vec<Effect>) {
        let Some(c) = &mut self.conversation else {
            return out.push(refused(NOT_AUTHENTICATED));
        };
        match (c.phase, &c.route) {
            (Phase::Starting { .. } | Phase::Unlocking { .. }, _) => out.push(refused(STARTING)),
            (Phase::Authenticating { .. }, _) => out.push(refused(NOT_AUTHENTICATED)),
            (Phase::Authenticated { .. }, Route::Resume { id, .. }) => {
                let id = id.clone();
                let phase = self.unlock(id, out);
                self.conversation.as_mut().expect("conversation").phase = phase;
            }
            (Phase::Authenticated { uid }, Route::Start) => {
                let Some(worker) = c.worker else {
                    return self.fail_conversation(ErrorKind::Failed, WORKER_ENDED.into(), out);
                };
                let user = c.user.clone();
                if let Some((id, vt)) = self.take_leftovers(world, &user, Some(uid), out) {
                    out.push(Effect::Kill(worker));
                    let phase = self.unlock(id.clone(), out);
                    let c = self.conversation.as_mut().expect("conversation");
                    c.worker = None;
                    c.route = Route::Resume { id, vt };
                    c.phase = phase;
                    return;
                }
                let Some(vt) = vt::pick(&self.taken_vts(), world.kernel_open, world.active_vt)
                else {
                    return self.fail_conversation(ErrorKind::Failed, NO_VT.into(), out);
                };
                self.conversation.as_mut().expect("conversation").phase = Phase::Starting { vt };
                send_start(worker, vt, out);
            }
        }
    }

    /// Login-service sessions of `uid` (of the name `user` when None) on a VT the table lacks:
    /// closing ones stop so none runs beside a new one, a live one on seat0 joins the table.
    fn take_leftovers(
        &mut self,
        world: &World,
        user: &str,
        uid: Option<u32>,
        out: &mut Vec<Effect>,
    ) -> Option<(String, u16)> {
        let mut live = None;
        for s in &world.login_sessions {
            let theirs = uid.map_or(s.user == user, |uid| s.uid == uid);
            if !theirs
                || s.vt == 0
                || !user_service(&s.service)
                || self.sessions.by_id(&s.id).is_some()
            {
                continue;
            }
            if s.closing {
                out.push(Effect::Log(
                    Level::Info,
                    format!("ending {user}'s leftover session {} before a new one", s.id),
                ));
                out.push(Effect::StopSession(s.id.clone()));
            } else if live.is_none() && s.seat == "seat0" && s.class == "user" {
                live = Some(s.clone());
            }
        }
        let s = live?;
        out.push(Effect::Log(
            Level::Info,
            format!(
                "{user}'s session {} was not in the table; taking it in",
                s.id
            ),
        ));
        self.sessions.insert(SessionEntry {
            id: s.id.clone(),
            user: s.user,
            uid: s.uid,
            vt: s.vt,
            started: None,
            background: false,
        });
        Some((s.id, s.vt))
    }

    fn worker_message(
        &mut self,
        now: Instant,
        worker: WorkerId,
        msg: FromWorker,
        out: &mut Vec<Effect>,
    ) {
        if self
            .conversation
            .as_ref()
            .is_some_and(|c| c.worker == Some(worker))
        {
            return self.conversation_message(now, worker, msg, out);
        }
        if let Some(s) = self.stalled.iter().find(|s| s.worker == worker) {
            if let FromWorker::Started { session_id, .. } = msg {
                let vt = s.vt;
                out.push(Effect::Log(
                    Level::Warn,
                    format!("session {session_id} opened after its start timed out; ending it"),
                ));
                out.push(Effect::TerminateSession(session_id));
                // Its worker activated `vt` before opening the session; a start under way
                // has the console now.
                if vt != GREETER_VT && !self.start_in_flight() {
                    self.bring_greeter(out);
                }
            }
            return;
        }
        let is_greeter = self.greeter.as_ref().is_some_and(|g| g.worker == worker);
        let autologin = self.autologin.as_ref().is_some_and(|a| a.worker == worker);
        let session = self.session_workers.iter().any(|s| s.worker == worker);
        if !(is_greeter || autologin || session) {
            return;
        }
        match msg {
            FromWorker::Prompt { .. } => out.push(Effect::ToWorker {
                worker,
                msg: ToWorker::Answer(None),
            }),
            FromWorker::Authenticated { user, .. } if autologin => {
                self.autologin.as_mut().expect("autologin worker").user = user;
            }
            FromWorker::Started { session_id, uid } if autologin => {
                let a = self.autologin.take().expect("autologin worker");
                let entry = SessionEntry {
                    id: session_id,
                    user: a.user,
                    uid,
                    vt: a.vt,
                    started: Some(now),
                    background: false,
                };
                self.session_started(now, worker, entry, None, out);
            }
            FromWorker::Started { session_id, .. } if is_greeter => {
                let g = self.greeter.as_mut().expect("greeter");
                g.started = true;
                g.session = Some(session_id);
            }
            FromWorker::Failed { text, .. } if is_greeter => out.push(Effect::Log(
                Level::Warn,
                format!("the login screen did not start: {text}"),
            )),
            FromWorker::Failed { text, .. } if autologin => out.push(Effect::Log(
                Level::Warn,
                format!("autologin did not start: {text}"),
            )),
            _ => {}
        }
    }

    fn conversation_message(
        &mut self,
        now: Instant,
        worker: WorkerId,
        msg: FromWorker,
        out: &mut Vec<Effect>,
    ) {
        let c = self.conversation.as_mut().expect("conversation");
        match msg {
            FromWorker::Prompt { kind, text } => match c.phase {
                Phase::Authenticating { .. } => {
                    c.phase = Phase::Authenticating { asked: true };
                    out.push(Effect::Reply(GreeterReply::AuthMessage { kind, text }));
                }
                _ => out.push(Effect::ToWorker {
                    worker,
                    msg: ToWorker::Answer(None),
                }),
            },
            FromWorker::Authenticated { user, uid } => {
                if let Phase::Authenticating { .. } = c.phase {
                    self.authenticated(worker, user, uid, out);
                }
            }
            FromWorker::Started { session_id, uid } => {
                let Phase::Starting { vt } = c.phase else {
                    out.push(Effect::Log(
                        Level::Warn,
                        format!(
                            "a login worker reported session {session_id} before its start; \
                         ending it"
                        ),
                    ));
                    out.push(Effect::TerminateSession(session_id));
                    return self.fail_conversation(ErrorKind::Failed, STARTED_EARLY.into(), out);
                };
                let c = self.conversation.take().expect("conversation");
                let entry = SessionEntry {
                    id: session_id,
                    user: c.user,
                    uid,
                    vt,
                    started: Some(now),
                    background: false,
                };
                self.session_started(now, worker, entry, c.greeter, out);
            }
            FromWorker::Failed { kind, text } => {
                c.worker = None;
                self.fail_conversation(kind, text, out);
            }
        }
    }

    /// Routes the login by PAM's account: a uid with a session resumes it, any other starts one.
    /// Root never logs in here, and is told so in a wrong password's words.
    fn authenticated(&mut self, worker: WorkerId, user: String, uid: u32, out: &mut Vec<Effect>) {
        if uid == 0 {
            out.push(Effect::Log(
                Level::Warn,
                "a login as root was refused".into(),
            ));
            return self.fail_conversation(ErrorKind::Auth, AUTH_FAILED.into(), out);
        }
        let route = match self.sessions.by_uid(uid) {
            Some(e) => Route::Resume {
                id: e.id.clone(),
                vt: e.vt,
            },
            None => Route::Start,
        };
        let c = self.conversation.as_mut().expect("conversation");
        let reauth = matches!(c.route, Route::Resume { .. });
        match (reauth, &route) {
            (true, Route::Start) => {
                c.worker = None;
                return self.fail_conversation(ErrorKind::Failed, NO_SESSION.into(), out);
            }
            (false, Route::Resume { .. }) => {
                c.worker = None;
                out.push(Effect::Kill(worker));
            }
            _ => {}
        }
        c.user = user;
        c.route = route;
        c.phase = Phase::Authenticated { uid };
        out.push(Effect::Reply(GreeterReply::Success));
    }

    /// Enters a new session in the table. A uid with one already ends it and resumes that, or for
    /// autologin (`greeter` None) shows the login screen; one logind removed already is over.
    fn session_started(
        &mut self,
        now: Instant,
        worker: WorkerId,
        entry: SessionEntry,
        greeter: Option<WorkerId>,
        out: &mut Vec<Effect>,
    ) {
        self.session_workers.push(SessionWorker {
            worker,
            vt: entry.vt,
        });
        let told = self.greeter_runs(greeter);
        if let Some(at) = self
            .removed_early
            .iter()
            .position(|(id, _)| *id == entry.id)
        {
            let (_, seat_active) = self.removed_early.remove(at);
            if told {
                out.push(Effect::Reply(error(
                    ErrorKind::Failed,
                    SESSION_ENDED.into(),
                )));
            } else {
                self.notice = Some(Notice::SessionEndedEarly);
            }
            if seat_active.is_none() {
                self.bring_greeter(out);
            }
            return;
        }
        if let Some(existing) = self.sessions.by_uid(entry.uid) {
            let route = Route::Resume {
                id: existing.id.clone(),
                vt: existing.vt,
            };
            let id = existing.id.clone();
            out.push(Effect::TerminateSession(entry.id));
            if greeter.is_none() {
                out.push(Effect::Log(
                    Level::Warn,
                    format!(
                        "autologin of {}: the account has a session already",
                        entry.user
                    ),
                ));
                return self.bring_greeter(out);
            }
            let phase = self.unlock(id, out);
            self.conversation = Some(Conversation {
                worker: None,
                greeter,
                user: entry.user,
                route,
                phase,
                heard: now,
            });
            return;
        }
        let user = entry.user.clone();
        self.sessions.insert(entry);
        if told {
            out.push(Effect::Reply(GreeterReply::Success));
        }
        self.remember_user(user, out);
        self.runs.reset();
        if told {
            self.greeter_leaves();
        }
    }

    /// Whether `greeter` is the one running now; a later greeter never asked for this login.
    fn greeter_runs(&self, greeter: Option<WorkerId>) -> bool {
        greeter.is_some() && self.greeter.as_ref().map(|g| g.worker) == greeter
    }

    fn start_in_flight(&self) -> bool {
        self.autologin.is_some()
            || matches!(
                &self.conversation,
                Some(Conversation {
                    phase: Phase::Starting { .. },
                    ..
                })
            )
    }

    /// Ends the conversation with an error and kills its worker if it runs. A failed start has
    /// taken the console to its VT already, so the login screen comes back.
    fn fail_conversation(&mut self, kind: ErrorKind, text: String, out: &mut Vec<Effect>) {
        let Some(c) = self.conversation.take() else {
            return;
        };
        if let Some(worker) = c.worker {
            out.push(Effect::Kill(worker));
        }
        if self.greeter_runs(c.greeter) {
            out.push(Effect::Reply(error(kind, text)));
        }
        if let Phase::Starting { .. } = c.phase {
            self.bring_greeter(out);
        }
    }

    /// Ends a conversation that has not reached `Start`, killing its worker.
    fn drop_conversation(&mut self, out: &mut Vec<Effect>) {
        let Some(c) = &self.conversation else {
            return;
        };
        if let Phase::Starting { .. } | Phase::Unlocking { .. } = c.phase {
            return;
        }
        if let Some(worker) = c.worker {
            out.push(Effect::Kill(worker));
        }
        self.conversation = None;
    }

    /// Gives up on a worker with no `Started` within `START_WAIT`: it is killed and the login
    /// screen comes back without waiting for its exit, which a worker stuck in PAM never makes.
    fn start_timed_out(&mut self, worker: WorkerId, out: &mut Vec<Effect>) {
        let text = format!("the session did not start within {START_WAIT:?}");
        let starting = match &self.conversation {
            Some(Conversation {
                worker: Some(w),
                phase: Phase::Starting { vt },
                ..
            }) if *w == worker => Some(*vt),
            _ => None,
        };
        if let Some(vt) = starting {
            self.stalled.push(Stalled { worker, vt });
            self.fail_conversation(ErrorKind::Failed, text, out);
        } else if self.autologin.as_ref().is_some_and(|a| a.worker == worker) {
            let a = self.autologin.take().expect("autologin worker");
            out.push(Effect::Kill(worker));
            out.push(Effect::Log(
                Level::Warn,
                format!("autologin of {}: {text}", a.user),
            ));
            self.stalled.push(Stalled { worker, vt: a.vt });
            self.bring_greeter(out);
        } else if (self.greeter.as_ref()).is_some_and(|g| g.worker == worker && !g.started) {
            out.push(Effect::Kill(worker));
            out.push(Effect::Log(
                Level::Warn,
                format!("the login screen: {text}"),
            ));
            self.stalled.push(Stalled {
                worker,
                vt: GREETER_VT,
            });
            self.greeter_exited(out);
        }
    }

    /// Cancels worker `worker`'s login once `AUTH_WAIT` has passed with no request from the
    /// greeter, which a PAM module that blocks without asking would leave on "Verifying".
    fn auth_wait(&mut self, now: Instant, worker: WorkerId, out: &mut Vec<Effect>) {
        let Some(Conversation {
            worker: Some(w),
            phase: Phase::Authenticating { .. },
            heard,
            ..
        }) = &self.conversation
        else {
            return;
        };
        if *w != worker {
            return;
        }
        let waited = now.saturating_duration_since(*heard);
        if waited < AUTH_WAIT {
            return out.push(Effect::Arm {
                timer: Timer::Auth(worker),
                after: AUTH_WAIT - waited,
            });
        }
        self.fail_conversation(ErrorKind::Failed, AUTH_TIMED_OUT.into(), out);
    }

    fn greeter_gone(&mut self, out: &mut Vec<Effect>) {
        self.drop_conversation(out);
        if let Some(g) = self.greeter.as_ref().filter(|g| !g.leaving) {
            out.push(Effect::Kill(g.worker));
        }
    }

    fn worker_exited(&mut self, worker: WorkerId, out: &mut Vec<Effect>) {
        if self.greeter.as_ref().is_some_and(|g| g.worker == worker) {
            return self.greeter_exited(out);
        }
        if let Some(at) = self.stalled.iter().position(|s| s.worker == worker) {
            let vt = self.stalled.remove(at).vt;
            if vt != GREETER_VT {
                out.push(Effect::Disallocate(vt));
            }
            return;
        }
        if let Some(c) = self
            .conversation
            .as_mut()
            .filter(|c| c.worker == Some(worker))
        {
            c.worker = None;
            let reauthed = matches!(c.route, Route::Resume { .. })
                && matches!(
                    c.phase,
                    Phase::Authenticated { .. } | Phase::Unlocking { .. }
                );
            if !reauthed {
                self.fail_conversation(ErrorKind::Failed, WORKER_ENDED.into(), out);
            }
            return;
        }
        if self.autologin.as_ref().is_some_and(|a| a.worker == worker) {
            let a = self.autologin.take().expect("autologin worker");
            out.push(Effect::Log(
                Level::Warn,
                format!("autologin of {}: {WORKER_ENDED}", a.user),
            ));
            return self.bring_greeter(out);
        }
        if let Some(at) = self.session_workers.iter().position(|s| s.worker == worker) {
            out.push(Effect::Disallocate(self.session_workers.remove(at).vt));
        }
    }

    fn greeter_exited(&mut self, out: &mut Vec<Effect>) {
        let g = self.greeter.take().expect("greeter");
        self.drop_conversation(out);
        if g.leaving {
            self.runs.ended(true);
            if g.relaunch {
                self.spawn_greeter(out);
            }
            return;
        }
        match self.runs.ended(false) {
            AfterGreeter::TextLogin => {
                out.push(Effect::SwitchTo(TEXT_LOGIN_VT));
                out.push(Effect::Log(
                    Level::Warn,
                    format!(
                        "the login screen failed {MAX_FAILURES} times in a row; \
                     the text login on VT {TEXT_LOGIN_VT} takes over"
                    ),
                ));
            }
            AfterGreeter::Again if self.conversation.is_some() => {}
            AfterGreeter::Again => self.spawn_greeter(out),
        }
    }

    fn session_request(
        &mut self,
        id: &str,
        request: SessionRequest,
        seat_active: Option<String>,
        out: &mut Vec<Effect>,
    ) {
        if self.sessions.by_id(id).is_none() {
            return;
        }
        match request {
            SessionRequest::Hello { interface } => {
                if self.remembered.last_interface != Some(interface) {
                    self.remembered.last_interface = Some(interface);
                    out.push(Effect::Remember(self.remembered.clone()));
                }
            }
            // Only the session on screen may hand the console to the login screen.
            SessionRequest::SwitchUser if seat_active.as_deref() == Some(id) => {
                self.bring_greeter(out)
            }
            SessionRequest::SwitchUser => {}
        }
    }

    fn session_removed(
        &mut self,
        now: Instant,
        world: &World,
        id: &str,
        seat_active: Option<String>,
        out: &mut Vec<Effect>,
    ) {
        // A closing session can still be seat0's active one; nothing else is on screen then.
        let seat_active = seat_active.filter(|a| a != id);
        if self.orphans.iter().any(|o| o.id == id) {
            return self.orphan_gone(world, id, seat_active, out);
        }
        let Some(entry) = self.sessions.remove(id) else {
            if self.start_in_flight() || self.remote_start_in_flight() {
                self.removed_early.push((id.into(), seat_active.clone()));
            }
            if !self.start_in_flight()
                && seat_active.is_none()
                && self.stalled.iter().any(|s| s.vt != GREETER_VT)
            {
                // Likely a timed-out worker's session, leaving its VT on screen.
                self.bring_greeter(out);
            }
            return;
        };
        let resuming = self.conversation.as_ref().is_some_and(
            |c| matches!(&c.route, Route::Resume { id: resumed, .. } if resumed == id),
        );
        if resuming {
            self.fail_conversation(ErrorKind::Failed, SESSION_ENDED.into(), out);
        }
        self.remote_session_gone(&entry);
        // A recovered session has no worker of this daemon to free its VT on exit.
        if entry.started.is_none() {
            out.push(Effect::Disallocate(entry.vt));
        }
        if seat_active.is_none() {
            if entry
                .started
                .is_some_and(|t| now.duration_since(t) < EARLY_END)
            {
                self.notice = Some(Notice::SessionEndedEarly);
            }
            self.bring_greeter(out);
        }
    }

    /// Drops orphan `id`; once none is left, follows `after_orphans`.
    fn orphan_gone(
        &mut self,
        world: &World,
        id: &str,
        seat_active: Option<String>,
        out: &mut Vec<Effect>,
    ) {
        self.orphans.retain(|o| o.id != id);
        if !self.orphans.is_empty() {
            return;
        }
        let seat_taken = (seat_active.as_deref()).is_some_and(|a| self.sessions.by_id(a).is_some());
        match std::mem::replace(&mut self.after_orphans, AfterOrphans::Nothing) {
            AfterOrphans::Greeter => self.bring_greeter(out),
            AfterOrphans::Replace if self.sessions.is_empty() => {
                self.autologin_or_greeter(world, out)
            }
            AfterOrphans::Replace if !seat_taken => self.bring_greeter(out),
            AfterOrphans::Replace | AfterOrphans::Nothing => {}
        }
    }

    /// Retries an orphan's end with a growing wait, and after `TERMINATE_TRIES` failures
    /// counts it gone, so no greeter waits on it for good.
    fn terminate_failed(
        &mut self,
        world: &World,
        id: String,
        seat_active: Option<String>,
        out: &mut Vec<Effect>,
    ) {
        let Some(o) = self.orphans.iter_mut().find(|o| o.id == id) else {
            return;
        };
        o.failed += 1;
        if o.failed < TERMINATE_TRIES {
            let after = TERMINATE_RETRY * 2u32.pow(o.failed - 1);
            return out.push(Effect::Arm {
                timer: Timer::Terminate(id),
                after,
            });
        }
        out.push(Effect::Log(
            Level::Warn,
            format!("greeter session {id} of an earlier start did not end; showing the login screen anyway"),
        ));
        self.orphan_gone(world, &id, seat_active, out);
    }

    /// Asks logind to unlock session `id` and arms the wait; returns the `Unlocking` phase.
    fn unlock(&mut self, id: String, out: &mut Vec<Effect>) -> Phase {
        self.unlocks += 1;
        out.push(Effect::UnlockSession(id));
        out.push(Effect::Arm {
            timer: Timer::UnlockAck(self.unlocks),
            after: UNLOCK_WAIT,
        });
        Phase::Unlocking {
            generation: self.unlocks,
        }
    }

    /// Ends an `Unlocking` resume once the session dropped its lock or `UNLOCK_WAIT` ran out.
    fn finish_resume(&mut self, timed_out: bool, out: &mut Vec<Effect>) {
        let Some(Conversation {
            route: Route::Resume { id, vt },
            greeter,
            user,
            ..
        }) = self.conversation.take()
        else {
            return;
        };
        if timed_out {
            out.push(Effect::Log(
                Level::Info,
                format!(
                    "session {id} did not unlock within {UNLOCK_WAIT:?}; switching to it anyway"
                ),
            ));
        }
        out.push(Effect::SwitchTo(vt));
        self.sessions.set_background(&id, false);
        if self.greeter_runs(greeter) {
            out.push(Effect::Reply(GreeterReply::Success));
            self.greeter_leaves();
        }
        self.remember_user(user, out);
    }

    /// The login screen picks `user` next.
    fn remember_user(&mut self, user: String, out: &mut Vec<Effect>) {
        if self.remembered.last_user.as_ref() != Some(&user) {
            self.remembered.last_user = Some(user);
            out.push(Effect::Remember(self.remembered.clone()));
        }
    }

    /// A session the console could not switch to may have just been unlocked for a resume:
    /// it is locked again, and the login screen stays.
    fn switch_failed(&mut self, vt: u16, out: &mut Vec<Effect>) {
        let Some(e) = self.sessions.iter().find(|e| e.vt == vt) else {
            return;
        };
        out.push(Effect::LockSession(e.id.clone()));
        self.bring_greeter(out);
    }

    /// VT 1 shows the login screen whenever the console is on it, unless autologin or a login
    /// in flight is about to take the console.
    fn console_on_greeter_vt(&mut self, out: &mut Vec<Effect>) {
        let leaving = self.greeter.as_ref().map(|g| g.leaving);
        let quiet = self.autologin.is_none() && self.conversation.is_none();
        match leaving {
            None if quiet => self.bring_greeter(out),
            Some(true) => self.bring_greeter(out),
            _ => {}
        }
    }

    fn greeter_leaves(&mut self) {
        if let Some(g) = &mut self.greeter {
            g.leaving = true;
        }
    }

    /// Shows the login screen: switches to a running greeter or starts one, once no orphaned
    /// greeter is left on VT 1.
    fn bring_greeter(&mut self, out: &mut Vec<Effect>) {
        if !self.orphans.is_empty() {
            self.after_orphans = AfterOrphans::Greeter;
            return;
        }
        match &mut self.greeter {
            Some(g) if g.leaving => {
                if !std::mem::replace(&mut g.relaunch, true) {
                    out.push(Effect::Kill(g.worker));
                    out.push(Effect::Arm {
                        timer: Timer::GreeterKill(g.worker),
                        after: KILL_WAIT,
                    });
                }
            }
            Some(_) => {
                out.push(Effect::SwitchTo(GREETER_VT));
                self.regreet = true;
            }
            None => self.spawn_greeter(out),
        }
    }

    /// A greeter still running `KILL_WAIT` after its kill hangs past SIGTERM: its session
    /// goes, which ends its worker too.
    fn greeter_outlived_kill(&mut self, worker: WorkerId, out: &mut Vec<Effect>) {
        let Some(g) = self.greeter.as_ref().filter(|g| g.worker == worker) else {
            return;
        };
        out.push(Effect::Log(
            Level::Warn,
            format!("the login screen did not end within {KILL_WAIT:?}; killing it"),
        ));
        if let Some(id) = &g.session {
            out.push(Effect::KillSession(id.clone()));
        }
    }

    fn spawn_greeter(&mut self, out: &mut Vec<Effect>) {
        let worker = self.next_worker();
        out.push(Effect::Spawn {
            worker,
            job: self.job(Mode::Greeter, GREETER_USER),
        });
        send_start(worker, GREETER_VT, out);
        self.greeter = Some(Greeter {
            worker,
            started: false,
            session: None,
            leaving: false,
            relaunch: false,
        });
    }
}

fn error(kind: ErrorKind, text: String) -> GreeterReply {
    GreeterReply::Error { kind, text }
}

fn refused(text: &str) -> Effect {
    Effect::Reply(error(ErrorKind::Refused, text.into()))
}

/// Whether `service` is the PAM service of a user session reprise-login starts.
fn user_service(service: &str) -> bool {
    [Mode::Login, Mode::Autologin, Mode::Remote, Mode::RemoteNla]
        .iter()
        .any(|m| m.service() == service)
}

/// Whether `service` is the PAM service of a remote login's session.
fn remote_service(service: &str) -> bool {
    [Mode::Remote, Mode::RemoteNla]
        .iter()
        .any(|m| m.service() == service)
}

fn send_start(worker: WorkerId, vt: u16, out: &mut Vec<Effect>) {
    out.push(Effect::ToWorker {
        worker,
        msg: ToWorker::Start {
            vt,
            background: false,
        },
    });
    out.push(Effect::Arm {
        timer: Timer::Start(worker),
        after: START_WAIT,
    });
}

#[cfg(test)]
mod remote_tests;
#[cfg(test)]
mod tests;
