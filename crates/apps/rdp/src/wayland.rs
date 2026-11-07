//! The session's Wayland display. A server the user manager starts before the compositor is up
//! learns `WAYLAND_DISPLAY` from the manager's environment once the session sets it.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::OnceLock;
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

/// Connects to the session's compositor: `WAYLAND_DISPLAY` when set, else the name the user
/// manager's environment gives within [`SESSION_READY_WAIT`]. Blocks while it waits.
pub fn connect() -> Result<Connection, String> {
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
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
    let socket = PathBuf::from(runtime).join(&name);
    let stream = UnixStream::connect(&socket)
        .map_err(|e| format!("no Wayland connection on {}: {e}", socket.display()))?;
    Connection::from_socket(stream).map_err(|e| format!("no Wayland connection: {e}"))
}

/// [`connect`], tried every [`POLL`] for [`SESSION_READY_WAIT`]: a user manager that outlived an
/// earlier session still names that session's display until the new compositor is up.
pub fn connect_when_ready() -> Result<Connection, String> {
    retried(SESSION_READY_WAIT, POLL, connect)
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
