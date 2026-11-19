//! Test support: a private bus daemon, connections to it whose calls give up ([`connect`]) and
//! that take calls once served ([`serve`]), [`until`] for work on other threads, and [`skipped`],
//! which every test that needs something this machine may lack reports through.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use zbus::ObjectPath;
use zbus::names::OwnedUniqueName;
use zbus::object_server::Interface;

/// How long [`until`] waits before it fails the test.
pub const CEILING: Duration = Duration::from_secs(60);

/// Polls `done` every 5 ms until it holds; fails the test naming `what` after [`CEILING`].
#[track_caller]
pub fn until(what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + CEILING;
    while !done() {
        assert!(Instant::now() < end, "{what} within {CEILING:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// How long a call on a [`connect`] connection waits for its reply, so a call nobody answers
/// fails the test in seconds.
pub const REPLY: Duration = Duration::from_secs(10);

/// A connection to the bus at `address` whose method calls give up after [`REPLY`].
pub async fn connect(address: &str) -> zbus::Connection {
    (zbus::connection::Builder::address(address).method_timeout(REPLY))
        .build()
        .await
        .unwrap()
}

/// Serves `iface` at `path` on `conn`, a bus connection, and returns once `conn` answers calls
/// to it. The one way tests serve on a built connection: zbus starts its object server's task on
/// first use, and a call that arrives before the task has subscribed is dropped without a reply.
/// `tests/served.rs` keeps `ObjectServer::at` out of every other test.
pub async fn serve<'p, P, I>(conn: &zbus::Connection, path: P, iface: I)
where
    P: TryInto<ObjectPath<'p>>,
    P::Error: std::fmt::Debug,
    I: Interface,
{
    serve_to(conn, conn, path, iface).await;
}

/// [`serve`] for `peer`'s calls; on a peer-to-peer connection, `peer` is its other end.
pub async fn serve_to<'p, P, I>(conn: &zbus::Connection, peer: &zbus::Connection, path: P, iface: I)
where
    P: TryInto<ObjectPath<'p>>,
    P::Error: std::fmt::Debug,
    I: Interface,
{
    let path = path.try_into().expect("an object path");
    let served = conn.object_server().at(&path, iface).await;
    assert!(matches!(served, Ok(None)), "{path} is served");
    let end = Instant::now() + REPLY;
    while !pinged(peer, conn.unique_name(), &path).await {
        assert!(Instant::now() < end, "{path} answers within {REPLY:?}");
    }
}

/// Whether a `Peer.Ping` from `peer` to `path` on `to` is answered within a short wait.
async fn pinged(
    peer: &zbus::Connection,
    to: Option<&OwnedUniqueName>,
    path: &ObjectPath<'_>,
) -> bool {
    let ping = peer.call_method(to, path, Some("org.freedesktop.DBus.Peer"), "Ping", &());
    let late = async {
        async_io::Timer::after(Duration::from_millis(50)).await;
        false
    };
    futures_lite::future::or(async { ping.await.is_ok() }, late).await
}

/// A `dbus-daemon`, killed on drop, and its address.
pub struct Bus(Child, pub String);

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A new session bus of the test's own, or None after [`skipped`] when `dbus-daemon` cannot run.
#[track_caller]
pub fn bus() -> Option<Bus> {
    let bus = spawn(true);
    if bus.is_none() {
        skipped("no dbus-daemon");
    }
    bus
}

/// A [`bus`] that starts no service: [`bus`] reads this machine's session service files, so a
/// call to a name one of them covers starts the real service.
#[track_caller]
pub fn bus_with_no_services() -> Option<Bus> {
    let bus = spawn(false);
    if bus.is_none() {
        skipped("no dbus-daemon");
    }
    bus
}

/// The session bus configuration without its service folders.
const NO_SERVICES: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

/// [`NO_SERVICES`] in the test binary's session folder ([`crate::testsession`]), written once.
fn no_services() -> Option<&'static std::path::Path> {
    static FILE: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    let file = FILE.get_or_init(|| {
        let file = crate::testsession::dir().join("no-services.conf");
        std::fs::create_dir_all(crate::testsession::dir()).ok()?;
        std::fs::write(&file, NO_SERVICES).ok()?;
        Some(file)
    });
    file.as_deref()
}

fn spawn(services: bool) -> Option<Bus> {
    let mut command = Command::new("dbus-daemon");
    if services {
        command.arg("--session");
    } else {
        command.arg("--config-file").arg(no_services()?);
    }
    let mut child = command
        .args(["--nofork", "--print-address"])
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

/// Reports a test that cannot run on this machine: panics under `REPRISE_TEST_STRICT`, else
/// prints `reason`. `reprisekit::pins::skipped` is the same for crates that don't link this one.
#[track_caller]
pub fn skipped(reason: &str) {
    skip(
        std::env::var_os("REPRISE_TEST_STRICT").is_some_and(|v| v != "0"),
        reason,
    );
}

#[track_caller]
fn skip(strict: bool, reason: &str) {
    assert!(
        !strict,
        "REPRISE_TEST_STRICT is set, so this test may not skip: {reason}"
    );
    eprintln!("skipped: {reason}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_skip_fails_only_under_strict() {
        assert!(std::panic::catch_unwind(|| super::skip(true, "a reason")).is_err());
        super::skip(false, "a reason");
    }
}
