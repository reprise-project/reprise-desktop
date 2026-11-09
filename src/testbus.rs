//! A private session bus for tests that need a real bus daemon.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

/// A `dbus-daemon`, killed on drop, and its address.
pub struct Bus(Child, pub String);

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A new bus, or None when `dbus-daemon` cannot run here.
pub fn bus() -> Option<Bus> {
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
