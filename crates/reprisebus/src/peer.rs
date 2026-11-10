//! Which process a session bus peer is, whether it runs in a Flatpak sandbox, which may own any
//! name under its app id, and which app it is ([`app_identity`]).

use crate::mutex::lock;
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use zbus::Connection;
use zbus::fdo::ConnectionCredentials;

/// Which app a bus peer is, as its process shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppIdentity {
    /// A Flatpak app by its app id, or a strictly confined snap by `<snap>_<app>`, the id of the
    /// desktop entry snapd exports for it. The sandbox set it; the app can't change it.
    Sandboxed { app_id: String },
    /// Outside any sandbox, so trusted as the user is; `desktop_id` names the Reprise app scope it
    /// runs in (no `.desktop`), which whatever that program starts shares.
    Host { desktop_id: Option<String> },
    /// Gone, or showing too little of itself to tell; never trusted.
    Unknown,
}

/// Which app `peer` on `conn` is, read through the pidfd the bus gives for it (dbus-broker and
/// dbus-daemon 1.15+ do). A peer without one, or gone, is [`AppIdentity::Unknown`].
pub async fn app_identity(conn: &Connection, peer: &str) -> AppIdentity {
    identity_under(conn, peer, Path::new("/proc")).await
}

async fn identity_under(conn: &Connection, peer: &str, proc: &Path) -> AppIdentity {
    let Ok(Some(fd)) = process_fd(conn, peer).await else {
        return AppIdentity::Unknown;
    };
    match pid_of(&fd).and_then(|pid| i32::try_from(pid).ok()) {
        Some(pid) => placement(fd.as_fd(), pid, proc).into(),
        None => AppIdentity::Unknown,
    }
}

impl From<Placement> for AppIdentity {
    fn from(placement: Placement) -> Self {
        match placement {
            Placement::Host { desktop_id } => AppIdentity::Host { desktop_id },
            Placement::Flatpak {
                app_id: Some(app_id),
            }
            | Placement::Snap {
                app_id: Some(app_id),
            } => AppIdentity::Sandboxed { app_id },
            _ => AppIdentity::Unknown,
        }
    }
}

/// Where a process runs, by the rules of docs/security.md, "Sandboxed programs". The compositor's
/// sandbox filter and [`app_identity`] both read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Outside any sandbox; `desktop_id` names the Reprise app scope it runs in (no `.desktop`).
    Host { desktop_id: Option<String> },
    /// Its root holds Flatpak's `/.flatpak-info`, or it runs in an `app-flatpak-*` scope; `app_id`
    /// is None when neither names an app.
    Flatpak { app_id: Option<String> },
    /// A strict or devmode snap: a `snap.*` AppArmor profile, else a `snap.*` scope; `app_id`,
    /// `<snap>_<app>`, is None when neither names an app.
    Snap { app_id: Option<String> },
    /// Gone by the time it was read, or hiding its root inside a pid namespace.
    Unknown,
}

/// Where the process `pidfd` names, `pid` here, runs, read from `proc` (`/proc` outside tests).
/// The reads count only if it still runs after them: its pidfd unreaped (so `pid` was not
/// reused), no zombie.
pub fn placement(pidfd: BorrowedFd<'_>, pid: i32, proc: &Path) -> Placement {
    let Ok(dir) = std::fs::File::open(proc.join(pid.to_string())) else {
        return Placement::Unknown;
    };
    let root = open_at(&dir, c"root", libc::O_PATH | libc::O_DIRECTORY);
    let seen = Seen {
        flatpak_info: root.as_ref().and_then(flatpak_info_at),
        cgroup: read_at(&dir, c"cgroup"),
        label: read_at(&dir, c"attr/apparmor/current"),
        pid_namespaced: read_at(&dir, c"status").map(|s| pid_namespaced(&s)),
    };
    let running = read_at(&dir, c"stat").is_some_and(|s| runs(&s));
    match running && alive(pidfd) {
        true => seen.placement(),
        false => Placement::Unknown,
    }
}

/// What `/proc/<pid>` showed of a live process. A non-dumpable process hides its root and
/// AppArmor label; its cgroup and status stay readable.
#[derive(Debug, Default)]
struct Seen {
    /// The text of the `/.flatpak-info` at its root, empty when it can't be read; Some(None)
    /// when there is none, None when that can't be told.
    flatpak_info: Option<Option<String>>,
    cgroup: Option<String>,
    label: Option<String>,
    /// Whether it runs in a pid namespace of its own, as bubblewrap puts every Flatpak app.
    pid_namespaced: Option<bool>,
}

impl Seen {
    fn placement(&self) -> Placement {
        let scopes = || self.cgroup.as_deref().into_iter().flat_map(units);
        if let Some(Some(info)) = &self.flatpak_info {
            let app_id = app_of_flatpak_info(info);
            return Placement::Flatpak { app_id };
        }
        if let Some(unit) =
            scopes().find(|u| u.starts_with("app-flatpak-") && u.ends_with(".scope"))
        {
            let app_id = flatpak_scope_app(unit);
            return Placement::Flatpak { app_id };
        }
        // A classic snap runs `unconfined` in a snap scope; only without a label does the scope
        // decide.
        let snap = match self.label.as_deref() {
            Some(label) => (label.trim_start().starts_with("snap.")).then(|| snap_of_label(label)),
            None => (scopes().find(|u| u.starts_with("snap.") && u.ends_with(".scope")))
                .map(snap_of_scope),
        };
        if let Some(app_id) = snap {
            return Placement::Snap { app_id };
        }
        let host = Placement::Host {
            desktop_id: scopes().find_map(scope_app),
        };
        match self.flatpak_info {
            Some(_) => host,
            None if self.cgroup.is_some() && self.pid_namespaced == Some(false) => host,
            None => Placement::Unknown,
        }
    }
}

/// The most of a `.flatpak-info` read; Flatpak writes a few KiB.
const INFO_BYTES: u64 = 64 * 1024;

/// [`Seen::flatpak_info`] under the root `root`. Whether it is there is told without following a
/// link; its text is read only from a regular file, opened without blocking on a FIFO.
fn flatpak_info_at(root: &OwnedFd) -> Option<Option<String>> {
    const INFO: &std::ffi::CStr = c".flatpak-info";
    // SAFETY: an all-zero stat is valid for fstatat to fill.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `INFO` is NUL-terminated, `st` live, `root` an open directory.
    let r = unsafe {
        libc::fstatat(
            root.as_raw_fd(),
            INFO.as_ptr(),
            &mut st,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if r != 0 {
        let absent = std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT);
        return absent.then_some(None);
    }
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY;
    let mut text = String::new();
    if let Some(fd) = open_at(root, INFO, flags) {
        let file = std::fs::File::from(fd);
        let regular = file.metadata().is_ok_and(|m| m.is_file());
        if !regular || file.take(INFO_BYTES).read_to_string(&mut text).is_err() {
            text.clear();
        }
    }
    Some(Some(text))
}

fn open_at(dir: &impl AsRawFd, name: &std::ffi::CStr, flags: libc::c_int) -> Option<OwnedFd> {
    // SAFETY: `name` is NUL-terminated and `dir` an open directory.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags | libc::O_CLOEXEC) };
    // SAFETY: a fresh fd nothing else owns.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

fn read_at(dir: &impl AsRawFd, name: &std::ffi::CStr) -> Option<String> {
    let fd = open_at(dir, name, libc::O_RDONLY)?;
    std::io::read_to_string(std::fs::File::from(fd)).ok()
}

/// Whether the process `pidfd` names has not been reaped.
fn alive(pidfd: BorrowedFd<'_>) -> bool {
    // SAFETY: signal 0 through a pidfd only checks that the process exists.
    let r = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            0,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    r == 0
}

/// `name` in the `[Application]` group of a `.flatpak-info` or instance `info` key file.
pub fn app_of_flatpak_info(info: &str) -> Option<String> {
    let mut group = "";
    for line in info.lines().map(str::trim) {
        if let Some(g) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = g;
        } else if group == "Application"
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == "name"
        {
            return Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// The systemd unit each line of a `/proc/<pid>/cgroup` puts the process in.
fn units(cgroup: &str) -> impl Iterator<Item = &str> {
    cgroup.lines().map(|l| l.rsplit('/').next().unwrap_or(""))
}

/// The app id in the scope Flatpak runs an app in, `app-flatpak-<app id>-<pid>.scope`.
fn flatpak_scope_app(unit: &str) -> Option<String> {
    let named = unit.strip_prefix("app-flatpak-")?.strip_suffix(".scope")?;
    let (app, pid) = named.rsplit_once('-')?;
    let pid = !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit());
    (pid && !app.is_empty()).then(|| app.to_string())
}

/// `<snap>_<app>` from a snap's AppArmor label, `snap.<snap>.<app> (enforce)`.
fn snap_of_label(label: &str) -> Option<String> {
    snap_app(label.split_whitespace().next()?.strip_prefix("snap.")?)
}

/// `<snap>_<app>` from the scope snapd runs a snap's app in, `snap.<snap>.<app>-<uuid>.scope`.
fn snap_of_scope(unit: &str) -> Option<String> {
    const UUID_CHARS: usize = 36;
    let named = unit.strip_prefix("snap.")?.strip_suffix(".scope")?;
    let cut = named.len().checked_sub(UUID_CHARS + 1)?;
    (named.as_bytes()[cut] == b'-').then(|| snap_app(&named[..cut]))?
}

fn snap_app(snap_dot_app: &str) -> Option<String> {
    let (snap, app) = snap_dot_app.split_once('.')?;
    (!snap.is_empty() && !app.is_empty()).then(|| format!("{snap}_{app}"))
}

/// The desktop entry id (no `.desktop`) in a Reprise app scope unit,
/// `app-reprise-<id>-<suffix>.scope`, unescaped; None for any other unit. The id is unit-escaped
/// and the suffix is fields joined by `x`, so the id ends at the last dash.
pub fn scope_app(unit: &str) -> Option<String> {
    let rest = unit.strip_prefix("app-reprise-")?.strip_suffix(".scope")?;
    let (id, suffix) = rest.rsplit_once('-')?;
    if id.is_empty() || suffix.is_empty() {
        return None;
    }
    Some(unescape(id))
}

/// `systemd-escape`'s `\xNN` back to bytes.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 2..i + 4)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (
            b[i],
            b.get(i + 1),
            hex.and_then(|h| u8::from_str_radix(h, 16).ok()),
        ) {
            (b'\\', Some(b'x'), Some(v)) => {
                out.push(v);
                i += 4;
            }
            (c, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether `/proc/<pid>/status` gives the process more than one pid (`NSpid`), one per nested
/// pid namespace.
fn pid_namespaced(status: &str) -> bool {
    status
        .lines()
        .find_map(|l| l.strip_prefix("NSpid:"))
        .is_some_and(|pids| pids.split_whitespace().count() > 1)
}

/// Whether `/proc/<pid>/stat` describes a process that has not exited: its state, after the
/// command's closing parenthesis, is neither zombie nor dead.
fn runs(stat: &str) -> bool {
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.trim_start().chars().next())
        .is_some_and(|state| !matches!(state, 'Z' | 'X' | 'x'))
}

/// [`app_identity`] kept per unique bus name, which the bus never hands out twice. A well-known
/// name is looked up by its current owner. [`Identities::forget`] a name once it leaves the bus.
pub struct Identities {
    proc: PathBuf,
    known: Mutex<HashMap<String, AppIdentity>>,
}

impl Default for Identities {
    fn default() -> Self {
        Self::under("/proc")
    }
}

impl Identities {
    /// Identities read from the process tree at `proc` in place of `/proc`.
    pub fn under(proc: impl Into<PathBuf>) -> Self {
        Identities {
            proc: proc.into(),
            known: Mutex::default(),
        }
    }

    pub async fn of(&self, conn: &Connection, peer: &str) -> AppIdentity {
        // A peer-to-peer connection has no bus to ask.
        if conn.unique_name().is_none() {
            return AppIdentity::Unknown;
        }
        let unique = match peer.starts_with(':') {
            true => peer.to_string(),
            false => match owner(conn, peer).await {
                Some(owner) => owner,
                None => return AppIdentity::Unknown,
            },
        };
        if let Some(known) = lock(&self.known).get(&unique) {
            return known.clone();
        }
        let identity = identity_under(conn, &unique, &self.proc).await;
        // Unknown may be a bus hiccup, so it is asked again next time.
        if identity != AppIdentity::Unknown {
            lock(&self.known).insert(unique, identity.clone());
        }
        identity
    }

    pub fn forget(&self, name: &str) {
        lock(&self.known).remove(name);
    }
}

/// The unique name owning `name`, when it has an owner.
async fn owner(conn: &Connection, name: &str) -> Option<String> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(name).ok()?;
    Some(dbus.get_name_owner(name).await.ok()?.to_string())
}

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

    const USER: &str = "0::/user.slice/user-1000.slice/user@1000.service/app.slice";

    /// A `proc` folder in which this process shows `files`, paths under `/proc/<pid>` and their
    /// texts, and runs.
    fn proc_showing(files: &[(&str, &str)]) -> tempfile::TempDir {
        let proc = tempfile::tempdir().unwrap();
        let dir = proc.path().join(std::process::id().to_string());
        std::fs::create_dir_all(dir.join("root")).unwrap();
        std::fs::write(dir.join("stat"), "42 (app) S 1 42\n").unwrap();
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        proc
    }

    /// What `Identities` under `proc` make of a fresh peer on `bus`, asked by its unique name and
    /// by a well-known name it owns.
    fn identity_on(bus: &Bus, proc: &Path) -> (AppIdentity, AppIdentity) {
        let connect =
            || zbus::block_on(zbus::connection::Builder::address(bus.1.as_str()).build()).unwrap();
        let (us, peer) = (connect(), connect());
        let name = peer.unique_name().unwrap().to_string();
        zbus::block_on(async {
            peer.request_name("org.example.Peer").await.unwrap();
            let ids = Identities::under(proc);
            let by_unique = ids.of(&us, &name).await;
            ids.forget(&name);
            (by_unique, ids.of(&us, "org.example.Peer").await)
        })
    }

    #[test]
    fn a_flatpak_peer_is_the_app_its_flatpak_info_names() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let proc = proc_showing(&[
            (
                "root/.flatpak-info",
                "[Application]\nname=org.example.Chat\n",
            ),
            (
                "cgroup",
                &format!("{USER}/app-reprise-firefox-1ax2.scope\n"),
            ),
            ("status", "NSpid:\t4242\t2\n"),
        ]);
        let chat = AppIdentity::Sandboxed {
            app_id: "org.example.Chat".into(),
        };
        assert_eq!(identity_on(&bus, proc.path()), (chat.clone(), chat));
    }

    #[test]
    fn a_host_peer_is_the_app_whose_scope_it_runs_in() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let scoped = proc_showing(&[
            (
                "cgroup",
                &format!("{USER}/app-reprise-reprise\\x2dnotepad-s7x1ax2x3.scope\n"),
            ),
            ("status", "NSpid:\t4242\n"),
        ]);
        let notepad = AppIdentity::Host {
            desktop_id: Some("reprise-notepad".into()),
        };
        assert_eq!(identity_on(&bus, scoped.path()), (notepad.clone(), notepad));
        let unscoped = proc_showing(&[
            ("cgroup", "0::/user.slice/user-1000.slice/session-7.scope\n"),
            ("status", "NSpid:\t4242\n"),
        ]);
        let host = AppIdentity::Host { desktop_id: None };
        assert_eq!(identity_on(&bus, unscoped.path()), (host.clone(), host));
    }

    #[test]
    fn a_peer_gone_from_the_bus_or_its_process_is_unknown() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let connect =
            || zbus::block_on(zbus::connection::Builder::address(bus.1.as_str()).build()).unwrap();
        let us = connect();
        let host = proc_showing(&[("cgroup", USER), ("status", "NSpid:\t4242\n")]);
        let ids = Identities::under(host.path());
        let gone = connect().unique_name().unwrap().to_string();
        zbus::block_on(async {
            assert_eq!(ids.of(&us, &gone).await, AppIdentity::Unknown, "left");
            assert_eq!(ids.of(&us, ":1.999").await, AppIdentity::Unknown);
            assert_eq!(
                ids.of(&us, "org.example.Nobody").await,
                AppIdentity::Unknown
            );
        });
        let zombie = proc_showing(&[("stat", "42 (app) Z 1 42\n"), ("cgroup", USER)]);
        assert_eq!(
            identity_on(&bus, zombie.path()).0,
            AppIdentity::Unknown,
            "exited"
        );
    }

    #[test]
    fn an_identity_is_kept_until_its_name_is_forgotten() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let connect =
            || zbus::block_on(zbus::connection::Builder::address(bus.1.as_str()).build()).unwrap();
        let (us, peer) = (connect(), connect());
        let name = peer.unique_name().unwrap().to_string();
        let proc = proc_showing(&[("root/.flatpak-info", "[Application]\nname=org.a.A\n")]);
        let ids = Identities::under(proc.path());
        let info = proc
            .path()
            .join(std::process::id().to_string())
            .join("root/.flatpak-info");
        zbus::block_on(async {
            let first = ids.of(&us, &name).await;
            std::fs::write(&info, "[Application]\nname=org.b.B\n").unwrap();
            assert_eq!(ids.of(&us, &name).await, first);
            ids.forget(&name);
            let app_id = "org.b.B".to_string();
            assert_eq!(ids.of(&us, &name).await, AppIdentity::Sandboxed { app_id });
        });
    }

    #[test]
    fn a_flatpak_info_that_cant_be_read_still_marks_a_sandbox() {
        let Some(bus) = bus() else {
            eprintln!("no dbus-daemon; skipped");
            return;
        };
        let proc = proc_showing(&[("cgroup", USER), ("status", "NSpid:\t4242\n")]);
        let info = proc
            .path()
            .join(std::process::id().to_string())
            .join("root/.flatpak-info");
        // A socket can't be opened, even by root.
        let _socket = std::os::unix::net::UnixListener::bind(&info).unwrap();
        assert_eq!(identity_on(&bus, proc.path()).0, AppIdentity::Unknown);
    }

    fn seen(info: Option<Option<&str>>, cgroup: &str, label: Option<&str>, ns: bool) -> Placement {
        Seen {
            flatpak_info: info.map(|i| i.map(String::from)),
            cgroup: Some(cgroup.into()),
            label: label.map(Into::into),
            pid_namespaced: Some(ns),
        }
        .placement()
    }

    #[test]
    fn a_process_is_placed_by_its_root_scope_or_profile() {
        let flatpak = |id: Option<&str>| Placement::Flatpak {
            app_id: id.map(Into::into),
        };
        let snapped = |id: Option<&str>| Placement::Snap {
            app_id: id.map(Into::into),
        };
        let host = Placement::Host { desktop_id: None };
        let in_flatpak = format!("{USER}/app-flatpak-org.example.my-app-12345.scope\n");
        let snap = format!("{USER}/snap.code.code-0f3a9c1e-2b4d-4e8a-9c2f-1d2e3f4a5b6c.scope\n");
        let foot = format!("{USER}/app-reprise-foot-3.scope\n");
        let enforce = Some("snap.code.code-insiders (enforce)\n");
        let unconfined = Some("unconfined\n");
        let info = Some(Some("[Application]\nname=org.example.App\n"));
        assert_eq!(
            seen(info, &foot, unconfined, true),
            flatpak(Some("org.example.App"))
        );
        assert_eq!(
            seen(None, &in_flatpak, None, true),
            flatpak(Some("org.example.my-app")),
            "a hidden root in a Flatpak scope"
        );
        assert_eq!(
            seen(Some(Some("[Application]\n")), USER, None, false),
            flatpak(None),
            "a .flatpak-info naming no app"
        );
        assert_eq!(
            seen(Some(None), &snap, enforce, false),
            snapped(Some("code_code-insiders"))
        );
        assert_eq!(
            seen(Some(None), &snap, None, false),
            snapped(Some("code_code")),
            "a snap scope without AppArmor"
        );
        assert_eq!(seen(None, &snap, None, false), snapped(Some("code_code")));
        assert_eq!(
            seen(Some(None), &snap, unconfined, false),
            host,
            "a classic snap"
        );
        let unnamed = format!("{USER}/snap.code.code.scope\n");
        assert_eq!(
            seen(Some(None), &unnamed, None, false),
            snapped(None),
            "a snap scope naming no app"
        );
        assert_eq!(
            seen(Some(None), &format!("{USER}/snap.mount\n"), None, false),
            host,
            "a unit that is no scope"
        );
        let in_foot = Placement::Host {
            desktop_id: Some("foot".into()),
        };
        assert_eq!(seen(Some(None), &foot, unconfined, false), in_foot);
        assert_eq!(seen(None, &foot, None, false), in_foot, "a hidden root");
        assert_eq!(
            seen(None, USER, None, true),
            Placement::Unknown,
            "a hidden root in a pid namespace outside any app scope"
        );
        let blind = Seen::default().placement();
        assert_eq!(blind, Placement::Unknown, "nothing readable");
    }

    #[test]
    fn a_sandbox_naming_no_app_is_an_unknown_app() {
        let unnamed = AppIdentity::from(Placement::Flatpak { app_id: None });
        assert_eq!(unnamed, AppIdentity::Unknown);
        assert_eq!(
            AppIdentity::from(Placement::Snap { app_id: None }),
            AppIdentity::Unknown
        );
    }

    #[test]
    fn nspid_counts_the_pid_namespaces() {
        assert!(!pid_namespaced("Name:\tfoot\nNSpid:\t4242\n"));
        assert!(pid_namespaced("Name:\tbwrap\nNSpid:\t4242\t2\n"));
        assert!(!pid_namespaced(""));
    }

    #[test]
    fn a_process_state_is_read_after_the_last_parenthesis() {
        assert!(runs("42 (foot) S 1 42"));
        assert!(
            !runs("42 (a) Z (b) Z 1"),
            "a command may hold a parenthesis"
        );
        assert!(runs("42 (a) Z (b) R 1"));
        assert!(!runs("42 (foot) X 1"));
        assert!(!runs(""));
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
