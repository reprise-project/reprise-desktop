//! Per-output commit threads. Main renders and assigns planes; each driven CRTC's thread runs
//! the KMS commit, so a commit stuck in the kernel stalls that output alone. `Track` is main's
//! view of one output; at most one job is outstanding per output.

use std::{
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
    Flip { modeset: bool },
}

impl Kind {
    /// Commits that wait in the kernel holding its modeset locks (on some drivers every
    /// CRTC's) until they return.
    pub fn blocking(self) -> bool {
        self == Kind::Flip { modeset: true }
    }
}

/// Whether an output whose commits `own` tracks may render, given every output's `all`: its
/// own thread is idle and no thread is in a blocking commit, whose locks the frame's plane test
/// and flip would wait on.
pub fn render_due<'a, M: 'a>(own: &Track<M>, all: impl IntoIterator<Item = &'a Track<M>>) -> bool {
    own.idle() && !all.into_iter().any(Track::blocking)
}

#[derive(Debug)]
pub enum Job {
    Run(Kind),
}

/// A finished job, as the thread reports it.
#[derive(Debug)]
pub struct Done {
    pub kind: Kind,
    pub result: Result<(), String>,
    /// Around the commit itself, after any `REPRISE_COMMIT_DELAY` sleep.
    pub started: Instant,
    pub ended: Instant,
}

/// Runs a job's commit on the thread.
pub trait Committer: Send + 'static {
    fn commit(&mut self, kind: Kind) -> Result<(), String>;
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

/// An output's commit thread. Dropping it closes the job queue and waits for the thread.
pub struct Thread {
    jobs: Option<mpsc::Sender<Job>>,
    handle: Option<JoinHandle<()>>,
}

impl Thread {
    /// Starts `name`'s thread; every job's `Done` goes to `replies` tagged with `id`.
    pub fn spawn<I, C>(
        name: &str,
        id: I,
        mut committer: C,
        delay: Option<Delay>,
        replies: Sender<(I, Done)>,
    ) -> std::io::Result<Thread>
    where
        I: Copy + Send + 'static,
        C: Committer,
    {
        let (jobs, queue) = mpsc::channel::<Job>();
        let connector = name.to_owned();
        let handle = std::thread::Builder::new()
            .name(format!("commit-{name}"))
            .spawn(move || {
                while let Ok(Job::Run(kind)) = queue.recv() {
                    let delayed = delay.filter(|d| d.applies(kind));
                    if let Some(d) = delayed {
                        let ms = d.wait.as_millis();
                        log::info!("{connector}: delayed commit {kind:?} starts, waiting {ms} ms");
                        std::thread::sleep(d.wait);
                    }
                    let started = Instant::now();
                    let result = committer.commit(kind);
                    if let Some(d) = delayed {
                        let ms = d.wait.as_millis();
                        log::info!(
                            "{connector}: delayed commit {kind:?} ends after {ms} ms, ok {}",
                            result.is_ok()
                        );
                    }
                    let done = Done {
                        kind,
                        result,
                        started,
                        ended: Instant::now(),
                    };
                    if replies.send((id, done)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Thread {
            jobs: Some(jobs),
            handle: Some(handle),
        })
    }

    /// Queues `job`; false when the thread is gone.
    pub fn send(&self, job: Job) -> bool {
        self.jobs.as_ref().is_some_and(|j| j.send(job).is_ok())
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        drop(self.jobs.take());
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
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
    /// The flip went to the kernel; its vblank has not come.
    Flipping,
}

/// What main does once a `Done` ended `Busy`.
#[derive(Debug, PartialEq, Eq)]
pub struct After<M> {
    /// Power the output off.
    pub off: bool,
    /// A vblank that came during `Busy`, to handle now; the output is idle again.
    pub vblank: Option<M>,
}

/// Main's view of one output's commits. `M` is a vblank's metadata.
#[derive(Debug)]
pub struct Track<M> {
    phase: Phase,
    vblank: Option<M>,
    reset: bool,
    off: bool,
}

impl<M> Default for Track<M> {
    fn default() -> Self {
        Track {
            phase: Phase::Idle,
            vblank: None,
            reset: false,
            off: false,
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

    /// A blocking commit is on the thread.
    pub fn blocking(&self) -> bool {
        matches!(self.phase, Phase::Busy { kind, .. } if kind.blocking())
    }

    /// A job went to the thread.
    pub fn sent(&mut self, kind: Kind, now: Instant) {
        debug_assert!(!self.busy(), "one job at a time per output");
        self.phase = Phase::Busy { kind, since: now };
    }

    /// The job was never queued (the thread is gone).
    pub fn unsent(&mut self) {
        self.phase = Phase::Idle;
    }

    /// The thread finished the job; `ok` says the commit took.
    pub fn done(&mut self, ok: bool) -> After<M> {
        let flipped = matches!(
            self.phase,
            Phase::Busy {
                kind: Kind::Flip { .. },
                ..
            }
        ) && ok;
        let reset = std::mem::take(&mut self.reset);
        self.phase = if flipped && !reset {
            Phase::Flipping
        } else {
            Phase::Idle
        };
        let vblank = self.vblank.take().filter(|_| self.phase == Phase::Flipping);
        if vblank.is_some() {
            self.phase = Phase::Idle;
        }
        After {
            off: std::mem::take(&mut self.off),
            vblank,
        }
    }

    /// A vblank came; Some when main handles it now, None when it waits for the `Done`.
    pub fn vblank(&mut self, meta: M) -> Option<M> {
        if self.busy() {
            self.vblank = Some(meta);
            return None;
        }
        self.phase = Phase::Idle;
        Some(meta)
    }

    /// The session came back: a flip in flight will not complete. Forgotten now when no job
    /// is out (true), else when its `Done` comes.
    pub fn reset(&mut self) -> bool {
        if self.busy() {
            self.reset = true;
            return false;
        }
        self.phase = Phase::Idle;
        self.vblank = None;
        true
    }

    /// Power off: now when no job is out (true), else once it is done.
    pub fn power_off(&mut self) -> bool {
        if self.busy() {
            self.off = true;
            return false;
        }
        self.phase = Phase::Idle;
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
        t.done(true);
        assert_eq!(t.phase(), Phase::Flipping);
        assert!(!t.idle() && !t.busy());
        assert_eq!(t.vblank(7), Some(7));
        assert!(t.idle());
    }

    #[test]
    fn a_modeset_anywhere_holds_every_outputs_render() {
        let idle = Track::<u32>::default();
        let mut flipping = Track::<u32>::default();
        flipping.sent(FLIP, Instant::now());
        assert!(
            render_due(&idle, [&idle, &flipping]),
            "a page flip blocks nobody else"
        );
        assert!(
            !render_due(&flipping, [&idle, &flipping]),
            "nor renders twice"
        );
        let mut modeset = Track::<u32>::default();
        modeset.sent(MODESET, Instant::now());
        assert!(!render_due(&idle, [&idle, &modeset]));
        modeset.done(false);
        assert!(render_due(&idle, [&idle, &modeset]));
    }

    #[test]
    fn a_vblank_during_a_commit_waits_for_its_done() {
        let mut t = busy();
        assert_eq!(t.vblank(7), None, "not before the Done");
        assert!(t.busy());
        let after = t.done(true);
        assert_eq!(after.vblank, Some(7));
        assert!(t.idle(), "the replayed vblank ends the flip");
    }

    #[test]
    fn a_failed_commit_is_idle_and_drops_a_stray_vblank() {
        let mut t = busy();
        t.vblank(7);
        let after = t.done(false);
        assert_eq!(after.vblank, None);
        assert!(t.idle());
    }

    #[test]
    fn a_reset_or_power_off_during_a_commit_waits_for_its_done() {
        let mut t = busy();
        assert!(!t.reset(), "not while the thread has the compositor");
        assert!(!t.power_off());
        t.vblank(7);
        let after = t.done(true);
        assert_eq!(
            after,
            After {
                off: true,
                vblank: None
            },
            "the old flip's vblank is gone with the reset"
        );
        assert!(t.idle());
        assert_eq!(
            t.done(true),
            After {
                off: false,
                vblank: None
            }
        );
    }

    #[test]
    fn power_on_cancels_a_waiting_power_off() {
        let mut t = busy();
        t.power_off();
        t.power_on();
        assert!(!t.done(true).off);
    }

    #[test]
    fn reset_and_power_off_run_at_once_off_the_thread() {
        let mut t = Track::<u32>::default();
        t.sent(FLIP, Instant::now());
        t.done(true);
        assert!(t.reset(), "a flipping output has no job out");
        assert!(t.idle());
        assert!(t.power_off());
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
        assert!(d("HDMI-A-1:20").unwrap().applies(FLIP));
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

    /// Commits until the test releases it; the first one blocks like a modeset stuck in the kernel.
    struct Stuck(mpsc::Receiver<()>);

    impl Committer for Stuck {
        fn commit(&mut self, _: Kind) -> Result<(), String> {
            self.0.recv().map_err(|e| e.to_string())
        }
    }

    struct Quick;

    impl Committer for Quick {
        fn commit(&mut self, _: Kind) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Loop {
        stuck_done: bool,
        quick_dones: Vec<Instant>,
        ticks: Vec<Instant>,
        resend: Option<mpsc::Sender<Job>>,
    }

    #[test]
    fn a_stuck_commit_leaves_the_loop_and_other_outputs_running() {
        let mut ev: EventLoop<Loop> = EventLoop::try_new().unwrap();
        let (replies, done) = channel::<(u8, Done)>();
        let (release, gate) = mpsc::channel();
        let stuck = Thread::spawn("A", 0u8, Stuck(gate), None, replies.clone()).unwrap();
        let quick = Thread::spawn("B", 1u8, Quick, None, replies).unwrap();
        let (to_quick, quick_jobs) = mpsc::channel::<Job>();
        ev.handle()
            .insert_source(done, |event, _, data: &mut Loop| {
                if let Event::Msg((id, done)) = event {
                    assert!(done.result.is_ok());
                    if id == 0 {
                        data.stuck_done = true;
                    } else {
                        data.quick_dones.push(Instant::now());
                        if let Some(r) = &data.resend {
                            let _ = r.send(Job::Run(FLIP));
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
        assert!(stuck.send(Job::Run(MODESET)));
        assert!(quick.send(Job::Run(FLIP)));
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
}
