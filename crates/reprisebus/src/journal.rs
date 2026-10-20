//! The `log` logger for the compositor and every program: on a journal stream each line carries
//! its `<N>` syslog priority, elsewhere env_logger's plain format.

use std::io::Write as _;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};

/// The default filter of Reprise's programs: their own crates (every target starting with
/// `reprise`) at info, dependencies at warn.
pub const PROGRAM_FILTER: &str = "warn,reprise=info";

/// Starts the logger with `default_filter` unless `RUST_LOG` is set; prefixed when stderr is the
/// stream `JOURNAL_STREAM` names.
pub fn init_logger(default_filter: &str) {
    builder(default_filter, on_stream()).init();
}

/// The logger [`init_logger`] starts, with the prefixes chosen by the caller.
pub fn builder(default_filter: &str, prefixed: bool) -> env_logger::Builder {
    let mut b =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter));
    if prefixed {
        b.format(|buf, r| {
            buf.write_all(format(r.level(), r.target(), &r.args().to_string()).as_bytes())
        });
    }
    b
}

/// True when stderr is the journal stream `JOURNAL_STREAM` names.
pub fn on_stream() -> bool {
    names(
        std::env::var("JOURNAL_STREAM").ok().as_deref(),
        std::io::stderr().as_fd(),
    )
}

fn names(journal_stream: Option<&str>, fd: BorrowedFd) -> bool {
    journal_stream.is_some_and(|v| Some(v) == stream_id(fd).as_deref())
}

/// `<dev>:<ino>` of `fd`, the form `JOURNAL_STREAM` names a stream in.
pub fn stream_id(fd: BorrowedFd) -> Option<String> {
    // SAFETY: stat is plain data, valid zeroed.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fd is open for the borrow and fstat writes only into st.
    let ok = unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } == 0;
    ok.then(|| format!("{}:{}", st.st_dev, st.st_ino))
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

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log as _;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn logged(prefixed: bool, level: log::Level, msg: &str) -> String {
        let sink = Sink::default();
        let logger = builder("info", prefixed)
            .parse_filters("trace")
            .target(env_logger::Target::Pipe(Box::new(sink.clone())))
            .build();
        logger.log(
            &log::Record::builder()
                .level(level)
                .target("reprise_sound")
                .args(format_args!("{msg}"))
                .build(),
        );
        String::from_utf8(sink.0.lock().unwrap().clone()).unwrap()
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

    #[test]
    fn a_prefixed_logger_writes_priority_lines() {
        assert_eq!(
            logged(true, log::Level::Warn, "no sink\nusing default"),
            "<4>reprise_sound: no sink\n<4>using default\n"
        );
        assert_eq!(
            logged(true, log::Level::Info, "ready"),
            "<6>reprise_sound: ready\n"
        );
    }

    #[test]
    fn a_plain_logger_writes_env_loggers_format() {
        let out = logged(false, log::Level::Warn, "no sink");
        assert!(!out.starts_with('<'), "plain line has no prefix: {out:?}");
        assert!(
            out.contains("WARN") && out.ends_with("reprise_sound] no sink\n"),
            "{out:?}"
        );
    }

    #[test]
    fn programs_log_their_own_info_and_dependencies_warnings() {
        let logger = env_logger::Builder::new()
            .parse_filters(PROGRAM_FILTER)
            .build();
        let on = |level, target| {
            logger.enabled(&log::Metadata::builder().level(level).target(target).build())
        };
        for own in [
            "reprise_updates::fwupd",
            "repriseapp",
            "reprisekit::text",
            "reprisebus",
        ] {
            assert!(on(log::Level::Info, own), "{own} logs at info");
            assert!(!on(log::Level::Debug, own), "{own} does not log at debug");
        }
        for dep in ["zbus::connection", "tracing::span"] {
            assert!(on(log::Level::Warn, dep), "{dep} logs warnings");
            assert!(!on(log::Level::Info, dep), "{dep} does not log at info");
        }
    }

    #[test]
    fn only_the_stream_journal_stream_names_counts() {
        let file = tempfile::tempfile().unwrap();
        let id = stream_id(file.as_fd()).unwrap();
        assert!(names(Some(&id), file.as_fd()));
        assert!(!names(Some("1:2"), file.as_fd()));
        assert!(!names(None, file.as_fd()));
    }
}
