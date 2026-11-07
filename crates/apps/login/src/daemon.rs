//! The login manager's state machine: inputs from the greeter, the workers, the sessions and
//! logind go in, effects for the backends come out. It does no I/O and reads no clock.

use std::time::{Duration, Instant};

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
use crate::worker::{FromWorker, Mode, ToWorker};

/// A session that ends sooner than this after its start brings the greeter back with a notice.
pub const EARLY_END: Duration = Duration::from_secs(10);
/// How long a resumed session gets to drop its lock before the switch happens anyway.
pub const UNLOCK_WAIT: Duration = Duration::from_secs(1);
/// Most users one `Greeting` lists; fewer when their entries would not fit one frame.
pub const MAX_USERS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorkerId(pub u32);

/// What a worker is spawned for: `reprise-login --worker <mode> <user>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub mode: Mode,
    pub user: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Timer {
    /// The wait for the named session's `LockedHint` to go false.
    UnlockAck(String),
}

#[derive(Debug, PartialEq)]
pub enum Input {
    GreeterRequest(GreeterRequest),
    GreeterGone,
    /// Every message a worker sent before it exited arrives before its `WorkerExited`.
    Worker(WorkerId, FromWorker),
    WorkerExited {
        worker: WorkerId,
        clean: bool,
    },
    Session {
        id: String,
        request: SessionRequest,
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
    /// Closes the worker's socket, which ends its PAM conversation; a worker whose session
    /// runs also gets SIGTERM, which it passes on to the session.
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
    Log(String),
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

/// What the backend reads fresh for each `Hello` and each start.
#[derive(Clone, Debug, PartialEq)]
pub struct World {
    /// The accounts the login screen lists, as `users::listed` picks them.
    pub users: Vec<UserEntry>,
    pub keyboard: Keyboard,
    /// `VT_GETSTATE`'s mask of open VTs 1-15.
    pub kernel_open: u16,
}

const BUSY: &str = "another login is in progress";
const NOT_ASKED: &str = "no question is waiting for an answer";
const NOT_AUTHENTICATED: &str = "the login is not authenticated";
const STARTING: &str = "the session is already starting";
const OTHERS_LOGGED_ON: &str = "other users are logged on";
const NO_VT: &str = "no free VT for a session";
const WORKER_ENDED: &str = "the login ended unexpectedly";
const SESSION_ENDED: &str = "the session ended";

struct Greeter {
    worker: WorkerId,
    /// Set by the final `Success`: the greeter exits by itself, and that exit is clean.
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
    Unlocking,
}

struct Conversation {
    /// `None` once the worker has exited, as a `Reauth` worker does after `Authenticated`.
    worker: Option<WorkerId>,
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
    id: String,
    vt: u16,
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
    /// Greeter sessions of an earlier daemon, terminated and not yet removed.
    orphans: Vec<String>,
    /// The greeter starts once every orphan is removed.
    greeter_after_orphans: bool,
    notice: Option<Notice>,
    next_worker: u32,
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
            orphans: Vec::new(),
            greeter_after_orphans: false,
            notice: None,
            next_worker: 1,
        }
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
    /// orphaned greeter was showing the login screen, so a new one replaces it once it is gone.
    pub fn start(&mut self, _now: Instant, world: &World) -> Vec<Effect> {
        let mut out = Vec::new();
        if !self.orphans.is_empty() {
            self.greeter_after_orphans = true;
            return out;
        }
        if !self.sessions.is_empty() {
            return out;
        }
        if let Some(user) = self.config.autologin_user.clone() {
            let listed = world.users.iter().any(|u| u.name == user);
            let vt = vt::pick(&self.sessions.taken_vts(), world.kernel_open);
            match (listed, vt) {
                (true, Some(vt)) => {
                    let worker = self.next_worker();
                    out.push(Effect::Spawn {
                        worker,
                        job: Job {
                            mode: Mode::Autologin,
                            user: user.clone(),
                        },
                    });
                    out.push(Effect::ToWorker {
                        worker,
                        msg: ToWorker::Start { vt },
                    });
                    self.autologin = Some(Autologin { worker, user, vt });
                    return out;
                }
                (true, None) => out.push(Effect::Log(format!("autologin of {user}: {NO_VT}"))),
                (false, _) => out.push(Effect::Log(format!(
                    "autologin_user {user:?} is not an account the login screen lists; \
                     showing the login screen"
                ))),
            }
        }
        self.spawn_greeter(&mut out);
        out
    }

    pub fn handle(&mut self, now: Instant, world: &World, input: Input) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::GreeterRequest(request) => self.greeter_request(world, request, &mut out),
            Input::GreeterGone => self.greeter_gone(&mut out),
            Input::Worker(worker, msg) => self.worker_message(now, worker, msg, &mut out),
            Input::WorkerExited { worker, clean } => self.worker_exited(worker, clean, &mut out),
            Input::Session { id, request } => self.session_request(&id, request, &mut out),
            Input::SessionRemoved { id, seat_active } => {
                self.session_removed(now, &id, seat_active, &mut out)
            }
            Input::Unlocked { id } => self.unlocked(&id, false, &mut out),
            Input::Timer(Timer::UnlockAck(id)) => self.unlocked(&id, true, &mut out),
            Input::PowerDone(result) => out.push(Effect::Reply(match result {
                Ok(()) => GreeterReply::Success,
                Err(text) => error(ErrorKind::Failed, text),
            })),
        }
        out
    }

    fn next_worker(&mut self) -> WorkerId {
        let id = WorkerId(self.next_worker);
        self.next_worker += 1;
        id
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
                    job: Job {
                        mode,
                        user: user.clone(),
                    },
                });
                self.conversation = Some(Conversation {
                    worker: Some(worker),
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
                Some(Phase::Starting { .. } | Phase::Unlocking) => out.push(refused(STARTING)),
                _ => {
                    self.drop_conversation(out);
                    out.push(Effect::Reply(GreeterReply::Success));
                }
            },
            GreeterRequest::Power { action } => {
                if self.sessions.is_empty() {
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
            (Phase::Starting { .. } | Phase::Unlocking, _) => out.push(refused(STARTING)),
            (Phase::Authenticating { .. }, _) => out.push(refused(NOT_AUTHENTICATED)),
            (Phase::Authenticated, Route::Resume { id, .. }) => {
                let id = id.clone();
                c.phase = Phase::Unlocking;
                unlock(id, out);
            }
            (Phase::Authenticated, Route::Start) => {
                let (Some(worker), Some(vt)) = (
                    c.worker,
                    vt::pick(&self.sessions.taken_vts(), world.kernel_open),
                ) else {
                    return self.fail_conversation(ErrorKind::Failed, NO_VT.into(), out);
                };
                c.phase = Phase::Starting { vt };
                out.push(Effect::ToWorker {
                    worker,
                    msg: ToWorker::Start { vt },
                });
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
            FromWorker::Started { session_id, uid } if autologin => {
                let a = self.autologin.take().expect("autologin worker");
                let entry = SessionEntry {
                    id: session_id,
                    user: a.user,
                    uid,
                    vt: a.vt,
                    started: Some(now),
                };
                self.session_started(worker, entry, out);
            }
            FromWorker::Failed { text, .. } if is_greeter => out.push(Effect::Log(format!(
                "the login screen did not start: {text}"
            ))),
            FromWorker::Failed { text, .. } if autologin => {
                out.push(Effect::Log(format!("autologin did not start: {text}")))
            }
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
            FromWorker::Authenticated => {
                if let Phase::Authenticating { .. } = c.phase {
                    c.phase = Phase::Authenticated;
                    out.push(Effect::Reply(GreeterReply::Success));
                }
            }
            FromWorker::Started { session_id, uid } => {
                let Phase::Starting { vt } = c.phase else {
                    return;
                };
                let c = self.conversation.take().expect("conversation");
                let entry = SessionEntry {
                    id: session_id,
                    user: c.user,
                    uid,
                    vt,
                    started: Some(now),
                };
                self.session_started(worker, entry, out);
            }
            FromWorker::Failed { kind, text } => self.fail_conversation(kind, text, out),
        }
    }

    /// Enters a new session in the table, or, when its uid has a session already, ends the
    /// new one and resumes that one instead.
    fn session_started(&mut self, worker: WorkerId, entry: SessionEntry, out: &mut Vec<Effect>) {
        self.session_workers.push(SessionWorker {
            worker,
            id: entry.id.clone(),
            vt: entry.vt,
        });
        if let Some(existing) = self.sessions.by_uid(entry.uid) {
            let route = Route::Resume {
                id: existing.id.clone(),
                vt: existing.vt,
            };
            out.push(Effect::TerminateSession(entry.id));
            unlock(existing.id.clone(), out);
            self.conversation = Some(Conversation {
                worker: None,
                user: entry.user,
                route,
                phase: Phase::Unlocking,
            });
            return;
        }
        let user = entry.user.clone();
        self.sessions.insert(entry);
        out.push(Effect::Reply(GreeterReply::Success));
        if self.remembered.last_user.as_ref() != Some(&user) {
            self.remembered.last_user = Some(user);
            out.push(Effect::Remember(self.remembered.clone()));
        }
        self.runs.reset();
        self.greeter_leaves();
    }

    /// Ends the conversation with an error; a session that failed to start has taken the
    /// console to its VT already, so the login screen comes back.
    fn fail_conversation(&mut self, kind: ErrorKind, text: String, out: &mut Vec<Effect>) {
        let Some(c) = self.conversation.take() else {
            return;
        };
        out.push(Effect::Reply(error(kind, text)));
        if let Phase::Starting { .. } = c.phase {
            self.bring_greeter(out);
        }
    }

    /// Ends a conversation that has not reached `Start`, closing its worker's socket.
    fn drop_conversation(&mut self, out: &mut Vec<Effect>) {
        let Some(c) = &self.conversation else {
            return;
        };
        if let Phase::Starting { .. } | Phase::Unlocking = c.phase {
            return;
        }
        if let Some(worker) = c.worker {
            out.push(Effect::Kill(worker));
        }
        self.conversation = None;
    }

    fn greeter_gone(&mut self, out: &mut Vec<Effect>) {
        self.drop_conversation(out);
        if let Some(g) = self.greeter.as_ref().filter(|g| !g.leaving) {
            out.push(Effect::Kill(g.worker));
        }
    }

    fn worker_exited(&mut self, worker: WorkerId, clean: bool, out: &mut Vec<Effect>) {
        if self.greeter.as_ref().is_some_and(|g| g.worker == worker) {
            return self.greeter_exited(clean, out);
        }
        if let Some(c) = self
            .conversation
            .as_mut()
            .filter(|c| c.worker == Some(worker))
        {
            c.worker = None;
            let reauthed = matches!(c.route, Route::Resume { .. })
                && matches!(c.phase, Phase::Authenticated | Phase::Unlocking);
            if !reauthed {
                self.fail_conversation(ErrorKind::Failed, WORKER_ENDED.into(), out);
            }
            return;
        }
        if self.autologin.as_ref().is_some_and(|a| a.worker == worker) {
            let a = self.autologin.take().expect("autologin worker");
            out.push(Effect::Log(format!(
                "autologin of {}: {WORKER_ENDED}",
                a.user
            )));
            return self.bring_greeter(out);
        }
        if let Some(at) = self.session_workers.iter().position(|s| s.worker == worker) {
            out.push(Effect::Disallocate(self.session_workers.remove(at).vt));
        }
    }

    fn greeter_exited(&mut self, clean: bool, out: &mut Vec<Effect>) {
        let g = self.greeter.take().expect("greeter");
        self.drop_conversation(out);
        if g.leaving {
            self.runs.ended(true);
            if g.relaunch {
                self.spawn_greeter(out);
            }
            return;
        }
        match self.runs.ended(clean) {
            AfterGreeter::TextLogin => {
                out.push(Effect::SwitchTo(TEXT_LOGIN_VT));
                out.push(Effect::Log(format!(
                    "the login screen failed {MAX_FAILURES} times in a row; \
                     the text login on VT {TEXT_LOGIN_VT} takes over"
                )));
            }
            AfterGreeter::Again if self.conversation.is_some() => {}
            AfterGreeter::Again => self.spawn_greeter(out),
        }
    }

    fn session_request(&mut self, id: &str, request: SessionRequest, out: &mut Vec<Effect>) {
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
            SessionRequest::SwitchUser => self.bring_greeter(out),
        }
    }

    fn session_removed(
        &mut self,
        now: Instant,
        id: &str,
        seat_active: Option<String>,
        out: &mut Vec<Effect>,
    ) {
        if let Some(at) = self.orphans.iter().position(|o| o == id) {
            self.orphans.remove(at);
            if self.orphans.is_empty() && std::mem::take(&mut self.greeter_after_orphans) {
                self.bring_greeter(out);
            }
            return;
        }
        let Some(entry) = self.sessions.remove(id) else {
            return;
        };
        let resuming = self.conversation.as_ref().is_some_and(
            |c| matches!(&c.route, Route::Resume { id: resumed, .. } if resumed == id),
        );
        if resuming {
            self.conversation.take();
            out.push(Effect::Reply(error(
                ErrorKind::Failed,
                SESSION_ENDED.into(),
            )));
        }
        if !self.session_workers.iter().any(|s| s.id == id) {
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

    /// Ends a resume once the session dropped its lock, or once `UNLOCK_WAIT` ran out.
    fn unlocked(&mut self, id: &str, timed_out: bool, out: &mut Vec<Effect>) {
        let Some(Conversation {
            route: Route::Resume { id: resumed, vt },
            phase: Phase::Unlocking,
            ..
        }) = &self.conversation
        else {
            return;
        };
        if resumed != id {
            return;
        }
        let vt = *vt;
        if timed_out {
            out.push(Effect::Log(format!(
                "session {id} did not unlock within {UNLOCK_WAIT:?}; switching to it anyway"
            )));
        }
        out.push(Effect::SwitchTo(vt));
        out.push(Effect::Reply(GreeterReply::Success));
        self.conversation = None;
        self.greeter_leaves();
    }

    fn greeter_leaves(&mut self) {
        if let Some(g) = &mut self.greeter {
            g.leaving = true;
        }
    }

    /// Shows the login screen: switches to a running greeter or starts one.
    fn bring_greeter(&mut self, out: &mut Vec<Effect>) {
        match &mut self.greeter {
            Some(g) if g.leaving => g.relaunch = true,
            Some(_) => out.push(Effect::SwitchTo(GREETER_VT)),
            None => self.spawn_greeter(out),
        }
    }

    fn spawn_greeter(&mut self, out: &mut Vec<Effect>) {
        let worker = self.next_worker();
        out.push(Effect::Spawn {
            worker,
            job: Job {
                mode: Mode::Greeter,
                user: GREETER_USER.into(),
            },
        });
        out.push(Effect::ToWorker {
            worker,
            msg: ToWorker::Start { vt: GREETER_VT },
        });
        self.greeter = Some(Greeter {
            worker,
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

fn unlock(id: String, out: &mut Vec<Effect>) {
    out.push(Effect::UnlockSession(id.clone()));
    out.push(Effect::Arm {
        timer: Timer::UnlockAck(id),
        after: UNLOCK_WAIT,
    });
}

#[cfg(test)]
mod tests;
