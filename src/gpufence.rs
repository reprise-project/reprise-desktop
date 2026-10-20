//! Waiting for the GPU to finish a render without blocking the loop: a capture's buffer or a
//! picture read back waits for its fence on the loop, or for a bounded time. A fence that never
//! signals (a GPU lost across a suspend) costs that one result.

use std::cell::RefCell;
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
