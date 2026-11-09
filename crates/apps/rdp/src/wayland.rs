//! The session's Wayland display, and whether the portal serves it. A server the user manager
//! starts before the compositor is up finds the display once the session offers it.

use std::collections::HashSet;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use wayland_client::Connection;
use wayland_client::backend::WaylandError;
use zbus::OwnedVariant;

/// How long a session just started may take to offer its display and portal.
pub const SESSION_READY_WAIT: Duration = Duration::from_secs(30);

/// How often the display and the portal are looked for while the session starts.
const POLL: Duration = Duration::from_millis(250);

/// The `WAYLAND_DISPLAY` entry of an environment as systemd lists it.
pub fn display_name(env: &[String]) -> Option<String> {
    (env.iter())
        .find_map(|entry| entry.strip_prefix("WAYLAND_DISPLAY="))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The display [`connect_when_ready`] found and the logind session its compositor runs in.
struct Found {
    display: String,
    session: String,
}

static FOUND: OnceLock<Found> = OnceLock::new();

static HAND_OVER: AtomicBool = AtomicBool::new(false);

const NOT_FOUND: &str = "the session's display is not found yet";

/// Makes [`connect`] go only to the display [`connect_when_ready`] finds, before any connection:
/// the environment the server started with may name another session's display.
pub fn hand_over_mode() {
    HAND_OVER.store(true, Ordering::Relaxed);
}

/// The id of the session the compositor whose `/proc/<pid>/cgroup` is `cgroup` runs in, when
/// reprise-login opened it; `record` reads logind's record of a session.
fn in_login_session(cgroup: &str, record: impl FnOnce(&str) -> String) -> Result<String, String> {
    let id = reprisebus::logind::session_of_cgroup(cgroup).ok_or("it runs in no login session")?;
    let record = record(id);
    match reprisebus::logind::service_of_session(&record) {
        Some(service) if repriselogin::USER_SERVICES.contains(&service) => Ok(id.to_owned()),
        Some(service) => Err(format!("its session {id} was opened by {service}")),
        None => Err(format!("logind has no service for its session {id}")),
    }
}

/// Connects to the session's compositor: the display [`connect_when_ready`] found, else (outside
/// [`hand_over_mode`]) `WAYLAND_DISPLAY` or the user manager's. Blocks while it waits.
pub fn connect() -> Result<Connection, String> {
    let found = FOUND.get().map(|f| f.display.as_str());
    connect_in(found, HAND_OVER.load(Ordering::Relaxed))
}

fn connect_in(found: Option<&str>, hand_over: bool) -> Result<Connection, String> {
    if let Some(name) = found {
        return connect_to(name);
    }
    if hand_over {
        return Err(NOT_FOUND.into());
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()) {
        return Connection::connect_to_env().map_err(|e| format!("no Wayland connection: {e}"));
    }
    static LEARNED: OnceLock<String> = OnceLock::new();
    let name = match LEARNED.get() {
        Some(name) => name.clone(),
        None => {
            let learned = learned_name()?;
            LEARNED.get_or_init(|| learned).clone()
        }
    };
    connect_to(&name)
}

/// The pid of the process serving the session's display, from its socket's peer credentials.
pub fn compositor_pid() -> Result<u32, String> {
    peer_pid(&connect()?)
}

/// The pid of the process at the other end of `conn`, from its socket's peer credentials.
fn peer_pid(conn: &Connection) -> Result<u32, String> {
    let fd = conn.backend().poll_fd().as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `fd` is the connection's socket, open while `conn` lives, and `cred` and `len`
    // describe a writable `ucred`.
    let got = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &raw mut len,
        )
    };
    if got != 0 {
        let e = io::Error::last_os_error();
        return Err(format!("the compositor's credentials: {e}"));
    }
    u32::try_from(cred.pid).map_err(|_| format!("the compositor's pid {}", cred.pid))
}

fn connect_to(name: &str) -> Result<Connection, String> {
    let socket = match name.starts_with('/') {
        true => PathBuf::from(name),
        false => runtime_dir()?.join(name),
    };
    let stream = UnixStream::connect(&socket)
        .map_err(|e| format!("no Wayland connection on {}: {e}", socket.display()))?;
    Connection::from_socket(stream).map_err(|e| format!("no Wayland connection: {e}"))
}

fn runtime_dir() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime))
}

/// Connects to the first display whose compositor runs in the session reprise-login opened: the
/// one the user manager names, then each socket in the runtime directory, every [`POLL`].
pub fn connect_when_ready() -> Result<Connection, String> {
    let bus =
        zbus::block_on(zbus::Connection::session()).map_err(|e| format!("no session bus: {e}"))?;
    let runtime = runtime_dir()?;
    let named = || {
        let env = zbus::block_on(manager_environment(&bus))
            .map_err(|e| log::debug!("the user manager's environment: {e}"));
        let named = env.ok().and_then(|env| display_name(&env));
        Ok(candidates(named, sockets_in(&runtime)))
    };
    let mut passed_over = HashSet::new();
    let open = |name: &str| {
        let conn = connect_to(name)?;
        let pid = peer_pid(&conn)?;
        let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
        let checked = in_login_session(&cgroup, reprisebus::logind::session_record)
            .map_err(|e| format!("{name} is not the login's session: {e}"));
        if let Err(e) = &checked
            && passed_over.insert(name.to_owned())
        {
            log::info!("{e}; waiting for the session's own display");
        }
        checked.map(|session| (conn, session))
    };
    let (display, (conn, session)) = ready(SESSION_READY_WAIT, POLL, named, open)?;
    log::info!("the session's display is {display}, in session {session}");
    let _ = FOUND.set(Found { display, session });
    Ok(conn)
}

/// The displays to try, each once: `named` first, then `sockets` in order.
fn candidates(named: Option<String>, mut sockets: Vec<String>) -> Vec<String> {
    sockets.sort();
    sockets.retain(|s| named.as_ref() != Some(s));
    named.into_iter().chain(sockets).collect()
}

/// The names of the Wayland sockets in `dir`, without their `.lock` files.
fn sockets_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    (entries.flatten())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with("wayland-"))
        .collect()
}

/// `open` on each name `named` lists until one succeeds, all tried again every `poll` until then
/// or `wait` has passed.
fn ready<T>(
    wait: Duration,
    poll: Duration,
    mut named: impl FnMut() -> Result<Vec<String>, String>,
    mut open: impl FnMut(&str) -> Result<T, String>,
) -> Result<(String, T), String> {
    retried(wait, poll, || {
        let mut last = "no Wayland display yet".to_string();
        for name in named()? {
            match open(&name) {
                Ok(opened) => return Ok((name, opened)),
                Err(e) => last = e,
            }
        }
        Err(last)
    })
}

/// The unit the portals are part of. A session's start stops it, ending an earlier session's
/// portals, and starts it again.
const PORTALS_TARGET: &str = "graphical-session.target";

const PORTAL_BACKEND: &str = "org.freedesktop.impl.portal.desktop.reprise";

/// Waits up to [`SESSION_READY_WAIT`] until the portal backend serves the display
/// [`connect_when_ready`] found; at once outside [`hand_over_mode`].
pub fn portal_ready() -> Result<(), String> {
    if !HAND_OVER.load(Ordering::Relaxed) {
        return Ok(());
    }
    let found = FOUND.get().ok_or(NOT_FOUND)?;
    let bus = reprisebus::session().map_err(|e| format!("no session bus: {e}"))?;
    let record = reprisebus::logind::session_record(&found.session);
    let opened = reprisebus::logind::opened_of_session(&record)
        .ok_or_else(|| format!("logind has no opening time for session {}", found.session))?;
    let mut told = false;
    retried(SESSION_READY_WAIT, POLL, || {
        let served = reprisebus::wait(backend_serves(&bus, &found.display, opened));
        if let Err(e) = &served
            && !std::mem::replace(&mut told, true)
        {
            log::info!(
                "the portal does not serve {} yet: {e}; waiting",
                found.display
            );
        }
        served
    })
}

/// Ok once [`PORTALS_TARGET`] restarted after the session `opened` (µs, `CLOCK_MONOTONIC`) and the
/// backend, started if none runs, has `display` as its `WAYLAND_DISPLAY`.
async fn backend_serves(bus: &zbus::Connection, display: &str, opened: u64) -> Result<(), String> {
    let entered = target_entered(bus)
        .await
        .map_err(|e| format!("{PORTALS_TARGET}: {e}"))?;
    restarted_since(entered, opened)?;
    let owner = match reprisebus::name_owner(bus, PORTAL_BACKEND).await {
        Some(owner) => owner,
        None => {
            start_backend(bus).await;
            (reprisebus::name_owner(bus, PORTAL_BACKEND).await).ok_or("no portal backend runs")?
        }
    };
    let pid: u32 = (bus.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "GetConnectionUnixProcessID",
        &(owner.as_str(),),
    ))
    .await
    .and_then(|m| m.body().deserialize())
    .map_err(|e| format!("the portal backend's pid: {e}"))?;
    let environ = std::fs::read(format!("/proc/{pid}/environ"))
        .map_err(|e| format!("the portal backend's environment: {e}"))?;
    serves(display, &environ)
}

/// When [`PORTALS_TARGET`] last became active, in µs of `CLOCK_MONOTONIC`; None while it is not.
async fn target_entered(bus: &zbus::Connection) -> zbus::Result<Option<u64>> {
    let path: zbus::OwnedObjectPath = (bus.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "GetUnit",
        &(PORTALS_TARGET,),
    ))
    .await?
    .body()
    .deserialize()?;
    let unit: zbus::Proxy = zbus::proxy::Builder::new(bus)
        .destination("org.freedesktop.systemd1")
        .path(path)
        .interface("org.freedesktop.systemd1.Unit")
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    let state: String = unit.get_property("ActiveState").await?;
    let entered: u64 = unit.get_property("ActiveEnterTimestampMonotonic").await?;
    Ok((state == "active").then_some(entered))
}

async fn start_backend(bus: &zbus::Connection) {
    let started = bus
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "StartServiceByName",
            &(PORTAL_BACKEND, 0u32),
        )
        .await;
    if let Err(e) = started {
        log::debug!("starting {PORTAL_BACKEND}: {e}");
    }
}

/// Ok when the portals' target became active (`entered`) after the session opened: any portal
/// from before then was stopped with it.
fn restarted_since(entered: Option<u64>, opened: u64) -> Result<(), String> {
    match entered {
        Some(at) if at > opened => Ok(()),
        Some(_) => Err("the portals are still an earlier session's".into()),
        None => Err("the session has not started its portals yet".into()),
    }
}

/// Ok when the process environment `environ` (`/proc/<pid>/environ`) names `display`.
fn serves(display: &str, environ: &[u8]) -> Result<(), String> {
    let named = (environ.split(|b| *b == 0))
        .find_map(|v| v.strip_prefix(b"WAYLAND_DISPLAY="))
        .map(String::from_utf8_lossy);
    match named {
        Some(name) if name == display => Ok(()),
        Some(name) => Err(format!("the portal backend serves {name}")),
        None => Err("the portal backend has no WAYLAND_DISPLAY".into()),
    }
}

/// `attempt` every `poll` until it succeeds or `wait` has passed; then its last error.
fn retried<T>(
    wait: Duration,
    poll: Duration,
    mut attempt: impl FnMut() -> Result<T, String>,
) -> Result<T, String> {
    let until = Instant::now() + wait;
    loop {
        match attempt() {
            Ok(done) => return Ok(done),
            Err(e) if Instant::now() >= until => return Err(e),
            Err(e) => {
                log::debug!("{e}; trying again");
                std::thread::sleep(poll);
            }
        }
    }
}

/// The display name from the user manager's environment, read until it is there.
fn learned_name() -> Result<String, String> {
    let until = Instant::now() + SESSION_READY_WAIT;
    zbus::block_on(async {
        let bus = zbus::Connection::session()
            .await
            .map_err(|e| format!("no session bus: {e}"))?;
        loop {
            match manager_environment(&bus).await {
                Ok(env) => {
                    if let Some(name) = display_name(&env) {
                        log::info!("the session's display is {name}");
                        return Ok(name);
                    }
                }
                Err(e) => log::debug!("the user manager's environment: {e}"),
            }
            if Instant::now() >= until {
                return Err(format!(
                    "the session set no WAYLAND_DISPLAY within {} s",
                    SESSION_READY_WAIT.as_secs()
                ));
            }
            std::thread::sleep(POLL);
        }
    })
}

async fn manager_environment(bus: &zbus::Connection) -> zbus::Result<Vec<String>> {
    let env: OwnedVariant = bus
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.freedesktop.systemd1.Manager", "Environment"),
        )
        .await?
        .body()
        .deserialize()?;
    Vec::<String>::try_from(env)
}

/// Resolves once the compositor closes `conn`; anything it sends meanwhile is read and dropped.
pub async fn closed(conn: Connection) {
    let fd = conn.backend().poll_fd().as_raw_fd();
    // SAFETY: `fd` is `conn`'s socket, which stays open until `conn` drops, after `watched`.
    let watched = match unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE) } {
        Ok(watched) => watched,
        Err(e) => {
            log::error!("cannot watch the Wayland connection: {e}");
            return;
        }
    };
    loop {
        let Ok(mut ready) = watched.readable().await else {
            return;
        };
        let Some(guard) = conn.prepare_read() else {
            continue;
        };
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => {
                ready.clear_ready();
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_name_comes_from_the_user_managers_environment() {
        let env = [
            "LANG=C".to_string(),
            "WAYLAND_DISPLAY=wayland-1".to_string(),
        ];
        assert_eq!(display_name(&env).as_deref(), Some("wayland-1"));
        assert_eq!(display_name(&[]), None);
    }

    #[test]
    fn a_display_not_up_yet_is_tried_again_until_the_wait_ends() {
        let mut tries = 0;
        let up = retried(Duration::from_secs(5), Duration::from_millis(5), || {
            tries += 1;
            if tries < 3 {
                Err("not up".to_string())
            } else {
                Ok(tries)
            }
        });
        assert_eq!(up, Ok(3));

        let mut tries = 0;
        let start = Instant::now();
        let never = retried(Duration::from_millis(50), Duration::from_millis(5), || {
            tries += 1;
            Err::<(), _>(format!("not up {tries}"))
        });
        assert!(start.elapsed() >= Duration::from_millis(50));
        assert_eq!(never, Err(format!("not up {tries}")), "the last error");
        assert!(tries > 1);
    }

    #[test]
    fn the_display_name_is_read_again_on_each_try() {
        let mut names = [
            Err("unset".to_string()),
            Ok(vec!["wayland-0".to_string()]),
            Ok(vec!["wayland-1".to_string()]),
        ]
        .into_iter();
        let mut reads = 0;
        let named = || {
            reads += 1;
            names.next().expect("no more tries than names")
        };
        let open = |name: &str| match name {
            "wayland-1" => Ok(()),
            stale => Err(format!("no compositor on {stale}")),
        };
        let found = ready(
            Duration::from_secs(5),
            Duration::from_millis(5),
            named,
            open,
        );
        assert_eq!(found, Ok(("wayland-1".to_string(), ())));
        assert_eq!(reads, 3);
    }

    #[test]
    fn a_compositor_is_the_hand_overs_only_in_a_session_reprise_login_opened() {
        let cg = |id: &str| format!("0::/user.slice/user-1001.slice/session-{id}.scope\n");
        let records = |id: &str| {
            match id {
                "11" => "TYPE=wayland\nSERVICE=reprise-login-remote\n",
                "2" => "TYPE=wayland\nSERVICE=reprise-login-autologin\n",
                "9" => "TYPE=tty\nSERVICE=login\n",
                _ => "",
            }
            .to_string()
        };
        assert_eq!(in_login_session(&cg("11"), records), Ok("11".into()));
        assert_eq!(in_login_session(&cg("2"), records), Ok("2".into()));
        assert!(
            in_login_session(&cg("9"), records).is_err(),
            "a text login's compositor"
        );
        assert!(
            in_login_session(&cg("12"), records).is_err(),
            "a session logind has no record of"
        );
        let app =
            "0::/user.slice/user-1001.slice/user@1001.service/app.slice/app-reprise-x-1.scope\n";
        assert!(
            in_login_session(app, records).is_err(),
            "a compositor in no session"
        );
    }

    #[test]
    fn a_hand_over_server_connects_only_to_the_display_it_found() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("wayland-1");
        let _compositor = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let found = socket.to_str();
        assert!(connect_in(found, true).is_ok(), "the display found");
        assert_eq!(
            connect_in(None, true).err().as_deref(),
            Some(NOT_FOUND),
            "nothing before it is found"
        );
    }

    #[test]
    fn a_display_the_user_manager_does_not_name_is_found_among_the_runtime_sockets() {
        let dir = tempfile::tempdir().unwrap();
        let mut listeners = Vec::new();
        for name in ["wayland-0", "wayland-1", "pipewire-0"] {
            listeners.push(std::os::unix::net::UnixListener::bind(dir.path().join(name)).unwrap());
        }
        for lock in ["wayland-0.lock", "wayland-1.lock"] {
            std::fs::write(dir.path().join(lock), "").unwrap();
        }
        let named = || Ok(candidates(Some("wayland-0".into()), sockets_in(dir.path())));
        let mut tried = Vec::new();
        let open = |name: &str| {
            tried.push(name.to_owned());
            match name {
                "wayland-1" => Ok(()),
                other => Err(format!("{other} is a text login's")),
            }
        };
        let found = ready(
            Duration::from_millis(50),
            Duration::from_millis(5),
            named,
            open,
        );
        assert_eq!(found, Ok(("wayland-1".to_string(), ())));
        assert_eq!(
            tried,
            ["wayland-0", "wayland-1"],
            "the named display first, once"
        );
    }

    #[test]
    fn the_portal_serves_the_session_once_its_start_restarted_the_portals_on_its_display() {
        let opened = 10_000_000;
        assert!(
            restarted_since(Some(9_000_000), opened).is_err(),
            "an earlier session's"
        );
        assert!(
            restarted_since(None, opened).is_err(),
            "stopped, not started again yet"
        );
        assert_eq!(restarted_since(Some(10_500_000), opened), Ok(()));
        let environ = b"LANG=C\0WAYLAND_DISPLAY=wayland-1\0XDG_SESSION_TYPE=wayland\0";
        assert_eq!(serves("wayland-1", environ), Ok(()));
        assert!(serves("wayland-0", environ).is_err(), "another display's");
        assert!(serves("wayland-1", b"LANG=C\0").is_err(), "no display");
    }

    #[tokio::test]
    async fn a_watched_display_ends_once_the_compositor_closes_it() {
        let (ours, compositor) = UnixStream::pair().unwrap();
        let conn = Connection::from_socket(ours).unwrap();
        let watch = tokio::spawn(closed(conn));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!watch.is_finished(), "an open display is still watched");
        drop(compositor);
        tokio::time::timeout(Duration::from_secs(5), watch)
            .await
            .expect("the watch ends with the display")
            .unwrap();
    }
}
