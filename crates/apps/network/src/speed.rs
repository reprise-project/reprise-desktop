//! The open Status sheet's interface counters, read from sysfs once a second on a thread of
//! their own; dropping the [`Ticker`] stops it.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use reprisefs::netspeed::{Counters, read_iface};

pub struct Ticker {
    stop: Arc<AtomicBool>,
    pub rx: Receiver<(Instant, Counters)>,
}

pub fn start(iface: String, wake: impl Fn() + Send + 'static) -> Ticker {
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let stopped = stop.clone();
    let run = move || {
        while !stopped.load(Ordering::Relaxed) {
            if let Some(c) = read_iface(Path::new("/"), &iface) {
                if tx.send((Instant::now(), c)).is_err() {
                    break;
                }
                wake();
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("net-speed".into())
        .spawn(run)
    {
        log::warn!("network speed: no thread: {e}");
    }
    Ticker { stop, rx }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
