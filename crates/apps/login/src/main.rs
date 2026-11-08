use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reprise_login::accountipc::{self, Replies};
use reprise_login::config;
use reprise_login::daemon::{
    Daemon, Effect, FrontUnit, HANDOVER_DEADLINE, Input, Job, LogindSession, LookUpKey, Timer,
    UNLOCK_WAIT, Unread, WorkerId, World,
};
use reprise_login::frontipc::{self, Front};
use reprise_login::intake::{self, Links};
use reprise_login::ipc::{self, Gate, Paths, Server};
use reprise_login::logind::{self, Ended, Inhibitor, Listing, Logind, SessionRemovedStream};
use reprise_login::remembered::{Remembered, STATE_PATH};
use reprise_login::remotepw::{self, STORE};
use reprise_login::vt::{self, GREETER_VT};
use reprise_login::worker::{FromWorker, LOG_FD, Mode, ToWorker, worker_main};
use reprise_login::{pidfd, users};
use repriselogin::GREETER_USER;
use repriselogin::account::ACCOUNT_SOCKET;
use repriselogin::frame::{self, FrameError};
use repriselogin::front::{FRONT_SOCKET, FRONT_USER};
use repriselogin::greeter::{ErrorKind, GreeterReply, GreeterRequest, UserEntry};
use repriselogin::machine::MACHINE_CONFIG;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
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
/// How long an account look-up may take before the account counts as no member.
const LOOKUP_WAIT: Duration = Duration::from_secs(5);
/// Threads for account look-ups and the account list. One stuck in NSS keeps its thread for
/// good, so the pool is capped; a look-up waiting for a thread counts against its own wait.
const LOOKUP_THREADS: usize = 8;
/// How long the login screen's account list may take before the last one read stands in.
const USERS_WAIT: Duration = Duration::from_secs(1);
/// The wait until a first read finishes: autologin and the greeter's tiles need the whole list.
const BOOT_USERS_WAIT: Duration = Duration::from_secs(60);
/// Listings at start while logind fails to describe a session, and the wait between them.
const BOOT_TRIES: u32 = 3;
const BOOT_RETRY: Duration = Duration::from_millis(200);
/// The wait between reads of the sessions logind still could not describe after start.
const UNREAD_RETRY: Duration = Duration::from_secs(5);

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
        .max_blocking_threads(LOOKUP_THREADS)
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("no runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let local = tokio::task::LocalSet::new();
    let code = match local.block_on(&rt, run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
    };
    drop(local);
    // A blocking thread stuck in NSS would hold a plain drop forever.
    rt.shutdown_background();
    code
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
    /// A SIGHUP: the machine settings are read again.
    Reload,
    Stop,
    /// The daemon can no longer follow logind; it exits so systemd restarts it.
    Lost(String),
}

type Events = mpsc::UnboundedSender<Event>;

async fn run() -> Result<(), String> {
    let config = config::load(Path::new(MACHINE_CONFIG));
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
    let (found, unread) = boot_sessions(&logind).await?;
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
    let (input, mut sockets) = mpsc::channel(ipc::QUEUE);
    let front = Rc::new(RefCell::new(Front::default()));
    let opener: intake::Opener = Rc::new(|uid| Box::pin(intake::open(uid)));
    let taker = logind.clone();
    let mut daemon = Daemon::new(config, remembered);
    let passwords = remotepw::current(remotepw::load(Path::new(STORE)), |user| {
        users::lookup(user).map(|e| e.map(|e| e.uid))
    });
    // No front is linked yet, so this emits nothing.
    let _ = daemon.set_remote_passwords(passwords);
    let mut rt = Runtime {
        daemon,
        logind: logind.clone(),
        gate: gate.clone(),
        workers: Workers::new(PathBuf::from("/proc/self/exe"), events.clone()),
        saver: Saver::new(PathBuf::from(STATE_PATH), events.clone()),
        events: events.clone(),
        world: WorldCache::default(),
        users: UserList::default(),
        listing_jobs: ListingJobs::default(),
        front: front.clone(),
        front_units: logind.front_units(),
        accounts: serve_account(input.clone()),
        links: Links::new(input.clone(), opener, HANDOVER_DEADLINE),
        inhibitor: Inhibitor::new(Rc::new(move |what, why| {
            let l = taker.clone();
            Box::pin(async move { l.inhibit(what, why).await })
        })),
    };
    let world = read_world(&logind, &mut rt.users).await;
    let unread = rt.daemon.recover_unread(unread);
    let effects = boot(&mut rt.daemon, found, seat_active.as_deref(), &world);
    let recovered: Vec<String> = rt.daemon.session_uids().into_keys().collect();
    rt.world = WorldCache(Some(world));

    let (greeter_l, session_l) = (ipc::bind(&Paths::default(), greeter.gid))
        .map_err(|e| format!("binding the sockets: {e}"))?;
    serve_front(input.clone(), front);
    let (lookup, seat) = (logind.clone(), logind.clone());
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
    tokio::task::spawn_local(read_unread(unread, logind.clone(), events.clone()));
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
            Event::Reload => {
                log::info!("reading {MACHINE_CONFIG} again");
                let config = config::load(Path::new(MACHINE_CONFIG));
                rt.input(Input::ConfigReloaded(config)).await;
            }
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

/// Binds `FRONT_SOCKET` for the front's account and serves it; without the account or the
/// socket the daemon runs on with no remote logins.
fn serve_front(input: mpsc::Sender<Input>, front: Rc<RefCell<Front>>) {
    let account = match users::lookup(FRONT_USER) {
        Ok(Some(account)) => account,
        Ok(None) => return log::warn!("there is no {FRONT_USER} account; no remote logins"),
        Err(e) => return log::warn!("{FRONT_USER}: {e}; no remote logins"),
    };
    let served = ipc::bind_group(Path::new(FRONT_SOCKET), account.gid)
        .and_then(|l| frontipc::serve(l, account.uid, input, front));
    if let Err(e) = served {
        log::warn!("{FRONT_SOCKET}: {e}; no remote logins");
    }
}

/// Binds `ACCOUNT_SOCKET` for every local user and serves it; without it nobody can change
/// their Remote Desktop password.
fn serve_account(input: mpsc::Sender<Input>) -> Replies {
    let served =
        ipc::bind_open(Path::new(ACCOUNT_SOCKET)).and_then(|l| accountipc::serve(l, input));
    served.unwrap_or_else(|e| {
        log::warn!("{ACCOUNT_SOCKET}: {e}; Remote Desktop passwords cannot be changed");
        Replies::default()
    })
}

/// logind's sessions, closing ones left out, and those it still could not describe after
/// `BOOT_TRIES` listings.
async fn boot_sessions(logind: &Logind) -> Result<(Vec<LogindSession>, Vec<Unread>), String> {
    let mut tries = 1;
    loop {
        let listing = (logind.listing().await).map_err(|e| format!("logind's sessions: {e}"))?;
        if listing.unread.is_empty() || tries == BOOT_TRIES {
            let found = listing.found.into_iter().filter(|s| !s.closing).collect();
            return Ok((found, listing.unread));
        }
        tries += 1;
        tokio::time::sleep(BOOT_RETRY).await;
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
    let mut effects = daemon.recover(found, seat_active);
    effects.extend(daemon.start(now, world));
    effects
}

/// Reads `unread` every `UNREAD_RETRY` until each reads, feeding it as `SessionRead`, or is gone.
async fn read_unread(mut unread: Vec<Unread>, logind: Logind, events: Events) {
    while !unread.is_empty() {
        tokio::time::sleep(UNREAD_RETRY).await;
        let mut left = Vec::new();
        for u in unread {
            match logind.read(&u).await {
                Ok(session) => {
                    log::info!("logind described session {} at last", u.id);
                    let seat_active = logind.seat_active().await;
                    let read = Input::SessionRead {
                        session,
                        seat_active,
                    };
                    let _ = events.send(Event::Input(read));
                }
                Err(e) if logind::session_gone(&e) => {}
                Err(_) => left.push(u),
            }
        }
        unread = left;
    }
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
        Ok(_) if logind.ended(&id).await == Ended::Yes => {}
        Ok(fd) => {
            if let Err(e) = pidfd::exited(fd).await {
                return log::warn!("session {id}: watching its leader {pid}: {e}");
            }
        }
        Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {}
        Err(e) => return log::warn!("session {id}: watching its leader {pid}: {e}"),
    }
    for _ in 0..LEADER_POLLS {
        if logind.ended(&id).await == Ended::Yes {
            let seat_active = logind.seat_active().await;
            let _ = events.send(Event::Input(Input::SessionRemoved { id, seat_active }));
            return;
        }
        tokio::time::sleep(LEADER_POLL).await;
    }
}

/// Feeds `ConsoleOnGreeterVt` or `ConsoleOnVt` each time the console moves, from a thread:
/// the kernel wakes a `poll` on a sysfs attribute, which tokio's reactor does not wait on.
fn follow_console(events: Events) {
    let spawned = std::thread::Builder::new()
        .name("console".into())
        .spawn(move || {
            vt::follow_active(|vt| {
                let input = match vt {
                    GREETER_VT => Input::ConsoleOnGreeterVt,
                    vt => Input::ConsoleOnVt(vt),
                };
                events.send(Event::Input(input)).is_ok()
            });
        });
    if let Err(e) = spawned {
        log::warn!("no thread to follow the console's VT: {e}");
    }
}

/// SIGCHLD reaps workers; SIGHUP reloads the machine settings; SIGTERM and SIGINT stop the
/// daemon.
fn follow_signals(events: Events) -> io::Result<()> {
    let mut child = signal(SignalKind::child())?;
    let mut hup = signal(SignalKind::hangup())?;
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    tokio::task::spawn_local(async move {
        loop {
            let event = tokio::select! {
                Some(()) = child.recv() => Event::Reap,
                Some(()) = hup.recv() => Event::Reload,
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
    world: WorldCache,
    users: UserList,
    listing_jobs: ListingJobs,
    front: Rc<RefCell<Front>>,
    front_units: mpsc::UnboundedSender<FrontUnit>,
    accounts: Replies,
    links: Links,
    inhibitor: Inhibitor,
}

/// The world the state machine last saw, read again for each input [`reads_world`] names.
#[derive(Default)]
struct WorldCache(Option<World>);

impl WorldCache {
    async fn for_input(&mut self, input: &Input, read: impl AsyncFnOnce() -> World) -> &World {
        if reads_world(input) || self.0.is_none() {
            self.0 = Some(read().await);
        }
        self.0.as_ref().expect("a world was read")
    }
}

/// Whether handling `input` reads `active_vt` (start, power, hand-over, disconnect end): a cached
/// one could be another VT's.
fn reads_world(input: &Input) -> bool {
    matches!(
        input,
        Input::GreeterRequest(
            _,
            GreeterRequest::Hello | GreeterRequest::StartSession | GreeterRequest::Power { .. }
        ) | Input::Worker(_, FromWorker::Authenticated { .. })
            | Input::SessionRemoved { .. }
            | Input::TerminateFailed { .. }
            | Input::Forwarded { .. }
            | Input::Timer(Timer::Disconnected { .. })
    )
}

impl Runtime {
    async fn input(&mut self, input: Input) {
        let read = match &input {
            Input::SessionRead { session, .. } => Some(session.id.clone()),
            _ => None,
        };
        if let Input::Worker(w, FromWorker::Started { session_id, .. }) = &input {
            self.workers.started(*w, session_id);
            self.gate.borrow_mut().greeter_started(*w, session_id);
        }
        let (logind, users) = (&self.logind, &mut self.users);
        let world = (self.world)
            .for_input(&input, async || read_world(logind, users).await)
            .await;
        let effects = self.daemon.handle(Instant::now(), world, input);
        self.run_effects(effects);
        self.sync_gate();
        if let Some(id) = read.filter(|id| self.daemon.session_uids().contains_key(id)) {
            let (logind, events) = (self.logind.clone(), self.events.clone());
            tokio::task::spawn_local(follow_leader(id, logind, events));
        }
    }

    /// Hands the state machine a worker's exit: the console leaves a timed-out worker's VT, and a
    /// session worker's session is removed once logind lists it closing, else reported orphaned.
    async fn exited(&mut self, exit: Exited) {
        self.gate.borrow_mut().greeter_exited(exit.id);
        let stalled = self.daemon.stalled_vt(exit.id);
        self.input(Input::WorkerExited(exit.id)).await;
        if stalled.is_some_and(|v| on_dead_vt(v, vt::active())) {
            log::warn!("a timed-out login left the console on its VT; showing the login screen");
            self.switch_to(GREETER_VT);
        }
        let Some(id) = exit.session.filter(|_| exit.mode != Mode::Greeter) else {
            return;
        };
        let ended = self.logind.ended(&id).await;
        if orphaned(exit.mode, ended) {
            self.report_orphan(id).await;
        } else if ended == Ended::Yes {
            let seat_active = self.logind.seat_active().await;
            self.input(Input::SessionRemoved { id, seat_active }).await;
        }
    }

    /// Reports session `id` orphaned with the uid logind lists for it, or removed once logind
    /// lists it closing or not at all. A failed read ends nothing.
    async fn report_orphan(&mut self, id: String) {
        let listing = match self.logind.listing().await {
            Ok(listing) => Some(listing),
            Err(e) => {
                log::warn!("session {id} lost its login worker; reading logind's sessions: {e}");
                None
            }
        };
        match orphan_report(&id, listing.as_ref()) {
            OrphanReport::Orphaned { uid } => {
                self.input(Input::SessionOrphaned { id, uid }).await;
            }
            OrphanReport::Removed => {
                let seat_active = self.logind.seat_active().await;
                self.input(Input::SessionRemoved { id, seat_active }).await;
            }
            OrphanReport::Nothing => {}
        }
    }

    fn sync_gate(&self) {
        self.gate
            .borrow_mut()
            .set_sessions(self.daemon.session_uids());
    }

    fn run_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            self.run(effect);
        }
    }

    fn run(&mut self, effect: Effect) {
        match effect {
            Effect::Reply(reply) => {
                match &reply {
                    GreeterReply::Error {
                        kind: ErrorKind::Failed,
                        text,
                    } => {
                        log::warn!("login screen told: {text}");
                    }
                    GreeterReply::Error {
                        kind: ErrorKind::Refused,
                        text,
                    } => {
                        log::info!("login screen told: {text}");
                    }
                    _ => {}
                }
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
            Effect::KillSession(id) => {
                let l = self.logind.clone();
                tokio::task::spawn_local(async move {
                    if let Err(e) = l.manager().kill_session(&id, "all", libc::SIGKILL).await {
                        log::warn!("killing session {id}: {e}");
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
            Effect::ToFront(event) => self.front.borrow_mut().send(&event),
            Effect::LookUp { key, user, group } => {
                let events = self.events.clone();
                let hold = match key {
                    LookUpKey::Listed { uid, .. } => match self.listing_jobs.hold(uid) {
                        Some(hold) => Some(hold),
                        None => {
                            log::warn!("{user}'s last group look-up still runs");
                            let failed = Input::Member {
                                key,
                                uid: None,
                                member: false,
                                failed: true,
                            };
                            let _ = events.send(Event::Input(failed));
                            return;
                        }
                    },
                    _ => None,
                };
                tokio::task::spawn_local(async move {
                    let job = move || {
                        let _hold = hold;
                        users::member_of(&user, &group)
                    };
                    let member = look_up(key, LOOKUP_WAIT, job).await;
                    let _ = events.send(Event::Input(member));
                });
            }
            Effect::OpenIntake { uid } => self.links.open(uid),
            Effect::Forward { conn, uid } => {
                let held = self.front.borrow_mut().take(conn);
                let queued = held.is_some_and(|(fd, h)| self.links.forward(uid, conn, fd, h));
                if !queued {
                    let failed = Input::Forwarded { conn, ok: false };
                    let _ = self.events.send(Event::Input(failed));
                }
            }
            Effect::DropConn { conn } => {
                drop(self.front.borrow_mut().take(conn));
                self.links.drop_conn(conn);
            }
            Effect::Inhibit(inhibit) => self.inhibitor.set(inhibit),
            Effect::Front(unit) => {
                let _ = self.front_units.send(unit);
            }
            Effect::CloseIntakes { keep } => self.links.close_all_but(&keep),
            Effect::ReopenIntake { uid } => self.links.reopen(uid),
            Effect::AccountReply { link, reply } => self.accounts.send(link, reply),
            Effect::StorePasswords(all) => {
                let events = self.events.clone();
                tokio::task::spawn_local(async move {
                    let save = move || remotepw::save(Path::new(STORE), &all);
                    let saved = (tokio::task::spawn_blocking(save).await)
                        .unwrap_or_else(|e| Err(io::Error::other(e)));
                    if let Err(e) = &saved {
                        log::warn!("{STORE}: {e}");
                    }
                    let ok = saved.is_ok();
                    let _ = events.send(Event::Input(Input::Stored { ok }));
                });
            }
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

/// What the backend reads fresh for each input [`reads_world`] names.
async fn read_world(logind: &Logind, users: &mut UserList) -> World {
    let (user_sessions, login_sessions) = match logind.listed().await {
        Ok(found) => {
            let open = found.iter().filter(|s| s.class == "user" && !s.closing);
            (open.count() as u32, found)
        }
        Err(e) => {
            // Counted as one, so power stays refused while logind can't say.
            log::warn!("logind's sessions: {e}");
            (1, Vec::new())
        }
    };
    let (active_vt, kernel_open) = vt::console().unwrap_or_else(|e| {
        log::warn!("VT state: {e}");
        (0, 0)
    });
    World {
        users: users.read().await,
        keyboard: logind.keyboard().await,
        kernel_open,
        active_vt,
        user_sessions,
        login_sessions,
    }
}

/// The login screen's accounts, enumerated off the loop with at most one enumeration running:
/// `getpwent` keeps one cursor per process.
struct UserList {
    last: Vec<UserEntry>,
    reading: Option<(Instant, JoinHandle<Vec<UserEntry>>)>,
    waits: [Duration; 2],
    read_once: bool,
}

impl Default for UserList {
    fn default() -> Self {
        Self {
            last: Vec::new(),
            reading: None,
            waits: [BOOT_USERS_WAIT, USERS_WAIT],
            read_once: false,
        }
    }
}

impl UserList {
    /// A fresh list, or the last one when the read runs past its wait from its start. A read
    /// that ran past its wait is waited on by later calls, and once done, replaced by a new one.
    async fn read(&mut self) -> Vec<UserEntry> {
        self.read_with(users::read_listed).await
    }

    async fn read_with(
        &mut self,
        job: impl FnOnce() -> Vec<UserEntry> + Send + 'static,
    ) -> Vec<UserEntry> {
        if let Some((_, late)) = self.reading.take_if(|(_, h)| h.is_finished()) {
            self.keep(late.await);
        }
        let wait = self.wait();
        let (started, reading) = (self.reading)
            .get_or_insert_with(|| (Instant::now(), tokio::task::spawn_blocking(job)));
        match tokio::time::timeout(wait.saturating_sub(started.elapsed()), reading).await {
            Ok(read) => {
                self.reading = None;
                self.keep(read);
            }
            Err(_) => log::warn!("listing the accounts took over {wait:?}; showing the last list"),
        }
        self.last.clone()
    }

    /// [`BOOT_USERS_WAIT`] until a read has finished, then [`USERS_WAIT`].
    fn wait(&self) -> Duration {
        self.waits[usize::from(self.read_once)]
    }

    fn keep(&mut self, read: Result<Vec<UserEntry>, tokio::task::JoinError>) {
        self.read_once = true;
        match read {
            Ok(users) => self.last = users,
            Err(e) => log::warn!("listing the accounts: {e}"),
        }
    }
}

/// Uids whose look-up for the front's list runs, past its wait too, so NSS that hangs holds
/// at most one such job per stored user.
#[derive(Clone, Default)]
struct ListingJobs(Arc<Mutex<HashSet<u32>>>);

impl ListingJobs {
    /// Holds `uid` until the hold drops with its job, or `None` while its last job runs.
    fn hold(&self, uid: u32) -> Option<ListingHold> {
        let added = self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(uid);
        added.then(|| ListingHold(self.clone(), uid))
    }
}

struct ListingHold(ListingJobs, u32);

impl Drop for ListingHold {
    fn drop(&mut self) {
        (self.0.0.lock().unwrap_or_else(|e| e.into_inner())).remove(&self.1);
    }
}

/// The answer to `Effect::LookUp` from `job`, run off the loop. One that fails or takes past
/// `wait` counts as no member and is marked failed.
async fn look_up(
    key: LookUpKey,
    wait: Duration,
    job: impl FnOnce() -> io::Result<Option<(u32, bool)>> + Send + 'static,
) -> Input {
    let found = tokio::time::timeout(wait, tokio::task::spawn_blocking(job)).await;
    let failed = |why: &dyn std::fmt::Display| {
        log::warn!("looking up an account: {why}");
        Input::Member {
            key,
            uid: None,
            member: false,
            failed: true,
        }
    };
    match found {
        Ok(Ok(Ok(found))) => Input::Member {
            key,
            uid: found.map(|(uid, _)| uid),
            member: found.is_some_and(|(_, member)| member),
            failed: false,
        },
        Ok(Ok(Err(e))) => failed(&e),
        Ok(Err(e)) => failed(&e),
        Err(_) => failed(&format_args!("it took over {wait:?}")),
    }
}

/// Whether the console still shows the VT of a worker whose start timed out. The login
/// screen's own VT never counts.
fn on_dead_vt(stalled_vt: u16, active: io::Result<u16>) -> bool {
    stalled_vt != GREETER_VT && active.is_ok_and(|a| a == stalled_vt)
}

/// Whether a worker's exit may have left its session open without a PAM handle. The worker leads its
/// session, so logind most often reports it closing already, after a logout or a crash alike.
fn orphaned(mode: Mode, ended: Ended) -> bool {
    mode != Mode::Greeter && ended != Ended::Yes
}

/// What the loop sends for a session whose worker exited while logind kept it open.
#[derive(Debug, PartialEq)]
enum OrphanReport {
    Orphaned {
        uid: u32,
    },
    /// logind lists it closing or not at all: it closed between the two reads.
    Removed,
    /// logind's sessions, or this one's properties, could not be read; nothing is ended.
    Nothing,
}

fn orphan_report(id: &str, listing: Option<&Listing>) -> OrphanReport {
    let Some(listing) = listing else {
        return OrphanReport::Nothing;
    };
    match listing.found.iter().find(|s| s.id == id) {
        Some(s) if !s.closing => OrphanReport::Orphaned { uid: s.uid },
        None if listing.unread.iter().any(|u| u.id == id) => OrphanReport::Nothing,
        _ => OrphanReport::Removed,
    }
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
    reader: Option<JoinHandle<()>>,
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
            rhost: job.rhost.clone(),
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
    // SAFETY: the closure runs in the child between fork and exec and calls only fcntl, dup2
    // and `cloexec_from`, which are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            // Copied above both targets first, so neither dup2 closes the other's source; the
            // CLOEXEC copies end at exec, and dup2's targets never carry CLOEXEC.
            let high = |fd| libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, LOG_FD + 1);
            let (sock, log) = (high(sock), high(log));
            // zbus receives fds without CLOEXEC on its own thread, so one may be open here.
            let failed = sock < 0
                || log < 0
                || libc::dup2(sock, WORKER_FD) < 0
                || libc::dup2(log, LOG_FD) < 0
                || !cloexec_from(LOG_FD + 1);
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

/// Sets FD_CLOEXEC on every fd from `first` up; false with errno set when it can't. Kernels
/// before 5.11 lack close_range's CLOEXEC flag, so there each fd below the fd limit gets fcntl.
fn cloexec_from(first: libc::c_int) -> bool {
    let flags = libc::CLOSE_RANGE_CLOEXEC as libc::c_long;
    // SAFETY: close_range takes plain integers.
    if unsafe { libc::syscall(libc::SYS_close_range, first, u32::MAX, flags) } == 0 {
        return true;
    }
    match io::Error::last_os_error().raw_os_error() {
        Some(libc::EINVAL | libc::ENOSYS) => cloexec_each(first),
        _ => false,
    }
}

/// `cloexec_from` by fcntl on each fd up to the soft fd limit, which no open fd reaches.
fn cloexec_each(first: libc::c_int) -> bool {
    // SAFETY: rlimit is plain data, valid zeroed.
    let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: getrlimit is one syscall in glibc and writes only into `limit`; fcntl takes
    // integers, and a closed fd just gives EBADF.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) < 0 {
            return false;
        }
        let end = libc::c_int::try_from(limit.rlim_cur).unwrap_or(libc::c_int::MAX);
        for fd in first..end {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    true
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
            rhost: None,
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
                    rhost: None,
                })
                .unwrap();
                let start = ToWorker::Start {
                    vt: 7,
                    background: false,
                };
                let start = frame::encode(&start).unwrap();
                let body = format!(
                    "echo \"$@\" >{o}; head -c {n} <&3 >>{o}",
                    o = out.display(),
                    n = begin.len() + start.len()
                );
                let (tx, mut rx) = mpsc::unbounded_channel();
                let mut w = Workers::new(script(dir.path(), &body), tx);
                w.spawn(WorkerId(2), &login("anna"));
                w.send(
                    WorkerId(2),
                    &ToWorker::Start {
                        vt: 7,
                        background: false,
                    },
                );
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
        use reprise_login::daemon::FrontUnit;
        use repriselogin::machine::MachineConfig;
        let orphan = LogindSession {
            id: "c1".into(),
            user: GREETER_USER.into(),
            uid: 977,
            vt: 1,
            seat: "seat0".into(),
            class: "greeter".into(),
            service: Mode::Greeter.service().into(),
            closing: false,
        };
        let world = World {
            users: Vec::new(),
            keyboard: Default::default(),
            kernel_open: 0,
            active_vt: 1,
            user_sessions: 0,
            login_sessions: Vec::new(),
        };
        let mut daemon = Daemon::new(MachineConfig::default(), Remembered::default());
        let effects = boot(&mut daemon, vec![orphan], None, &world);
        assert_eq!(
            effects,
            [
                Effect::TerminateSession("c1".into()),
                Effect::Front(FrontUnit::Stop)
            ]
        );
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

    #[test]
    fn a_worker_exit_reports_its_session_orphaned_only_while_logind_keeps_it_open() {
        assert!(orphaned(Mode::Login, Ended::No));
        assert!(orphaned(Mode::Remote, Ended::No));
        assert!(
            orphaned(Mode::Login, Ended::Unknown),
            "the orphan report reads logind again"
        );
        assert!(
            !orphaned(Mode::Login, Ended::Yes),
            "a logout closes the session first"
        );
        assert!(
            !orphaned(Mode::Greeter, Ended::No),
            "the login screen's own session"
        );
    }

    #[test]
    fn an_orphaned_session_is_reported_open_removed_or_not_at_all() {
        let open = |id: &str, uid| LogindSession {
            id: id.into(),
            user: "anna".into(),
            uid,
            vt: 7,
            seat: "seat0".into(),
            class: "user".into(),
            service: Mode::Login.service().into(),
            closing: false,
        };
        let listing = |found| Listing {
            found,
            unread: vec![],
        };
        let both = listing(vec![open("c3", 1001), open("c1", 1000)]);
        assert_eq!(
            orphan_report("c1", Some(&both)),
            OrphanReport::Orphaned { uid: 1000 }
        );
        assert_eq!(
            orphan_report("c1", Some(&listing(vec![open("c3", 1001)]))),
            OrphanReport::Removed,
            "closed between the two reads"
        );
        let closing = LogindSession {
            closing: true,
            ..open("c1", 1000)
        };
        assert_eq!(
            orphan_report("c1", Some(&listing(vec![closing]))),
            OrphanReport::Removed,
            "closing since the first read"
        );
        assert_eq!(
            orphan_report("c1", None),
            OrphanReport::Nothing,
            "logind could not be read"
        );
    }

    #[test]
    fn a_session_logind_lists_but_cannot_describe_is_not_reported_removed() {
        let listing = Listing {
            found: vec![],
            unread: vec![Unread {
                id: "c1".into(),
                uid: 1000,
                user: "anna".into(),
                seat: "seat0".into(),
            }],
        };
        assert_eq!(orphan_report("c1", Some(&listing)), OrphanReport::Nothing);
    }

    fn world_on(active_vt: u16) -> World {
        World {
            users: Vec::new(),
            keyboard: Default::default(),
            kernel_open: 0,
            active_vt,
            user_sessions: 0,
            login_sessions: Vec::new(),
        }
    }

    #[test]
    fn the_inputs_that_can_start_a_session_read_the_world_again() {
        use repriselogin::greeter::PowerAction;
        let (w, id) = (WorkerId(1), String::from("c1"));
        let greeter = |r| Input::GreeterRequest(w, r);
        let fresh = [
            greeter(GreeterRequest::Hello),
            greeter(GreeterRequest::StartSession),
            greeter(GreeterRequest::Power {
                action: PowerAction::PowerOff,
            }),
            Input::Worker(
                w,
                FromWorker::Authenticated {
                    user: "anna".into(),
                    uid: 1000,
                },
            ),
            Input::SessionRemoved {
                id: id.clone(),
                seat_active: None,
            },
            Input::TerminateFailed {
                id: id.clone(),
                seat_active: None,
            },
        ];
        for input in &fresh {
            assert!(reads_world(input), "{input:?} reads the world");
        }
        let cached = [
            greeter(GreeterRequest::CancelSession),
            Input::Worker(
                w,
                FromWorker::Started {
                    session_id: id,
                    uid: 1000,
                },
            ),
            Input::Viewers {
                uid: 1000,
                count: 1,
            },
            Input::FrontLinked,
        ];
        for input in &cached {
            assert!(!reads_world(input), "{input:?} uses the cached world");
        }
    }

    #[test]
    fn a_forwarded_hand_over_reads_the_world_fresh() {
        assert!(reads_world(&Input::Forwarded { conn: 1, ok: true }));
    }

    #[test]
    fn a_disconnect_time_running_out_reads_the_world_fresh() {
        let disconnected = Timer::Disconnected {
            id: "c2".into(),
            generation: 1,
        };
        assert!(reads_world(&Input::Timer(disconnected)));
    }

    #[tokio::test]
    async fn a_stale_cached_active_vt_is_never_picked() {
        use repriselogin::front::{FrontRequest, Secret};
        use repriselogin::greeter::AuthKind;
        use repriselogin::machine::{MachineConfig, MachineRemote};

        let config = MachineConfig {
            remote: MachineRemote {
                enabled: true,
                ..MachineRemote::default()
            },
            ..MachineConfig::default()
        };
        let mut daemon = Daemon::new(config, Remembered::default());
        let now = Instant::now();
        daemon.recover(Vec::new(), None);
        daemon.start(now, &world_on(1));
        let mut cache = WorldCache(Some(world_on(1)));
        let on_screen = world_on(7);
        let mut handle = async |input: Input| {
            let world = cache.for_input(&input, async || on_screen.clone()).await;
            daemon.handle(now, world, input)
        };
        handle(Input::Front(FrontRequest::Authenticate {
            conn: 7,
            peer: "192.0.2.1:50000".parse().unwrap(),
            user: "anna".into(),
            secret: Secret::Password(Zeroizing::new("pw".into())),
        }))
        .await;
        let member = Input::Member {
            key: LookUpKey::Conn(7),
            uid: Some(1001),
            member: true,
            failed: false,
        };
        let spawned = handle(member).await;
        let Some(Effect::Spawn { worker, .. }) = spawned.first() else {
            panic!("a check worker spawns: {spawned:?}");
        };
        let worker = *worker;
        let prompt = FromWorker::Prompt {
            kind: AuthKind::Secret,
            text: "Password: ".into(),
        };
        handle(Input::Worker(worker, prompt)).await;
        let authenticated = FromWorker::Authenticated {
            user: "anna".into(),
            uid: 1001,
        };
        let effects = handle(Input::Worker(worker, authenticated)).await;
        let start = effects.iter().find_map(|e| match e {
            Effect::ToWorker {
                msg: ToWorker::Start { vt, .. },
                ..
            } => Some(*vt),
            _ => None,
        });
        assert_eq!(start, Some(8), "VT 7 is on screen now: {effects:?}");
    }

    #[tokio::test]
    async fn an_account_look_up_past_its_wait_counts_as_a_failed_no_member() {
        let key = LookUpKey::Conn(3);
        let slow = || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(Some((1001, true)))
        };
        assert_eq!(
            look_up(key, Duration::from_millis(20), slow).await,
            Input::Member {
                key,
                uid: None,
                member: false,
                failed: true,
            }
        );
        assert_eq!(
            look_up(key, Duration::from_secs(5), || Ok(Some((1001, true)))).await,
            Input::Member {
                key,
                uid: Some(1001),
                member: true,
                failed: false,
            }
        );
    }

    #[tokio::test]
    async fn a_missing_account_is_no_member_and_an_erring_look_up_is_failed() {
        let key = LookUpKey::Conn(3);
        assert_eq!(
            look_up(key, Duration::from_secs(5), || Ok(None)).await,
            Input::Member {
                key,
                uid: None,
                member: false,
                failed: false,
            }
        );
        let erring = || Err(io::Error::other("sssd is down"));
        assert_eq!(
            look_up(key, Duration::from_secs(5), erring).await,
            Input::Member {
                key,
                uid: None,
                member: false,
                failed: true,
            }
        );
    }

    #[test]
    fn a_listing_look_up_runs_once_per_uid_until_its_job_ends() {
        let jobs = ListingJobs::default();
        let anna = jobs.hold(1001).expect("the first hold");
        assert!(jobs.hold(1001).is_none(), "anna's job still runs");
        let bert = jobs.hold(1002);
        assert!(bert.is_some(), "another uid has its own");
        drop(anna);
        assert!(jobs.hold(1001).is_some(), "anna's job ended");
    }

    fn entry(name: &str) -> UserEntry {
        UserEntry {
            name: name.into(),
            real_name: String::new(),
            picture: None,
            note: None,
        }
    }

    fn waiting(first: Duration, then: Duration) -> UserList {
        UserList {
            waits: [first, then],
            ..UserList::default()
        }
    }

    #[tokio::test]
    async fn the_first_account_read_waits_the_boot_wait_and_later_ones_the_loop_wait() {
        let mut users = UserList::default();
        assert_eq!(users.wait(), BOOT_USERS_WAIT);
        assert_eq!(
            users.read_with(|| vec![entry("anna")]).await,
            [entry("anna")]
        );
        assert_eq!(users.wait(), USERS_WAIT);
    }

    #[tokio::test]
    async fn an_account_read_already_past_its_wait_returns_the_last_list_at_once() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let wait = Duration::from_secs(60);
        let mut users = waiting(wait, wait);
        let hung = move || {
            let _ = held.recv();
            vec![entry("anna")]
        };
        let started = Instant::now().checked_sub(wait).unwrap();
        users.reading = Some((started, tokio::task::spawn_blocking(hung)));
        let read = tokio::time::timeout(Duration::from_secs(5), users.read_with(Vec::new)).await;
        assert_eq!(read.ok(), Some(Vec::new()), "no second wait of {wait:?}");
        drop(release);
    }

    #[tokio::test]
    async fn an_account_read_that_finished_late_is_followed_by_a_fresh_one() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let mut users = waiting(Duration::ZERO, Duration::from_secs(5));
        let late = move || {
            let _ = held.recv();
            vec![entry("anna")]
        };
        assert_eq!(users.read_with(late).await, Vec::new());
        release.send(()).unwrap();
        while !users.reading.as_ref().unwrap().1.is_finished() {
            tokio::task::yield_now().await;
        }
        let fresh = || vec![entry("anna"), entry("bert")];
        assert_eq!(
            users.read_with(fresh).await,
            vec![entry("anna"), entry("bert")]
        );
    }

    #[test]
    fn without_close_ranges_flag_each_fd_gets_cloexec_by_fcntl() {
        let (pipe_end, _w) = io::pipe().unwrap();
        let fd = pipe_end.as_raw_fd();
        // SAFETY: the child makes only fcntl, getrlimit and _exit calls; `fd` is open in it.
        let child = unsafe { libc::fork() };
        assert!(child >= 0, "{}", io::Error::last_os_error());
        if child == 0 {
            // SAFETY: F_DUPFD and F_GETFD on open fds in the child.
            unsafe {
                // F_DUPFD leaves the copy without FD_CLOEXEC.
                let high = libc::fcntl(fd, libc::F_DUPFD, 40);
                if high < 40 || !cloexec_each(40) {
                    libc::_exit(2);
                }
                let flags = libc::fcntl(high, libc::F_GETFD);
                libc::_exit(if flags & libc::FD_CLOEXEC != 0 { 0 } else { 1 });
            }
        }
        let mut status = 0;
        // SAFETY: `child` is this process's child and `status` outlives the call.
        assert_eq!(unsafe { libc::waitpid(child, &mut status, 0) }, child);
        assert!(libc::WIFEXITED(status), "{status:#x}");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "0: FD_CLOEXEC set; 1: the fd stays open across exec; 2: no fd or the loop failed"
        );
    }

    #[test]
    fn a_worker_inherits_no_fd_left_without_cloexec() {
        use std::os::fd::OwnedFd;
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let (pipe_end, _w) = io::pipe().unwrap();
        // SAFETY: F_DUPFD on an open fd; the copy, above the worker's fixed fds and without
        // FD_CLOEXEC, is one zbus received.
        let leak = unsafe { libc::fcntl(pipe_end.as_raw_fd(), libc::F_DUPFD, 20) };
        assert!(leak > LOG_FD, "{}", io::Error::last_os_error());
        // SAFETY: the fd was just made, and nothing else owns it.
        let leak = unsafe { OwnedFd::from_raw_fd(leak) };
        let pipe = format!(
            "pipe:[{}]",
            std::fs::metadata(format!("/proc/self/fd/{}", leak.as_raw_fd()))
                .unwrap()
                .ino()
        );
        let program = script(dir.path(), "ls -l /proc/$$/fd/ > \"$(dirname \"$0\")/fds\"");
        let (mut child, _sock) = start(&program, &login("anna")).unwrap();
        assert!(child.wait().unwrap().success());
        let fds = std::fs::read_to_string(dir.path().join("fds")).unwrap();
        assert!(
            fds.contains("socket:["),
            "the worker's socket is there: {fds}"
        );
        assert!(!fds.contains(&pipe), "{pipe} leaked into the worker: {fds}");
    }
}
