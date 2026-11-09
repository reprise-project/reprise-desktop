//! A text-login Reprise takes the user manager back once the Reprise session it gave way to ends.

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread::JoinHandle;

use smithay::reexports::calloop::channel::{self, Sender};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};

use crate::compositor::State;
use crate::spawn::SessionEnv;

/// What [`take_back`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Another session holds the user manager.
    Stays,
    /// Nothing else holds it: asked for the bus names again, and left the variables and units.
    Names,
    /// The names, and published again; a login manager's session started meanwhile keeps the
    /// target.
    Yields,
    /// The names, published again and restarted the session target.
    Takes,
}

/// The Reprise sessions of the user a text login leaves the user manager to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GivesWay {
    LoginManager,
    /// A login manager's, or one logind opened before this one's: what a session that deferred
    /// at its start goes on deferring to.
    LoginManagerOrOlder,
}

/// What [`take_back`] reads and does.
pub trait World {
    /// Whether a Reprise session `to` names runs.
    fn gives_way(&self, to: GivesWay) -> bool;
    /// Whether the Wayland display `WAYLAND_DISPLAY` names accepts a connection.
    fn accepts(&self, display: &str) -> bool;
    fn retake_names(&mut self);
    /// Write the GTK portal's theme drop-in, reloading the user manager when it changed.
    fn retheme(&mut self);
    fn run(&mut self, argv: &[String]);
}

/// Nothing happens while a Reprise session the text-login `env` gives way to runs, or while
/// `manager` (the user manager's `WAYLAND_DISPLAY`) names another display that accepts
/// connections. Otherwise the bus names come back, and the variables and the target too when
/// `manager` names one of `holder`'s: the displays of the session this one gave way to.
pub fn take_back(
    env: &SessionEnv,
    manager: Option<&str>,
    holder: &[String],
    w: &mut impl World,
) -> Outcome {
    if !env.is_text_login() {
        return Outcome::Stays;
    }
    let to = if env.defers_user_units {
        GivesWay::LoginManagerOrOlder
    } else {
        GivesWay::LoginManager
    };
    if w.gives_way(to) {
        return Outcome::Stays;
    }
    let foreign = manager.filter(|d| *d != env.wayland_display);
    if foreign.is_some_and(|d| w.accepts(d)) {
        return Outcome::Stays;
    }
    w.retake_names();
    if !foreign.is_some_and(|d| holder.iter().any(|h| h == d)) {
        return Outcome::Names;
    }
    let owner = SessionEnv {
        defers_user_units: false,
        ..env.clone()
    };
    if let Some(unset) = crate::spawn::unset_unpublished(&owner) {
        w.run(&unset);
    }
    let mut cmds = crate::spawn::begin_commands(&owner).into_iter();
    // The publish comes first; a session that started since then publishes after it.
    cmds.by_ref().take(1).for_each(|argv| w.run(&argv));
    if w.gives_way(GivesWay::LoginManager) {
        return Outcome::Yields;
    }
    // Before the stop, so the target's restart of the GTK portal reads the drop-in.
    w.retheme();
    cmds.for_each(|argv| w.run(&argv));
    Outcome::Takes
}

enum Msg {
    /// logind removed a session or changed hands, or the session given way to exited.
    Ended,
    /// The compositor of another of the user's Reprise sessions took or holds the tray's name.
    Holder {
        pidfd: OwnedFd,
        displays: Vec<String>,
    },
    Done(Outcome),
}

/// The worker in flight, which [`finish`] waits for.
static WORKER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// Handed to the session's begin, which calls [`Begun::done`] once its commands ran: until then
/// no [`take_back`] runs.
pub struct Begun(Option<Sender<Msg>>);

impl Begun {
    pub fn done(self) {
        if let Some(tx) = self.0 {
            let _ = tx.send(Msg::Done(Outcome::Stays));
        }
    }
}

/// In a text-login session, run [`take_back`] on a thread of its own, one at a time, whenever
/// logind removes a session or the session this one gave way to exits.
pub fn setup(state: &mut State) -> Begun {
    if !state.env.is_text_login() {
        return Begun(None);
    }
    let (tx, rx) = channel::channel::<Msg>();
    let ours = tx.clone();
    // The begin counts as the first worker.
    let (mut running, mut again) = (true, false);
    let mut holder: Vec<String> = Vec::new();
    let inserted = state.handle.insert_source(rx, move |event, _, state| {
        let channel::Event::Msg(msg) = event else {
            return;
        };
        match msg {
            Msg::Ended if running => again = true,
            Msg::Ended => running = start(state, &ours, &holder),
            Msg::Holder { pidfd, displays } => {
                log::info!("takeback: another Reprise session holds the tray on {displays:?}");
                holder = displays;
                let ended = ours.clone();
                let exit = Generic::new(pidfd, Interest::READ, Mode::Level);
                let watched = state.handle.insert_source(exit, move |_, _, _| {
                    let _ = ended.send(Msg::Ended);
                    Ok(PostAction::Remove)
                });
                if let Err(e) = watched {
                    log::warn!("takeback: {}; only logind's report hands back", e.error);
                }
            }
            Msg::Done(outcome) => {
                took(state, outcome);
                running = std::mem::take(&mut again) && start(state, &ours, &holder);
            }
        }
    });
    if let Err(e) = inserted {
        log::warn!(
            "takeback: no channel ({}); an ended session hands nothing back",
            e.error
        );
        return Begun(None);
    }
    let defers = state.env.defers_user_units;
    let watcher = std::thread::Builder::new()
        .name("takeback-watch".into())
        .spawn({
            let tx = tx.clone();
            move || zbus::block_on(watch(tx, defers))
        });
    if let Err(e) = watcher {
        log::warn!("takeback: {e}; an ended session hands nothing back");
    }
    Begun(Some(tx))
}

/// Wait for a worker in flight, so its target start lands before the session's end stops it.
pub fn finish() {
    let worker = WORKER.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(worker) = worker {
        let _ = worker.join();
    }
}

/// The world [`take_back`] acts on in a running session.
struct Live {
    /// This session's, deferring to none.
    owner: SessionEnv,
    theme: String,
    tray: Option<async_channel::Sender<crate::tray::TrayCommand>>,
}

impl World for Live {
    fn gives_way(&self, to: GivesWay) -> bool {
        match to {
            GivesWay::LoginManager => crate::spawn::login_manager_session_runs_reprise(),
            GivesWay::LoginManagerOrOlder => {
                crate::spawn::login_manager_or_older_session_runs_reprise()
            }
        }
    }

    fn accepts(&self, display: &str) -> bool {
        accepts(display)
    }

    fn retake_names(&mut self) {
        if let Some(tray) = &self.tray {
            let _ = tray.try_send(crate::tray::TrayCommand::RetakeNames);
        }
    }

    fn retheme(&mut self) {
        if crate::spawn::portal_gtk_drop_in(&self.owner, &self.theme) {
            crate::spawn::run_to_end(&[
                "systemctl".into(),
                "--user".into(),
                "daemon-reload".into(),
            ]);
        }
    }

    fn run(&mut self, argv: &[String]) {
        crate::spawn::run_to_end(argv);
    }
}

/// Whether the worker thread started.
fn start(state: &State, done: &Sender<Msg>, holder: &[String]) -> bool {
    let env = state.env.clone();
    let mut world = Live {
        owner: SessionEnv {
            defers_user_units: false,
            ..env.clone()
        },
        theme: crate::tray::settings::Appearance::live(state)
            .gtk_theme()
            .to_string(),
        tray: state.tray.as_ref().map(|t| t.commands.clone()),
    };
    let holder = holder.to_vec();
    let done = done.clone();
    let worker = std::thread::Builder::new()
        .name("takeback".into())
        .spawn(move || {
            let outcome = match manager_display() {
                Ok(manager) => take_back(&env, manager.as_deref(), &holder, &mut world),
                Err(e) => {
                    log::warn!("takeback: the user manager's environment: {e}");
                    Outcome::Stays
                }
            };
            let _ = done.send(Msg::Done(outcome));
        });
    match worker {
        Ok(worker) => {
            *WORKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(worker);
            true
        }
        Err(_) => false,
    }
}

/// The user manager's `WAYLAND_DISPLAY`, None when unset.
fn manager_display() -> std::io::Result<Option<String>> {
    let mut c = std::process::Command::new("systemctl");
    c.args(["--user", "show-environment"]);
    crate::spawn::detach(&mut c);
    let out = c.stderr(std::process::Stdio::null()).output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!("systemctl {}", out.status)));
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let display = listing
        .lines()
        .find_map(|l| l.strip_prefix("WAYLAND_DISPLAY="));
    Ok(display.map(String::from))
}

/// Whether `display` accepts a connection; true when that can't be told. Non-blocking, so a
/// compositor with a full backlog counts as live.
fn accepts(display: &str) -> bool {
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType};
    let path = match std::env::var_os("XDG_RUNTIME_DIR") {
        _ if Path::new(display).is_absolute() => PathBuf::from(display),
        Some(runtime) => Path::new(&runtime).join(display),
        None => return true,
    };
    let Ok(addr) = SocketAddrUnix::new(path.as_path()) else {
        return true;
    };
    let flags = SocketFlags::CLOEXEC | SocketFlags::NONBLOCK;
    let Ok(fd) = rustix::net::socket_with(AddressFamily::UNIX, SocketType::STREAM, flags, None)
    else {
        return true;
    };
    match rustix::net::connect(&fd, &addr) {
        Ok(()) => true,
        Err(e) => e == rustix::io::Errno::AGAIN || e == rustix::io::Errno::INPROGRESS,
    }
}

/// The rest of a start once the worker is done: from then on the session defers to none.
fn took(state: &mut State, outcome: Outcome) {
    match outcome {
        Outcome::Stays => return,
        Outcome::Names if state.env.defers_user_units => log::info!(
            "the session this one gave way to ended; the bus names nobody holds are ours again"
        ),
        Outcome::Names => {}
        Outcome::Yields => log::info!(
            "the user manager has this session's variables again; a login manager's session \
             started meanwhile keeps the target"
        ),
        Outcome::Takes => {
            log::info!(
                "the session this one gave way to ended; its user manager setup is ours again"
            )
        }
    }
    state.env.defers_user_units = false;
}

/// Sends [`Msg::Ended`] for each session logind removes, and [`Msg::Holder`] for the compositor
/// of each other Reprise session holding the tray's name; then one `Ended` if `defers`, for a
/// holder that ended before this watch began.
async fn watch(ended: Sender<Msg>, defers: bool) {
    use crate::power::{LOGIN1_MANAGER, LOGIN1_NAME, LOGIN1_PATH, signal_rule};
    use futures_util::stream::{BoxStream, SelectAll, StreamExt};
    let mut wakes: SelectAll<BoxStream<'static, Msg>> = SelectAll::new();
    let system = zbus::Connection::system().await;
    match &system {
        Ok(conn) => {
            if let Some(rule) =
                signal_rule(LOGIN1_NAME, LOGIN1_PATH, LOGIN1_MANAGER, "SessionRemoved")
            {
                let removed = reprisebus::signals(conn, LOGIN1_NAME, vec![rule]).await;
                wakes.push(removed.map(|_| Msg::Ended).boxed());
            }
        }
        Err(e) => log::info!("takeback: no system bus ({e}); only the holder's exit hands back"),
    }
    let session = zbus::Connection::session().await;
    match &session {
        Ok(conn) => match holders(conn).await {
            Ok(h) => wakes.push(h),
            Err(e) => log::info!("takeback: the tray's owner: {e}"),
        },
        Err(e) => log::info!("takeback: no session bus ({e}); no holder is watched"),
    }
    if defers && ended.send(Msg::Ended).is_err() {
        return;
    }
    while let Some(msg) = wakes.next().await {
        if ended.send(msg).is_err() {
            return;
        }
    }
}

/// [`Msg::Holder`] for the tray name's owner now and each one after.
async fn holders(
    conn: &zbus::Connection,
) -> zbus::Result<futures_util::stream::BoxStream<'static, Msg>> {
    use futures_util::stream::StreamExt;
    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    let name = reprisebus::tray::NAME;
    let changes = (dbus
        .receive_name_owner_changed_with_args(&[(0, name)])
        .await?)
        .filter_map(|s| async move { Some(s.args().ok()?.new_owner().as_ref()?.to_string()) });
    let now = dbus
        .get_name_owner(zbus::names::BusName::try_from(name)?)
        .await
        .ok()
        .map(|o| o.to_string());
    let owners = futures_util::stream::iter(now).chain(changes);
    Ok(owners
        .filter_map(move |owner| {
            let dbus = dbus.clone();
            async move {
                let name = zbus::names::BusName::try_from(owner.as_str()).ok()?;
                holder_of(dbus.get_connection_unix_process_id(name).await.ok()?)
            }
        })
        .boxed())
}

/// [`Msg::Holder`] when `pid` is this compositor's program in another of the user's sessions.
fn holder_of(pid: u32) -> Option<Msg> {
    let pid = i32::try_from(pid).ok()?;
    // Before the reads, so they are of the process the pidfd watches.
    let pidfd = crate::hang::process_pidfd(pid).ok()?;
    let read = |f: &str| std::fs::read_to_string(format!("/proc/{pid}/{f}")).ok();
    (read("comm")? == std::fs::read_to_string("/proc/self/comm").ok()?).then_some(())?;
    let cgroup = read("cgroup")?;
    let own = reprisebus::logind::own_cgroup();
    let session = reprisebus::logind::session_of_cgroup(&cgroup)?;
    (Some(session) != reprisebus::logind::session_of_cgroup(&own)).then_some(())?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let links = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .ok()?
        .flatten()
        .filter_map(|e| std::fs::read_link(e.path()).ok());
    let displays = displays_held(links, Path::new(&runtime));
    Some(Msg::Holder { pidfd, displays })
}

/// The Wayland displays whose lock files are among `open` (a process's open files), named as
/// `WAYLAND_DISPLAY` names them: by file name when directly in `runtime`.
fn displays_held(open: impl IntoIterator<Item = PathBuf>, runtime: &Path) -> Vec<String> {
    open.into_iter()
        .filter_map(|path| {
            let socket = PathBuf::from(path.to_str()?.strip_suffix(".lock")?);
            let name = match socket.strip_prefix(runtime) {
                Ok(name) if name.components().count() == 1 => name,
                _ => &socket,
            };
            name.to_str().map(String::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tty() -> SessionEnv {
        SessionEnv {
            wayland_display: "wayland-0".into(),
            ..SessionEnv::default()
        }
    }

    /// Answers `gives_way` in turn from `gives_way`, the last one repeating; `live` are the
    /// displays that accept a connection. `did` lists what was done, commands by their first
    /// three words.
    struct Fake {
        gives_way: Vec<bool>,
        asked: std::cell::RefCell<Vec<GivesWay>>,
        live: Vec<&'static str>,
        did: Vec<String>,
    }

    fn fake(gives_way: &[bool], live: &[&'static str]) -> Fake {
        Fake {
            gives_way: gives_way.to_vec(),
            asked: Default::default(),
            live: live.to_vec(),
            did: Vec::new(),
        }
    }

    impl World for Fake {
        fn gives_way(&self, to: GivesWay) -> bool {
            let mut asked = self.asked.borrow_mut();
            asked.push(to);
            self.gives_way[(asked.len() - 1).min(self.gives_way.len() - 1)]
        }
        fn accepts(&self, display: &str) -> bool {
            self.live.contains(&display)
        }
        fn retake_names(&mut self) {
            self.did.push("names".into());
        }
        fn retheme(&mut self) {
            self.did.push("theme".into());
        }
        fn run(&mut self, argv: &[String]) {
            self.did.push(argv[..3].join(" "));
        }
    }

    /// What `take_back` decided and did, with `holder` serving wayland-1.
    fn run(env: &SessionEnv, manager: Option<&str>, w: &mut Fake) -> (Outcome, Vec<String>) {
        let outcome = take_back(env, manager, &["wayland-1".into()], w);
        (outcome, std::mem::take(&mut w.did))
    }

    fn did(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    const TAKES: [&str; 6] = [
        "names",
        "systemctl --user unset-environment",
        "dbus-update-activation-environment --systemd WAYLAND_DISPLAY=wayland-0",
        "theme",
        "systemctl --user stop",
        "systemctl --user --no-block",
    ];

    #[test]
    fn a_text_login_takes_the_user_manager_back_once_the_display_it_gave_way_to_is_gone() {
        assert_eq!(
            run(&tty(), Some("wayland-1"), &mut fake(&[true], &[])),
            (Outcome::Stays, vec![]),
            "while that session runs, the user manager stays its own"
        );
        assert_eq!(
            run(&tty(), Some("wayland-1"), &mut fake(&[false], &[])),
            (Outcome::Takes, did(&TAKES)),
            "names first, stale variables unset, the theme written before the stop"
        );
    }

    #[test]
    fn beside_another_desktop_of_the_user_an_ended_session_restarts_nothing() {
        assert_eq!(
            run(
                &tty(),
                Some("wayland-2"),
                &mut fake(&[false], &["wayland-2"])
            ),
            (Outcome::Stays, vec![]),
            "a live display that is not Reprise's"
        );
        assert_eq!(
            run(
                &tty(),
                Some("wayland-1"),
                &mut fake(&[false], &["wayland-1"])
            ),
            (Outcome::Stays, vec![]),
            "the session given way to still serves its display"
        );
        assert_eq!(
            run(&tty(), Some("wayland-2"), &mut fake(&[false], &[])),
            (Outcome::Names, did(&["names"])),
            "a gone display that was not the session's given way to"
        );
        assert_eq!(
            run(&tty(), None, &mut fake(&[false], &[])),
            (Outcome::Names, did(&["names"])),
            "nothing published"
        );
    }

    #[test]
    fn a_text_login_that_published_first_still_takes_its_names_back() {
        assert_eq!(
            run(&tty(), Some("wayland-0"), &mut fake(&[false], &[])),
            (Outcome::Names, did(&["names"]))
        );
    }

    #[test]
    fn a_text_login_that_deferred_at_its_start_defers_on_to_an_older_session() {
        let deferring = SessionEnv {
            defers_user_units: true,
            ..tty()
        };
        let mut w = fake(&[true], &[]);
        assert_eq!(
            run(&deferring, Some("wayland-1"), &mut w),
            (Outcome::Stays, vec![])
        );
        assert_eq!(w.asked.into_inner(), [GivesWay::LoginManagerOrOlder]);
        let mut w = fake(&[false], &[]);
        assert_eq!(
            run(&deferring, Some("wayland-1"), &mut w),
            (Outcome::Takes, did(&TAKES))
        );
        assert_eq!(
            w.asked.into_inner(),
            [GivesWay::LoginManagerOrOlder, GivesWay::LoginManager],
            "then a login manager's started meanwhile"
        );
    }

    #[test]
    fn a_login_managers_session_started_while_it_publishes_keeps_the_target() {
        assert_eq!(
            run(&tty(), Some("wayland-1"), &mut fake(&[false, true], &[])),
            (Outcome::Yields, did(&TAKES[..3])),
            "the newer session restarts the target on its own display"
        );
    }

    #[test]
    fn a_login_managers_or_a_nested_session_takes_nothing_back() {
        let dm = SessionEnv {
            logind_session: Some("7".into()),
            ..tty()
        };
        assert_eq!(
            run(&dm, Some("wayland-1"), &mut fake(&[false], &[])),
            (Outcome::Stays, vec![])
        );
        let nested = SessionEnv {
            parent: vec![("WAYLAND_DISPLAY", "wayland-1".into())],
            ..tty()
        };
        assert_eq!(
            run(&nested, Some("wayland-1"), &mut fake(&[false], &[])),
            (Outcome::Stays, vec![])
        );
    }

    #[test]
    fn a_holders_displays_are_the_wayland_locks_it_holds_open() {
        let open = [
            "/run/user/1000/wayland-1.lock",
            "/run/user/1000/wayland-1",
            "/tmp/elsewhere/wayland-9.lock",
            "/run/user/1000/sub/x.lock",
            "/dev/null",
        ]
        .map(PathBuf::from);
        assert_eq!(
            displays_held(open, Path::new("/run/user/1000")),
            [
                "wayland-1",
                "/tmp/elsewhere/wayland-9",
                "/run/user/1000/sub/x"
            ]
        );
    }

    #[test]
    fn a_display_with_a_listener_accepts_and_a_gone_one_does_not() {
        let dir = std::env::temp_dir().join(format!("takeback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("wayland-7");
        let _ = std::fs::remove_file(&socket);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let display = socket.to_str().unwrap();
        assert!(accepts(display));
        drop(listener);
        assert!(!accepts(display), "a socket file nobody listens on");
        std::fs::remove_file(&socket).unwrap();
        assert!(!accepts(display), "no socket file");
        std::fs::remove_dir(&dir).unwrap();
    }
}
