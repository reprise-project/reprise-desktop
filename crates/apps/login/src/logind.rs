//! logind and localed on the system bus: the proxies, and the reads and calls the daemon makes.

use std::cell::RefCell;
use std::future::Future;
use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use futures_util::StreamExt;
use repriselogin::greeter::{Keyboard, PowerAction};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use zbus::OwnedObjectPath;
use zbus::proxy::CacheProperties;
use zbus::{Connection, names::InterfaceName};

use crate::daemon::{FrontUnit, Inhibit, LogindSession, Unread};

const LOGIN1: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const SEAT0: &str = "/org/freedesktop/login1/seat/seat0";
const SESSIONS: &str = "/org/freedesktop/login1/session";
/// A session logind keeps after its leader is gone, until its last process exits.
const CLOSING: &str = "closing";
/// logind's error for a session id it does not know.
const NO_SUCH_SESSION: &str = "org.freedesktop.login1.NoSuchSession";
/// systemd's error for a unit or unit file that isn't installed.
const NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";
/// systemd's error for a second `Subscribe()` from one connection.
const ALREADY_SUBSCRIBED: &str = "org.freedesktop.systemd1.AlreadySubscribed";
/// How long a method call on the system bus waits for its reply.
const METHOD_TIMEOUT: Duration = Duration::from_secs(5);
/// How long systemd's `Reload()` may take; it replies only once the reload is done.
const RELOAD_TIMEOUT: Duration = Duration::from_secs(25);
/// The front's unit, which the daemon starts and stops by `[remote_desktop] enabled`.
const FRONT_UNIT: &str = "reprise-login-rdp.service";
/// The socket holding the front's port from boot, enabled while remote logins are on.
const FRONT_SOCKET: &str = "reprise-login-rdp.socket";
/// The socket's port, as `[remote_desktop] port`; system.control holds settings made over the
/// manager's bus API.
const FRONT_PORT_DROP_IN: &str = "/etc/systemd/system.control/reprise-login-rdp.socket.d/port.conf";
/// The pause between two reads of the port; one read of `/proc/net/tcp` can miss a listener
/// while others open and close.
const REREAD_PAUSE: Duration = Duration::from_millis(100);
/// How long a job on the front's units may take before it is reported unfinished.
const JOB_WAIT: Duration = Duration::from_secs(30);
const INHIBIT_WHO: &str = "Reprise login manager";

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
pub trait Manager {
    /// Each session as (id, uid, user name, seat, path).
    fn list_sessions(&self) -> zbus::Result<Vec<(String, u32, String, String, OwnedObjectPath)>>;
    #[zbus(name = "GetSessionByPID")]
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
    fn get_session(&self, id: &str) -> zbus::Result<OwnedObjectPath>;
    fn power_off(&self, interactive: bool) -> zbus::Result<()>;
    fn reboot(&self, interactive: bool) -> zbus::Result<()>;
    fn terminate_session(&self, id: &str) -> zbus::Result<()>;
    fn kill_session(&self, id: &str, who: &str, signal: i32) -> zbus::Result<()>;
    fn unlock_session(&self, id: &str) -> zbus::Result<()>;
    fn lock_session(&self, id: &str) -> zbus::Result<()>;
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<zbus::OwnedFd>;
    #[zbus(signal)]
    fn session_removed(&self, id: String, path: OwnedObjectPath) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Seat",
    default_service = "org.freedesktop.login1"
)]
pub trait Seat {
    fn switch_to(&self, vtnr: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn active_session(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
pub trait Session {
    #[zbus(property)]
    fn id(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, OwnedObjectPath)>;
    #[zbus(property, name = "VTNr")]
    fn vtnr(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn seat(&self) -> zbus::Result<(String, OwnedObjectPath)>;
    #[zbus(property)]
    fn class(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn service(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn leader(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn scope(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Unit",
    default_service = "org.freedesktop.systemd1"
)]
pub trait SystemdUnit {
    #[zbus(property)]
    fn active_state(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn need_daemon_reload(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait Systemd {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn restart_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn reload(&self) -> zbus::Result<()>;
    fn subscribe(&self) -> zbus::Result<()>;
    fn reset_failed_unit(&self, name: &str) -> zbus::Result<()>;
    fn load_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
    /// Sent with `active` false when a daemon-reload is done.
    #[zbus(signal)]
    fn reloading(&self, active: bool);
    #[zbus(signal)]
    fn job_removed(&self, id: u32, job: OwnedObjectPath, unit: String, result: String);
    fn enable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
        force: bool,
    ) -> zbus::Result<(bool, Vec<(String, String, String)>)>;
    fn disable_unit_files(
        &self,
        files: &[&str],
        runtime: bool,
    ) -> zbus::Result<Vec<(String, String, String)>>;
}

/// Heads a drop-in whose port the socket failed to take, so the next write of that port differs.
const UNTAKEN: &str = "# The socket failed to take this port; reprise-login tries it again.\n";

/// The socket drop-in that listens on `port` alone.
fn port_drop_in(port: u16) -> String {
    format!("[Socket]\nListenStream=\nListenStream={port}\n")
}

/// Writes the socket drop-in at `path` that listens on `port`; true when it changed.
fn write_port(path: &Path, port: u16) -> io::Result<bool> {
    write_drop_in(path, &port_drop_in(port))
}

fn write_drop_in(path: &Path, text: &str) -> io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(true)
}

/// A systemd job on a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnitJob {
    Start,
    Restart,
    Stop,
}

/// What systemd says of a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UnitState {
    pub active: bool,
    /// Its unit files changed on disk since systemd last loaded them.
    pub need_reload: bool,
}

/// The systemd manager calls a [`FrontUnit`] is made of; a fake stands in for tests.
pub(crate) trait Units {
    async fn reload(&self) -> zbus::Result<()>;
    /// Enables `unit`; true when a unit file link changed, which takes a reload.
    async fn enable(&self, unit: &str) -> zbus::Result<bool>;
    /// Disables `unit`; true when a unit file link changed, which takes a reload.
    async fn disable(&self, unit: &str) -> zbus::Result<bool>;
    /// Clears `unit`'s failed state and its start limit's count.
    async fn reset_failed(&self, unit: &str) -> zbus::Result<()>;
    async fn state(&self, unit: &str) -> zbus::Result<UnitState>;
    /// Whether systemd holds a socket listening on TCP `port`.
    async fn listening(&self, port: u16) -> io::Result<bool>;
    /// Runs `job` on `unit` to its end; a job that ends other than "done" is an error.
    async fn run(&self, job: UnitJob, unit: &str) -> zbus::Result<()>;
}

/// Applies the front changes sent on the returned queue one at a time, in order, so a Stop
/// never runs between the calls of a Start. After each daemon-reload sent on `reloads` while
/// remote logins are on, a socket the reload left without its port is restarted with the front.
/// The task ends when the queue closes.
pub(crate) fn apply_front_units<U: Units + 'static>(
    units: U,
    drop_in: PathBuf,
    mut reloads: mpsc::UnboundedReceiver<()>,
) -> (mpsc::UnboundedSender<FrontUnit>, JoinHandle<()>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<FrontUnit>();
    let task = tokio::task::spawn_local(async move {
        let mut on = None;
        loop {
            let unit = tokio::select! {
                unit = rx.recv() => match unit {
                    Some(unit) => unit,
                    None => break,
                },
                Some(()) = reloads.recv() => match on {
                    Some(port) if socket_unheld(&units, port).await => FrontUnit::Start { port },
                    _ => continue,
                },
            };
            on = match unit {
                FrontUnit::Start { port } => Some(port),
                FrontUnit::Stop => None,
            };
            if let Err(e) = apply(&units, &drop_in, unit).await {
                log::error!("{unit:?} of {FRONT_UNIT}: {e}");
            }
        }
    });
    (tx, task)
}

/// Starts [`FRONT_UNIT`] on `port` held by [`FRONT_SOCKET`], or stops both and frees the port.
async fn apply(units: &impl Units, drop_in: &Path, unit: FrontUnit) -> Result<(), String> {
    match unit {
        FrontUnit::Start { port } => {
            let moved =
                write_port(drop_in, port).map_err(|e| format!("{}: {e}", drop_in.display()))?;
            let held = hold(units, port, moved).await;
            if held.is_err() && moved {
                // Kept on the port so a boot binds no other, and marked so the next Start finds
                // it moved and reloads and restarts the socket again.
                let _ = write_drop_in(drop_in, &format!("{UNTAKEN}{}", port_drop_in(port)));
            }
            held?;
            (units.run(UnitJob::Start, FRONT_UNIT).await).map_err(|e| format!("{FRONT_UNIT}: {e}"))
        }
        FrontUnit::Stop => {
            let front = units.run(UnitJob::Stop, FRONT_UNIT).await;
            let socket = units.run(UnitJob::Stop, FRONT_SOCKET).await;
            let disabled = units.disable(FRONT_SOCKET).await;
            let reloaded = match disabled {
                Ok(true) => units.reload().await,
                _ => Ok(()),
            };
            let steps = [
                (FRONT_UNIT, front),
                (FRONT_SOCKET, socket),
                (FRONT_SOCKET, disabled.map(drop)),
                ("reloading systemd", reloaded),
            ];
            let failed: Vec<String> = (steps.into_iter())
                .filter_map(|(unit, done)| match done {
                    Ok(()) => None,
                    Err(e) if no_such_unit(&e) => {
                        log::debug!("{unit}: {e}");
                        None
                    }
                    Err(e) => Some(format!("{unit}: {e}")),
                })
                .collect();
            match failed.is_empty() {
                true => Ok(()),
                false => Err(failed.join("; ")),
            }
        }
    }
}

/// Has [`FRONT_SOCKET`] hold `port` from boot on, restarting it when the port `moved` or systemd
/// holds no socket on it. Clears both units' failures first and loads unit files changed on disk.
async fn hold(units: &impl Units, port: u16, moved: bool) -> Result<(), String> {
    for unit in [FRONT_SOCKET, FRONT_UNIT] {
        if let Err(e) = units.reset_failed(unit).await {
            log::debug!("{unit}: resetting its failures: {e}");
        }
    }
    let enabled = (units.enable(FRONT_SOCKET).await).map_err(|e| format!("{FRONT_SOCKET}: {e}"))?;
    let socket = units.state(FRONT_SOCKET).await;
    let front = units.state(FRONT_UNIT).await;
    let stale = [&socket, &front]
        .iter()
        .any(|s| s.as_ref().is_ok_and(|s| s.need_reload));
    if moved || enabled || stale {
        (units.reload().await).map_err(|e| format!("reloading systemd: {e}"))?;
    }
    // A reload that finds a running socket's unit changed can leave it holding no fds, and the
    // front then fails to its start limit. A restart stops the front too (Requires=), and the
    // Start after this one brings it back.
    let deaf = !moved && socket.is_ok_and(|s| s.active) && unheld(units, port).await;
    if deaf {
        log::warn!("{FRONT_SOCKET} is active but holds no socket on port {port}; restarting it");
    }
    let job = if moved || deaf {
        UnitJob::Restart
    } else {
        UnitJob::Start
    };
    (units.run(job, FRONT_SOCKET).await).map_err(|e| {
        format!(
            "{FRONT_SOCKET}: {e}; if another program holds port {port}, `journalctl -u {FRONT_SOCKET}` says so"
        )
    })
}

/// Whether systemd holds no socket on `port` in two reads [`REREAD_PAUSE`] apart; a failed
/// read counts as held.
async fn unheld(units: &impl Units, port: u16) -> bool {
    let unheld = async || (units.listening(port).await).is_ok_and(|held| !held);
    unheld().await && {
        tokio::time::sleep(REREAD_PAUSE).await;
        unheld().await
    }
}

/// Whether [`FRONT_SOCKET`] is active while systemd holds no socket on `port`.
async fn socket_unheld(units: &impl Units, port: u16) -> bool {
    (units.state(FRONT_SOCKET).await).is_ok_and(|s| s.active) && unheld(units, port).await
}

/// The inodes of the sockets listening on `port` in `table`, a `/proc/net/tcp` or `tcp6`.
fn listening_inodes(table: &str, port: u16) -> Vec<u64> {
    const LISTEN: &str = "0A";
    let port = format!(":{port:04X}");
    (table.lines().skip(1))
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (local, state, inode) = (fields.get(1)?, fields.get(3)?, fields.get(9)?);
            (local.ends_with(&port) && *state == LISTEN).then(|| inode.parse().ok())?
        })
        .collect()
}

/// Whether `e` is systemd's answer for a unit that isn't installed.
fn no_such_unit(e: &zbus::Error) -> bool {
    reprisebus::error::name(e).is_some_and(|n| n == NO_SUCH_UNIT)
}

/// `Subscribe()`'s result, with a connection that was already subscribed counted as success.
fn subscribed(r: zbus::Result<()>) -> zbus::Result<()> {
    match r {
        Err(e) if reprisebus::error::name(&e).is_some_and(|n| n == ALREADY_SUBSCRIBED) => Ok(()),
        r => r,
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.locale1",
    default_service = "org.freedesktop.locale1",
    default_path = "/org/freedesktop/locale1"
)]
pub trait Locale {
    #[zbus(property, name = "X11Layout")]
    fn x11_layout(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Variant")]
    fn x11_variant(&self) -> zbus::Result<String>;
    #[zbus(property, name = "X11Options")]
    fn x11_options(&self) -> zbus::Result<String>;
}

/// The console keyboard as localed names it; an empty or missing layout is `us`.
pub fn keyboard_from(
    layout: Option<String>,
    variant: Option<String>,
    options: Option<String>,
) -> Keyboard {
    Keyboard {
        layout: layout
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| "us".into()),
        variant: variant.unwrap_or_default(),
        options: options.unwrap_or_default(),
    }
}

/// Whether `e` says the session is gone: logind's `NoSuchSession`, or `UnknownObject` from a
/// session object that went away. A greeter of the last daemon is often gone this way.
pub fn session_gone(e: &zbus::Error) -> bool {
    matches!(
        reprisebus::error::name(e).as_deref(),
        Some(NO_SUCH_SESSION | "org.freedesktop.DBus.Error.UnknownObject")
    )
}

/// logind's sessions as one `ListSessions` lists them.
#[derive(Debug, Default)]
pub struct Listing {
    /// Those whose properties could be read, closing ones included.
    pub found: Vec<LogindSession>,
    /// Those whose properties could not be read.
    pub unread: Vec<Unread>,
}

/// Whether a session is gone or closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Yes,
    No,
    /// logind could not be read.
    Unknown,
}

/// What a read of whether a session is closing says of its end.
fn ended_by(closing: zbus::Result<bool>) -> Ended {
    match closing {
        Ok(true) => Ended::Yes,
        Ok(false) => Ended::No,
        Err(e) if session_gone(&e) => Ended::Yes,
        Err(_) => Ended::Unknown,
    }
}

/// The daemon's system bus connection with logind's manager on it.
#[derive(Clone)]
pub struct Logind {
    conn: Connection,
    manager: ManagerProxy<'static>,
}

impl Logind {
    pub async fn connect() -> zbus::Result<Logind> {
        let conn = zbus::connection::Builder::system()
            .method_timeout(METHOD_TIMEOUT)
            .build()
            .await?;
        let manager = ManagerProxy::builder(&conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        Ok(Logind { conn, manager })
    }

    pub fn manager(&self) -> &ManagerProxy<'static> {
        &self.manager
    }

    async fn session(&self, path: OwnedObjectPath) -> zbus::Result<SessionProxy<'static>> {
        SessionProxy::builder(&self.conn)
            .path(path)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    pub async fn listing(&self) -> zbus::Result<Listing> {
        let mut listing = Listing::default();
        for (id, uid, user, seat, path) in self.manager.list_sessions().await? {
            match self.describe(path).await {
                Ok((vt, class, service, closing)) => listing.found.push(LogindSession {
                    id,
                    user,
                    uid,
                    vt,
                    seat,
                    class,
                    service,
                    closing,
                }),
                Err(e) if session_gone(&e) => log::debug!("logind session {id}: {e}"),
                Err(e) => {
                    log::warn!("logind session {id}: {e}");
                    listing.unread.push(Unread {
                        id,
                        uid,
                        user,
                        seat,
                    });
                }
            }
        }
        Ok(listing)
    }

    /// Describes session `u` again.
    pub async fn read(&self, u: &Unread) -> zbus::Result<LogindSession> {
        let path = self.manager.get_session(&u.id).await?;
        let (vt, class, service, closing) = self.describe(path).await?;
        Ok(LogindSession {
            id: u.id.clone(),
            user: u.user.clone(),
            uid: u.uid,
            vt,
            seat: u.seat.clone(),
            class,
            service,
            closing,
        })
    }

    /// The VT, class and service of the session at `path`, and whether it is closing.
    async fn describe(&self, path: OwnedObjectPath) -> zbus::Result<(u16, String, String, bool)> {
        let s = self.session(path).await?;
        let closing = s.state().await? == CLOSING;
        let vt = u16::try_from(s.vtnr().await?).unwrap_or(0);
        Ok((vt, s.class().await?, s.service().await?, closing))
    }

    /// Stops session `id`'s scope, which `TerminateSession` leaves running once the session
    /// is closing.
    pub async fn stop_session(&self, id: &str) -> zbus::Result<()> {
        let path = self.manager.get_session(id).await?;
        let scope = self.session(path).await?.scope().await?;
        self.systemd().await?.stop_unit(&scope, "replace").await?;
        Ok(())
    }

    async fn systemd(&self) -> zbus::Result<SystemdProxy<'static>> {
        SystemdProxy::builder(&self.conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// The queue the daemon's front changes go through, applied in order by
    /// [`apply_front_units`].
    pub fn front_units(&self) -> mpsc::UnboundedSender<FrontUnit> {
        let (reloaded, reloads) = mpsc::unbounded_channel();
        tokio::task::spawn_local(self.clone().send_reloads(reloaded));
        apply_front_units(self.clone(), PathBuf::from(FRONT_PORT_DROP_IN), reloads).0
    }

    /// Sends on `reloaded` each time systemd finishes a daemon-reload, until it closes.
    async fn send_reloads(self, reloaded: mpsc::UnboundedSender<()>) {
        let followed = async {
            let systemd = self.systemd().await?;
            let mut reloading = systemd.receive_reloading().await?;
            subscribed(systemd.subscribe().await)?;
            while let Some(signal) = reloading.next().await {
                if signal.args().is_ok_and(|a| !a.active) && reloaded.send(()).is_err() {
                    break;
                }
            }
            zbus::Result::Ok(())
        };
        if let Err(e) = followed.await {
            log::warn!("following systemd's reloads: {e}");
        }
    }

    /// A block inhibitor of `what`, held until the fd is dropped.
    pub async fn inhibit(&self, what: &str, why: &str) -> zbus::Result<OwnedFd> {
        let fd = self
            .manager
            .inhibit(what, INHIBIT_WHO, why, "block")
            .await?;
        Ok(cloexec(fd.into())?)
    }

    /// The pid of session `id`'s leader.
    pub async fn leader(&self, id: &str) -> Option<u32> {
        let path = self.manager.get_session(id).await.ok()?;
        self.session(path).await.ok()?.leader().await.ok()
    }

    /// The open logind session of process `pid`.
    pub async fn session_of(&self, pid: u32) -> Option<String> {
        let path = self.manager.get_session_by_pid(pid).await.ok()?;
        let s = self.session(path).await.ok()?;
        match s.state().await.ok()?.as_str() {
            CLOSING => None,
            _ => s.id().await.ok(),
        }
    }

    /// Whether session `id` is gone or closing; `Unknown` when logind fails to say.
    pub async fn ended(&self, id: &str) -> Ended {
        let state = match self.manager.get_session(id).await {
            Ok(path) => match self.session(path).await {
                Ok(s) => s.state().await,
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        if let Err(e) = &state {
            log::debug!("logind session {id}: {e}");
        }
        ended_by(state.map(|state| state == CLOSING))
    }

    async fn seat0(&self) -> zbus::Result<SeatProxy<'static>> {
        SeatProxy::builder(&self.conn)
            .path(SEAT0)
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    /// seat0's active session, `None` when it has none.
    pub async fn seat_active(&self) -> Option<String> {
        let (id, _) = self.seat0().await.ok()?.active_session().await.ok()?;
        (!id.is_empty()).then_some(id)
    }

    pub async fn switch_to(&self, vt: u16) -> zbus::Result<()> {
        self.seat0().await?.switch_to(vt.into()).await
    }

    pub async fn power(&self, action: PowerAction) -> zbus::Result<()> {
        match action {
            PowerAction::PowerOff => self.manager.power_off(false).await,
            PowerAction::Reboot => self.manager.reboot(false).await,
        }
    }

    /// `UnlockSession(id)`, then true once the session's `LockedHint` is false, or false
    /// when it is still set after `wait`.
    pub async fn unlock(&self, id: &str, wait: Duration) -> zbus::Result<bool> {
        let path = self.manager.get_session(id).await?;
        let props = zbus::fdo::PropertiesProxy::builder(&self.conn)
            .destination(LOGIN1)
            .path(path.clone())
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        // Subscribed before the call, so a change in between is seen.
        let mut changes = props.receive_properties_changed().await?;
        self.manager.unlock_session(id).await?;
        let session = self.session(path).await?;
        if !session.locked_hint().await? {
            return Ok(true);
        }
        let iface = InterfaceName::from_static_str_unchecked(SESSION);
        let dropped = async {
            while let Some(change) = changes.next().await {
                let Ok(args) = change.args() else { continue };
                if *args.interface_name() != iface {
                    continue;
                }
                let value = match args.changed_properties().get("LockedHint") {
                    Some(v) => bool::try_from(v).ok(),
                    None if args.invalidated_properties().contains(&"LockedHint") => {
                        session.locked_hint().await.ok()
                    }
                    None => continue,
                };
                if value == Some(false) {
                    return true;
                }
            }
            false
        };
        Ok(tokio::time::timeout(wait, dropped).await.unwrap_or(false))
    }

    /// Calls `send(id, locked)` with every session's `LockedHint`, then on each change, until it
    /// returns false. A signal only prompts a read from logind, so a forged one changes nothing.
    pub async fn follow_locked_hints(
        &self,
        send: impl Fn(String, bool) -> bool,
    ) -> zbus::Result<()> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.DBus.Properties")
            .member("PropertiesChanged")
            .path_namespace(SESSIONS)
            .arg(0, SESSION)
            .build()?;
        // Subscribed before the reads, so a change in between is seen.
        let mut changes = zbus::MessageStream::for_match_rule(rule, &self.conn, None).await?;
        for (id, _, _, _, path) in self.manager.list_sessions().await? {
            let Ok(locked) = self.session(path).await?.locked_hint().await else {
                continue;
            };
            if !send(id, locked) {
                return Ok(());
            }
        }
        while let Some(msg) = changes.next().await {
            let Ok(msg) = msg else { continue };
            let Some(change) = zbus::fdo::PropertiesChanged::from_message(msg.clone()) else {
                continue;
            };
            let Ok(args) = change.args() else { continue };
            let named = args.changed_properties().contains_key("LockedHint")
                || args.invalidated_properties().contains(&"LockedHint");
            let path = msg
                .header()
                .path()
                .map(|p| OwnedObjectPath::from(p.to_owned()));
            let (true, Some(path)) = (named, path) else {
                continue;
            };
            let s = self.session(path).await?;
            let (Ok(id), Ok(locked)) = (s.id().await, s.locked_hint().await) else {
                continue;
            };
            if !send(id, locked) {
                break;
            }
        }
        Ok(())
    }

    /// The console keyboard from localed, `us` when it names none.
    pub async fn keyboard(&self) -> Keyboard {
        let locale = LocaleProxy::builder(&self.conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await;
        let Ok(locale) = locale else {
            return keyboard_from(None, None, None);
        };
        keyboard_from(
            locale.x11_layout().await.ok(),
            locale.x11_variant().await.ok(),
            locale.x11_options().await.ok(),
        )
    }
}

/// `fd` with `FD_CLOEXEC` set. zbus receives fds without it, and a worker spawned while one
/// is open would keep it, and with it the inhibitor.
fn cloexec(fd: OwnedFd) -> std::io::Result<OwnedFd> {
    use std::os::fd::AsRawFd;
    // SAFETY: F_SETFD on an fd this function owns has no other preconditions.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

/// logind's `what` and `why` for an inhibitor; `None` for none.
fn inhibit_args(inhibit: Inhibit) -> Option<(&'static str, &'static str)> {
    match inhibit {
        Inhibit::Nothing => None,
        Inhibit::Idle => Some(("idle", "a session runs in the background")),
        Inhibit::SleepAndIdle => Some(("sleep:idle", "a remote client is connected")),
    }
}

/// Takes an inhibitor of `what` for `why`; production passes [`Logind::inhibit`].
pub type TakeInhibitor =
    Rc<dyn Fn(&'static str, &'static str) -> Pin<Box<dyn Future<Output = zbus::Result<OwnedFd>>>>>;

impl Units for Logind {
    async fn reload(&self) -> zbus::Result<()> {
        let conn = zbus::connection::Builder::system()
            .method_timeout(RELOAD_TIMEOUT)
            .build()
            .await?;
        SystemdProxy::builder(&conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?
            .reload()
            .await
    }

    async fn enable(&self, unit: &str) -> zbus::Result<bool> {
        let systemd = self.systemd().await?;
        let (_, changes) = systemd.enable_unit_files(&[unit], false, true).await?;
        Ok(!changes.is_empty())
    }

    async fn disable(&self, unit: &str) -> zbus::Result<bool> {
        let systemd = self.systemd().await?;
        let changes = systemd.disable_unit_files(&[unit], false).await?;
        Ok(!changes.is_empty())
    }

    async fn reset_failed(&self, unit: &str) -> zbus::Result<()> {
        self.systemd().await?.reset_failed_unit(unit).await
    }

    async fn state(&self, unit: &str) -> zbus::Result<UnitState> {
        let path = self.systemd().await?.load_unit(unit).await?;
        let unit = SystemdUnitProxy::builder(&self.conn)
            .path(path)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        Ok(UnitState {
            active: unit.active_state().await? == "active",
            need_reload: unit.need_daemon_reload().await?,
        })
    }

    async fn listening(&self, port: u16) -> io::Result<bool> {
        let mut inodes = Vec::new();
        for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
            match std::fs::read_to_string(table) {
                Ok(text) => inodes.extend(listening_inodes(&text, port)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        if inodes.is_empty() {
            return Ok(false);
        }
        for fd in std::fs::read_dir("/proc/1/fd")? {
            let Ok(link) = std::fs::read_link(fd?.path()) else {
                continue;
            };
            let inode = (link.to_str())
                .and_then(|l| l.strip_prefix("socket:[")?.strip_suffix(']'))
                .and_then(|i| i.parse::<u64>().ok());
            if inode.is_some_and(|i| inodes.contains(&i)) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn run(&self, job: UnitJob, unit: &str) -> zbus::Result<()> {
        let systemd = self.systemd().await?;
        // Taken before the job is queued, so its removal can't come first.
        let mut removed = systemd.receive_job_removed().await?;
        subscribed(systemd.subscribe().await)?;
        let path = match job {
            UnitJob::Start => systemd.start_unit(unit, "replace").await?,
            UnitJob::Restart => systemd.restart_unit(unit, "replace").await?,
            UnitJob::Stop => systemd.stop_unit(unit, "replace").await?,
        };
        let ended = async {
            while let Some(signal) = removed.next().await {
                let args = signal.args()?;
                if *args.job() == path {
                    return Ok(args.result().clone());
                }
            }
            Err(zbus::Error::Failure("systemd's JobRemoved ended".into()))
        };
        let result = tokio::time::timeout(JOB_WAIT, ended).await.map_err(|_| {
            zbus::Error::Failure(format!("{job:?} still runs after {JOB_WAIT:?}"))
        })??;
        match result.as_str() {
            "done" => Ok(()),
            _ => Err(zbus::Error::Failure(format!("{job:?} ended {result}"))),
        }
    }
}

/// The inhibitor the daemon holds. A new one is taken before the old one is dropped, and
/// one that arrives after a later change is dropped at once. One take runs at a time; changes
/// during it leave only the latest to take after it.
pub struct Inhibitor {
    take: TakeInhibitor,
    state: Rc<RefCell<Held>>,
}

#[derive(Default)]
struct Held {
    /// Each change's number.
    change: u64,
    fd: Option<OwnedFd>,
    taking: bool,
    /// The change to take once the take in flight ends.
    next: Option<(u64, &'static str, &'static str)>,
}

impl Inhibitor {
    pub fn new(take: TakeInhibitor) -> Inhibitor {
        Inhibitor {
            take,
            state: Rc::default(),
        }
    }

    /// Holds `want` from now on, in a task of the current `LocalSet`.
    pub fn set(&self, want: Inhibit) {
        let mut st = self.state.borrow_mut();
        st.change += 1;
        let change = st.change;
        st.next = None;
        let Some((what, why)) = inhibit_args(want) else {
            st.fd = None;
            return;
        };
        if std::mem::replace(&mut st.taking, true) {
            st.next = Some((change, what, why));
            return;
        }
        drop(st);
        let (take, state) = (self.take.clone(), self.state.clone());
        tokio::task::spawn_local(async move {
            let (mut change, mut what, mut why) = (change, what, why);
            loop {
                let taken = take(what, why).await;
                let mut st = state.borrow_mut();
                match taken {
                    Ok(fd) if st.change == change => st.fd = Some(fd),
                    Ok(_) => {}
                    Err(e) if st.fd.is_some() => {
                        log::warn!("logind's {what} inhibitor: {e}; previous one stays held");
                    }
                    Err(e) => log::warn!("logind's {what} inhibitor: {e}"),
                }
                let Some(next) = st.next.take() else {
                    st.taking = false;
                    return;
                };
                (change, what, why) = next;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_drop_in_is_rewritten_only_when_the_port_moves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise-login-rdp.socket.d/port.conf");
        assert!(
            write_port(&path, 3390).unwrap(),
            "a missing drop-in is written"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Socket]\nListenStream=\nListenStream=3390\n"
        );
        assert!(
            !write_port(&path, 3390).unwrap(),
            "the same port is left alone"
        );
        assert!(write_port(&path, 3389).unwrap());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .ends_with("ListenStream=3389\n")
        );
    }

    /// systemd as far as the front's units go. A reload yields, so tasks can run inside it.
    #[derive(Clone, Default)]
    struct FakeUnits(Rc<RefCell<Fake>>);

    #[derive(Default)]
    struct Fake {
        /// Neither unit is installed.
        missing: bool,
        enabled: bool,
        active: std::collections::BTreeSet<&'static str>,
        /// Another program holds the port, so the socket fails to start.
        port_held: bool,
        failing_stop: Option<&'static str>,
        jobs: Vec<(UnitJob, &'static str)>,
        /// Unit files changed since the last reload, which `systemctl` would warn about.
        stale: bool,
        reloads: usize,
        /// The socket is active but holds no fds, so the front starts without its port.
        deaf: bool,
        /// Units past their start limit, which refuse a start until reset.
        limited: std::collections::BTreeSet<&'static str>,
        /// Whether systemd listens on the port can't be read.
        listening_unknown: bool,
        /// Reads of the port that miss systemd's socket, as a read of the table during churn can.
        missed_reads: usize,
        reads: usize,
    }

    impl FakeUnits {
        fn unit(&self, unit: &str) -> zbus::Result<&'static str> {
            if self.0.borrow().missing {
                return Err(error_reply(NO_SUCH_UNIT));
            }
            Ok([FRONT_UNIT, FRONT_SOCKET]
                .into_iter()
                .find(|u| *u == unit)
                .expect("a front unit"))
        }

        fn start(&self, unit: &'static str) -> zbus::Result<()> {
            let mut fake = self.0.borrow_mut();
            if unit == FRONT_SOCKET && fake.port_held {
                return Err(zbus::Error::Failure("Start ended failed".into()));
            }
            if unit == FRONT_UNIT && fake.port_held && !fake.active.contains(FRONT_SOCKET) {
                return Err(zbus::Error::Failure("Start ended dependency".into()));
            }
            if fake.limited.contains(unit) || (unit == FRONT_UNIT && fake.deaf) {
                return Err(zbus::Error::Failure("Start ended failed".into()));
            }
            fake.active.insert(FRONT_SOCKET);
            fake.active.insert(unit);
            Ok(())
        }
    }

    impl Units for FakeUnits {
        async fn reload(&self) -> zbus::Result<()> {
            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
            let mut fake = self.0.borrow_mut();
            fake.stale = false;
            fake.reloads += 1;
            Ok(())
        }

        async fn enable(&self, unit: &str) -> zbus::Result<bool> {
            self.unit(unit)?;
            let mut fake = self.0.borrow_mut();
            let changed = !fake.enabled;
            fake.stale |= changed;
            fake.enabled = true;
            Ok(changed)
        }

        async fn disable(&self, unit: &str) -> zbus::Result<bool> {
            self.unit(unit)?;
            let mut fake = self.0.borrow_mut();
            let changed = fake.enabled;
            fake.stale |= changed;
            fake.enabled = false;
            Ok(changed)
        }

        async fn reset_failed(&self, unit: &str) -> zbus::Result<()> {
            let unit = self.unit(unit)?;
            self.0.borrow_mut().limited.remove(unit);
            Ok(())
        }

        async fn state(&self, unit: &str) -> zbus::Result<UnitState> {
            let unit = self.unit(unit)?;
            let fake = self.0.borrow();
            Ok(UnitState {
                active: fake.active.contains(unit),
                need_reload: fake.stale,
            })
        }

        async fn listening(&self, _port: u16) -> io::Result<bool> {
            let mut fake = self.0.borrow_mut();
            fake.reads += 1;
            if fake.listening_unknown {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            if fake.missed_reads > 0 {
                fake.missed_reads -= 1;
                return Ok(false);
            }
            Ok(fake.active.contains(FRONT_SOCKET) && !fake.deaf)
        }

        async fn run(&self, job: UnitJob, unit: &str) -> zbus::Result<()> {
            let unit = self.unit(unit)?;
            self.0.borrow_mut().jobs.push((job, unit));
            match job {
                UnitJob::Start => self.start(unit),
                UnitJob::Restart => {
                    let mut fake = self.0.borrow_mut();
                    fake.active.remove(unit);
                    if unit == FRONT_SOCKET {
                        fake.active.remove(FRONT_UNIT);
                        fake.deaf = false;
                    }
                    drop(fake);
                    self.start(unit)
                }
                UnitJob::Stop => {
                    let mut fake = self.0.borrow_mut();
                    if fake.failing_stop == Some(unit) {
                        return Err(zbus::Error::Failure("Stop ended failed".into()));
                    }
                    fake.active.remove(unit);
                    if unit == FRONT_SOCKET {
                        fake.active.remove(FRONT_UNIT);
                    }
                    Ok(())
                }
            }
        }
    }

    #[tokio::test]
    async fn turning_remote_logins_on_then_off_at_once_ends_with_the_port_freed() {
        let dir = tempfile::tempdir().unwrap();
        let units = FakeUnits::default();
        let applied = units.clone();
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (_reloaded, reloads) = mpsc::unbounded_channel();
                let (tx, task) = apply_front_units(applied, dir.path().join("port.conf"), reloads);
                tx.send(FrontUnit::Start { port: 3390 }).unwrap();
                tx.send(FrontUnit::Stop).unwrap();
                drop(tx);
                task.await.unwrap();
                // Gives any apply still in flight its turn.
                for _ in 0..20 {
                    tokio::task::yield_now().await;
                }
            })
            .await;
        let fake = units.0.borrow();
        assert!(fake.jobs.contains(&(UnitJob::Start, FRONT_UNIT)));
        assert_eq!(fake.jobs.last(), Some(&(UnitJob::Stop, FRONT_SOCKET)));
        assert!(!fake.enabled, "the socket is disabled");
        assert!(fake.active.is_empty(), "{:?} still run", fake.active);
    }

    #[tokio::test(start_paused = true)]
    async fn a_daemon_reload_that_leaves_the_socket_without_its_port_restarts_it_while_remote_logins_are_on()
     {
        let dir = tempfile::tempdir().unwrap();
        let units = FakeUnits::default();
        let applied = units.clone();
        let settle = || tokio::time::sleep(Duration::from_secs(1));
        tokio::task::LocalSet::new()
            .run_until(async move {
                let (reloaded, reloads) = mpsc::unbounded_channel();
                let (tx, _task) = apply_front_units(applied, dir.path().join("port.conf"), reloads);
                tx.send(FrontUnit::Start { port: 3389 }).unwrap();
                settle().await;
                units.0.borrow_mut().jobs.clear();
                reloaded.send(()).unwrap();
                settle().await;
                assert!(
                    units.0.borrow().jobs.is_empty(),
                    "a socket on its port is left alone"
                );

                units.0.borrow_mut().deaf = true;
                reloaded.send(()).unwrap();
                settle().await;
                {
                    let fake = units.0.borrow();
                    assert!(!fake.deaf, "the socket holds its port again");
                    assert_eq!(
                        fake.jobs,
                        [
                            (UnitJob::Restart, FRONT_SOCKET),
                            (UnitJob::Start, FRONT_UNIT)
                        ]
                    );
                    assert_eq!(fake.active.len(), 2, "{:?}", fake.active);
                }

                tx.send(FrontUnit::Stop).unwrap();
                settle().await;
                {
                    let mut fake = units.0.borrow_mut();
                    fake.active.insert(FRONT_SOCKET);
                    fake.deaf = true;
                    fake.jobs.clear();
                }
                reloaded.send(()).unwrap();
                settle().await;
                assert!(units.0.borrow().jobs.is_empty(), "remote logins are off");
            })
            .await;
    }

    #[tokio::test]
    async fn a_port_the_socket_could_not_take_is_tried_again_by_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let drop_in = dir.path().join("port.conf");
        let units = FakeUnits::default();
        units.0.borrow_mut().port_held = true;
        let start = FrontUnit::Start { port: 3390 };
        let e = apply(&units, &drop_in, start).await.unwrap_err();
        assert!(e.contains(FRONT_SOCKET), "the socket is named: {e}");
        assert!(!units.0.borrow().active.contains(FRONT_UNIT));
        units.0.borrow_mut().port_held = false;
        units.0.borrow_mut().jobs.clear();
        apply(&units, &drop_in, start).await.unwrap();
        let fake = units.0.borrow();
        assert_eq!(
            fake.jobs,
            [
                (UnitJob::Restart, FRONT_SOCKET),
                (UnitJob::Start, FRONT_UNIT)
            ],
            "the socket moves to the port"
        );
        assert!(fake.enabled);
        assert_eq!(fake.active.len(), 2, "{:?}", fake.active);
    }

    /// The ports systemd binds for the socket at boot: the shipped unit's, then the drop-in's.
    fn boot_ports(drop_in: &Path) -> Vec<u16> {
        let unit = include_str!("../../../../data/systemd/reprise-login-rdp.socket");
        let drop_in = std::fs::read_to_string(drop_in).unwrap_or_default();
        let mut ports = Vec::new();
        for line in unit.lines().chain(drop_in.lines()) {
            match line.strip_prefix("ListenStream=") {
                Some("") => ports.clear(),
                Some(port) => ports.push(port.parse().unwrap()),
                None => {}
            }
        }
        ports
    }

    #[tokio::test]
    async fn a_socket_that_failed_to_take_its_port_binds_no_other_at_boot() {
        let dir = tempfile::tempdir().unwrap();
        let drop_in = dir.path().join("port.conf");
        let units = FakeUnits::default();
        units.0.borrow_mut().port_held = true;
        let start = FrontUnit::Start { port: 3390 };
        apply(&units, &drop_in, start).await.unwrap_err();
        assert!(units.0.borrow().enabled);
        assert_eq!(boot_ports(&drop_in), [3390]);
        units.0.borrow_mut().port_held = false;
        apply(&units, &drop_in, start).await.unwrap();
        assert_eq!(boot_ports(&drop_in), [3390]);
    }

    #[tokio::test]
    async fn systemd_is_reloaded_only_after_the_socket_is_enabled_or_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let drop_in = dir.path().join("port.conf");
        write_port(&drop_in, 3389).unwrap();
        let units = FakeUnits::default();
        let on = FrontUnit::Start { port: 3389 };
        apply(&units, &drop_in, on).await.unwrap();
        assert!(!units.0.borrow().stale, "reloaded after enabling");
        apply(&units, &drop_in, on).await.unwrap();
        assert_eq!(
            units.0.borrow().reloads,
            1,
            "an enabled socket needs no reload"
        );
        apply(&units, &drop_in, FrontUnit::Stop).await.unwrap();
        assert!(!units.0.borrow().stale, "reloaded after disabling");
        apply(&units, &drop_in, FrontUnit::Stop).await.unwrap();
        assert_eq!(
            units.0.borrow().reloads,
            2,
            "a disabled socket needs no reload"
        );
    }

    #[tokio::test]
    async fn a_start_after_an_upgrade_reloads_resets_and_gives_the_front_its_socket() {
        let dir = tempfile::tempdir().unwrap();
        let drop_in = dir.path().join("port.conf");
        write_port(&drop_in, 3389).unwrap();
        let units = FakeUnits::default();
        {
            let mut fake = units.0.borrow_mut();
            fake.enabled = true;
            fake.stale = true;
            fake.active.insert(FRONT_SOCKET);
            fake.deaf = true;
            fake.limited.insert(FRONT_UNIT);
        }
        let on = FrontUnit::Start { port: 3389 };
        assert_eq!(apply(&units, &drop_in, on).await, Ok(()));
        let fake = units.0.borrow();
        assert!(!fake.stale, "the changed unit files are loaded");
        assert!(fake.limited.is_empty(), "the start limit is reset");
        assert!(!fake.deaf, "the socket holds its port again");
        assert!(fake.jobs.contains(&(UnitJob::Restart, FRONT_SOCKET)));
        assert_eq!(fake.active.len(), 2, "{:?}", fake.active);
    }

    #[tokio::test]
    async fn a_start_at_boot_leaves_the_socket_holding_its_port() {
        for unknown in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let drop_in = dir.path().join("port.conf");
            write_port(&drop_in, 3389).unwrap();
            let units = FakeUnits::default();
            {
                let mut fake = units.0.borrow_mut();
                fake.enabled = true;
                fake.active.insert(FRONT_SOCKET);
                fake.listening_unknown = unknown;
            }
            let on = FrontUnit::Start { port: 3389 };
            assert_eq!(apply(&units, &drop_in, on).await, Ok(()));
            assert_eq!(
                units.0.borrow().jobs,
                [(UnitJob::Start, FRONT_SOCKET), (UnitJob::Start, FRONT_UNIT)],
                "listening unknown: {unknown}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_socket_one_read_of_the_table_misses_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let drop_in = dir.path().join("port.conf");
        write_port(&drop_in, 3389).unwrap();
        let units = FakeUnits::default();
        {
            let mut fake = units.0.borrow_mut();
            fake.enabled = true;
            fake.active.insert(FRONT_SOCKET);
            fake.missed_reads = 1;
        }
        let on = FrontUnit::Start { port: 3389 };
        assert_eq!(apply(&units, &drop_in, on).await, Ok(()));
        let fake = units.0.borrow();
        assert_eq!(
            fake.jobs,
            [(UnitJob::Start, FRONT_SOCKET), (UnitJob::Start, FRONT_UNIT)]
        );
        assert_eq!(fake.reads, 2, "the table is read again before a restart");
    }

    #[test]
    fn the_sockets_listening_on_a_port_are_read_from_the_kernels_table() {
        let table = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:0D3D 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 4711 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000000000000:0016 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 17 1 0000000000000000 100 0 0 10 0
   2: 0000000000000000FFFF0000D301A8C0:0D3D 0000000000000000FFFF0000F201A8C0:A2E0 01 00000000:00000000 00:00000000 00000000   980        0 4712 1 0000000000000000 20 4 30 10 -1
";
        assert_eq!(listening_inodes(table, 3389), [4711]);
        assert_eq!(listening_inodes(table, 22), [17]);
        assert_eq!(listening_inodes(table, 3390), [] as [u64; 0]);
    }

    #[tokio::test]
    async fn a_start_while_the_front_runs_leaves_its_socket_alone() {
        let dir = tempfile::tempdir().unwrap();
        let drop_in = dir.path().join("port.conf");
        let units = FakeUnits::default();
        let on = FrontUnit::Start { port: 3389 };
        apply(&units, &drop_in, on).await.unwrap();
        units.0.borrow_mut().jobs.clear();
        apply(&units, &drop_in, on).await.unwrap();
        assert_eq!(
            units.0.borrow().jobs,
            [(UnitJob::Start, FRONT_SOCKET), (UnitJob::Start, FRONT_UNIT)]
        );
    }

    #[tokio::test]
    async fn a_port_that_cannot_be_written_starts_no_front() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "").unwrap();
        let units = FakeUnits::default();
        let start = FrontUnit::Start { port: 3390 };
        let e = (apply(&units, &file.join("port.conf"), start).await).unwrap_err();
        assert!(e.contains("port.conf"), "the drop-in is named: {e}");
        assert!(units.0.borrow().active.is_empty());
    }

    #[tokio::test]
    async fn stopping_goes_on_past_a_failed_step() {
        let dir = tempfile::tempdir().unwrap();
        let units = FakeUnits::default();
        let on = FrontUnit::Start { port: 3389 };
        apply(&units, &dir.path().join("port.conf"), on)
            .await
            .unwrap();
        units.0.borrow_mut().failing_stop = Some(FRONT_UNIT);
        let e = (apply(&units, Path::new("/nonexistent"), FrontUnit::Stop).await).unwrap_err();
        assert!(e.contains(FRONT_UNIT), "{e}");
        let fake = units.0.borrow();
        assert!(!fake.enabled, "the socket is disabled");
        assert!(fake.active.is_empty(), "{:?} still run", fake.active);
    }

    #[tokio::test]
    async fn stopping_front_units_that_are_not_installed_is_no_failure() {
        let units = FakeUnits::default();
        units.0.borrow_mut().missing = true;
        let stopped = apply(&units, Path::new("/nonexistent"), FrontUnit::Stop).await;
        assert_eq!(stopped, Ok(()));
    }

    fn error_reply(name: &str) -> zbus::Error {
        let call = zbus::Message::method_call("/org/freedesktop/login1", "TerminateSession")
            .build(&())
            .unwrap();
        let reply = zbus::Message::error(&call.header(), name)
            .build(&("gone",))
            .unwrap();
        reply.into()
    }

    #[test]
    fn a_connection_already_subscribed_to_systemd_counts_as_subscribed() {
        assert!(subscribed(Ok(())).is_ok());
        assert!(subscribed(Err(error_reply(ALREADY_SUBSCRIBED))).is_ok());
        assert!(subscribed(Err(error_reply(NO_SUCH_UNIT))).is_err());
        assert!(subscribed(Err(error_reply("org.freedesktop.DBus.Error.AccessDenied"))).is_err());
        assert!(subscribed(Err(zbus::Error::InvalidReply)).is_err());
    }

    #[test]
    fn a_session_that_went_away_is_told_from_other_failures() {
        assert!(session_gone(&error_reply(NO_SUCH_SESSION)));
        assert!(session_gone(&error_reply(
            "org.freedesktop.DBus.Error.UnknownObject"
        )));
        assert!(!session_gone(&error_reply(
            "org.freedesktop.DBus.Error.AccessDenied"
        )));
        assert!(!session_gone(&error_reply(
            "org.freedesktop.login1.NoSuchUser"
        )));
        assert!(!session_gone(&zbus::Error::InvalidReply));
    }

    #[test]
    fn a_session_is_ended_only_when_gone_or_closing() {
        assert_eq!(ended_by(Ok(true)), Ended::Yes);
        assert_eq!(ended_by(Ok(false)), Ended::No);
        assert_eq!(ended_by(Err(error_reply(NO_SUCH_SESSION))), Ended::Yes);
        assert_eq!(
            ended_by(Err(error_reply("org.freedesktop.DBus.Error.UnknownObject"))),
            Ended::Yes
        );
        assert_eq!(
            ended_by(Err(error_reply("org.freedesktop.DBus.Error.NoReply"))),
            Ended::Unknown
        );
        assert_eq!(
            ended_by(Err(error_reply("org.freedesktop.DBus.Error.AccessDenied"))),
            Ended::Unknown
        );
        assert_eq!(ended_by(Err(zbus::Error::InvalidReply)), Ended::Unknown);
    }

    #[test]
    fn keyboard_falls_back_to_us() {
        let us = Keyboard {
            layout: "us".into(),
            ..Keyboard::default()
        };
        assert_eq!(keyboard_from(None, None, None), us);
        assert_eq!(keyboard_from(Some(String::new()), None, None), us);
        assert_eq!(
            keyboard_from(
                Some("de,us".into()),
                Some("nodeadkeys,".into()),
                Some("grp:alt_shift_toggle".into())
            ),
            Keyboard {
                layout: "de,us".into(),
                variant: "nodeadkeys,".into(),
                options: "grp:alt_shift_toggle".into(),
            }
        );
    }

    #[test]
    fn a_received_fd_is_closed_on_exec() {
        use std::os::fd::{AsRawFd, FromRawFd};
        let mut ends = [0; 2];
        // SAFETY: `ends` has room for the two fds pipe writes.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        // SAFETY: pipe just returned both fds, and nothing else owns them.
        let (r, _w) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
        let flags = |fd: &OwnedFd| {
            // SAFETY: F_GETFD on an open fd has no other preconditions.
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) }
        };
        assert_eq!(flags(&r) & libc::FD_CLOEXEC, 0, "a bare pipe is inherited");
        let r = cloexec(r).unwrap();
        assert_eq!(flags(&r) & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
    }

    /// Lets every ready task of the `LocalSet` run.
    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    fn open(w: &mut std::io::PipeWriter) -> bool {
        use std::io::Write;
        w.write(b"x").is_ok()
    }

    #[tokio::test]
    async fn changes_during_a_take_in_flight_take_only_the_latest_after_it() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let taken: Rc<RefCell<Vec<(&str, std::io::PipeWriter)>>> = Rc::default();
                let slow = Rc::new(tokio::sync::Notify::new());
                let take: TakeInhibitor = {
                    let (taken, slow) = (taken.clone(), slow.clone());
                    Rc::new(move |what, _why| {
                        let (r, w) = std::io::pipe().unwrap();
                        taken.borrow_mut().push((what, w));
                        let slow = slow.clone();
                        Box::pin(async move {
                            slow.notified().await;
                            Ok(r.into())
                        })
                    })
                };
                let inhibitor = Inhibitor::new(take);
                inhibitor.set(Inhibit::Idle);
                settle().await;
                for _ in 0..50 {
                    inhibitor.set(Inhibit::SleepAndIdle);
                    inhibitor.set(Inhibit::Idle);
                }
                inhibitor.set(Inhibit::SleepAndIdle);
                settle().await;
                assert_eq!(taken.borrow().len(), 1, "one take in flight");
                slow.notify_one();
                settle().await;
                slow.notify_one();
                settle().await;
                let mut t = taken.borrow_mut();
                let whats: Vec<_> = t.iter().map(|(what, _)| *what).collect();
                assert_eq!(whats, ["idle", "sleep:idle"]);
                assert!(!open(&mut t[0].1), "the stale one is dropped");
                assert!(open(&mut t[1].1), "the latest is held");
            })
            .await;
    }

    #[tokio::test]
    async fn an_inhibitor_is_held_until_the_next_change_and_a_late_one_is_dropped() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let taken: Rc<RefCell<Vec<(&str, std::io::PipeWriter)>>> = Rc::default();
                let slow = Rc::new(tokio::sync::Notify::new());
                let hold = Rc::new(std::cell::Cell::new(false));
                let take: TakeInhibitor = {
                    let (taken, slow, hold) = (taken.clone(), slow.clone(), hold.clone());
                    Rc::new(move |what, _why| {
                        let (r, w) = std::io::pipe().unwrap();
                        taken.borrow_mut().push((what, w));
                        let (slow, hold) = (slow.clone(), hold.get());
                        Box::pin(async move {
                            if hold {
                                slow.notified().await;
                            }
                            Ok(r.into())
                        })
                    })
                };
                let inhibitor = Inhibitor::new(take);
                inhibitor.set(Inhibit::Idle);
                settle().await;
                inhibitor.set(Inhibit::SleepAndIdle);
                settle().await;
                {
                    let mut t = taken.borrow_mut();
                    assert_eq!((t[0].0, t[1].0), ("idle", "sleep:idle"));
                    assert!(!open(&mut t[0].1), "the idle one is dropped once replaced");
                    assert!(open(&mut t[1].1), "the sleep:idle one is held");
                }
                inhibitor.set(Inhibit::Nothing);
                assert!(
                    !open(&mut taken.borrow_mut()[1].1),
                    "Nothing drops it at once"
                );

                hold.set(true);
                inhibitor.set(Inhibit::Idle);
                settle().await;
                inhibitor.set(Inhibit::Nothing);
                slow.notify_one();
                settle().await;
                assert!(
                    !open(&mut taken.borrow_mut()[2].1),
                    "an inhibitor arriving after a later change is dropped"
                );
            })
            .await;
    }
}
