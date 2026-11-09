//! Where a tab's shell is and what runs in its foreground: `/proc`, and the OSC 7 the shell
//! reports its directory with, read off the PTY on the way to the parser.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

/// The directory an OSC 7 `uri` names when it is `shell_cwd` itself, by the name the shell gave
/// it; a host reached through ssh reports its own directories, which this machine may also have.
pub fn osc7_dir(uri: &str, local: &str, shell_cwd: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let path = osc7_path(uri, local)?;
    let (at, cwd) = (
        std::fs::metadata(&path).ok()?,
        std::fs::metadata(shell_cwd).ok()?,
    );
    (at.is_dir() && at.dev() == cwd.dev() && at.ino() == cwd.ino()).then_some(path)
}

/// The PTY's reader: passes its bytes through an [`escapes::Filter`] and keeps the last OSC 7
/// directory in `cwd`.
pub struct Reader {
    pub file: File,
    /// The tab's shell: an OSC 7 counts only while it holds the foreground.
    pub shell: u32,
    pub filter: escapes::Filter,
    /// What the filter passed on, before it goes into the caller's buffer.
    pub out: Vec<u8>,
    pub host: String,
    pub cwd: Arc<Mutex<Option<PathBuf>>>,
}

impl Read for Reader {
    /// Reads only as much as leaves room for what the filter holds, so a held escape it lets
    /// go always fits in `buf`.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let room = buf.len().saturating_sub(self.filter.held());
        if room == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = self.file.read(&mut buf[..room])?;
        self.out.clear();
        let uri = self.filter.feed(&buf[..n], &mut self.out);
        buf[..self.out.len()].copy_from_slice(&self.out);
        if let Some(uri) = uri
            && fg_pgrp(self.shell) == Some(self.shell)
            && let Some(path) = osc7_dir(
                &uri,
                &self.host,
                Path::new(&format!("/proc/{}/cwd", self.shell)),
            )
        {
            *self.cwd.lock().unwrap() = Some(path);
        }
        Ok(self.out.len())
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
    fn a_held_escape_comes_back_within_the_buffer() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let (mut tx, rx) = UnixStream::pair().unwrap();
        let mut r = Reader {
            file: File::from(std::os::fd::OwnedFd::from(rx)),
            shell: 0,
            filter: Default::default(),
            out: Vec::new(),
            host: String::new(),
            cwd: Default::default(),
        };
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
        let at = |uri: String| osc7_dir(&uri, "box", &here);
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
