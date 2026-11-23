//! The thread that samples: a [`Snapshot`] every interval while one is set, posted to the event
//! loop, and what End Task and End Process send: window closes and signals.

use std::os::fd::{AsRawFd, OwnedFd};
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
        if let Err(e) = signal_scope(&unit, sig) {
            log::warn!("signal {sig} to {unit}: {e}");
        }
    });
}

fn signal_scope(unit: &str, sig: i32) -> Result<(), String> {
    let conn = reprisebus::session().map_err(|e| e.to_string())?;
    reprisebus::wait(conn.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "KillUnit",
        &(unit, "all", sig),
    ))
    .map(drop)
    .map_err(|e| e.to_string())
}

/// End Task on a program that answers: [`end_gently`] scope `unit` whose processes are `pids`,
/// off the event loop.
pub fn end_task(unit: String, pids: Vec<i32>) {
    std::thread::spawn(move || {
        let close = || {
            reprisebus::tray::close_windows(&pids).unwrap_or_else(|e| {
                log::warn!("End Task {unit}: windows not closed: {e}");
                0
            })
        };
        let term = || {
            if let Err(e) = signal_scope(&unit, libc::SIGTERM) {
                log::debug!("End Task {unit}: {e}");
            }
        };
        end_gently(close, term);
    });
}

/// Asks a program's windows to close (`close` returns how many it asked) and runs `term` only
/// when it showed none, so a tray program whose prompt closed keeps running.
pub fn end_gently(close: impl FnOnce() -> u32, term: impl FnOnce()) {
    if close() == 0 {
        term();
    }
}

/// Asks `tx`'s sampler for a snapshot once `delay` is up, whatever its interval.
pub fn sample_after(tx: Sender<Cmd>, delay: Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let _ = tx.send(Cmd::Now);
    });
}

/// A pidfd for `pid` while `/proc` still shows it started at `start`; a pid reused since is refused.
pub fn open_process(pid: i32, start: u64) -> std::io::Result<OwnedFd> {
    let gone = || std::io::Error::from_raw_os_error(libc::ESRCH);
    let fd = reprisebus::pidfd::open(u32::try_from(pid).map_err(|_| gone())?)?;
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    match crate::procfs::parse_stat(&stat) {
        Some(s) if s.start == start => Ok(fd),
        _ => Err(gone()),
    }
}

/// Sends `sig` to the process `fd` names; ESRCH once it has ended.
pub fn signal(fd: &OwnedFd, sig: i32) -> std::io::Result<()> {
    // SAFETY: pidfd_send_signal takes an open fd, a signal, a null siginfo and no flags.
    let r = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            sig,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
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

    /// Runs [`end_gently`] with `closed` windows asked; whether it signalled.
    fn gently(closed: u32) -> bool {
        let termed = std::cell::Cell::new(false);
        end_gently(|| closed, || termed.set(true));
        termed.get()
    }

    #[test]
    fn a_process_is_signalled_only_as_the_one_the_box_was_opened_for() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let pid = child.id() as i32;
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let start = crate::procfs::parse_stat(&stat).unwrap().start;
        assert!(
            open_process(pid, start + 1).is_err(),
            "another start is another process"
        );
        let fd = open_process(pid, start).unwrap();
        signal(&fd, libc::SIGTERM).unwrap();
        let status = child.wait().unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(libc::SIGTERM)
        );
        let gone = signal(&fd, libc::SIGTERM).unwrap_err();
        assert_eq!(
            gone.raw_os_error(),
            Some(libc::ESRCH),
            "a reaped process takes no signal"
        );
    }

    #[test]
    fn a_wait_asks_for_a_sample_once_it_is_up() {
        let (tx, rx) = channel();
        sample_after(tx, Duration::ZERO);
        let got = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(got, Cmd::Now));
    }

    #[test]
    fn end_task_signals_a_program_only_when_it_shows_no_window() {
        assert!(gently(0), "no window: signalled at once");
        assert!(
            !gently(2),
            "windows asked to close: the program is left running, as a tray program stays"
        );
    }

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
