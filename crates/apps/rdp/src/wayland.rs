//! The session's Wayland display. A server the user manager starts before the compositor is up
//! learns `WAYLAND_DISPLAY` from the manager's environment once the session sets it.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
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

/// How often the user manager's environment is read while it lacks `WAYLAND_DISPLAY`.
const POLL: Duration = Duration::from_millis(250);

/// The `WAYLAND_DISPLAY` entry of an environment as systemd lists it.
pub fn display_name(env: &[String]) -> Option<String> {
    (env.iter())
        .find_map(|entry| entry.strip_prefix("WAYLAND_DISPLAY="))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The display [`connect_when_ready`] found, for later connections.
static FOUND: OnceLock<String> = OnceLock::new();

/// Set by [`hand_over_mode`].
static HAND_OVER: AtomicBool = AtomicBool::new(false);

/// Makes [`connect`] go only to the display [`connect_when_ready`] finds, before any connection:
/// the environment the server started with may name another session's display.
pub fn hand_over_mode() {
    HAND_OVER.store(true, Ordering::Relaxed);
}

/// Where [`connect`] goes.
#[derive(Debug, PartialEq)]
enum Target<'a> {
    Found(&'a str),
    NotYet,
    Environment,
}

fn target(found: Option<&str>, hand_over: bool) -> Target<'_> {
    match (found, hand_over) {
        (Some(name), _) => Target::Found(name),
        (None, true) => Target::NotYet,
        (None, false) => Target::Environment,
    }
}

/// Ok when the compositor whose `/proc/<pid>/cgroup` is `cgroup` runs in a session reprise-login
/// opened, the one hand-overs go to; `record` reads logind's record of a session.
fn in_login_session(cgroup: &str, record: impl FnOnce(&str) -> String) -> Result<(), String> {
    let id = reprisebus::logind::session_of_cgroup(cgroup).ok_or("it runs in no login session")?;
    let record = record(id);
    match reprisebus::logind::service_of_session(&record) {
        Some(service) if repriselogin::USER_SERVICES.contains(&service) => Ok(()),
        Some(service) => Err(format!("its session {id} was opened by {service}")),
        None => Err(format!("logind has no service for its session {id}")),
    }
}

/// Connects to the session's compositor: the display [`connect_when_ready`] found, an error
/// before that in [`hand_over_mode`], else `WAYLAND_DISPLAY` when set, else the name the user
/// manager's environment gives within [`SESSION_READY_WAIT`]. Blocks while it waits.
pub fn connect() -> Result<Connection, String> {
    match target(
        FOUND.get().map(String::as_str),
        HAND_OVER.load(Ordering::Relaxed),
    ) {
        Target::Found(name) => return connect_to(name),
        Target::NotYet => return Err("the session's display is not found yet".into()),
        Target::Environment => {}
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
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let socket = PathBuf::from(runtime).join(name);
    let stream = UnixStream::connect(&socket)
        .map_err(|e| format!("no Wayland connection on {}: {e}", socket.display()))?;
    Connection::from_socket(stream).map_err(|e| format!("no Wayland connection: {e}"))
}

/// Connects to the display the user manager's environment names once its compositor runs in the
/// session reprise-login opened, both read every [`POLL`] for [`SESSION_READY_WAIT`]: until the new
/// compositor publishes, the manager names an earlier or a text-login session's display.
pub fn connect_when_ready() -> Result<Connection, String> {
    let bus =
        zbus::block_on(zbus::Connection::session()).map_err(|e| format!("no session bus: {e}"))?;
    let named = || {
        let env = zbus::block_on(manager_environment(&bus))
            .map_err(|e| format!("the user manager's environment: {e}"))?;
        display_name(&env).ok_or_else(|| "the session set no WAYLAND_DISPLAY yet".to_string())
    };
    let mut passed_over = None;
    let open = |name: &str| {
        let conn = connect_to(name)?;
        let pid = peer_pid(&conn)?;
        let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap_or_default();
        let checked = in_login_session(&cgroup, reprisebus::logind::session_record)
            .map_err(|e| format!("{name} is not the login's session: {e}"));
        if let Err(e) = &checked
            && passed_over.as_deref() != Some(name)
        {
            log::info!("{e}; waiting for the session's own display");
            passed_over = Some(name.to_owned());
        }
        checked.map(|()| conn)
    };
    let (name, conn) = ready(SESSION_READY_WAIT, POLL, named, open)?;
    log::info!("the session's display is {name}");
    let _ = FOUND.set(name);
    Ok(conn)
}

/// `open` on the name `named` gives, both tried every `poll` until `open` succeeds or `wait` has
/// passed.
fn ready<T>(
    wait: Duration,
    poll: Duration,
    mut named: impl FnMut() -> Result<String, String>,
    mut open: impl FnMut(&str) -> Result<T, String>,
) -> Result<(String, T), String> {
    retried(wait, poll, || {
        let name = named()?;
        let opened = open(&name)?;
        Ok((name, opened))
    })
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
            Ok("wayland-0".to_string()),
            Ok("wayland-1".to_string()),
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
        assert_eq!(in_login_session(&cg("11"), records), Ok(()));
        assert_eq!(in_login_session(&cg("2"), records), Ok(()));
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
        assert_eq!(target(None, true), Target::NotYet);
        assert_eq!(target(Some("wayland-1"), true), Target::Found("wayland-1"));
        assert_eq!(target(None, false), Target::Environment);
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
