//! The log lines tests read back.

use std::sync::{Mutex, Once};

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        LINES.lock().unwrap().push(r.args().to_string());
    }
    fn flush(&self) {}
}

pub(crate) fn capture_log() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Info);
    });
}

pub(crate) fn times_logged(line: &str) -> usize {
    LINES.lock().unwrap().iter().filter(|l| *l == line).count()
}
