//! Per-output commit threads. Main renders and assigns planes; each driven CRTC's thread runs
//! the KMS commits (frames, power-off, gamma, link retrain) and finally drops the output's
//! compositor, so a commit stuck in the kernel stalls that output alone. `Track` is main's view
//! of one output; at most one job is outstanding per output.

use std::{
    ops::{Deref, DerefMut},
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use smithay::reexports::calloop::channel::Sender;

/// What a job asks of the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Queue the rendered frame; `modeset` when it carries a mode or connector change and so
    /// commits blocking.
    Flip {
        modeset: bool,
    },
    /// Power the CRTC off.
    Clear,
    Gamma,
    /// Set a BAD link-status back to GOOD.
    Retrain,
    /// Turn the device off after a session return (`reset::plan`).
    Reset,
}

impl Kind {
    /// Commits that wait in the kernel holding its modeset locks (on some drivers every
    /// CRTC's) until they return.
    pub fn blocking(self) -> bool {
        self != Kind::Flip { modeset: false }
    }
}

/// How long a job may sit in the kernel, or a flip wait for its vblank, before the output is
/// stalled (`Track::stall`).
pub const STALL: Duration = Duration::from_secs(2);
/// After this long without its vblank a flip's commit has left the kernel's commit list (its
/// 10 s flip_done wait, plus slack).
pub const FLIP_LOST: Duration = Duration::from_secs(11);
/// The longest wait between modeset tries (`Track::retry_at`).
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Whether main may issue ioctls that take every modeset lock (any output's plane test, property
/// reads, probes): no blocking job, stall, flip lost 2-11 s ago or `retiring` thread.
pub fn quiet<'a, M: 'a>(
    all: impl IntoIterator<Item = &'a Track<M>>,
    retiring: bool,
    now: Instant,
) -> bool {
    !retiring && all.into_iter().all(|t| t.quiet(now))
}

/// Whether `own`'s next frame is held past `STALL`: its own job or flip stalled, or another
/// output's stall keeps the device from going quiet, and no output renders until it is.
pub fn held<M>(own: &Track<M>, all: &[&Track<M>], retiring: bool, now: Instant) -> bool {
    own.stalled || (all.iter().any(|t| t.stalled) && !quiet(all.iter().copied(), retiring, now))
}

pub enum Job<P> {
    Run(Kind, P),
    /// Drop the committer (and the compositor it holds) and end the thread.
    Exit,
}

/// A finished job, as the thread reports it.
#[derive(Debug)]
pub struct Done<O> {
    pub kind: Kind,
    pub result: Result<O, String>,
    /// Around the commit itself, after any `REPRISE_COMMIT_DELAY` sleep.
    pub started: Instant,
    pub ended: Instant,
}

#[derive(Debug)]
pub enum Reply<O> {
    Done(Done<O>),
    /// The thread dropped its committer and ended.
    Exited,
}

/// Runs a job's commit on the thread. `Payload` is what main sends with a job, `Output` what
/// a commit that took gives back.
pub trait Committer: Send + 'static {
    type Payload: Send + 'static;
    type Output: Send + 'static;
    fn commit(&mut self, kind: Kind, payload: Self::Payload) -> Result<Self::Output, String>;
}

/// The frame calls of an output's compositor. Smithay's `DrmCompositor` queues a frame behind
/// one still waiting for its vblank and commits it from `submitted`.
pub trait Frames {
    /// Commit the rendered frame (`queue_frame`).
    fn queue(&mut self) -> Result<(), String>;
    /// The pending frame reached the screen (`frame_submitted`).
    fn submitted(&mut self) -> Result<(), String>;
    /// Power the CRTC off, dropping any pending frame.
    fn clear(&mut self) -> Result<(), String>;
}

/// An output's compositor and whether a frame it committed still waits for its vblank. A
/// frame is never queued behind a pending one, so `submitted`, which main calls, never
/// commits.
pub struct Scanout<C> {
    compositor: C,
    pending: bool,
}

impl<C: Frames> Scanout<C> {
    pub fn new(compositor: C) -> Self {
        Scanout {
            compositor,
            pending: false,
        }
    }

    /// Commits the rendered frame; fails while the last one waits for its vblank.
    pub fn queue_frame(&mut self) -> Result<(), String> {
        off_main();
        if self.pending {
            return Err("the last frame still waits for its vblank".into());
        }
        self.compositor.queue()?;
        self.pending = true;
        Ok(())
    }

    /// The pending frame, if any, reached the screen or never will (a session reset).
    pub fn frame_submitted(&mut self) -> Result<(), String> {
        if !std::mem::take(&mut self.pending) {
            return Ok(());
        }
        self.compositor.submitted()
    }

    pub fn clear(&mut self) -> Result<(), String> {
        off_main();
        self.compositor.clear()?;
        self.pending = false;
        Ok(())
    }
}

/// Frames are committed on the output's thread only (main renders and assigns planes).
fn off_main() {
    debug_assert_ne!(
        std::thread::current().name(),
        Some("main"),
        "main committed a frame"
    );
}

impl<C> Deref for Scanout<C> {
    type Target = C;
    fn deref(&self) -> &C {
        &self.compositor
    }
}

impl<C> DerefMut for Scanout<C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.compositor
    }
}

/// `REPRISE_COMMIT_DELAY=<connector>:<ms>[:modeset]`: that output's thread sleeps `ms` before
/// each commit (only before modesets with `:modeset`), to provoke a commit stuck in the kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delay {
    pub wait: Duration,
    pub modeset_only: bool,
}

impl Delay {
    pub const VAR: &str = "REPRISE_COMMIT_DELAY";

    /// The delay `value` sets for `connector`, if any.
    pub fn parse(value: &str, connector: &str) -> Option<Delay> {
        let mut parts = value.split(':');
        if parts.next()? != connector {
            return None;
        }
        let ms = parts.next()?.parse().ok()?;
        let modeset_only = match parts.next() {
            None => false,
            Some("modeset") => true,
            Some(_) => return None,
        };
        parts.next().is_none().then_some(Delay {
            wait: Duration::from_millis(ms),
            modeset_only,
        })
    }

    fn applies(&self, kind: Kind) -> bool {
        !self.modeset_only || matches!(kind, Kind::Flip { modeset: true })
    }
}

/// An output's commit thread. Dropping it closes the job queue; the thread ends after its
/// current job without being waited for.
pub struct Thread<P> {
    jobs: mpsc::Sender<Job<P>>,
    handle: Option<JoinHandle<()>>,
}

impl<P: Send + 'static> Thread<P> {
    /// Starts `name`'s thread; every job's `Done` and the final `Exited` go to `replies`
    /// tagged with `id`.
    pub fn spawn<I, C>(
        name: &str,
        id: I,
        committer: C,
        delay: Option<Delay>,
        replies: Sender<(I, Reply<C::Output>)>,
    ) -> std::io::Result<Thread<P>>
    where
        I: Copy + Send + 'static,
        C: Committer<Payload = P>,
    {
        let (jobs, queue) = mpsc::channel::<Job<P>>();
        let connector = name.to_owned();
        let handle = std::thread::Builder::new()
            .name(format!("commit-{name}"))
            .spawn(move || {
                let mut committer = committer;
                while let Ok(Job::Run(kind, payload)) = queue.recv() {
                    let delayed = delay.filter(|d| d.applies(kind));
                    if let Some(d) = delayed {
                        let ms = d.wait.as_millis();
                        log::info!("{connector}: delayed commit {kind:?} starts, waiting {ms} ms");
                        std::thread::sleep(d.wait);
                    }
                    let started = Instant::now();
                    let result = committer.commit(kind, payload);
                    if let Some(d) = delayed {
                        let ms = d.wait.as_millis();
                        let ok = result.is_ok();
                        log::info!(
                            "{connector}: delayed commit {kind:?} ends after {ms} ms, ok {ok}"
                        );
                    }
                    let done = Done {
                        kind,
                        result,
                        started,
                        ended: Instant::now(),
                    };
                    if replies.send((id, Reply::Done(done))).is_err() {
                        break;
                    }
                }
                drop(committer);
                let _ = replies.send((id, Reply::Exited));
            })?;
        Ok(Thread {
            jobs,
            handle: Some(handle),
        })
    }

    /// Queues `job`; false when the thread is gone.
    pub fn send(&self, job: Job<P>) -> bool {
        self.jobs.send(job).is_ok()
    }

    /// Asks the thread to drop its committer and end once its current job is done.
    pub fn exit(&mut self) -> Option<JoinHandle<()>> {
        let _ = self.jobs.send(Job::Exit);
        self.handle.take()
    }
}

/// Waits for `threads` until `deadline` has passed in total, then leaves the rest running.
/// Returns how many were left.
pub fn join_until(threads: Vec<JoinHandle<()>>, deadline: Duration) -> usize {
    let until = Instant::now() + deadline;
    let mut left = threads;
    loop {
        let (done, running): (Vec<_>, Vec<_>) = left.into_iter().partition(|h| h.is_finished());
        for h in done {
            let _ = h.join();
        }
        left = running;
        if left.is_empty() || Instant::now() >= until {
            return left.len();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// One commit's timings: main to thread, the commit itself, thread back to main.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Times {
    pub to_thread: Duration,
    pub ioctl: Duration,
    pub to_main: Duration,
}

/// The 99th percentile of each of `times`' fields.
pub fn tail(times: &[Times]) -> Times {
    let p99 = |f: fn(&Times) -> Duration| {
        let mut v: Vec<Duration> = times.iter().map(f).collect();
        v.sort_unstable();
        v.get((v.len() * 99).div_ceil(100).saturating_sub(1))
            .copied()
            .unwrap_or_default()
    };
    Times {
        to_thread: p99(|t| t.to_thread),
        ioctl: p99(|t| t.ioctl),
        to_main: p99(|t| t.to_main),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Main owns the compositor; a frame may render.
    Idle,
    /// A job is on the thread, sent at `since`.
    Busy { kind: Kind, since: Instant },
    /// The flip went to the kernel at `since`; its vblank has not come.
    Flipping { since: Instant },
}

/// What main does once a `Done` ended `Busy`.
#[derive(Debug, PartialEq, Eq)]
pub struct After<M> {
    /// Power the output off.
    pub off: bool,
    /// A vblank that came during `Busy`, to handle now; the output is idle again.
    pub vblank: Option<M>,
    /// A modeset failed or stalled: the next one waits this long (`Track::retry_at`).
    pub backoff: Option<Duration>,
}

/// Main's view of one output's commits. `M` is a vblank's metadata.
#[derive(Debug)]
pub struct Track<M> {
    phase: Phase,
    vblank: Option<M>,
    reset: bool,
    off: bool,
    stalled: bool,
    /// Modeset failures in a row, and when the next modeset may go.
    tries: u32,
    retry_at: Option<Instant>,
}

impl<M> Default for Track<M> {
    fn default() -> Self {
        Track {
            phase: Phase::Idle,
            vblank: None,
            reset: false,
            off: false,
            stalled: false,
            tries: 0,
            retry_at: None,
        }
    }
}

impl<M> Track<M> {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Busy { .. })
    }

    /// How long the job has been on the thread, or the flip waited for its vblank.
    pub fn waited(&self, now: Instant) -> Option<Duration> {
        match self.phase {
            Phase::Idle => None,
            Phase::Busy { since, .. } | Phase::Flipping { since } => {
                Some(now.saturating_duration_since(since))
            }
        }
    }

    fn quiet(&self, now: Instant) -> bool {
        let waited = self.waited(now).unwrap_or_default();
        match self.phase {
            Phase::Idle => true,
            Phase::Busy { kind, .. } => !kind.blocking() && waited <= STALL,
            Phase::Flipping { .. } => waited <= STALL || waited > FLIP_LOST,
        }
    }

    /// True once, when the job or flip has waited past `STALL`; the output stays `stalled`
    /// until it ends.
    pub fn stall(&mut self, now: Instant) -> bool {
        if self.stalled || self.waited(now).is_none_or(|w| w <= STALL) {
            return false;
        }
        self.stalled = true;
        true
    }

    pub fn stalled(&self) -> bool {
        self.stalled
    }

    /// When the next modeset may go after failed or stalled ones.
    pub fn retry_at(&self) -> Option<Instant> {
        self.retry_at
    }

    /// A job went to the thread. A frame goes only from `Idle`; a power-off also from
    /// `Flipping`, whose vblank may then come during the job.
    pub fn sent(&mut self, kind: Kind, now: Instant) {
        debug_assert!(!self.busy(), "one job at a time per output");
        self.phase = Phase::Busy { kind, since: now };
        self.stalled = false;
    }

    /// The job was never queued (the thread is gone).
    pub fn unsent(&mut self) {
        self.idle_now();
    }

    fn idle_now(&mut self) {
        self.phase = Phase::Idle;
        self.stalled = false;
    }

    /// The thread finished the job at `now`; `ok` says the commit took. A failed or stalled
    /// modeset backs the next one off 1, 2, 4 ... `BACKOFF_MAX` s; one in time clears it.
    pub fn done(&mut self, ok: bool, now: Instant) -> After<M> {
        let (kind, waited) = match self.phase {
            Phase::Busy { kind, since } => (Some(kind), now.saturating_duration_since(since)),
            _ => (None, Duration::ZERO),
        };
        let flip = matches!(kind, Some(Kind::Flip { .. }));
        let mut backoff = None;
        if kind == Some(Kind::Flip { modeset: true }) {
            if ok && waited <= STALL {
                self.tries = 0;
                self.retry_at = None;
            } else {
                let wait = Duration::from_secs(1 << self.tries.min(5)).min(BACKOFF_MAX);
                self.tries = self.tries.saturating_add(1);
                self.retry_at = Some(now + wait);
                backoff = Some(wait);
            }
        }
        let reset = std::mem::take(&mut self.reset);
        self.idle_now();
        if flip && ok && !reset {
            self.phase = Phase::Flipping { since: now };
        }
        let vblank = self.vblank.take().filter(|_| ok || !flip);
        if vblank.is_some() {
            self.phase = Phase::Idle;
        }
        After {
            off: std::mem::take(&mut self.off),
            vblank,
            backoff,
        }
    }

    /// A vblank came; Some when main handles it now, None when it waits for the `Done`.
    pub fn vblank(&mut self, meta: M) -> Option<M> {
        if self.busy() {
            self.vblank = Some(meta);
            return None;
        }
        self.idle_now();
        Some(meta)
    }

    /// The session came back: a flip in flight may never see its vblank. Forgotten now when no
    /// job is out (true), else when its `Done` comes; a vblank that came during the job is still
    /// handed on, as that commit took. The compositor's pending frame is dropped separately.
    pub fn reset(&mut self) -> bool {
        if self.busy() {
            self.reset = true;
            return false;
        }
        self.idle_now();
        self.vblank = None;
        true
    }

    /// Power off: send the `Clear` now when no job is out (true), else once it is done.
    pub fn power_off(&mut self) -> bool {
        if self.busy() {
            self.off = true;
            return false;
        }
        true
    }

    /// Power on cancels an off still waiting for the job.
    pub fn power_on(&mut self) {
        self.off = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::calloop::{
        EventLoop,
        channel::{Event, channel},
        timer::{TimeoutAction, Timer},
    };

    const FLIP: Kind = Kind::Flip { modeset: false };
    const MODESET: Kind = Kind::Flip { modeset: true };

    fn busy() -> Track<u32> {
        let mut t = Track::default();
        t.sent(FLIP, Instant::now());
        t
    }

    #[test]
    fn only_an_idle_output_renders() {
        let mut t = Track::<u32>::default();
        assert!(t.idle());
        t.sent(FLIP, Instant::now());
        assert!(!t.idle() && t.busy());
        t.done(true, Instant::now());
        assert!(matches!(t.phase(), Phase::Flipping { .. }));
        assert!(!t.idle() && !t.busy());
        assert_eq!(t.vblank(7), Some(7));
        assert!(t.idle());
    }

    #[test]
    fn a_vblank_during_a_commit_waits_for_its_done() {
        let mut t = busy();
        assert_eq!(t.vblank(7), None, "not before the Done");
        assert!(t.busy());
        let after = t.done(true, Instant::now());
        assert_eq!(after.vblank, Some(7));
        assert!(t.idle(), "the replayed vblank ends the flip");
    }

    #[test]
    fn a_failed_commit_is_idle_and_drops_a_stray_vblank() {
        let mut t = busy();
        t.vblank(7);
        let after = t.done(false, Instant::now());
        assert_eq!(after.vblank, None);
        assert!(t.idle());
    }

    #[test]
    fn a_clear_sent_while_flipping_replays_the_flips_vblank() {
        let mut t = Track::<u32>::default();
        t.sent(FLIP, Instant::now());
        t.done(true, Instant::now());
        assert!(t.power_off(), "no job out while flipping");
        t.sent(Kind::Clear, Instant::now());
        assert_eq!(t.vblank(7), None);
        assert_eq!(
            t.done(false, Instant::now()).vblank,
            Some(7),
            "even when the clear failed"
        );
        assert!(t.idle());
        t.sent(Kind::Clear, Instant::now());
        t.done(true, Instant::now());
        assert!(t.idle(), "a clear never leaves the output flipping");
    }

    #[test]
    fn a_reset_or_power_off_during_a_commit_waits_for_its_done() {
        let mut t = busy();
        assert!(!t.reset(), "not while the thread has the compositor");
        assert!(!t.power_off());
        let after = t.done(true, Instant::now());
        assert_eq!(
            after,
            After {
                off: true,
                vblank: None,
                backoff: None
            }
        );
        assert!(t.idle(), "a flip from before the reset is not waited for");
        assert_eq!(
            t.done(true, Instant::now()),
            After {
                off: false,
                vblank: None,
                backoff: None
            }
        );
    }

    #[test]
    fn a_vblank_that_came_during_a_commit_outliving_a_reset_is_handed_on() {
        let mut t = Track::<u32>::default();
        t.sent(MODESET, Instant::now());
        assert!(!t.reset());
        assert_eq!(t.vblank(7), None);
        assert_eq!(t.done(true, Instant::now()).vblank, Some(7));
        assert!(t.idle());
    }

    /// Smithay's `DrmCompositor`: a frame queued behind a pending one is committed by
    /// `submitted`.
    #[derive(Default)]
    struct Drm {
        pending: bool,
        queued: bool,
        commits: u32,
    }

    impl Drm {
        fn submit(&mut self) {
            self.queued = false;
            self.pending = true;
            self.commits += 1;
        }
    }

    impl Frames for Drm {
        fn queue(&mut self) -> Result<(), String> {
            self.queued = true;
            if !self.pending {
                self.submit();
            }
            Ok(())
        }
        fn submitted(&mut self) -> Result<(), String> {
            self.pending = false;
            if self.queued {
                self.submit();
            }
            Ok(())
        }
        fn clear(&mut self) -> Result<(), String> {
            self.pending = false;
            self.queued = false;
            Ok(())
        }
    }

    #[test]
    fn a_frame_is_never_queued_behind_one_waiting_for_its_vblank() {
        let mut s = Scanout::new(Drm::default());
        s.queue_frame().unwrap();
        assert_eq!(s.commits, 1);
        assert!(s.queue_frame().is_err());
        assert!(!s.queued);
        s.frame_submitted().unwrap();
        s.frame_submitted().unwrap();
        assert_eq!(s.commits, 1, "the vblank commits nothing");
        s.queue_frame().unwrap();
        assert_eq!(s.commits, 2);
        s.clear().unwrap();
        s.queue_frame().unwrap();
        assert_eq!(s.commits, 3, "a clear drops the pending frame");
    }

    #[test]
    fn the_frame_a_reset_forgot_is_dropped_before_the_next_one() {
        let mut t = Track::<u32>::default();
        let mut s = Scanout::new(Drm::default());
        t.sent(MODESET, Instant::now());
        assert!(!t.reset(), "the session came back during the modeset");
        s.queue_frame().unwrap();
        t.done(true, Instant::now());
        assert!(t.idle(), "its flip is forgotten");
        assert!(
            s.queue_frame().is_err(),
            "the compositor still waits for it"
        );
        s.frame_submitted().unwrap();
        t.sent(MODESET, Instant::now());
        s.queue_frame().unwrap();
        assert_eq!(s.commits, 2, "the next modeset reaches the kernel");
    }

    #[test]
    fn power_on_cancels_a_waiting_power_off() {
        let mut t = busy();
        t.power_off();
        t.power_on();
        assert!(!t.done(true, Instant::now()).off);
    }

    #[test]
    fn a_reset_runs_at_once_off_the_thread() {
        let mut t = Track::<u32>::default();
        t.sent(FLIP, Instant::now());
        t.done(true, Instant::now());
        assert!(t.reset(), "a flipping output has no job out");
        assert!(t.idle());
    }

    fn secs(s: f64) -> Duration {
        Duration::from_secs_f64(s)
    }

    #[test]
    fn an_output_stalls_once_past_two_seconds_until_its_job_ends() {
        let t0 = Instant::now();
        let mut t = Track::<u32>::default();
        t.sent(MODESET, t0);
        assert!(!t.stall(t0 + secs(1.9)) && !t.stalled());
        assert!(t.stall(t0 + secs(2.1)), "labelled when it passes 2 s");
        assert!(!t.stall(t0 + secs(5.0)), "and only once");
        assert!(t.stalled());
        t.done(true, t0 + secs(6.0));
        assert!(!t.stalled(), "the Done ends it");
        assert!(
            !t.stall(t0 + secs(7.0)),
            "a flip just queued is not stalled"
        );
        assert!(t.stall(t0 + secs(8.1)), "a flip 2 s without its vblank is");
        t.vblank(7);
        assert!(!t.stalled());
    }

    #[test]
    fn quiet_means_no_blocking_job_no_stall_and_no_lost_flip_held() {
        let t0 = Instant::now();
        let idle = Track::<u32>::default();
        assert!(quiet([&idle], false, t0));
        assert!(
            !quiet([&idle], true, t0),
            "a thread dropping its compositor"
        );
        for kind in [MODESET, Kind::Clear, Kind::Gamma, Kind::Retrain] {
            let mut t = Track::<u32>::default();
            t.sent(kind, t0);
            assert!(!quiet([&idle, &t], false, t0), "{kind:?}");
            t.done(false, t0);
            assert!(quiet([&idle, &t], false, t0), "{kind:?} done");
        }
        let mut flip = Track::<u32>::default();
        flip.sent(FLIP, t0);
        assert!(
            quiet([&flip], false, t0 + secs(1.0)),
            "a page flip in the ioctl"
        );
        assert!(
            !quiet([&flip], false, t0 + secs(2.5)),
            "a page flip 2 s in the ioctl"
        );
        flip.done(true, t0);
        for (at, want) in [(1.9, true), (2.1, false), (10.9, false), (11.1, true)] {
            assert_eq!(
                quiet([&flip], false, t0 + secs(at)),
                want,
                "flip waiting {at} s for its vblank"
            );
        }
    }

    #[test]
    fn another_outputs_stall_holds_every_output_until_the_device_is_quiet() {
        let t0 = Instant::now();
        let idle = Track::<u32>::default();
        let mut modeset = Track::<u32>::default();
        modeset.sent(MODESET, t0);
        let held = |own: &Track<u32>, other: &Track<u32>, at: f64| {
            held(own, &[own, other], false, t0 + secs(at))
        };
        assert!(!held(&idle, &modeset, 1.0), "a modeset not yet stalled");
        assert!(modeset.stall(t0 + secs(2.1)));
        assert!(held(&modeset, &idle, 2.1));
        assert!(held(&idle, &modeset, 2.1), "the idle output cannot render");
        modeset.done(true, t0 + secs(3.0));
        assert!(!held(&idle, &modeset, 3.0), "the Done ends the hold");
        assert!(modeset.stall(t0 + secs(5.1)), "its flip lost a vblank");
        assert!(held(&idle, &modeset, 5.1), "a stalled flip holds it too");
        assert!(held(&modeset, &idle, 14.1));
        assert!(
            !held(&idle, &modeset, 14.1),
            "a lost flip no longer holds the others"
        );
    }

    #[test]
    fn the_device_reset_holds_every_output_until_its_done() {
        let t0 = Instant::now();
        let output = Track::<u32>::default();
        let mut device = Track::<u32>::default();
        device.sent(Kind::Reset, t0);
        assert!(!quiet([&output, &device], false, t0), "no output renders");
        assert!(!held(&output, &[&output, &device], false, t0 + secs(1.0)));
        assert!(device.stall(t0 + secs(2.1)));
        assert!(held(&output, &[&output, &device], false, t0 + secs(2.1)));
        let after = device.done(true, t0 + secs(3.0));
        assert_eq!(
            after,
            After {
                off: false,
                vblank: None,
                backoff: None
            }
        );
        assert!(device.idle(), "a reset leaves no flip waiting");
        assert!(quiet([&output, &device], false, t0 + secs(3.0)));
    }

    #[test]
    fn a_failed_or_stalled_modeset_backs_off_up_to_thirty_seconds() {
        let t0 = Instant::now();
        let mut t = Track::<u32>::default();
        let mut waits = Vec::new();
        for _ in 0..7 {
            t.sent(MODESET, t0);
            let after = t.done(false, t0);
            assert_eq!(t.retry_at(), after.backoff.map(|w| t0 + w));
            waits.push(after.backoff.unwrap().as_secs());
        }
        assert_eq!(waits, [1, 2, 4, 8, 16, 30, 30]);
        t.sent(MODESET, t0);
        assert_eq!(t.done(true, t0 + secs(0.5)).backoff, None);
        assert_eq!(t.retry_at(), None, "a modeset that took clears it");
        t.sent(MODESET, t0);
        assert_eq!(
            t.done(true, t0 + secs(3.0)).backoff,
            Some(secs(1.0)),
            "one that took 3 s backs off too"
        );
        for kind in [FLIP, Kind::Clear, Kind::Gamma, Kind::Retrain] {
            let mut t = Track::<u32>::default();
            t.sent(kind, t0);
            assert_eq!(t.done(false, t0).backoff, None, "{kind:?}");
            assert_eq!(t.retry_at(), None, "{kind:?}");
        }
    }

    #[test]
    fn only_a_page_flip_is_nonblocking() {
        assert!(!FLIP.blocking());
        for k in [
            MODESET,
            Kind::Clear,
            Kind::Gamma,
            Kind::Retrain,
            Kind::Reset,
        ] {
            assert!(k.blocking(), "{k:?}");
        }
    }

    #[test]
    fn the_delay_names_a_connector_a_time_and_maybe_modesets() {
        let d = |v| Delay::parse(v, "HDMI-A-1");
        assert_eq!(
            d("HDMI-A-1:1500"),
            Some(Delay {
                wait: Duration::from_millis(1500),
                modeset_only: false
            })
        );
        assert_eq!(
            d("HDMI-A-1:20:modeset"),
            Some(Delay {
                wait: Duration::from_millis(20),
                modeset_only: true
            })
        );
        for other in [
            "HDMI-A-2:1500",
            "HDMI-A-1",
            "HDMI-A-1:x",
            "HDMI-A-1:5:flip",
            "HDMI-A-1:5:modeset:1",
            "",
        ] {
            assert_eq!(d(other), None, "{other}");
        }
        let modesets = d("HDMI-A-1:20:modeset").unwrap();
        assert!(modesets.applies(MODESET) && !modesets.applies(FLIP));
        assert!(!modesets.applies(Kind::Clear));
        assert!(d("HDMI-A-1:20").unwrap().applies(Kind::Gamma));
    }

    #[test]
    fn the_tail_is_the_99th_percentile() {
        let us = Duration::from_micros;
        let times: Vec<_> = (1..=200)
            .map(|i| Times {
                to_thread: us(i),
                ioctl: us(1000 + i),
                to_main: us(401 - i),
            })
            .collect();
        assert_eq!(
            tail(&times),
            Times {
                to_thread: us(198),
                ioctl: us(1198),
                to_main: us(398)
            }
        );
        assert_eq!(tail(&[]), Times::default());
    }

    /// Commits once the test releases it, like a modeset stuck in the kernel.
    struct Stuck(mpsc::Receiver<()>);

    impl Committer for Stuck {
        type Payload = ();
        type Output = ();
        fn commit(&mut self, _: Kind, _: ()) -> Result<(), String> {
            self.0.recv().map_err(|e| e.to_string())
        }
    }

    struct Quick;

    impl Committer for Quick {
        type Payload = ();
        type Output = ();
        fn commit(&mut self, _: Kind, _: ()) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Loop {
        stuck_done: bool,
        quick_dones: Vec<Instant>,
        ticks: Vec<Instant>,
        resend: Option<mpsc::Sender<Job<()>>>,
    }

    #[test]
    fn a_stuck_commit_leaves_the_loop_and_other_outputs_running() {
        let mut ev: EventLoop<Loop> = EventLoop::try_new().unwrap();
        let (replies, done) = channel::<(u8, Reply<()>)>();
        let (release, gate) = mpsc::channel();
        let stuck = Thread::spawn("A", 0u8, Stuck(gate), None, replies.clone()).unwrap();
        let quick = Thread::spawn("B", 1u8, Quick, None, replies).unwrap();
        let (to_quick, quick_jobs) = mpsc::channel::<Job<()>>();
        ev.handle()
            .insert_source(done, |event, _, data: &mut Loop| {
                if let Event::Msg((id, Reply::Done(done))) = event {
                    assert!(done.result.is_ok());
                    if id == 0 {
                        data.stuck_done = true;
                    } else {
                        data.quick_dones.push(Instant::now());
                        if let Some(r) = &data.resend {
                            let _ = r.send(Job::Run(FLIP, ()));
                        }
                    }
                }
            })
            .unwrap();
        ev.handle()
            .insert_source(
                Timer::from_duration(Duration::from_millis(2)),
                |_, _, data| {
                    data.ticks.push(Instant::now());
                    TimeoutAction::ToDuration(Duration::from_millis(2))
                },
            )
            .unwrap();
        let mut data = Loop {
            resend: Some(to_quick),
            ..Loop::default()
        };
        assert!(stuck.send(Job::Run(MODESET, ())));
        assert!(quick.send(Job::Run(FLIP, ())));
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            ev.dispatch(Duration::from_millis(5), &mut data).unwrap();
            while let Ok(job) = quick_jobs.try_recv() {
                assert!(quick.send(job));
            }
        }
        assert!(!data.stuck_done, "A is still in its commit");
        let gaps = |at: &[Instant]| {
            at.windows(2)
                .map(|w| w[1] - w[0])
                .max()
                .unwrap_or(Duration::MAX)
        };
        assert!(
            data.quick_dones.len() > 20,
            "B kept flipping: {}",
            data.quick_dones.len()
        );
        assert!(
            gaps(&data.quick_dones) < Duration::from_millis(25),
            "B's flips kept coming"
        );
        assert!(
            data.ticks.len() > 20,
            "the timer kept firing: {}",
            data.ticks.len()
        );
        assert!(
            gaps(&data.ticks) < Duration::from_millis(25),
            "no tick waited on A"
        );
        release.send(()).unwrap();
        let start = Instant::now();
        while !data.stuck_done && start.elapsed() < Duration::from_secs(5) {
            ev.dispatch(Duration::from_millis(5), &mut data).unwrap();
        }
        assert!(data.stuck_done, "A's Done arrives once the kernel lets go");
    }

    /// Reports the thread it was dropped on.
    struct Dropped(mpsc::Sender<std::thread::ThreadId>);

    impl Committer for Dropped {
        type Payload = ();
        type Output = ();
        fn commit(&mut self, _: Kind, _: ()) -> Result<(), String> {
            Ok(())
        }
    }

    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.0.send(std::thread::current().id());
        }
    }

    #[test]
    fn exit_drops_the_committer_on_its_thread_then_says_so() {
        let mut ev: EventLoop<Vec<Reply<()>>> = EventLoop::try_new().unwrap();
        let (replies, done) = channel::<(u8, Reply<()>)>();
        ev.handle()
            .insert_source(done, |event, _, got: &mut Vec<Reply<()>>| {
                if let Event::Msg((_, r)) = event {
                    got.push(r);
                }
            })
            .unwrap();
        let (tx, dropped_on) = mpsc::channel();
        let mut t = Thread::spawn("A", 0u8, Dropped(tx), None, replies).unwrap();
        assert!(t.send(Job::Run(Kind::Clear, ())));
        let handle = t.exit().unwrap();
        let worker = handle.thread().id();
        handle.join().unwrap();
        assert_eq!(dropped_on.recv().unwrap(), worker);
        let mut got = Vec::new();
        ev.dispatch(Duration::from_millis(50), &mut got).unwrap();
        assert!(
            matches!(got.as_slice(), [Reply::Done(_), Reply::Exited]),
            "{got:?}"
        );
    }

    #[test]
    fn the_exit_deadline_leaves_a_stuck_thread_running() {
        let (replies, _done) = channel::<(u8, Reply<()>)>();
        let (release, gate) = mpsc::channel();
        let mut stuck = Thread::spawn("A", 0u8, Stuck(gate), None, replies.clone()).unwrap();
        let mut quick = Thread::spawn("B", 1u8, Quick, None, replies).unwrap();
        assert!(stuck.send(Job::Run(Kind::Clear, ())));
        let handles = [stuck.exit(), quick.exit()].into_iter().flatten().collect();
        let start = Instant::now();
        assert_eq!(join_until(handles, Duration::from_millis(100)), 1);
        let took = start.elapsed();
        assert!(
            took >= Duration::from_millis(100) && took < Duration::from_secs(2),
            "{took:?}"
        );
        release.send(()).unwrap();
    }
}
