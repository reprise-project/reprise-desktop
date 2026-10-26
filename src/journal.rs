//! The session's log in the journal: unless stderr is a terminal or `REPRISE_LOG_STDERR=1`, the
//! compositor writes to a journald stream (`sd_journal_stream_fd`) named `reprise`, and each
//! program it spawns gets a stream named after its app.

use reprisebus::journal::stream_id;
use std::io::Write as _;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
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
    if reprisebus::journal::on_stream() {
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

/// Logs every panic with a backtrace at error level, then runs the hook that was set before. Runs
/// once the logger is up.
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!("panic: {info}\n{backtrace}");
        previous(info);
    }));
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
    fn a_panic_logs_its_message_and_a_backtrace_through_the_logger() {
        static LINES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        struct Capture;
        impl log::Log for Capture {
            fn enabled(&self, m: &log::Metadata) -> bool {
                m.level() == log::Level::Error
            }
            fn log(&self, r: &log::Record) {
                if self.enabled(r.metadata()) {
                    LINES.lock().unwrap().push(r.args().to_string());
                }
            }
            fn flush(&self) {}
        }
        log::set_logger(&Capture).expect("no other test sets a logger");
        log::set_max_level(log::LevelFilter::Error);
        let before = std::panic::take_hook();
        log_panics();
        let caught = std::panic::catch_unwind(|| panic!("panic hook test"));
        std::panic::set_hook(before);
        assert!(caught.is_err());
        let lines = LINES.lock().unwrap();
        let line = lines
            .iter()
            .find(|l| l.contains("panic hook test"))
            .expect("the panic was logged");
        assert!(
            line.starts_with("panic: "),
            "record starts with \"panic: \": {line}"
        );
        assert!(
            line.contains("a_panic_logs_its_message_and_a_backtrace_through_the_logger"),
            "backtrace names the test function: {line}"
        );
    }
}
