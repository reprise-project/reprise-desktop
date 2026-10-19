//! How far a put or a removal has got: totals from a walk of its sources, bytes and items done,
//! pause and cancel. The job's thread writes a [`Meter`]; the host reads a [`Snapshot`].

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Walking the sources for the totals.
    #[default]
    Preparing,
    Copying,
    /// The last byte is written; the destination's drive is being flushed.
    Finishing,
}

/// The counts a [`Meter`] held at one moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    pub paused: bool,
    pub bytes: u64,
    pub total_bytes: u64,
    pub items: u64,
    pub total_items: u64,
    /// The name of the file being written.
    pub current: String,
}

#[derive(Debug, Default)]
pub struct Meter {
    pub(crate) cancel: AtomicBool,
    paused: AtomicBool,
    phase: AtomicU8,
    bytes: AtomicU64,
    total_bytes: AtomicU64,
    items: AtomicU64,
    total_items: AtomicU64,
    current: Mutex<String>,
    /// Bytes of small files written since their drive's last flush started ([`Meter::wrote_file`]).
    unflushed: AtomicU64,
    flushing: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Bumped by every step the job takes; a job whose count stands still is stuck in a call.
    stir: AtomicU64,
    /// Waits on a drive's flush under way: a slow disk, not a stuck job.
    flush_waits: AtomicU32,
    /// The index of the item the job is on.
    at: AtomicUsize,
}

/// A file this large is flushed in windows as it is written ([`Writeback`]).
pub(crate) const WINDOW: u64 = 4 << 20;
/// Smaller files start a flush of their drive once this much of them is written...
const KICK_EVERY: u64 = 2 << 20;
/// ...and wait for it once this much is.
const FLUSH_EVERY: u64 = 8 << 20;
const PAUSE_POLL: Duration = Duration::from_millis(50);

impl Meter {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: match self.phase.load(Ordering::Relaxed) {
                0 => Phase::Preparing,
                1 => Phase::Copying,
                _ => Phase::Finishing,
            },
            paused: self.paused.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            items: self.items.load(Ordering::Relaxed),
            total_items: self.total_items.load(Ordering::Relaxed),
            current: self.current.lock().map(|s| s.clone()).unwrap_or_default(),
        }
    }

    pub fn set_paused(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Stop the job at its next file.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Count one more file or link removed.
    pub(crate) fn removed(&self) {
        self.items.fetch_add(1, Ordering::Relaxed);
    }

    fn stir(&self) {
        self.stir.fetch_add(1, Ordering::Relaxed);
    }

    /// The step count and whether the job is paused or waiting on a flush, for the watch on a put
    /// that stops answering.
    pub(crate) fn stirring(&self) -> (u64, bool) {
        let resting =
            self.paused.load(Ordering::Relaxed) || self.flush_waits.load(Ordering::Relaxed) > 0;
        (self.stir.load(Ordering::Relaxed), resting)
    }

    /// Run `f`, a wait on a drive's flush, outside the watch: a slow disk takes as long as it takes.
    pub(crate) fn flush_wait<T>(&self, f: impl FnOnce() -> T) -> T {
        self.flush_waits.fetch_add(1, Ordering::Relaxed);
        let out = f();
        self.flush_waits.fetch_sub(1, Ordering::Relaxed);
        self.stir();
        out
    }

    pub(crate) fn set_at(&self, index: usize) {
        self.at.store(index, Ordering::Relaxed);
        self.stir();
    }

    pub(crate) fn at(&self) -> usize {
        self.at.load(Ordering::Relaxed)
    }

    pub(crate) fn phase(&self) -> Phase {
        self.snapshot().phase
    }

    pub(crate) fn set_phase(&self, p: Phase) {
        self.stir();
        let v = match p {
            Phase::Preparing => 0,
            Phase::Copying => 1,
            Phase::Finishing => 2,
        };
        self.phase.store(v, Ordering::Relaxed);
    }

    pub(crate) fn add_totals(&self, t: Totals) {
        self.stir();
        self.total_bytes.fetch_add(t.bytes, Ordering::Relaxed);
        self.total_items.fetch_add(t.items, Ordering::Relaxed);
    }

    /// The counts once a put's item `done` has landed, whatever its copy counted on the way:
    /// the walk's totals stay the one measure of the bar.
    pub(crate) fn settle(&self, done: Totals) {
        self.stir();
        self.bytes.store(done.bytes, Ordering::Relaxed);
        self.items.store(done.items, Ordering::Relaxed);
    }

    pub(crate) fn set_current(&self, name: &OsStr) {
        self.stir();
        if let Ok(mut s) = self.current.lock() {
            *s = name.to_string_lossy().into_owned();
        }
    }

    /// Count `n` more bytes written, then hold while paused. Fails with `Interrupted` once
    /// cancelled.
    pub(crate) fn wrote(&self, n: u64) -> std::io::Result<()> {
        self.bytes.fetch_add(n, Ordering::Relaxed);
        self.hold()
    }

    /// Wait out a pause; `Interrupted` once cancelled.
    pub(crate) fn hold(&self) -> std::io::Result<()> {
        while self.paused.load(Ordering::Relaxed) && !self.cancelled() {
            std::thread::sleep(PAUSE_POLL);
        }
        self.stir();
        if self.cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "The operation was cancelled.",
            ));
        }
        Ok(())
    }

    /// `file` is written whole, `len` bytes; `windowed` when [`Writeback`] flushed it as it went.
    /// Other files start a background flush every [`KICK_EVERY`] and wait for it past [`FLUSH_EVERY`].
    pub(crate) fn wrote_file(&self, file: &std::fs::File, len: u64, windowed: bool) {
        self.items.fetch_add(1, Ordering::Relaxed);
        self.stir();
        if windowed {
            return;
        }
        let piled = self.unflushed.fetch_add(len, Ordering::Relaxed) + len;
        if piled < KICK_EVERY {
            return;
        }
        let Ok(mut flushing) = self.flushing.lock() else {
            return;
        };
        if flushing.as_ref().is_some_and(|h| !h.is_finished()) {
            if piled < FLUSH_EVERY {
                return;
            }
            if let Some(h) = flushing.take() {
                let _ = self.flush_wait(|| h.join());
            }
        }
        self.unflushed.store(0, Ordering::Relaxed);
        if let Ok(f) = file.try_clone() {
            *flushing = Some(std::thread::spawn(move || sync_fs(&f)));
        }
    }
}

/// What a put's sources add up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub bytes: u64,
    pub items: u64,
}

impl std::ops::Add for Totals {
    type Output = Totals;
    fn add(self, o: Totals) -> Totals {
        Totals {
            bytes: self.bytes + o.bytes,
            items: self.items + o.items,
        }
    }
}

/// The bytes and files under `src` a copy writes: a link is one item of no bytes, a top-level
/// link is followed (a copy follows it), and what is neither file, folder nor link is skipped as
/// the copy skips it. `None` once `m` is cancelled.
pub(crate) fn walk(src: &Path, follow: bool, m: &Meter) -> Option<Totals> {
    let meta = if follow {
        std::fs::metadata(src)
    } else {
        std::fs::symlink_metadata(src)
    };
    let Ok(meta) = meta else {
        return Some(Totals::default());
    };
    if meta.is_file() {
        return Some(Totals {
            bytes: meta.len(),
            items: 1,
        });
    }
    if !meta.is_dir() {
        let link = meta.file_type().is_symlink();
        return Some(Totals {
            bytes: 0,
            items: u64::from(link),
        });
    }
    let mut sum = Totals::default();
    for entry in std::fs::read_dir(src).into_iter().flatten().flatten() {
        if m.cancelled() {
            return None;
        }
        m.stir();
        sum = sum + walk(&entry.path(), false, m)?;
    }
    Some(sum)
}

// The copy routines under a put (a folder copy, a move to another drive through
// `trash::move_to_free`) find the meter here rather than through every caller's arguments.
thread_local! {
    static METER: RefCell<Option<Arc<Meter>>> = const { RefCell::new(None) };
}

/// Run `f` with `meter` counting what the copy routines write on this thread.
pub(crate) fn metered<T>(meter: &Arc<Meter>, f: impl FnOnce() -> T) -> T {
    METER.with(|m| *m.borrow_mut() = Some(meter.clone()));
    let out = f();
    METER.with(|m| *m.borrow_mut() = None);
    out
}

/// The meter this thread's put counts into, if it runs under one.
pub(crate) fn current() -> Option<Arc<Meter>> {
    METER.with(|m| m.borrow().clone())
}

/// Keeps a large file's dirty pages to about two windows: each full window is sent to the disk
/// and the one before it waited for.
pub(crate) struct Writeback {
    sent: u64,
}

impl Writeback {
    pub(crate) fn new() -> Writeback {
        Writeback { sent: 0 }
    }

    /// `file` holds `len` bytes so far.
    pub(crate) fn advance(&mut self, file: &std::fs::File, len: u64, meter: &Meter) {
        use std::os::fd::AsRawFd;
        while len >= self.sent + WINDOW {
            let fd = file.as_raw_fd();
            // SAFETY: `fd` is open for the duration of the calls; sync_file_range reads no memory.
            unsafe {
                libc::sync_file_range(
                    fd,
                    self.sent as i64,
                    WINDOW as i64,
                    libc::SYNC_FILE_RANGE_WRITE,
                );
                if self.sent >= WINDOW {
                    meter.flush_wait(|| {
                        libc::sync_file_range(
                            fd,
                            (self.sent - WINDOW) as i64,
                            WINDOW as i64,
                            libc::SYNC_FILE_RANGE_WAIT_BEFORE
                                | libc::SYNC_FILE_RANGE_WRITE
                                | libc::SYNC_FILE_RANGE_WAIT_AFTER,
                        )
                    });
                }
            }
            self.sent += WINDOW;
        }
    }

    /// Whether any window was sent: the file was large enough to be flushed as it went.
    pub(crate) fn used(&self) -> bool {
        self.sent > 0
    }
}

/// Flush the file system holding `file` (`syncfs`), logging a failure: the data is written
/// either way, and the bar only waits for it.
pub(crate) fn sync_fs(file: &std::fs::File) {
    use std::os::fd::AsRawFd;
    // SAFETY: the fd is open for the call.
    if unsafe { libc::syncfs(file.as_raw_fd()) } != 0 {
        log::warn!(
            "reprisefs: flushing a put's drive: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walk_counts_files_and_links_and_their_bytes() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("d");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a"), vec![0; 300]).unwrap();
        std::fs::write(d.join("sub/b"), vec![0; 700]).unwrap();
        std::os::unix::fs::symlink("a", d.join("link")).unwrap();
        let got = walk(&d, true, &Meter::default()).unwrap();
        assert_eq!(
            got,
            Totals {
                bytes: 1000,
                items: 3
            }
        );
        let cancelled = Meter::default();
        cancelled.cancel();
        assert_eq!(walk(&d, true, &cancelled), None);
    }

    #[test]
    fn a_paused_meter_holds_until_resumed_or_cancelled() {
        let m = Arc::new(Meter::default());
        m.set_paused(true);
        let held = m.clone();
        let t = std::thread::spawn(move || held.wrote(10));
        std::thread::sleep(Duration::from_millis(120));
        assert!(!t.is_finished(), "a paused write waits");
        m.set_paused(false);
        assert!(t.join().unwrap().is_ok());
        assert_eq!(m.snapshot().bytes, 10);

        m.set_paused(true);
        let held = m.clone();
        let t = std::thread::spawn(move || held.wrote(1));
        m.cancel.store(true, Ordering::Relaxed);
        let e = t.join().unwrap().unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::Interrupted);
    }
}
