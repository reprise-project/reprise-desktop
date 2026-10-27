//! Whether the session the portal was started in is Reprise's, and staying
//! only as long as it is.
//!
//! xdg-desktop-portal picks one backend per interface at start from
//! `XDG_CURRENT_DESKTOP` and `portals.conf`, with no fallback on failure, so
//! refusing calls can't hand a dialog to another backend. Instead the portal
//! serves only while the compositor on `WAYLAND_DISPLAY` is Reprise, exits
//! when it goes, and when started for another session restarts a frontend
//! still routing for Reprise so it rereads the desktop.

use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

const FRONTEND: &str = "org.freedesktop.portal.Desktop";

pub struct Compositor(UnixStream);

/// Connect to the compositor on `WAYLAND_DISPLAY`, checking that the process
/// on the socket runs a binary named `reprise`.
pub fn connect() -> Result<Compositor, String> {
    let display = std::env::var("WAYLAND_DISPLAY").map_err(|_| "no WAYLAND_DISPLAY")?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok();
    let path = socket_path(&display, runtime.as_deref())
        .ok_or_else(|| format!("no socket path for WAYLAND_DISPLAY={display}"))?;
    let stream = UnixStream::connect(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let pid = peer_pid(&stream).map_err(|e| format!("{}: peer: {e}", path.display()))?;
    let exe = std::fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(|e| format!("compositor {pid}: {e}"))?;
    if !is_reprise_exe(&exe) {
        return Err(format!(
            "the compositor on {display} is {}, not Reprise",
            exe.display()
        ));
    }
    Ok(Compositor(stream))
}

/// Exit once the compositor closes the connection; the frontend sees the bus
/// name go, and a later call starts a portal that checks again.
pub fn exit_with(c: Compositor) {
    let mut stream = c.0;
    let spawned = std::thread::Builder::new()
        .name("compositor watch".into())
        .spawn(move || {
            // The portal never speaks on this connection, so nothing arrives before end-of-file.
            let mut buf = [0u8; 64];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            log::info!("the compositor is gone; the session is over");
            std::process::exit(0);
        });
    if let Err(e) = spawned {
        log::warn!("compositor watch: {e}; the portal outlives its compositor");
    }
}

fn socket_path(display: &str, runtime_dir: Option<&str>) -> Option<PathBuf> {
    if display.starts_with('/') {
        return Some(PathBuf::from(display));
    }
    if display.is_empty() {
        return None;
    }
    Some(Path::new(runtime_dir.filter(|d| !d.is_empty())?).join(display))
}

fn peer_pid(s: &UnixStream) -> std::io::Result<libc::pid_t> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `s` is an open socket for the whole call, and `cred` and
    // `len` are live locals of the size SO_PEERCRED writes.
    let r = unsafe {
        libc::getsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(cred.pid)
}

/// Whether `exe`, a `/proc/<pid>/exe` link, is the compositor: named `reprise`
/// wherever installed, also after an upgrade (" (deleted)").
fn is_reprise_exe(exe: &Path) -> bool {
    let name = exe.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.strip_suffix(" (deleted)").unwrap_or(name) == "reprise"
}

fn names_reprise(desktops: &str) -> bool {
    desktops
        .split(':')
        .any(|d| d.eq_ignore_ascii_case("reprise"))
}

fn environ_value(environ: &[u8], key: &str) -> Option<String> {
    environ.split(|&b| b == 0).find_map(|entry| {
        let rest = entry.strip_prefix(key.as_bytes())?.strip_prefix(b"=")?;
        Some(String::from_utf8_lossy(rest).into_owned())
    })
}

fn manager_desktop(show_environment: &str) -> Option<String> {
    show_environment
        .lines()
        .find_map(|l| l.strip_prefix("XDG_CURRENT_DESKTOP="))
        .map(str::to_string)
}

/// Whether to restart the frontend: it was started for Reprise (`frontend`)
/// and the user manager, which a restarted frontend reads, now names another
/// desktop or none. A manager still naming Reprise would start the same frontend again.
fn frontend_needs_restart(frontend: Option<&str>, manager: Option<&str>) -> bool {
    frontend.is_some_and(names_reprise) && !manager.is_some_and(names_reprise)
}

fn frontend_desktop() -> Option<String> {
    let pid = zbus::block_on(async {
        let conn = zbus::Connection::session().await.ok()?;
        let dbus = zbus::fdo::DBusProxy::new(&conn).await.ok()?;
        let name = zbus::names::BusName::try_from(FRONTEND).ok()?;
        dbus.get_connection_unix_process_id(name).await.ok()
    })?;
    let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    environ_value(&environ, "XDG_CURRENT_DESKTOP")
}

/// The session is not Reprise's: the frontend may still route for a Reprise
/// session that ended uncleanly. Restart it when that changes its routing.
/// Best effort.
pub fn release_frontend() {
    let frontend = frontend_desktop();
    let manager = std::process::Command::new("systemctl")
        .args(["--user", "show-environment"])
        .output()
        .ok()
        .and_then(|o| manager_desktop(&String::from_utf8_lossy(&o.stdout)));
    if !frontend_needs_restart(frontend.as_deref(), manager.as_deref()) {
        return;
    }
    log::warn!(
        "xdg-desktop-portal still routes for {}; restarting it for {}",
        frontend.unwrap_or_default(),
        manager.as_deref().unwrap_or("no desktop")
    );
    let restarted = std::process::Command::new("systemctl")
        .args([
            "--user",
            "--no-block",
            "try-restart",
            "xdg-desktop-portal.service",
        ])
        .status();
    if let Err(e) = restarted {
        log::warn!("systemctl: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_binary_named_reprise_is_the_compositor() {
        for exe in [
            "/usr/bin/reprise",
            "/usr/local/bin/reprise",
            "/home/u/src/reprise/target/release/reprise",
            "/usr/bin/reprise (deleted)",
        ] {
            assert!(is_reprise_exe(Path::new(exe)), "{exe} is Reprise");
        }
        for exe in [
            "/usr/bin/kwin_wayland",
            "/usr/bin/reprise-portal",
            "/usr/bin/sway",
            "/usr/bin/notreprise",
        ] {
            assert!(!is_reprise_exe(Path::new(exe)), "{exe} is not Reprise");
        }
    }

    #[test]
    fn wayland_display_names_a_socket_under_the_runtime_dir() {
        assert_eq!(
            socket_path("wayland-1", Some("/run/user/1000")),
            Some(PathBuf::from("/run/user/1000/wayland-1"))
        );
        assert_eq!(
            socket_path("/tmp/w/sock", None),
            Some(PathBuf::from("/tmp/w/sock"))
        );
        assert_eq!(socket_path("wayland-1", None), None);
        assert_eq!(socket_path("wayland-1", Some("")), None);
        assert_eq!(socket_path("", Some("/run/user/1000")), None);
    }

    #[test]
    fn the_peer_is_the_process_listening_on_the_socket() {
        // In /tmp, not $TMPDIR: a socket path must fit sun_path's 108 bytes.
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let path = dir.path().join("wayland-test");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let s = UnixStream::connect(&path).unwrap();
        assert_eq!(peer_pid(&s).unwrap(), std::process::id() as libc::pid_t);
        let exe = std::fs::read_link("/proc/self/exe").unwrap();
        assert!(!is_reprise_exe(&exe));
    }

    #[test]
    fn a_desktop_list_names_reprise_in_any_case_and_position() {
        assert!(names_reprise("reprise"));
        assert!(names_reprise("Reprise"));
        assert!(names_reprise("GNOME:reprise"));
        assert!(!names_reprise("KDE"));
        assert!(!names_reprise("reprise-dev"));
        assert!(!names_reprise(""));
    }

    #[test]
    fn environ_and_show_environment_give_the_desktop() {
        let environ = b"HOME=/home/u\0XDG_CURRENT_DESKTOP=reprise\0XDG_CURRENT_DESKTOP_X=1\0";
        assert_eq!(
            environ_value(environ, "XDG_CURRENT_DESKTOP"),
            Some("reprise".into())
        );
        assert_eq!(environ_value(environ, "XDG_SESSION_TYPE"), None);
        let shown = "HOME=/home/u\nXDG_CURRENT_DESKTOP=KDE\nXDG_SESSION_TYPE=wayland\n";
        assert_eq!(manager_desktop(shown), Some("KDE".into()));
        assert_eq!(manager_desktop("HOME=/home/u\n"), None);
    }

    #[test]
    fn a_stale_frontend_restarts_only_when_that_moves_its_routing() {
        assert!(frontend_needs_restart(Some("reprise"), Some("KDE")));
        assert!(frontend_needs_restart(Some("reprise"), None));
        assert!(!frontend_needs_restart(Some("reprise"), Some("reprise")));
        assert!(!frontend_needs_restart(Some("KDE"), Some("KDE")));
        assert!(!frontend_needs_restart(None, Some("KDE")));
    }
}
