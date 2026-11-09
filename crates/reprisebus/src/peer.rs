//! Which process a session bus peer is, and whether it runs in a Flatpak sandbox, which may own
//! any name under its app id.

use std::os::fd::{AsFd, AsRawFd};
use std::path::Path;
use zbus::Connection;
use zbus::fdo::ConnectionCredentials;

/// Whether `peer` on `conn` runs in a Flatpak sandbox. A peer the bus gives no process fd for
/// (dbus-broker and dbus-daemon 1.15+ do), or that is gone, counts as sandboxed.
pub async fn sandboxed(conn: &Connection, peer: &str) -> bool {
    sandboxed_under(conn, peer, Path::new("/proc")).await
}

pub(crate) async fn sandboxed_under(conn: &Connection, peer: &str, proc: &Path) -> bool {
    let Ok(Some(fd)) = process_fd(conn, peer).await else {
        return true;
    };
    let Some(pid) = pid_of(&fd) else {
        return true;
    };
    // The pidfd keeps `pid` from being reused while it shows the pid: if it still does after the
    // read, the root read was the peer's.
    shows_flatpak_info(proc, pid) || pid_of(&fd) != Some(pid)
}

/// Whether `peer` on `conn` is the process `pid`; not when the bus gives no process fd for it, and
/// an error when the bus doesn't answer.
pub async fn is_process(conn: &Connection, peer: &str, pid: u32) -> zbus::Result<bool> {
    let fd = process_fd(conn, peer).await?;
    // A pidfd that still names `pid` means that process is alive and is the peer.
    Ok(fd.is_some_and(|fd| pid_of(&fd) == Some(pid)))
}

/// The pidfd of `peer`'s process, as the bus reports it.
async fn process_fd(conn: &Connection, peer: &str) -> zbus::Result<Option<zbus::OwnedFd>> {
    let reply = conn
        .call_method(
            Some(crate::DBUS),
            crate::DBUS_PATH,
            Some(crate::DBUS),
            "GetConnectionCredentials",
            &(peer,),
        )
        .await?;
    let credentials = reply.body().deserialize::<ConnectionCredentials>()?;
    Ok(credentials.into_process_fd())
}

/// The pid a pidfd refers to, from its fdinfo; None once the process is gone (`Pid: -1`).
fn pid_of(fd: &impl AsFd) -> Option<u32> {
    let fd = fd.as_fd().as_raw_fd();
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}")).ok()?;
    let pid = info.lines().find_map(|l| l.strip_prefix("Pid:"))?;
    pid.trim().parse().ok()
}

/// Whether process `pid` under `proc` shows a Flatpak sandbox's `/.flatpak-info`; a root that
/// can't be read counts as sandboxed.
fn shows_flatpak_info(proc: &Path, pid: u32) -> bool {
    let info = proc.join(pid.to_string()).join("root/.flatpak-info");
    match std::fs::symlink_metadata(info) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    /// A private bus daemon, killed on drop, and its address.
    pub(crate) struct Bus(Child, pub(crate) String);

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub(crate) fn bus() -> Option<Bus> {
        let mut child = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut line = String::new();
        BufReader::new(child.stdout.take()?)
            .read_line(&mut line)
            .ok()?;
        Some(Bus(child, line.trim().to_string()))
    }

    /// A `proc` folder in which this process shows a `/.flatpak-info`.
    pub(crate) fn sandboxed_proc() -> tempfile::TempDir {
        let proc = tempfile::tempdir().unwrap();
        let root = proc
            .path()
            .join(std::process::id().to_string())
            .join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".flatpak-info"), "[Application]\nname=x\n").unwrap();
        proc
    }

    #[test]
    fn a_process_showing_a_flatpak_info_counts_as_sandboxed() {
        let proc = sandboxed_proc();
        std::fs::create_dir_all(proc.path().join("8/root")).unwrap();
        assert!(shows_flatpak_info(proc.path(), std::process::id()));
        assert!(!shows_flatpak_info(proc.path(), 8), "no .flatpak-info");
        assert!(!shows_flatpak_info(proc.path(), 9), "gone");
    }

    #[test]
    fn a_peer_is_judged_by_the_process_its_pidfd_names() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let connect =
            || zbus::block_on(zbus::connection::Builder::address(bus.1.as_str()).build()).unwrap();
        let (us, peer) = (connect(), connect());
        let name = peer.unique_name().unwrap().to_string();
        let proc = sandboxed_proc();
        zbus::block_on(async {
            assert!(
                !sandboxed(&us, &name).await,
                "this test runs outside a sandbox"
            );
            assert!(sandboxed_under(&us, &name, proc.path()).await);
            assert!(sandboxed(&us, ":1.999").await, "no such peer");
        });
    }

    #[test]
    fn a_peer_is_matched_to_the_process_its_pidfd_names() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let connect =
            || zbus::block_on(zbus::connection::Builder::address(bus.1.as_str()).build()).unwrap();
        let (us, peer) = (connect(), connect());
        let name = peer.unique_name().unwrap().to_string();
        zbus::block_on(async {
            assert_eq!(is_process(&us, &name, std::process::id()).await, Ok(true));
            assert_eq!(
                is_process(&us, &name, 1).await,
                Ok(false),
                "another process"
            );
            assert!(
                is_process(&us, ":1.999", std::process::id()).await.is_err(),
                "the bus cannot answer for a peer it doesn't know"
            );
        });
    }
}
