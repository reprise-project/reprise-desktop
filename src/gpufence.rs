//! Waiting for the GPU to finish a render without blocking the loop: a capture's buffer or a
//! picture read back waits for its fence on the loop, or for a bounded time. A fence that never
//! signals (a GPU lost across a suspend) costs that one result, and the renders after it are
//! refused (`lost`) rather than stalling in the driver on textures the lost one still reads.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use smithay::{
    backend::{
        egl::fence::EGLFence,
        renderer::{gles::GlesRenderer, sync::SyncPoint},
    },
    reexports::calloop::{
        Interest, LoopHandle, Mode, PostAction, RegistrationToken,
        generic::Generic,
        timer::{TimeoutAction, Timer},
    },
};

/// How long a capture waits for its render before it fails.
pub const CAPTURE_DEADLINE: Duration = Duration::from_secs(2);

/// How long a picture read back on the loop may hold it.
pub const READ_DEADLINE: Duration = Duration::from_millis(500);

/// A render the GPU has not finished this long after it was issued counts as lost.
pub const STUCK: Duration = Duration::from_millis(500);

/// How many unfinished renders are remembered; past it the newer ones make way.
const KEPT: usize = 16;

/// How often a fence that has no fd is checked.
const POLL: Duration = Duration::from_millis(1);

type Then<D> = Box<dyn FnOnce(&mut D, bool)>;

struct Wait<D> {
    then: Option<Then<D>>,
    fd: Option<RegistrationToken>,
    timer: Option<RegistrationToken>,
}

/// Runs `then` on the loop with `true` once `sync` is reached, or with `false` once `deadline`
/// passes first. Never waits on the fence: its sync_file is watched, or without one it is polled.
pub fn when_reached<D: 'static>(
    handle: &LoopHandle<'static, D>,
    sync: SyncPoint,
    deadline: Duration,
    then: impl FnOnce(&mut D, bool) + 'static,
) {
    let wait = Rc::new(RefCell::new(Wait {
        then: Some(Box::new(then) as Then<D>),
        fd: None,
        timer: None,
    }));
    let end = Instant::now() + deadline;
    let fd = (!sync.is_reached()).then(|| sync.export()).flatten();
    if let Some(fd) = fd {
        let w = Rc::clone(&wait);
        let h = handle.clone();
        let token = handle.insert_source(
            Generic::new(fd, Interest::READ, Mode::OneShot),
            move |_, _, d| {
                finish(&w, &h, d, Fired::Fd, true);
                Ok(PostAction::Remove)
            },
        );
        wait.borrow_mut().fd = token.ok();
    }
    let watched = wait.borrow().fd.is_some();
    let first = if watched { deadline } else { Duration::ZERO };
    let w = Rc::clone(&wait);
    let h = handle.clone();
    let token = handle.insert_source(Timer::from_duration(first), move |_, _, d| {
        let reached = sync.is_reached();
        if reached || Instant::now() >= end {
            finish(&w, &h, d, Fired::Timer, reached);
            return TimeoutAction::Drop;
        }
        TimeoutAction::ToDuration(if watched {
            end.saturating_duration_since(Instant::now())
        } else {
            POLL
        })
    });
    match token {
        Ok(t) => wait.borrow_mut().timer = Some(t),
        Err(e) => {
            log::warn!("gpu fence: no timer: {e}");
            if let Some(then) = wait.borrow_mut().then.take() {
                handle.insert_idle(move |d| then(d, false));
            }
        }
    }
}

enum Fired {
    Fd,
    Timer,
}

fn finish<D>(
    wait: &Rc<RefCell<Wait<D>>>,
    handle: &LoopHandle<'static, D>,
    d: &mut D,
    fired: Fired,
    reached: bool,
) {
    let (then, other) = {
        let mut w = wait.borrow_mut();
        let other = match fired {
            Fired::Fd => w.timer.take(),
            Fired::Timer => w.fd.take(),
        };
        (w.then.take(), other)
    };
    // The source that fired removes itself by what it returns.
    if let Some(t) = other {
        handle.remove(t);
    }
    if let Some(then) = then {
        then(d, reached);
    }
}

/// A fence after every command the renderer has issued so far, such as a `copy_framebuffer`.
pub fn fence_now(renderer: &mut GlesRenderer) -> Result<EGLFence, String> {
    let display = renderer.egl_context().display().clone();
    renderer
        .with_context(|gl| {
            let fence = EGLFence::create(&display);
            // SAFETY: the context is current inside `with_context`.
            unsafe { gl.Flush() };
            fence
        })
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Waits at most `deadline` for `fence`; whether it was reached.
pub fn reached_within(fence: &EGLFence, deadline: Duration) -> bool {
    fence.client_wait(Some(deadline), false).unwrap_or(false)
}

/// A fence after the renderer's last command, as a capture waits for it.
pub fn sync_now(renderer: &mut GlesRenderer) -> Result<SyncPoint, String> {
    fence_now(renderer).map(SyncPoint::from)
}

/// The renders main issued that the GPU has not reached yet, oldest first. One GL context
/// reaches its fences in order, so the oldest tells how far behind the GPU is.
#[derive(Default)]
pub struct Renders {
    pending: VecDeque<(SyncPoint, Instant)>,
    told: bool,
}

impl Renders {
    /// Checks no fence until `KEPT` are held, so a frame pays nothing.
    pub fn issued(&mut self, sync: SyncPoint, at: Instant) {
        if self.pending.len() >= KEPT {
            self.forget_reached();
        }
        if self.pending.len() >= KEPT {
            self.pending.remove(1);
        }
        self.pending.push_back((sync, at));
    }

    fn forget_reached(&mut self) {
        while self.pending.front().is_some_and(|(s, _)| s.is_reached()) {
            self.pending.pop_front();
        }
    }

    /// Whether the oldest render not reached was issued over `STUCK` before `now`. Never waits.
    pub fn lost(&mut self, now: Instant) -> bool {
        self.forget_reached();
        let lost = (self.pending.front())
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) > STUCK);
        if lost != self.told {
            self.told = lost;
            match lost {
                true => log::warn!(
                    "gpu: a render is unfinished after {STUCK:?}; stills and captures are refused"
                ),
                false => log::info!(
                    "gpu: the unfinished render is done; stills and captures render again"
                ),
            }
        }
        lost
    }
}

fn renders(renderer: &GlesRenderer) -> &RefCell<Renders> {
    let data = renderer.egl_context().user_data();
    data.insert_if_missing(|| RefCell::new(Renders::default()));
    data.get().expect("inserted above")
}

/// Remembers a render `renderer` issued, finished once `sync` is reached.
pub fn issued(renderer: &GlesRenderer, sync: &SyncPoint) {
    renders(renderer)
        .borrow_mut()
        .issued(sync.clone(), Instant::now());
}

/// Err while a render `renderer` issued is lost (`Renders::lost`). A render off the screen's
/// pace gives up then: building its elements writes textures the lost render reads, and the
/// driver waits for that render first.
pub fn ready(renderer: &GlesRenderer) -> Result<(), String> {
    match renders(renderer).borrow_mut().lost(Instant::now()) {
        true => Err(format!("an earlier render is unfinished after {STUCK:?}")),
        false => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::renderer::sync::{Fence, Interrupted};
    use smithay::reexports::calloop::EventLoop;
    use std::io::Write;
    use std::os::fd::{AsFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A fence a test signals by hand. Its fd turns readable when `signal` writes to the other
    /// end, as a sync_file turns readable when its fence signals.
    #[derive(Debug)]
    struct HeldFence {
        fd: Option<OwnedFd>,
        signalled: Arc<AtomicBool>,
    }

    impl Fence for HeldFence {
        fn is_signaled(&self) -> bool {
            self.signalled.load(Ordering::SeqCst)
        }
        fn wait(&self) -> Result<(), Interrupted> {
            panic!("waited on a fence on the loop");
        }
        fn is_exportable(&self) -> bool {
            self.fd.is_some()
        }
        fn export(&self) -> Option<OwnedFd> {
            self.fd
                .as_ref()
                .and_then(|f| f.as_fd().try_clone_to_owned().ok())
        }
    }

    struct Held {
        sync: SyncPoint,
        signalled: Arc<AtomicBool>,
        writer: UnixStream,
        _reader: UnixStream,
    }

    fn held(exported: bool) -> Held {
        let (reader, writer) = UnixStream::pair().unwrap();
        let signalled = Arc::new(AtomicBool::new(false));
        let fence = HeldFence {
            fd: exported.then(|| OwnedFd::from(reader.try_clone().unwrap())),
            signalled: Arc::clone(&signalled),
        };
        Held {
            sync: SyncPoint::from(fence),
            signalled,
            writer,
            _reader: reader,
        }
    }

    impl Held {
        fn signal(&mut self) {
            self.signalled.store(true, Ordering::SeqCst);
            self.writer.write_all(&[1]).unwrap();
        }
    }

    /// Dispatches until `got` is set or `limit` passes; each dispatch must return promptly.
    fn run(lp: &mut EventLoop<'static, Option<bool>>, got: &mut Option<bool>, limit: Duration) {
        let start = Instant::now();
        while got.is_none() && start.elapsed() < limit {
            let t = Instant::now();
            lp.dispatch(Some(Duration::from_millis(20)), got).unwrap();
            assert!(
                t.elapsed() < Duration::from_millis(500),
                "the loop was held"
            );
        }
    }

    #[test]
    fn a_fence_that_never_signals_fails_at_the_deadline_without_holding_the_loop() {
        for exported in [true, false] {
            let mut lp = EventLoop::<Option<bool>>::try_new().unwrap();
            let h = held(exported);
            let deadline = Duration::from_millis(100);
            let start = Instant::now();
            when_reached(&lp.handle(), h.sync.clone(), deadline, |got, r| {
                *got = Some(r)
            });
            let mut got = None;
            run(&mut lp, &mut got, Duration::from_secs(5));
            assert_eq!(got, Some(false), "exported {exported}: deadline passed");
            assert!(start.elapsed() >= deadline);
        }
    }

    #[test]
    fn a_fence_that_signals_runs_its_work_once_with_reached() {
        for exported in [true, false] {
            let mut lp = EventLoop::<Option<bool>>::try_new().unwrap();
            let mut h = held(exported);
            let calls = Rc::new(RefCell::new(0));
            let c = Rc::clone(&calls);
            when_reached(
                &lp.handle(),
                h.sync.clone(),
                Duration::from_millis(200),
                move |got, r| {
                    *c.borrow_mut() += 1;
                    *got = Some(r);
                },
            );
            let mut got = None;
            lp.dispatch(Some(Duration::from_millis(10)), &mut got)
                .unwrap();
            assert_eq!(got, None, "exported {exported}: not signalled yet");
            h.signal();
            run(&mut lp, &mut got, Duration::from_secs(5));
            assert_eq!(got, Some(true), "exported {exported}");
            // Past the deadline nothing runs it a second time.
            let after = Instant::now();
            while after.elapsed() < Duration::from_millis(300) {
                lp.dispatch(Some(Duration::from_millis(20)), &mut got)
                    .unwrap();
            }
            assert_eq!(*calls.borrow(), 1, "exported {exported}");
        }
    }

    #[test]
    fn a_render_unfinished_past_stuck_is_lost_until_it_is_reached() {
        let mut h = held(true);
        let mut r = Renders::default();
        let t0 = Instant::now();
        r.issued(h.sync.clone(), t0);
        assert!(!r.lost(t0 + STUCK), "not past STUCK yet");
        assert!(r.lost(t0 + STUCK + Duration::from_millis(1)));
        h.signal();
        assert!(!r.lost(t0 + STUCK * 10), "reached");
    }

    #[test]
    fn the_oldest_unfinished_render_decides_and_reached_ones_are_forgotten() {
        let mut first = held(false);
        let second = held(false);
        let mut r = Renders::default();
        let t0 = Instant::now();
        r.issued(SyncPoint::signaled(), t0);
        r.issued(first.sync.clone(), t0);
        r.issued(second.sync.clone(), t0 + STUCK);
        assert!(r.lost(t0 + STUCK * 2), "the first is past STUCK");
        first.signal();
        assert!(!r.lost(t0 + STUCK * 2), "the second is not past STUCK yet");
        assert!(r.lost(t0 + STUCK * 3));
    }

    #[test]
    fn a_full_list_keeps_the_oldest_render() {
        let oldest = held(false);
        let rest: Vec<Held> = (0..KEPT * 2).map(|_| held(false)).collect();
        let mut r = Renders::default();
        let t0 = Instant::now();
        r.issued(oldest.sync.clone(), t0);
        for h in &rest {
            r.issued(h.sync.clone(), t0 + STUCK * 4);
        }
        assert_eq!(r.pending.len(), KEPT);
        assert!(
            r.lost(t0 + STUCK * 2),
            "the oldest render is still the one timed"
        );
    }

    #[test]
    fn a_full_list_of_reached_renders_is_emptied_by_the_next() {
        let mut r = Renders::default();
        let t0 = Instant::now();
        for _ in 0..KEPT {
            r.issued(SyncPoint::signaled(), t0);
        }
        let h = held(false);
        r.issued(h.sync.clone(), t0);
        assert_eq!(r.pending.len(), 1);
    }

    #[test]
    fn a_renderer_refuses_renders_while_one_it_issued_is_lost() {
        let r = match reprisefx::headless::renderer() {
            Ok(r) => r,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        assert_eq!(ready(&r), Ok(()));
        let mut h = held(true);
        let long_ago = Instant::now() - STUCK * 2;
        renders(&r).borrow_mut().issued(h.sync.clone(), long_ago);
        assert!(ready(&r).is_err(), "the render is lost");
        h.signal();
        assert_eq!(ready(&r), Ok(()), "the GPU caught up");
    }

    #[test]
    fn a_render_without_a_fence_is_reached_at_once() {
        let mut lp = EventLoop::<Option<bool>>::try_new().unwrap();
        when_reached(
            &lp.handle(),
            SyncPoint::signaled(),
            Duration::from_secs(5),
            |got, r| *got = Some(r),
        );
        let mut got = None;
        lp.dispatch(Some(Duration::from_millis(50)), &mut got)
            .unwrap();
        assert_eq!(got, Some(true));
    }
}
