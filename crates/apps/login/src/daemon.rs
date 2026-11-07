//! The login manager's state machine: inputs from the greeter, the workers, the sessions and
//! logind go in, effects for the backends come out. It does no I/O and reads no clock.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use log::Level;
use repriselogin::greeter::{
    ErrorKind, GreeterReply, GreeterRequest, Greeting, Keyboard, Notice, PowerAction, UserEntry,
};
use repriselogin::session::SessionRequest;
use repriselogin::{GREETER_USER, Interface, frame};

use crate::config::LoginConfig;
use crate::greeterruns::{AfterGreeter, GreeterRuns, MAX_FAILURES};
use crate::remembered::Remembered;
use crate::table::{Route, SessionEntry, Sessions};
use crate::vt::{self, GREETER_VT, TEXT_LOGIN_VT};
use crate::worker::{AUTH_FAILED, FromWorker, Mode, ToWorker};

/// A session that ends sooner than this after its start brings the greeter back with a notice.
pub const EARLY_END: Duration = Duration::from_secs(10);
/// How long a resumed session gets to drop its lock before the switch happens anyway.
pub const UNLOCK_WAIT: Duration = Duration::from_secs(1);
/// How long a worker gets from `Start` to `Started` before its login fails.
pub const START_WAIT: Duration = Duration::from_secs(30);
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Timer {
    /// The wait for the resumed session's `LockedHint` to go false; the number tells one
    /// resume from the next.
    UnlockAck(u32),
    /// The wait for a worker's `Started` after its `Start`.
    Start(WorkerId),
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
    /// logind reports session `id`'s `LockedHint` false after an `UnlockSession`.
    Unlocked {
        id: String,
    },
    PowerDone(Result<(), String>),
    Timer(Timer),
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
    /// logind's `UnlockSession`, answered by `Input::Unlocked`.
    UnlockSession(String),
    Arm {
        timer: Timer,
        after: Duration,
    },
    Remember(Remembered),
    Disallocate(u16),
    Log(log::Level, String),
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
}

/// What the backend reads fresh for each `Hello`, each start and each power request.
#[derive(Clone, Debug, PartialEq)]
pub struct World {
    /// The accounts the login screen lists, as `users::listed` picks them.
    pub users: Vec<UserEntry>,
    pub keyboard: Keyboard,
    /// `VT_GETSTATE`'s mask of open VTs 1-15.
    pub kernel_open: u16,
    /// logind's sessions of class `user` on any seat, text and ssh logins included.
    pub user_sessions: u32,
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

struct Greeter {
    worker: WorkerId,
    /// Its worker reported `Started`.
    started: bool,
    /// Set by the final `Success`: the greeter exits by itself, and that exit counts no failure.
    leaving: bool,
    /// The greeter is wanted again once the leaving one has gone.
    relaunch: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// `asked` while a prompt waits for the greeter's answer.
    Authenticating {
        asked: bool,
    },
    Authenticated,
    /// `Start` went to the worker, which opens the session on `vt`.
    Starting {
        vt: u16,
    },
    /// `UnlockSession` went out; `generation` names its `Timer::UnlockAck`.
    Unlocking {
        generation: u32,
    },
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
    config: LoginConfig,
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
    orphans: Vec<String>,
    after_orphans: AfterOrphans,
    /// Removals of unknown sessions while a start is in flight, with seat0's active session.
    removed_early: Vec<(String, Option<String>)>,
    notice: Option<Notice>,
    next_worker: u32,
    unlocks: u32,
}

impl Daemon {
    pub fn new(config: LoginConfig, remembered: Remembered) -> Daemon {
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
            removed_early: Vec::new(),
            notice: None,
            next_worker: 1,
            unlocks: 0,
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

    /// Rebuilds the session table from logind after a daemon start and ends orphaned greeters.
    pub fn recover(&mut self, _now: Instant, found: Vec<LogindSession>) -> Vec<Effect> {
        let mut out = Vec::new();
        let user_services = [Mode::Login.service(), Mode::Autologin.service()];
        for s in found {
            if s.seat == "seat0" && s.class == "user" && user_services.contains(&&*s.service) {
                self.sessions.insert(SessionEntry {
                    id: s.id,
                    user: s.user,
                    uid: s.uid,
                    vt: s.vt,
                    started: None,
                });
            } else if s.class == "greeter" && s.service == Mode::Greeter.service() {
                self.orphans.push(s.id.clone());
                out.push(Effect::TerminateSession(s.id));
            }
        }
        out
    }

    /// Starts the autologin session or the greeter, unless sessions were recovered. An
    /// orphaned greeter was showing the login screen, so that choice waits until it is gone.
    pub fn start(&mut self, _now: Instant, world: &World) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.orphans.is_empty() {
            self.after_orphans = AfterOrphans::Replace;
            return out;
        }
        if !self.sessions.is_empty() {
            return out;
        }
        self.autologin_or_greeter(world, &mut out);
        out
    }

    /// With no session running: the autologin session if one is configured and can start,
    /// else the greeter.
    fn autologin_or_greeter(&mut self, world: &World, out: &mut Vec<Effect>) {
        if let Some(user) = self.config.autologin_user.clone() {
            let listed = world.users.iter().any(|u| u.name == user);
            let vt = vt::pick(&self.sessions.taken_vts(), world.kernel_open);
            match (listed, vt) {
                (true, Some(vt)) => {
                    let worker = self.next_worker();
                    out.push(Effect::Spawn {
                        worker,
                        job: self.job(Mode::Autologin, &user),
                    });
                    send_start(worker, vt, out);
                    self.autologin = Some(Autologin { worker, user, vt });
                    return;
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
                self.greeter_request(world, request, &mut out)
            }
            Input::GreeterGone(w) if self.is_greeter(w) => self.greeter_gone(&mut out),
            Input::GreeterRequest(..) | Input::GreeterGone(_) => {}
            Input::Worker(worker, msg) => self.worker_message(now, worker, msg, &mut out),
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
            Input::Timer(Timer::UnlockAck(generation)) => {
                let current = matches!(&self.conversation, Some(Conversation {
                    phase: Phase::Unlocking { generation: g },
                    ..
                }) if *g == generation);
                if current {
                    self.finish_resume(true, &mut out);
                }
            }
            Input::Timer(Timer::Start(worker)) => self.start_timed_out(worker, &mut out),
            Input::PowerDone(Ok(())) => out.push(Effect::Reply(GreeterReply::Success)),
            Input::PowerDone(Err(text)) => {
                out.push(Effect::Log(
                    Level::Warn,
                    format!("turning off or restarting: {text}"),
                ));
                out.push(Effect::Reply(error(ErrorKind::Failed, text)));
            }
        }
        if !self.start_in_flight() {
            self.removed_early.clear();
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
                .then(|| self.config.autologin_user.clone())
                .flatten(),
            greeter_socket: mode == Mode::Greeter && self.config.greeter_socket,
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

    fn greeter_request(&mut self, world: &World, request: GreeterRequest, out: &mut Vec<Effect>) {
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
                self.conversation = Some(Conversation {
                    worker: Some(worker),
                    greeter: self.greeter.as_ref().map(|g| g.worker),
                    user,
                    route,
                    phase: Phase::Authenticating { asked: false },
                });
            }
            GreeterRequest::PostAuthResponse { response } => match &mut self.conversation {
                Some(Conversation {
                    worker: Some(worker),
                    phase: phase @ Phase::Authenticating { asked: true },
                    ..
                }) => {
                    *phase = Phase::Authenticating { asked: false };
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
        }
    }

    /// The configured interface, else the last one used; users capped to fit one frame.
    fn greeting(&mut self, world: &World) -> Greeting {
        let notes = self.sessions.notes();
        let mut greeting = Greeting {
            interface: (self.config.interface)
                .or(self.remembered.last_interface)
                .unwrap_or(Interface::Classic),
            show_users: self.config.show_users,
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
            (Phase::Authenticated, Route::Resume { id, .. }) => {
                let id = id.clone();
                let phase = self.unlock(id, out);
                self.conversation.as_mut().expect("conversation").phase = phase;
            }
            (Phase::Authenticated, Route::Start) => {
                let Some(worker) = c.worker else {
                    return self.fail_conversation(ErrorKind::Failed, WORKER_ENDED.into(), out);
                };
                let Some(vt) = vt::pick(&self.sessions.taken_vts(), world.kernel_open) else {
                    return self.fail_conversation(ErrorKind::Failed, NO_VT.into(), out);
                };
                c.phase = Phase::Starting { vt };
                send_start(worker, vt, out);
            }
        }
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
                // Its worker activated `vt` before opening the session.
                if vt != GREETER_VT {
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
                };
                self.session_started(worker, entry, None, out);
            }
            FromWorker::Started { .. } if is_greeter => {
                self.greeter.as_mut().expect("greeter").started = true;
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
                };
                self.session_started(worker, entry, c.greeter, out);
            }
            FromWorker::Failed { kind, text } => {
                c.worker = None;
                self.fail_conversation(kind, text, out);
            }
        }
    }

    /// Routes the login by the account PAM resolved: a uid with a session resumes it, so no
    /// second session opens, and any other account starts one. Root never logs in here, and is
    /// told so in a wrong password's words.
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
        c.phase = Phase::Authenticated;
        out.push(Effect::Reply(GreeterReply::Success));
    }

    /// Enters a new session in the table. When its uid has a session already, it ends the
    /// new one and resumes that one instead, or for autologin, which asked no password, shows
    /// the login screen; when logind removed it already, it is over. `greeter` asked for the
    /// session, `None` for autologin.
    fn session_started(
        &mut self,
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
            });
            return;
        }
        let user = entry.user.clone();
        self.sessions.insert(entry);
        if told {
            out.push(Effect::Reply(GreeterReply::Success));
        }
        if self.remembered.last_user.as_ref() != Some(&user) {
            self.remembered.last_user = Some(user);
            out.push(Effect::Remember(self.remembered.clone()));
        }
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

    /// Ends the conversation with an error and kills its worker if it still runs. A session
    /// that failed to start has taken the console to its VT already, so the login screen
    /// comes back.
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

    /// Gives up on a worker that sent no `Started` within `START_WAIT` of its `Start`: it is
    /// killed and the login screen comes back without waiting for its exit, which may never
    /// come from a worker stuck in PAM.
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
                && matches!(c.phase, Phase::Authenticated | Phase::Unlocking { .. });
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
        if let Some(at) = self.orphans.iter().position(|o| o == id) {
            self.orphans.remove(at);
            if !self.orphans.is_empty() {
                return;
            }
            let seat_taken =
                (seat_active.as_deref()).is_some_and(|a| self.sessions.by_id(a).is_some());
            match std::mem::replace(&mut self.after_orphans, AfterOrphans::Nothing) {
                AfterOrphans::Greeter => self.bring_greeter(out),
                AfterOrphans::Replace if self.sessions.is_empty() => {
                    self.autologin_or_greeter(world, out)
                }
                AfterOrphans::Replace if !seat_taken => self.bring_greeter(out),
                AfterOrphans::Replace | AfterOrphans::Nothing => {}
            }
            return;
        }
        let Some(entry) = self.sessions.remove(id) else {
            if self.start_in_flight() {
                self.removed_early.push((id.into(), seat_active));
            } else if seat_active.is_none() && self.stalled.iter().any(|s| s.vt != GREETER_VT) {
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
        if self.greeter_runs(greeter) {
            out.push(Effect::Reply(GreeterReply::Success));
            self.greeter_leaves();
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
                }
            }
            Some(_) => out.push(Effect::SwitchTo(GREETER_VT)),
            None => self.spawn_greeter(out),
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

fn send_start(worker: WorkerId, vt: u16, out: &mut Vec<Effect>) {
    out.push(Effect::ToWorker {
        worker,
        msg: ToWorker::Start { vt },
    });
    out.push(Effect::Arm {
        timer: Timer::Start(worker),
        after: START_WAIT,
    });
}

#[cfg(test)]
mod tests;
