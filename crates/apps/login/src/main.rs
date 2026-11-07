use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reprise_login::config::{CONFIG_PATH, LoginConfig};
use reprise_login::daemon::{
    Daemon, Effect, Input, Job, LogindSession, UNLOCK_WAIT, WorkerId, World,
};
use reprise_login::ipc::{self, Gate, Paths, Server};
use reprise_login::logind::{self, Logind, SessionRemovedStream};
use reprise_login::remembered::{Remembered, STATE_PATH};
use reprise_login::vt::{self, GREETER_VT};
use reprise_login::worker::{FromWorker, LOG_FD, Mode, ToWorker, worker_main};
use reprise_login::{pidfd, users};
use repriselogin::GREETER_USER;
use repriselogin::frame::{self, FrameError};
use repriselogin::greeter::GreeterRequest;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

const USAGE: &str = "\
Usage: reprise-login

Reprise's login manager. It runs as root from reprise-login.service.
";

/// The worker socket the daemon passes to `--worker`.
const WORKER_FD: i32 = 3;
/// How long a worker's socket may stay open after the worker exited, held by a child of it.
const EOF_GRACE: Duration = Duration::from_secs(1);
/// How long remembered state waits before it is written, so a burst of changes is one write.
const REMEMBER_DELAY: Duration = Duration::from_millis(500);
/// How often, and how many times, a recovered session whose leader exited is checked for
/// logind marking it closing.
const LEADER_POLL: Duration = Duration::from_millis(500);
const LEADER_POLLS: u32 = 10;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_] => daemon(),
        [_, flag, mode] if flag == "--worker" => std::process::exit(worker(mode)),
        [_, flag] if flag == "--help" || flag == "-h" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn worker(mode: &str) -> i32 {
    // SAFETY: F_SETFD only changes the flags of fd LOG_FD, which nothing else here owns.
    match unsafe { libc::fcntl(LOG_FD, libc::F_SETFD, libc::FD_CLOEXEC) } {
        0 => reprisebus::journal::init_logger_to(
            reprisebus::journal::PROGRAM_FILTER,
            // SAFETY: fd LOG_FD is open, and the logger owns it for the process's life.
            unsafe { std::fs::File::from_raw_fd(LOG_FD) },
        ),
        _ => reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER),
    }
    // Its stderr is /dev/null, so a panic reaches the daemon only through the log.
    reprisebus::journal::log_panics();
    let Some(mode) = Mode::parse(mode) else {
        log::error!("unknown worker mode {mode:?}");
        return 2;
    };
    // SAFETY: fstat fills a local struct; F_SETFD only changes fd 3's flags.
    let is_socket = unsafe {
        let mut st: libc::stat = std::mem::zeroed();
        libc::fstat(WORKER_FD, &mut st) == 0
            && st.st_mode & libc::S_IFMT == libc::S_IFSOCK
            && libc::fcntl(WORKER_FD, libc::F_SETFD, libc::FD_CLOEXEC) == 0
    };
    if !is_socket {
        log::error!("--worker needs its socket on fd {WORKER_FD}");
        return 2;
    }
    // SAFETY: fd 3 is an open socket nothing else in this process owns.
    let sock = unsafe { UnixStream::from_raw_fd(WORKER_FD) };
    worker_main(mode, sock)
}

fn daemon() -> ExitCode {
    reprisebus::journal::init_logger(reprisebus::journal::PROGRAM_FILTER);
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("no runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let local = tokio::task::LocalSet::new();
    match local.block_on(&rt, run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// What the loop reacts to, in the order it happened.
enum Event {
    Input(Input),
    /// The worker's socket closed; every frame it sent came before this.
    WorkerEof(WorkerId),
    /// A SIGCHLD: some worker may have exited.
    Reap,
    /// The worker exited `EOF_GRACE` ago and its socket is still open.
    EofLate(WorkerId),
    /// A worker that never ran.
    Exited(Exited),
    /// `REMEMBER_DELAY` passed since remembered state changed.
    Save,
    Stop,
    /// The daemon can no longer follow logind; it exits so systemd restarts it.
    Lost(String),
}

type Events = mpsc::UnboundedSender<Event>;

async fn run() -> Result<(), String> {
    let config = LoginConfig::load(Path::new(CONFIG_PATH));
    let remembered = Remembered::load(Path::new(STATE_PATH));
    let greeter = match users::lookup(GREETER_USER) {
        Ok(Some(account)) => account,
        _ => return Err(format!("there is no {GREETER_USER} account")),
    };
    let logind = (Logind::connect().await).map_err(|e| format!("logind: {e}"))?;
    let (events, mut rx) = mpsc::unbounded_channel();
    // Subscribed before the sessions are listed, so no removal falls between.
    let removed = (logind.manager().receive_session_removed().await)
        .map_err(|e| format!("logind's SessionRemoved: {e}"))?;
    let found = (logind.sessions().await).map_err(|e| format!("logind's sessions: {e}"))?;
    let seat_active = logind.seat_active().await;
    // Registered before the first spawn, so no worker's exit goes unseen.
    follow_signals(events.clone()).map_err(|e| format!("signals: {e}"))?;
    // Held for the daemon's life: with VT 1 busy, logind never starts autovt@tty1, which is
    // getty@tty1 and would stop this unit through its Conflicts=.
    let _greeter_vt = (vt::hold(GREETER_VT))
        .map_err(|e| log::warn!("/dev/tty{GREETER_VT}: {e}"))
        .ok();
    follow_console(events.clone());

    let gate = Rc::new(RefCell::new(Gate::new(greeter.uid)));
    let mut rt = Runtime {
        daemon: Daemon::new(config, remembered),
        logind: logind.clone(),
        gate: gate.clone(),
        workers: Workers::new(PathBuf::from("/proc/self/exe"), events.clone()),
        saver: Saver::new(PathBuf::from(STATE_PATH), events.clone()),
        events: events.clone(),
        world: None,
    };
    let world = rt.read_world().await;
    let effects = boot(&mut rt.daemon, found, seat_active.as_deref(), &world);
    let recovered: Vec<String> = rt.daemon.session_uids().into_keys().collect();
    rt.world = Some(world);

    let (greeter_l, session_l) = (ipc::bind(&Paths::default(), greeter.gid))
        .map_err(|e| format!("binding the sockets: {e}"))?;
    let (lookup, seat) = (logind.clone(), logind.clone());
    let (input, mut sockets) = mpsc::channel(ipc::QUEUE);
    let server = Server {
        gate,
        session_of: Rc::new(move |pid| {
            let l = lookup.clone();
            Box::pin(async move { l.session_of(pid).await })
        }),
        seat_active: Rc::new(move || {
            let l = seat.clone();
            Box::pin(async move { l.seat_active().await })
        }),
        input,
    };
    ipc::serve(greeter_l, session_l, server).map_err(|e| format!("the sockets: {e}"))?;
    for id in recovered {
        tokio::task::spawn_local(follow_leader(id, logind.clone(), events.clone()));
    }
    tokio::task::spawn_local(follow_removals(removed, logind, events));
    rt.run_effects(effects);
    rt.sync_gate();

    loop {
        let event = tokio::select! {
            Some(event) = rx.recv() => event,
            Some(input) = sockets.recv() => Event::Input(input),
            else => return Ok(()),
        };
        match event {
            Event::Input(input) => rt.input(input).await,
            Event::WorkerEof(w) => {
                if let Some(exit) = rt.workers.eof(w) {
                    rt.exited(exit).await;
                }
            }
            Event::Reap => {
                for exit in rt.workers.reap() {
                    rt.exited(exit).await;
                }
            }
            Event::EofLate(w) => {
                if let Some(exit) = rt.workers.eof_late(w) {
                    rt.exited(exit).await;
                }
            }
            Event::Exited(exit) => rt.exited(exit).await,
            Event::Save => rt.saver.save(),
            Event::Stop => {
                log::info!("stopping; user sessions keep running");
                rt.workers.kill_greeters();
                rt.saver.save();
                return Ok(());
            }
            Event::Lost(why) => {
                rt.workers.kill_greeters();
                rt.saver.save();
                return Err(why);
            }
        }
    }
}

/// Rebuilds the table from logind's sessions and starts the login screen or autologin, with
/// no event in between: a removal handled before `start` could leave VT 1 empty.
fn boot(
    daemon: &mut Daemon,
    found: Vec<LogindSession>,
    seat_active: Option<&str>,
    world: &World,
) -> Vec<Effect> {
    let now = Instant::now();
    let mut effects = daemon.recover(now, found, seat_active);
    effects.extend(daemon.start(now, world));
    effects
}

async fn follow_removals(mut removed: SessionRemovedStream, logind: Logind, events: Events) {
    while let Some(signal) = removed.next().await {
        let Ok(args) = signal.args() else { continue };
        let id = args.id().clone();
        let seat_active = logind.seat_active().await;
        let _ = events.send(Event::Input(Input::SessionRemoved { id, seat_active }));
    }
    let _ = events.send(Event::Lost(
        "logind's SessionRemoved signal stream ended".into(),
    ));
}

/// Ends recovered session `id` once its leader exits: logind then keeps it closing, with no
/// `SessionRemoved`, while any process of it runs.
async fn follow_leader(id: String, logind: Logind, events: Events) {
    let Some(pid) = logind.leader(&id).await else {
        return;
    };
    match pidfd::open(pid) {
        // The leader may have exited, and its pid gone to another process, before the open.
        Ok(_) if logind.ended(&id).await => {}
        Ok(fd) => {
            if let Err(e) = pidfd::exited(fd).await {
                return log::warn!("session {id}: watching its leader {pid}: {e}");
            }
        }
        Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {}
        Err(e) => return log::warn!("session {id}: watching its leader {pid}: {e}"),
    }
    for _ in 0..LEADER_POLLS {
        if logind.ended(&id).await {
            let seat_active = logind.seat_active().await;
            let _ = events.send(Event::Input(Input::SessionRemoved { id, seat_active }));
            return;
        }
        tokio::time::sleep(LEADER_POLL).await;
    }
}

/// Feeds `ConsoleOnGreeterVt` each time the console moves to VT 1, from a thread: the
/// kernel wakes a `poll` on a sysfs attribute, which tokio's reactor does not wait on.
fn follow_console(events: Events) {
    let spawned = std::thread::Builder::new()
        .name("console".into())
        .spawn(move || {
            let followed = vt::follow_active(|vt| {
                vt != GREETER_VT || events.send(Event::Input(Input::ConsoleOnGreeterVt)).is_ok()
            });
            if let Err(e) = followed {
                log::warn!("following the console's VT: {e}");
            }
        });
    if let Err(e) = spawned {
        log::warn!("no thread to follow the console's VT: {e}");
    }
}

/// SIGCHLD reaps workers; SIGTERM and SIGINT stop the daemon.
fn follow_signals(events: Events) -> io::Result<()> {
    let mut child = signal(SignalKind::child())?;
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    tokio::task::spawn_local(async move {
        loop {
            let event = tokio::select! {
                Some(()) = child.recv() => Event::Reap,
                Some(()) = term.recv() => Event::Stop,
                Some(()) = int.recv() => Event::Stop,
            };
            if events.send(event).is_err() {
                return;
            }
        }
    });
    Ok(())
}

struct Runtime {
    daemon: Daemon,
    logind: Logind,
    gate: Rc<RefCell<Gate>>,
    workers: Workers,
    saver: Saver,
    events: Events,
    /// The last world read; `None` before the first.
    world: Option<World>,
}

impl Runtime {
    async fn input(&mut self, input: Input) {
        if let Input::Worker(w, FromWorker::Started { session_id, .. }) = &input {
            self.workers.started(*w, session_id);
            self.gate.borrow_mut().greeter_started(*w, session_id);
        }
        let fresh = matches!(
            input,
            Input::GreeterRequest(
                _,
                GreeterRequest::Hello | GreeterRequest::StartSession | GreeterRequest::Power { .. }
            )
        );
        if fresh || self.world.is_none() {
            self.world = Some(self.read_world().await);
        }
        let world = self.world.as_ref().expect("a world was read");
        let effects = self.daemon.handle(Instant::now(), world, input);
        self.run_effects(effects);
        self.sync_gate();
    }

    /// Hands the state machine a worker's exit. The console leaves a timed-out worker's VT,
    /// and a session worker whose logind session is closing or gone ends that session.
    async fn exited(&mut self, exit: Exited) {
        self.gate.borrow_mut().greeter_exited(exit.id);
        let stalled = self.daemon.stalled_vt(exit.id);
        self.input(Input::WorkerExited(exit.id)).await;
        if stalled.is_some_and(|v| on_dead_vt(v, vt::active())) {
            log::warn!("a timed-out login left the console on its VT; showing the login screen");
            self.switch_to(GREETER_VT);
        }
        if let Some(id) = exit.session.filter(|_| exit.mode != Mode::Greeter)
            && self.logind.ended(&id).await
        {
            let seat_active = self.logind.seat_active().await;
            self.input(Input::SessionRemoved { id, seat_active }).await;
        }
    }

    fn sync_gate(&self) {
        self.gate
            .borrow_mut()
            .set_sessions(self.daemon.session_uids());
    }

    async fn read_world(&self) -> World {
        let (user_sessions, login_sessions) = match self.logind.listed().await {
            Ok(found) => {
                let open = found
                    .iter()
                    .filter(|(s, closing)| s.class == "user" && !closing);
                let count = open.count() as u32;
                (count, found.into_iter().map(|(s, _)| s).collect())
            }
            Err(e) => {
                // Counted as one, so power stays refused while logind can't say.
                log::warn!("logind's sessions: {e}");
                (1, Vec::new())
            }
        };
        let kernel_open = vt::open_mask().unwrap_or_else(|e| {
            log::warn!("VT state: {e}");
            0
        });
        World {
            users: users::read_listed(),
            keyboard: self.logind.keyboard().await,
            kernel_open,
            user_sessions,
            login_sessions,
        }
    }

    fn run_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            self.run(effect);
        }
    }

    fn run(&mut self, effect: Effect) {
        match effect {
            Effect::Reply(reply) => {
                if let Some(greeter) = self.gate.borrow_mut().reply(&reply) {
                    let _ = (self.events).send(Event::Input(Input::GreeterGone(greeter)));
                }
            }
            Effect::Spawn { worker, job } => {
                if job.mode == Mode::Greeter {
                    self.gate.borrow_mut().greeter_spawned(worker);
                }
                self.workers.spawn(worker, &job);
            }
            Effect::ToWorker { worker, msg } => self.workers.send(worker, &msg),
            Effect::Kill(worker) => self.workers.kill(worker),
            Effect::SwitchTo(vt) => self.switch_to(vt),
            Effect::Power(action) => {
                let (l, events) = (self.logind.clone(), self.events.clone());
                tokio::task::spawn_local(async move {
                    let done = l.power(action).await.map_err(|e| e.to_string());
                    let _ = events.send(Event::Input(Input::PowerDone(done)));
                });
            }
            Effect::TerminateSession(id) => {
                let (l, events) = (self.logind.clone(), self.events.clone());
                tokio::task::spawn_local(async move {
                    if let Err(e) = l.manager().terminate_session(&id).await {
                        if logind::session_gone(&e) {
                            log::info!("terminating session {id}: already gone");
                        } else {
                            log::warn!("terminating session {id}: {e}");
                        }
                        let seat_active = l.seat_active().await;
                        let failed = Input::TerminateFailed { id, seat_active };
                        let _ = events.send(Event::Input(failed));
                    }
                });
            }
            Effect::StopSession(id) => {
                let l = self.logind.clone();
                tokio::task::spawn_local(async move {
                    if let Err(e) = l.stop_session(&id).await {
                        log::warn!("stopping session {id}: {e}");
                    }
                });
            }
            Effect::LockSession(id) => {
                let l = self.logind.clone();
                tokio::task::spawn_local(async move {
                    if let Err(e) = l.manager().lock_session(&id).await {
                        log::warn!("locking session {id}: {e}");
                    }
                });
            }
            Effect::UnlockSession(id) => {
                let (l, events) = (self.logind.clone(), self.events.clone());
                tokio::task::spawn_local(async move {
                    match l.unlock(&id, UNLOCK_WAIT).await {
                        Ok(true) => {
                            let _ = events.send(Event::Input(Input::Unlocked { id }));
                        }
                        Ok(false) => {}
                        Err(e) => log::warn!("unlocking session {id}: {e}"),
                    }
                });
            }
            Effect::Arm { timer, after } => {
                let events = self.events.clone();
                tokio::task::spawn_local(async move {
                    tokio::time::sleep(after).await;
                    let _ = events.send(Event::Input(Input::Timer(timer)));
                });
            }
            Effect::Remember(remembered) => self.saver.remember(remembered),
            Effect::Disallocate(vt) => vt::disallocate(vt),
            Effect::Log(level, text) => log::log!(level, "{text}"),
        }
    }

    fn switch_to(&self, vt: u16) {
        let (l, events) = (self.logind.clone(), self.events.clone());
        tokio::task::spawn_local(async move {
            if let Err(e) = l.switch_to(vt).await {
                log::warn!("switching to VT {vt}: {e}");
                let _ = events.send(Event::Input(Input::SwitchFailed { vt }));
            }
        });
    }
}

/// Whether the console still shows the VT of a worker whose start timed out. The login
/// screen's own VT never counts.
fn on_dead_vt(stalled_vt: u16, active: io::Result<u16>) -> bool {
    stalled_vt != GREETER_VT && active.is_ok_and(|a| a == stalled_vt)
}

/// Writes remembered state `REMEMBER_DELAY` after it changes, the latest value winning.
struct Saver {
    path: PathBuf,
    events: Events,
    pending: Option<Remembered>,
    armed: bool,
}

impl Saver {
    fn new(path: PathBuf, events: Events) -> Saver {
        Saver {
            path,
            events,
            pending: None,
            armed: false,
        }
    }

    fn remember(&mut self, remembered: Remembered) {
        self.pending = Some(remembered);
        if !std::mem::replace(&mut self.armed, true) {
            let events = self.events.clone();
            tokio::task::spawn_local(async move {
                tokio::time::sleep(REMEMBER_DELAY).await;
                let _ = events.send(Event::Save);
            });
        }
    }

    /// Writes what is pending, if anything.
    fn save(&mut self) {
        self.armed = false;
        let Some(remembered) = self.pending.take() else {
            return;
        };
        if let Err(e) = remembered.save(&self.path) {
            log::warn!("{}: {e}", self.path.display());
        }
    }
}

/// A worker process and the daemon's end of its socket.
struct Worker {
    child: Child,
    mode: Mode,
    /// The outbound queue; `None` once the socket is closed.
    tx: Option<mpsc::UnboundedSender<Zeroizing<Vec<u8>>>>,
    reader: Option<tokio::task::JoinHandle<()>>,
    eof: bool,
    exited: bool,
    session: Option<String>,
}

/// A worker that exited, its socket drained.
#[derive(Debug, PartialEq)]
struct Exited {
    id: WorkerId,
    mode: Mode,
    session: Option<String>,
}

struct Workers {
    program: PathBuf,
    events: Events,
    running: HashMap<WorkerId, Worker>,
}

impl Workers {
    fn new(program: PathBuf, events: Events) -> Workers {
        Workers {
            program,
            events,
            running: HashMap::new(),
        }
    }

    /// Runs `program --worker <mode>` with a socket on fd 3 whose first frame is `Begin`. A
    /// worker that cannot start is reported as exited.
    fn spawn(&mut self, id: WorkerId, job: &Job) {
        let started = start(&self.program, job).and_then(|(child, stream)| {
            stream.set_nonblocking(true)?;
            Ok((child, tokio::net::UnixStream::from_std(stream)?))
        });
        let (child, stream) = match started {
            Ok(started) => started,
            Err(e) => {
                log::error!("a {} worker did not start: {e}", job.mode.as_arg());
                let exit = Exited {
                    id,
                    mode: job.mode,
                    session: None,
                };
                let _ = self.events.send(Event::Exited(exit));
                return;
            }
        };
        let (rd, wr) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel();
        tokio::task::spawn_local(ipc::write_frames(wr, async move || rx.recv().await));
        let reader = tokio::task::spawn_local(read_worker(id, rd, self.events.clone()));
        self.running.insert(
            id,
            Worker {
                child,
                mode: job.mode,
                tx: Some(tx),
                reader: Some(reader),
                eof: false,
                exited: false,
                session: None,
            },
        );
        let begin = ToWorker::Begin {
            user: job.user.clone(),
            autologin_user: job.autologin_user.clone(),
            greeter_socket: job.greeter_socket,
        };
        self.send(id, &begin);
    }

    fn send(&self, id: WorkerId, msg: &ToWorker) {
        let Some(tx) = self.running.get(&id).and_then(|w| w.tx.as_ref()) else {
            return;
        };
        match frame::encode(msg) {
            Ok(f) => {
                let _ = tx.send(f);
            }
            Err(e) => log::warn!("worker {}: {}", id.0, ipc::error_kind(&e)),
        }
    }

    /// Closes the worker's socket and sends it SIGTERM.
    fn kill(&mut self, id: WorkerId) {
        let Some(w) = self.running.get_mut(&id) else {
            return;
        };
        w.tx = None;
        if let Some(reader) = w.reader.take() {
            reader.abort();
        }
        w.eof = true;
        if w.exited {
            return;
        }
        let Ok(pid) = libc::pid_t::try_from(w.child.id()) else {
            return;
        };
        // SAFETY: the child is not reaped yet, so the pid is still this worker's.
        if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
            log::warn!("SIGTERM to worker {pid}: {}", io::Error::last_os_error());
        }
    }

    fn kill_greeters(&mut self) {
        let greeters: Vec<_> = (self.running.iter())
            .filter(|(_, w)| w.mode == Mode::Greeter)
            .map(|(id, _)| *id)
            .collect();
        for id in greeters {
            self.kill(id);
        }
    }

    fn started(&mut self, id: WorkerId, session: &str) {
        if let Some(w) = self.running.get_mut(&id) {
            w.session = Some(session.to_owned());
        }
    }

    fn eof(&mut self, id: WorkerId) -> Option<Exited> {
        let w = self.running.get_mut(&id)?;
        w.eof = true;
        w.reader = None;
        self.finish(id)
    }

    fn eof_late(&mut self, id: WorkerId) -> Option<Exited> {
        let w = self.running.get_mut(&id)?;
        if let Some(reader) = w.reader.take() {
            log::warn!("worker {}: its socket outlived it", id.0);
            reader.abort();
        }
        w.eof = true;
        self.finish(id)
    }

    /// Every worker that exited and whose socket is drained.
    fn reap(&mut self) -> Vec<Exited> {
        let mut done = Vec::new();
        for (id, w) in &mut self.running {
            if !w.exited && matches!(w.child.try_wait(), Ok(Some(_))) {
                w.exited = true;
                if !w.eof {
                    let (id, events) = (*id, self.events.clone());
                    tokio::task::spawn_local(async move {
                        tokio::time::sleep(EOF_GRACE).await;
                        let _ = events.send(Event::EofLate(id));
                    });
                }
            }
            if w.exited && w.eof {
                done.push(*id);
            }
        }
        done.into_iter().filter_map(|id| self.finish(id)).collect()
    }

    fn finish(&mut self, id: WorkerId) -> Option<Exited> {
        if !self.running.get(&id).is_some_and(|w| w.exited && w.eof) {
            return None;
        }
        let w = self.running.remove(&id)?;
        Some(Exited {
            id,
            mode: w.mode,
            session: w.session,
        })
    }
}

/// Forks `program --worker <mode>` with the child's end of a socket pair on fd 3 and the
/// daemon's stderr on `LOG_FD`.
fn start(program: &Path, job: &Job) -> io::Result<(Child, UnixStream)> {
    let (ours, theirs) = UnixStream::pair()?;
    let log_stream = io::stderr().as_fd().try_clone_to_owned()?;
    let (sock, log) = (theirs.as_raw_fd(), log_stream.as_raw_fd());
    let mut cmd = Command::new(program);
    cmd.arg("--worker")
        .arg(job.mode.as_arg())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the closure runs in the child between fork and exec and calls only fcntl and
    // dup2, which are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            // Copied above both targets first, so neither dup2 closes the other's source; the
            // CLOEXEC copies end at exec, and dup2's targets never carry CLOEXEC.
            let high = |fd| libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, LOG_FD + 1);
            let (sock, log) = (high(sock), high(log));
            let failed = sock < 0
                || log < 0
                || libc::dup2(sock, WORKER_FD) < 0
                || libc::dup2(log, LOG_FD) < 0;
            match failed {
                true => Err(io::Error::last_os_error()),
                false => Ok(()),
            }
        })
    };
    let child = cmd.spawn()?;
    drop((theirs, log_stream));
    Ok((child, ours))
}

async fn read_worker(id: WorkerId, rd: tokio::net::unix::OwnedReadHalf, events: Events) {
    let mut frames = ipc::Frames::new(rd);
    let why = loop {
        match frames.next().await.and_then(|body| frame::decode(&body)) {
            Ok(msg) => {
                let _ = events.send(Event::Input(Input::Worker(id, msg)));
            }
            Err(e) => break e,
        }
    };
    if why != FrameError::Closed {
        log::warn!("worker {}: its socket: {}", id.0, ipc::error_kind(&why));
    }
    let _ = events.send(Event::WorkerEof(id));
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use repriselogin::greeter::ErrorKind;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::*;

    fn script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("worker");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn login(user: &str) -> Job {
        Job {
            mode: Mode::Login,
            user: user.into(),
            autologin_user: None,
            greeter_socket: false,
        }
    }

    /// Runs the loop's worker half until `id` has exited, returning what it heard first.
    async fn until_exited(
        w: &mut Workers,
        rx: &mut UnboundedReceiver<Event>,
        id: WorkerId,
    ) -> (Vec<FromWorker>, Exited) {
        let mut heard = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let exit = match tokio::time::timeout(Duration::from_millis(20), rx.recv()).await {
                Ok(Some(Event::Input(Input::Worker(from, msg)))) => {
                    assert_eq!(from, id);
                    heard.push(msg);
                    None
                }
                Ok(Some(Event::WorkerEof(from))) => w.eof(from),
                Ok(Some(Event::EofLate(from))) => w.eof_late(from),
                _ => w.reap().pop(),
            };
            if let Some(exit) = exit {
                return (heard, exit);
            }
        }
        panic!("worker {} never exited", id.0);
    }

    #[tokio::test]
    async fn a_workers_frames_all_arrive_before_its_exit() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let sent = [
                    FromWorker::Authenticated {
                        user: "anna".into(),
                        uid: 1000,
                    },
                    FromWorker::Failed {
                        kind: ErrorKind::Failed,
                        text: "no VT".into(),
                    },
                ];
                let mut bytes = Vec::new();
                for m in &sent {
                    bytes.extend_from_slice(&frame::encode(m).unwrap());
                }
                std::fs::write(dir.path().join("frames"), bytes).unwrap();
                let program = script(dir.path(), "cat \"$(dirname \"$0\")/frames\" >&3");
                let (tx, mut rx) = mpsc::unbounded_channel();
                let mut w = Workers::new(program, tx);
                w.spawn(WorkerId(4), &login("anna"));
                let (heard, exit) = until_exited(&mut w, &mut rx, WorkerId(4)).await;
                assert_eq!(heard, sent);
                assert_eq!(
                    exit,
                    Exited {
                        id: WorkerId(4),
                        mode: Mode::Login,
                        session: None
                    }
                );
                assert!(w.running.is_empty());
            })
            .await;
    }

    #[tokio::test]
    async fn the_worker_gets_its_mode_in_argv_and_its_user_first_on_fd_3() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let out = dir.path().join("out");
                let begin = frame::encode(&ToWorker::Begin {
                    user: "anna".into(),
                    autologin_user: None,
                    greeter_socket: false,
                })
                .unwrap();
                let start = frame::encode(&ToWorker::Start { vt: 7 }).unwrap();
                let body = format!(
                    "echo \"$@\" >{o}; head -c {n} <&3 >>{o}",
                    o = out.display(),
                    n = begin.len() + start.len()
                );
                let (tx, mut rx) = mpsc::unbounded_channel();
                let mut w = Workers::new(script(dir.path(), &body), tx);
                w.spawn(WorkerId(2), &login("anna"));
                w.send(WorkerId(2), &ToWorker::Start { vt: 7 });
                until_exited(&mut w, &mut rx, WorkerId(2)).await;
                let got = std::fs::read(&out).unwrap();
                let want = [&b"--worker login\n"[..], &begin, &start].concat();
                assert_eq!(got, want, "argv names no user; Begin comes before Start");
            })
            .await;
    }

    #[tokio::test]
    async fn a_workers_stdout_and_stderr_are_dev_null_and_its_log_goes_to_fd_4() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let out = dir.path().join("out");
                let body = format!(
                    "readlink /proc/$$/fd/1 /proc/$$/fd/2 /proc/$$/fd/{LOG_FD} >{}",
                    out.display()
                );
                let (tx, mut rx) = mpsc::unbounded_channel();
                let mut w = Workers::new(script(dir.path(), &body), tx);
                w.spawn(WorkerId(6), &login("anna"));
                until_exited(&mut w, &mut rx, WorkerId(6)).await;
                let ours = std::fs::read_link("/proc/self/fd/2").unwrap();
                let want = format!("/dev/null\n/dev/null\n{}\n", ours.display());
                assert_eq!(std::fs::read_to_string(&out).unwrap(), want);
            })
            .await;
    }

    #[tokio::test]
    async fn kill_sigterms_the_worker_and_its_exit_still_comes() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (tx, mut rx) = mpsc::unbounded_channel();
                let mut w = Workers::new(script(dir.path(), "exec sleep 30"), tx);
                w.spawn(WorkerId(3), &login("anna"));
                w.kill(WorkerId(3));
                w.send(WorkerId(3), &ToWorker::Cancel);
                let started = Instant::now();
                let (heard, exit) = until_exited(&mut w, &mut rx, WorkerId(3)).await;
                assert!(heard.is_empty());
                assert_eq!(exit.id, WorkerId(3));
                assert!(started.elapsed() < Duration::from_secs(5));
            })
            .await;
    }

    #[tokio::test]
    async fn a_worker_that_cannot_start_is_reported_exited() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (tx, mut rx) = mpsc::unbounded_channel();
                let mut w = Workers::new(dir.path().join("missing"), tx);
                w.spawn(WorkerId(5), &login("anna"));
                assert!(matches!(
                    rx.recv().await,
                    Some(Event::Exited(Exited {
                        id: WorkerId(5),
                        mode: Mode::Login,
                        session: None,
                    }))
                ));
                assert!(w.running.is_empty());
            })
            .await;
    }

    #[tokio::test]
    async fn remembered_state_is_written_once_per_burst_with_the_latest_value() {
        use repriselogin::Interface;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut saver = Saver::new(path.clone(), tx);
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                for interface in [Interface::Classic, Interface::Glass] {
                    saver.remember(Remembered {
                        last_user: None,
                        last_interface: Some(interface),
                    });
                }
                assert!(!path.exists(), "nothing is written at once");
                let due = tokio::time::timeout(REMEMBER_DELAY * 4, rx.recv()).await;
                assert!(matches!(due, Ok(Some(Event::Save))));
                saver.save();
                assert_eq!(
                    Remembered::load(&path).last_interface,
                    Some(Interface::Glass)
                );
                let again = tokio::time::timeout(REMEMBER_DELAY * 2, rx.recv()).await;
                assert!(again.is_err(), "one write for the burst");
            })
            .await;
    }

    #[test]
    fn boot_starts_before_any_removal_so_an_orphans_end_brings_the_greeter() {
        use reprise_login::config::LoginConfig;
        let orphan = LogindSession {
            id: "c1".into(),
            user: GREETER_USER.into(),
            uid: 977,
            vt: 1,
            seat: "seat0".into(),
            class: "greeter".into(),
            service: Mode::Greeter.service().into(),
        };
        let world = World {
            users: Vec::new(),
            keyboard: Default::default(),
            kernel_open: 0,
            user_sessions: 0,
            login_sessions: Vec::new(),
        };
        let mut daemon = Daemon::new(LoginConfig::default(), Remembered::default());
        let effects = boot(&mut daemon, vec![orphan], None, &world);
        assert_eq!(effects, [Effect::TerminateSession("c1".into())]);
        let removed = Input::SessionRemoved {
            id: "c1".into(),
            seat_active: None,
        };
        let effects = daemon.handle(Instant::now(), &world, removed);
        assert!(
            matches!(&effects[0], Effect::Spawn { job, .. } if job.mode == Mode::Greeter),
            "{effects:?}"
        );
    }

    #[test]
    fn a_timed_out_worker_on_the_active_vt_sends_the_console_to_vt_1() {
        assert!(on_dead_vt(7, Ok(7)));
        assert!(!on_dead_vt(7, Ok(1)), "the console moved on");
        assert!(!on_dead_vt(1, Ok(1)), "the greeter's own VT");
        assert!(!on_dead_vt(7, Err(io::ErrorKind::NotFound.into())));
    }
}
