//! The session's log in the journal: unless stderr is a terminal or `REPRISE_LOG_STDERR=1`, the
//! compositor writes to a journald stream (`sd_journal_stream_fd`) named `reprise`, and each
//! program it spawns gets a stream named after its app.

use std::io::Write as _;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};

const SOCKET: &str = "/run/systemd/journal/stdout";

/// Set once [`init`] chose the journal; spawned programs then get streams of their own.
static ON: AtomicBool = AtomicBool::new(false);

/// Points stdout and stderr at the journal unless they log to a terminal or already to a journal
/// stream. True when log lines should carry `<N>` priority prefixes. Runs first in `main`, before
/// any thread starts.
pub fn init() -> bool {
    let stderr = std::io::stderr();
    if std::env::var_os("REPRISE_LOG_STDERR").is_some_and(|v| v == "1")
        // SAFETY: isatty only reads the descriptor's state.
        || unsafe { libc::isatty(stderr.as_raw_fd()) } == 1
    {
        return false;
    }
    if std::env::var("JOURNAL_STREAM").is_ok_and(|v| Some(v) == stream_id(stderr.as_fd())) {
        ON.store(true, Ordering::Relaxed);
        return true;
    }
    let Ok(fd) = open("reprise") else {
        return false;
    };
    // SAFETY: dup2 only replaces the standard descriptors; fd is open until it drops below.
    unsafe {
        libc::dup2(fd.as_raw_fd(), libc::STDOUT_FILENO);
        libc::dup2(fd.as_raw_fd(), libc::STDERR_FILENO);
    }
    if let Some(id) = stream_id(fd.as_fd()) {
        // SAFETY: called before any thread starts.
        unsafe { std::env::set_var("JOURNAL_STREAM", id) };
    }
    ON.store(true, Ordering::Relaxed);
    true
}

/// A stream for a spawned program's stdout and stderr, and its `JOURNAL_STREAM` value. None in a
/// terminal run or without a journal; the program then shares the compositor's.
pub fn stream_for(identifier: &str) -> Option<(OwnedFd, String)> {
    if !ON.load(Ordering::Relaxed) {
        return None;
    }
    let fd = open(identifier)
        .inspect_err(|e| log::warn!("journal stream for {identifier}: {e}"))
        .ok()?;
    let id = stream_id(fd.as_fd())?;
    Some((fd, id))
}

/// One log record as journal stream lines: each line prefixed with its syslog priority.
pub fn format(level: log::Level, target: &str, msg: &str) -> String {
    let p = priority(level);
    let mut out = String::new();
    for (i, line) in msg.split('\n').enumerate() {
        if i == 0 {
            out.push_str(&format!("<{p}>{target}: {line}\n"));
        } else {
            out.push_str(&format!("<{p}>{line}\n"));
        }
    }
    out
}

fn priority(level: log::Level) -> u8 {
    match level {
        log::Level::Error => 3,
        log::Level::Warn => 4,
        log::Level::Info => 6,
        log::Level::Debug | log::Level::Trace => 7,
    }
}

/// The stream protocol's header: identifier, unit id, default priority (info), level prefix on,
/// no forwarding to syslog, kmsg or console.
fn header(identifier: &str) -> String {
    let identifier: String = identifier.chars().filter(|c| *c != '\n').collect();
    format!("{identifier}\n\n6\n1\n0\n0\n0\n")
}

fn open(identifier: &str) -> std::io::Result<OwnedFd> {
    let s = std::os::unix::net::UnixStream::connect(SOCKET)?;
    s.shutdown(std::net::Shutdown::Read)?;
    (&s).write_all(header(identifier).as_bytes())?;
    Ok(s.into())
}

/// `<dev>:<ino>` of `fd`, the form `JOURNAL_STREAM` names a stream in.
fn stream_id(fd: BorrowedFd) -> Option<String> {
    // SAFETY: stat is plain data, valid zeroed.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fd is open for the borrow and fstat writes only into st.
    let ok = unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } == 0;
    ok.then(|| format!("{}:{}", st.st_dev, st.st_ino))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_names_the_identifier_with_info_default_and_level_prefixes_on() {
        assert_eq!(header("reprise-sound"), "reprise-sound\n\n6\n1\n0\n0\n0\n");
    }

    #[test]
    fn a_newline_in_the_identifier_cannot_shift_the_header() {
        assert_eq!(header("a\nb"), "ab\n\n6\n1\n0\n0\n0\n");
    }

    #[test]
    fn log_levels_map_to_syslog_priorities() {
        assert_eq!(format(log::Level::Error, "t", "m"), "<3>t: m\n");
        assert_eq!(format(log::Level::Warn, "t", "m"), "<4>t: m\n");
        assert_eq!(format(log::Level::Info, "t", "m"), "<6>t: m\n");
        assert_eq!(format(log::Level::Debug, "t", "m"), "<7>t: m\n");
        assert_eq!(format(log::Level::Trace, "t", "m"), "<7>t: m\n");
    }

    #[test]
    fn every_line_of_a_multiline_record_keeps_its_priority() {
        assert_eq!(
            format(log::Level::Warn, "reprise::drm", "commit failed\n  retry"),
            "<4>reprise::drm: commit failed\n<4>  retry\n"
        );
    }
}
