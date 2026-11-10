//! Where a tab's shell is and what runs in its foreground: `/proc`, and the OSC 7 the shell
//! reports its directory with, read off the PTY on the way to the parser.

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::escapes;

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

/// `path` from an OSC 7 when it is `shell_cwd` itself, by the name the shell gave it; a host
/// reached through ssh reports its own directories, which this machine may also have.
pub fn osc7_dir(path: PathBuf, shell_cwd: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let (at, cwd) = (
        std::fs::metadata(&path).ok()?,
        std::fs::metadata(shell_cwd).ok()?,
    );
    (at.is_dir() && at.dev() == cwd.dev() && at.ino() == cwd.ino()).then_some(path)
}

/// How long a new tab waits on the stat of its directory; a dead network mount never answers.
const CHECK_WAIT: Duration = Duration::from_millis(200);

/// Where a tab opened from shell `pid`'s starts: `reported`, its last OSC 7, when that is the
/// shell's own directory, else the shell's; home when the stat takes past [`CHECK_WAIT`].
pub fn start_dir(reported: Option<PathBuf>, pid: u32) -> Option<PathBuf> {
    let found = within(CHECK_WAIT, move || {
        let shell = PathBuf::from(format!("/proc/{pid}/cwd"));
        let named = reported.and_then(|p| osc7_dir(p, &shell));
        named.or_else(|| cwd_of(pid).filter(|d| d.is_dir()))
    });
    found.unwrap_or_else(|| std::env::var_os("HOME").map(PathBuf::from))
}

/// `f`'s result from a thread of its own, or None once `wait` passes; the thread is left behind.
fn within<T: Send + 'static>(wait: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("reprise-terminal-cwd".into())
        .spawn(move || tx.send(f()));
    spawned.ok()?;
    rx.recv_timeout(wait).ok()
}

/// The PTY's reader: passes its bytes through an [`escapes::Filter`] and keeps the last OSC 7
/// path in `cwd` as reported, touching no file system, since this thread also writes the keys.
pub struct Reader {
    file: File,
    /// The tab's shell: an OSC 7 counts only while it holds the foreground.
    shell: u32,
    filter: escapes::Filter,
    /// What the filter passed on; `out[sent..]` did not fit the caller's buffer yet.
    out: Vec<u8>,
    sent: usize,
    host: String,
    cwd: Arc<Mutex<Option<PathBuf>>>,
}

impl Reader {
    pub fn new(file: File, shell: u32, host: String, cwd: Arc<Mutex<Option<PathBuf>>>) -> Self {
        Reader {
            file,
            shell,
            filter: Default::default(),
            out: Vec::new(),
            sent: 0,
            host,
            cwd,
        }
    }

    fn shell_in_front(&self) -> bool {
        // SAFETY: tcgetpgrp only reads the foreground group of the open PTY master `file` holds.
        let pgrp = unsafe { libc::tcgetpgrp(self.file.as_raw_fd()) };
        u32::try_from(pgrp).is_ok_and(|g| g == self.shell)
    }
}

impl Read for Reader {
    /// Hands on what the filter let go before reading more, so a read reports `WouldBlock` only
    /// when nothing waits, and alacritty's loop blocks on the terminal once its buffer is full.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.sent == self.out.len() {
            let n = self.file.read(buf)?;
            self.out.clear();
            self.sent = 0;
            let uri = self.filter.feed(&buf[..n], &mut self.out);
            if let Some(uri) = uri
                && self.shell_in_front()
                && let Some(path) = osc7_path(&uri, &self.host)
            {
                *self.cwd.lock().unwrap() = Some(path);
            }
        }
        let n = buf.len().min(self.out.len() - self.sent);
        buf[..n].copy_from_slice(&self.out[self.sent..][..n]);
        self.sent += n;
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

    fn reader(rx: std::os::unix::net::UnixStream) -> Reader {
        let file = File::from(std::os::fd::OwnedFd::from(rx));
        Reader::new(file, 0, String::new(), Default::default())
    }

    #[test]
    fn a_held_escape_comes_back_within_the_buffer() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (mut tx, rx) = UnixStream::pair().unwrap();
        let mut r = reader(rx);
        let osc = b"\x1b]7;file:///tmp\x07";
        let mut buf = [0u8; 20];
        tx.write_all(&osc[..10]).unwrap();
        assert_eq!(r.read(&mut buf).unwrap(), 0, "an unended OSC is held");
        tx.write_all(&osc[10..]).unwrap();
        tx.write_all(b"after").unwrap();
        let mut got = Vec::new();
        while got.len() < osc.len() + 5 {
            let n = r.read(&mut buf).unwrap();
            got.extend_from_slice(&buf[..n]);
        }
        assert_eq!(got, [&osc[..], b"after"].concat());
    }

    #[test]
    fn a_read_stops_only_once_nothing_waits_even_past_a_held_escape() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (mut tx, rx) = UnixStream::pair().unwrap();
        rx.set_nonblocking(true).unwrap();
        let mut r = reader(rx);
        let sent = [&b"\x1b]7;file:///tmp\x07"[..], b"after"].concat();
        tx.write_all(&sent).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4];
        loop {
            match r.read(&mut buf) {
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(got, sent, "a 4-byte buffer takes every waiting byte");
    }

    /// The PTY thread also writes the keys typed, so a stat on a dead mount there would freeze the
    /// tab: a directory that doesn't exist still lands as reported.
    #[test]
    fn the_reader_keeps_an_osc7_unchecked() {
        use alacritty_terminal::event::WindowSize;
        use alacritty_terminal::tty::{self, Options, Shell};
        let script = r"printf '\033]7;file:///nonexistent-reprise\007'; exec sleep 5";
        let options = Options {
            shell: Some(Shell::new(
                "/bin/sh".into(),
                vec!["-c".into(), script.into()],
            )),
            working_directory: None,
            drain_on_exit: false,
            env: Default::default(),
        };
        let size = WindowSize {
            num_lines: 24,
            num_cols: 80,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&options, size, 0).unwrap();
        let cwd = Arc::new(Mutex::new(None));
        let file = pty.file().try_clone().unwrap();
        let mut r = Reader::new(file, pty.child().id(), String::new(), cwd.clone());
        let mut buf = [0u8; 256];
        for _ in 0..500 {
            if cwd.lock().unwrap().is_some() {
                break;
            }
            match r.read(&mut buf) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => break,
                Ok(_) => {}
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(*cwd.lock().unwrap(), Some("/nonexistent-reprise".into()));
    }

    #[test]
    fn a_check_past_its_wait_gives_up() {
        let slow = || std::thread::sleep(Duration::from_secs(1));
        assert_eq!(within(Duration::from_millis(20), slow), None);
        assert_eq!(within(Duration::from_secs(5), || 7), Some(7));
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
    fn an_osc7_counts_only_for_the_directory_the_shell_is_in() {
        let tmp = tempfile::tempdir().unwrap();
        let (here, other) = (tmp.path().join("here"), tmp.path().join("other"));
        std::fs::create_dir(&here).unwrap();
        std::fs::create_dir(&other).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&here, &link).unwrap();
        let at = |uri: String| osc7_path(&uri, "box").and_then(|p| osc7_dir(p, &here));
        let (h, o, l) = (here.display(), other.display(), link.display());
        assert_eq!(at(format!("file://{h}")), Some(here.clone()));
        assert_eq!(
            at(format!("file://box{l}")),
            Some(link.clone()),
            "a link keeps its name"
        );
        assert_eq!(
            at(format!("file://{o}")),
            None,
            "a directory the shell is not in, as an ssh host reports"
        );
        assert_eq!(at(format!("file://{h}/gone")), None, "a missing directory");
        assert_eq!(at(format!("file://elsewhere{h}")), None, "another machine");
    }
}
