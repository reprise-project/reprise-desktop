//! The thread that samples: a [`Snapshot`] every interval while one is set, posted to the event
//! loop, and the signals End Task and End Process send.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use crate::sample::{Sampler, Snapshot};

pub enum Cmd {
    /// Sample every this long, or (None) not at all.
    Every(Option<Duration>),
    /// Sample at once.
    Now,
    /// Read each process's storage I/O, for the Disk column.
    Disk(bool),
    /// Read each process's GPU clients, for the GPU column.
    Gpu(bool),
}

pub struct Handle {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Snapshot>,
}

/// Starts the sampler, paused until the first [`Cmd::Every`]. `wake` runs after each snapshot.
pub fn spawn(mut sampler: Sampler, wake: impl Fn() + Send + 'static) -> Handle {
    let (tx, cmds) = channel::<Cmd>();
    let (out, rx) = channel();
    std::thread::spawn(move || {
        let mut every: Option<Duration> = None;
        loop {
            let got = match every {
                Some(d) => cmds.recv_timeout(d),
                None => cmds.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match got {
                Ok(Cmd::Every(d)) => {
                    let resumed = every.is_none() && d.is_some();
                    every = d;
                    if !resumed {
                        continue;
                    }
                }
                Ok(Cmd::Disk(on)) => {
                    sampler.disk = on;
                    continue;
                }
                Ok(Cmd::Gpu(on)) => {
                    if let Some(d) = sampler.devices.as_mut() {
                        d.proc_gpu = on;
                    }
                    continue;
                }
                Ok(Cmd::Now) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if out.send(sampler.sample()).is_err() {
                return;
            }
            wake();
        }
    });
    Handle { tx, rx }
}

/// Sends `sig` to every process of scope `unit` through the user manager, off the event loop.
pub fn kill_scope(unit: String, sig: i32) {
    std::thread::spawn(move || {
        let call = reprisebus::session().and_then(|conn| {
            reprisebus::wait(conn.call_method(
                Some("org.freedesktop.systemd1"),
                "/org/freedesktop/systemd1",
                Some("org.freedesktop.systemd1.Manager"),
                "KillUnit",
                &(&unit, "all", sig),
            ))
        });
        if let Err(e) = call {
            log::warn!("signal {sig} to {unit}: {e}");
        }
    });
}

/// Sends `sig` to process `pid`.
pub fn kill_pid(pid: i32, sig: i32) -> std::io::Result<()> {
    // SAFETY: kill takes plain integers and touches no memory.
    if unsafe { libc::kill(pid, sig) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn the_sampler_waits_for_an_interval_and_samples_on_ask() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("stat"), "cpu 1 0 0 1 0 0 0 0\n").unwrap();
        let s = Sampler::new(root.path(), None, HashMap::new());
        let h = spawn(s, || {});
        assert!(
            h.rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "paused at start"
        );
        h.tx.send(Cmd::Every(Some(Duration::from_millis(20))))
            .unwrap();
        assert!(
            h.rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "resuming samples at once"
        );
        assert!(
            h.rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "and again each interval"
        );
        h.tx.send(Cmd::Every(None)).unwrap();
        while h.rx.recv_timeout(Duration::from_millis(100)).is_ok() {}
        h.tx.send(Cmd::Now).unwrap();
        assert!(
            h.rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "Refresh Now while paused"
        );
    }
}
