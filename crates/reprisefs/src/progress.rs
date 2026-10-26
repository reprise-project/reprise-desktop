//! How far a put or a removal has got: totals from a walk of its sources, bytes and items done,
//! pause and cancel. The job's thread writes a [`Meter`]; the host reads a [`Snapshot`].

use std::cell::RefCell;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Walking the sources for the totals.
    #[default]
    Preparing,
    Copying,
    /// The last byte is written; the destination's drive is being flushed.
    Finishing,
}

/// What the host picked for an item that failed in a job that asks ([`Meter::ask_on_failure`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Decision {
    /// Not answered yet; the job holds as in a pause.
    Pending = 1,
    /// Go on with the next item.
    Skip,
    /// Run the same item again.
    Retry,
    /// End the job here.
    Abort,
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
    /// Trees [`walk`] found holding no other device, which
    /// [`crate::ops::refuse_holding_mount`] need not walk again.
    clear: Mutex<Vec<PathBuf>>,
    /// A failed item waits for a [`Decision`] instead of ending the job.
    asks: AtomicBool,
    /// The [`Decision`] on the failed item, 0 when none is asked for.
    decision: AtomicU8,
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

    /// Have a failed item wait for the host's [`Decision`] ([`Meter::decide`]) instead of
    /// ending the job.
    pub fn ask_on_failure(&self) {
        self.asks.store(true, Ordering::Relaxed);
    }

    /// Answer the failed item the job holds on.
    pub fn decide(&self, d: Decision) {
        self.decision.store(d as u8, Ordering::Relaxed);
    }

    /// Mark the item that just failed as waiting for a [`Decision`], before the host hears of it.
    pub(crate) fn ask(&self) {
        if self.asks.load(Ordering::Relaxed) {
            self.decide(Decision::Pending);
        }
    }

    /// The host's [`Decision`] on the item [`Meter::ask`] marked, held for like a pause:
    /// [`Decision::Abort`] when the job does not ask or is cancelled.
    pub(crate) fn decision(&self) -> Decision {
        if !self.asks.load(Ordering::Relaxed) {
            return Decision::Abort;
        }
        let d = loop {
            if self.cancelled() {
                break Decision::Abort;
            }
            match self.decision.load(Ordering::Relaxed) {
                2 => break Decision::Skip,
                3 => break Decision::Retry,
                4 => break Decision::Abort,
                _ => std::thread::sleep(PAUSE_POLL),
            }
        };
        self.decision.store(0, Ordering::Relaxed);
        self.stir();
        d
    }

    /// The step count and whether the job is paused or waiting on a flush, for [`watch`].
    pub(crate) fn stirring(&self) -> (u64, Rest) {
        let asked = self.decision.load(Ordering::Relaxed) == Decision::Pending as u8;
        let rest = if self.paused.load(Ordering::Relaxed) || asked {
            Rest::Paused
        } else if self.flush_waits.load(Ordering::Relaxed) > 0 {
            Rest::Flushing
        } else {
            Rest::No
        };
        (self.stir.load(Ordering::Relaxed), rest)
    }

    /// Run `f`, a wait on a drive's flush, under [`watch`]'s longer flush bound.
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

    /// Whether [`walk`] went through `root` and met no folder on another device.
    pub(crate) fn walked_clear(&self, root: &Path) -> bool {
        self.clear.lock().is_ok_and(|c| c.iter().any(|p| p == root))
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

/// Why a job's step count may stand still without the job being stuck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rest {
    No,
    /// Held by the user, or waiting for a [`Decision`]: never a give-up.
    Paused,
    /// In a wait on a drive's flush: given [`FLUSH_ALLOWANCE`] deadlines.
    Flushing,
}

/// A wait on one flush gives up after this many deadlines (5 min at
/// [`crate::deadline::ACTION_DEADLINE`]). A job keeps only a few MB unflushed, but a drive's flush
/// also writes what other programs left dirty: a 10 MB/s stick flushes 3 GB in that time. A soft
/// NFS mount fails its call within it; a hard one, or a pulled stick, never answers.
const FLUSH_ALLOWANCE: u32 = 20;

/// Cancels the job and runs `give_up` (which reports it as not responding) once `meter` stands
/// still for `deadline` outside a pause, or one flush lasts [`FLUSH_ALLOWANCE`] deadlines. `ended` is set by whichever ends the job first,
/// the job or the watch, so only one of them reports its end.
pub(crate) fn watch(
    meter: &Arc<Meter>,
    ended: &Arc<AtomicBool>,
    deadline: Duration,
    job: &'static str,
    give_up: impl FnOnce(&Meter) + Send + 'static,
) {
    let (m, ended) = (meter.clone(), ended.clone());
    let every = (deadline / 4).min(Duration::from_secs(1));
    let flush_deadline = deadline * FLUSH_ALLOWANCE;
    let spawned = std::thread::Builder::new()
        .name(format!("reprisefs-{job}-watch"))
        .spawn(move || {
            let mut headway = Headway::new(m.stirring().0, Instant::now());
            while !ended.load(Ordering::Acquire) {
                std::thread::sleep(every);
                if !headway.stalled(m.stirring(), Instant::now(), deadline, flush_deadline)
                    || ended.swap(true, Ordering::AcqRel)
                {
                    continue;
                }
                m.cancel();
                log::warn!("reprisefs: a {job} stood still past its deadline; giving up");
                give_up(&m);
                return;
            }
        });
    if let Err(e) = spawned {
        log::warn!("reprisefs: no watch on a {job}: {e}");
    }
}

/// [`watch`]'s memory of the last step it saw and when, and when the flush it is in began.
struct Headway {
    stir: u64,
    at: Instant,
    flush_from: Option<Instant>,
}

impl Headway {
    fn new(stir: u64, at: Instant) -> Self {
        Headway {
            stir,
            at,
            flush_from: None,
        }
    }

    /// Whether `stirring` ([`Meter::stirring`]) read at `now` has stood still for `deadline`, or
    /// one flush has lasted `flush_deadline`.
    fn stalled(
        &mut self,
        (stir, rest): (u64, Rest),
        now: Instant,
        deadline: Duration,
        flush_deadline: Duration,
    ) -> bool {
        if rest == Rest::Paused || stir != self.stir {
            *self = Headway::new(stir, now);
        }
        match rest {
            Rest::Paused => false,
            Rest::Flushing => {
                self.at = now;
                now.duration_since(*self.flush_from.get_or_insert(now)) >= flush_deadline
            }
            Rest::No => {
                self.flush_from = None;
                now.duration_since(self.at) >= deadline
            }
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
/// the copy skips it. `None` once `m` is cancelled. A tree with no folder on another device
/// than its parent's is noted on `m` ([`Meter::walked_clear`]).
pub(crate) fn walk(src: &Path, follow: bool, m: &Meter) -> Option<Totals> {
    let mut mount = false;
    let t = walk_in(src, follow, None, m, &mut mount)?;
    if !mount && let Ok(mut c) = m.clear.lock() {
        c.push(src.to_path_buf());
    }
    Some(t)
}

/// [`walk`] of `src`, inside a folder on `parent_dev`; sets `mount` on meeting a mount root.
fn walk_in(
    src: &Path,
    follow: bool,
    parent_dev: Option<u64>,
    m: &Meter,
    mount: &mut bool,
) -> Option<Totals> {
    use std::os::unix::fs::MetadataExt;
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
    *mount |= parent_dev.is_some_and(|d| crate::ops::mount_root(meta.dev(), d));
    let mut sum = Totals::default();
    for entry in std::fs::read_dir(src).into_iter().flatten().flatten() {
        if m.cancelled() {
            return None;
        }
        m.stir();
        sum = sum + walk_in(&entry.path(), false, Some(meta.dev()), m, mount)?;
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

/// One step of an uncounted walk or remove under this thread's job, so [`watch`] sees it moving.
pub(crate) fn step() {
    METER.with(|m| {
        if let Some(m) = m.borrow().as_ref() {
            m.stir();
        }
    });
}

/// Run `f`, a flush, outside this thread's job's [`watch`] ([`Meter::flush_wait`]).
pub(crate) fn flushing<T>(f: impl FnOnce() -> T) -> T {
    match current() {
        Some(m) => m.flush_wait(f),
        None => f(),
    }
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

    /// Polls a [`Headway`] every 25 ms of a made-up clock for 400 ms while `stirring` gives the
    /// meter at each poll; true when it stalled at a 100 ms deadline and a `flush_ms` flush bound.
    fn stalls_by(flush_ms: u64, mut stirring: impl FnMut(u32) -> (u64, Rest)) -> bool {
        let start = Instant::now();
        let mut h = Headway::new(0, start);
        (1..=16).any(|n| {
            let now = start + Duration::from_millis(25) * n;
            let (deadline, flush) = (Duration::from_millis(100), Duration::from_millis(flush_ms));
            h.stalled(stirring(n), now, deadline, flush)
        })
    }

    fn stalls(stirring: impl FnMut(u32) -> (u64, Rest)) -> bool {
        stalls_by(1000, stirring)
    }

    #[test]
    fn a_slow_step_that_keeps_stepping_or_flushes_outlives_the_deadline() {
        assert!(
            !stalls(|n| (u64::from(n * 25 / 40), Rest::No)),
            "400 ms of steps 40 ms apart"
        );
        assert!(!stalls(|_| (0, Rest::Flushing)), "a 400 ms flush");
        assert!(!stalls(|_| (0, Rest::Paused)), "a 400 ms pause");
        assert!(stalls(|_| (0, Rest::No)), "400 ms without a step gives up");
    }

    #[test]
    fn a_flush_past_its_own_bound_gives_up_and_a_pause_never_does() {
        assert!(
            stalls_by(300, |_| (0, Rest::Flushing)),
            "a 400 ms flush with a 300 ms bound"
        );
        assert!(
            !stalls_by(300, |n| (u64::from(n / 8), Rest::Flushing)),
            "two 200 ms flushes, each under the bound"
        );
        assert!(
            !stalls_by(300, |n| (
                0,
                if n % 4 == 0 {
                    Rest::Paused
                } else {
                    Rest::Flushing
                }
            )),
            "a flush paused every 100 ms starts its bound again"
        );
        assert!(
            stalls_by(300, |n| (
                0,
                if n <= 10 { Rest::Flushing } else { Rest::No }
            )),
            "a flush that ends without a step still owes the step deadline"
        );
    }

    #[test]
    fn a_step_stirs_and_a_flush_rests_the_meter_it_runs_under() {
        let m = Arc::new(Meter::default());
        metered(&m, || {
            step();
            assert_eq!(m.stirring(), (1, Rest::No));
            flushing(|| assert_eq!(m.stirring(), (1, Rest::Flushing)));
            m.set_paused(true);
            flushing(|| assert_eq!(m.stirring(), (2, Rest::Paused)));
            m.set_paused(false);
        });
        assert_eq!(m.stirring(), (3, Rest::No), "a flush's end is a step");
    }

    #[test]
    fn a_watch_gives_up_on_a_meter_that_stands_still() {
        let m = Arc::new(Meter::default());
        let ended = Arc::new(AtomicBool::new(false));
        let (gave_up, given) = std::sync::mpsc::channel();
        watch(&m, &ended, Duration::from_millis(20), "test", move |_| {
            gave_up.send(()).expect("the test waits")
        });
        assert_eq!(given.recv_timeout(Duration::from_secs(30)), Ok(()));
        assert!(m.cancelled());
        assert!(ended.load(Ordering::Acquire), "the watch ended the job");
    }

    #[test]
    fn a_measure_and_an_uncounted_erase_step_per_entry() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("d");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        for i in 0..40 {
            std::fs::write(
                d.join(if i % 2 == 0 { "sub" } else { "." })
                    .join(format!("f{i}")),
                "x",
            )
            .unwrap();
        }
        let m = Arc::new(Meter::default());
        let before = m.stirring().0;
        metered(&m, || crate::bincap::item_size(&d, u64::MAX)).unwrap();
        let measured = m.stirring().0;
        assert!(measured - before >= 40, "one step per entry measured");
        metered(&m, || crate::ops::erase_stepped(&d)).unwrap();
        assert!(m.stirring().0 - measured >= 40, "one step per entry erased");
        assert!(!d.exists());
        assert_eq!(m.snapshot().items, 0, "nothing counts on the bar");
    }
}
