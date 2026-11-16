//! Test support: a private bus daemon, [`until`] for work on other threads, and [`skipped`],
//! which every test that needs something this machine may lack reports through.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
    let bus = spawn();
    if bus.is_none() {
        skipped("no dbus-daemon");
    }
    bus
}

fn spawn() -> Option<Bus> {
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
