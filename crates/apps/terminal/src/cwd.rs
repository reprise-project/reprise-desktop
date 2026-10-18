//! Where a tab's shell is and what runs in its foreground: `/proc`, and the OSC 7 the shell
//! reports its directory with, read off the PTY on the way to the parser.

use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The working directory of process `pid`.
pub fn cwd_of(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

/// The foreground process group of the terminal `pid` controls.
pub fn fg_pgrp(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name in parentheses may hold spaces; tpgid is the sixth field after it.
    let rest = &stat[stat.rfind(')')? + 1..];
    let tpgid: i64 = rest.split_whitespace().nth(5)?.parse().ok()?;
    u32::try_from(tpgid).ok().filter(|&g| g > 0)
}

/// Whether closing a tab asks first: something other than its shell holds the foreground.
pub fn needs_confirm(fg_pgrp: Option<u32>, shell_pid: u32) -> bool {
    fg_pgrp.is_some_and(|g| g != shell_pid)
}

/// The directory a `file://host/path` URI names, when `host` is empty, `localhost` or `local`.
pub fn osc7_path(uri: &str, local: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let (host, path) = rest.split_at(rest.find('/')?);
    if !(host.is_empty() || host == "localhost" || host == local) {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(std::ffi::OsString::from_vec(out).into())
}

/// The longest OSC kept; a longer one is dropped.
const MAX_OSC: usize = 4096;

/// Picks OSC 7 sequences out of a byte stream split anywhere.
#[derive(Default)]
pub struct Osc7 {
    state: State,
    buf: Vec<u8>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    OscEsc,
}

impl Osc7 {
    /// Feed the next bytes; the URI of the last OSC 7 they complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<String> {
        let mut found = None;
        for &b in bytes {
            self.state = match (self.state, b) {
                (State::Ground | State::Esc, 0x1b) => State::Esc,
                (State::Esc | State::OscEsc, b']') => {
                    self.buf.clear();
                    State::Osc
                }
                (State::Osc, 0x07) | (State::OscEsc, b'\\') => {
                    if let Some(uri) = self.buf.strip_prefix(b"7;") {
                        found = Some(String::from_utf8_lossy(uri).into_owned());
                    }
                    State::Ground
                }
                (State::Osc, 0x1b) => State::OscEsc,
                (State::Osc, b) if self.buf.len() < MAX_OSC => {
                    self.buf.push(b);
                    State::Osc
                }
                _ => State::Ground,
            };
        }
        found
    }
}

/// The PTY's reader: passes every byte on and keeps the last OSC 7 directory in `cwd`.
pub struct Reader {
    pub file: File,
    pub scan: Osc7,
    pub host: String,
    pub cwd: Arc<Mutex<Option<PathBuf>>>,
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(buf)?;
        if let Some(uri) = self.scan.feed(&buf[..n])
            && let Some(path) = osc7_path(&uri, &self.host)
        {
            *self.cwd.lock().unwrap() = Some(path);
        }
        Ok(n)
    }
}

/// This machine's host name.
pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_of_reads_proc() {
        assert_eq!(
            cwd_of(std::process::id()),
            std::env::current_dir().ok(),
            "this process's own directory"
        );
        assert_eq!(cwd_of(u32::MAX), None);
    }

    #[test]
    fn fg_pgrp_of_a_process_without_a_terminal_is_none() {
        assert_eq!(fg_pgrp(u32::MAX), None);
    }

    #[test]
    fn needs_confirm_only_when_another_group_holds_the_foreground() {
        assert!(!needs_confirm(Some(100), 100), "the shell itself");
        assert!(needs_confirm(Some(200), 100), "a program the shell started");
        assert!(!needs_confirm(None, 100), "the shell is gone");
    }

    #[test]
    fn osc7_paths_are_local_and_decoded() {
        let p = |u: &str| osc7_path(u, "box");
        assert_eq!(p("file://box/home/me"), Some("/home/me".into()));
        assert_eq!(p("file:///tmp"), Some("/tmp".into()));
        assert_eq!(p("file://localhost/a%20b"), Some("/a b".into()));
        assert_eq!(p("file://other/home/me"), None, "another machine's path");
        assert_eq!(p("http://box/x"), None);
    }

    #[test]
    fn osc7_found_across_chunks_with_either_terminator() {
        let mut s = Osc7::default();
        assert_eq!(s.feed(b"hi \x1b]0;title\x07 \x1b]7;file://h"), None);
        assert_eq!(s.feed(b"/x\x1b"), None);
        assert_eq!(s.feed(b"\\ more"), Some("file://h/x".into()));
        assert_eq!(
            s.feed(b"\x1b]7;file:///a\x07\x1b]7;file:///b\x07"),
            Some("file:///b".into())
        );
        assert_eq!(
            s.feed(b"\x1b[31m\x1b]70;x\x07"),
            None,
            "OSC 70 is not OSC 7"
        );
    }
}
