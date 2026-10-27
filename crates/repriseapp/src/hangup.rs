//! Hangs up the Wayland connection before the process ends: a thread blocked on a dead mount keeps
//! the fd table past exit, and with it the socket and the program's windows.

use smithay_client_toolkit::reexports::calloop::{
    Interest, LoopHandle, Mode, PostAction, generic::Generic,
};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicI32, Ordering};

static SOCKET: AtomicI32 = AtomicI32::new(-1);
/// The write end of the pipe that hands an ending signal to the run loop, or -1.
static WAKE: AtomicI32 = AtomicI32::new(-1);
/// The signal handed to the run loop, or 0 while none was.
static PENDING: AtomicI32 = AtomicI32::new(0);

/// Seconds the loop has to pick up a routed signal; a loop stuck on a dead mount never would.
const GRACE_S: libc::c_uint = 3;
/// Seconds the rescue has once the loop picked the signal up; below systemd's stop timeout.
const RESCUE_S: libc::c_uint = 10;

/// The signals another process ends a program with. SIGKILL can't be caught: End Program drops
/// the connection itself.
const ENDING: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// Hangs up `socket` if the program is ended by a signal. A signal it started with ignored
/// (`nohup`, a shell's background job) stays ignored.
pub(crate) fn on_signals(socket: BorrowedFd) {
    SOCKET.store(socket.as_raw_fd(), Ordering::Release);
    for sig in ENDING {
        // SAFETY: the handler calls only async-signal-safe functions; the structs are zeroed and
        // filled before the calls read them.
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(sig, std::ptr::null(), &mut old) == 0
                && old.sa_sigaction == libc::SIG_IGN
            {
                continue;
            }
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_ending as *const () as libc::sighandler_t;
            sa.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            if libc::sigaction(sig, &sa, std::ptr::null_mut()) != 0 {
                log::warn!(
                    "no handler for signal {sig}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
}

/// Hands the first ending signal to `handle`'s loop, which runs `rescue` and then ends the
/// program by that signal. A second signal, [`GRACE_S`] without the loop, or a rescue past
/// [`RESCUE_S`] ends it at once.
pub(crate) fn route_to<D>(handle: &LoopHandle<'_, D>, mut rescue: impl FnMut(&mut D) + 'static) {
    let mut fds = [-1; 2];
    // SAFETY: pipe2 fills `fds` on success.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        log::warn!("no signal pipe: {}", std::io::Error::last_os_error());
        return;
    }
    // SAFETY: both fds were just opened and are owned here alone.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    let source = Generic::new(read, Interest::READ, Mode::Level);
    let inserted = handle.insert_source(source, move |_, _, data: &mut D| {
        let sig = PENDING.load(Ordering::Acquire);
        if sig == 0 {
            return Ok(PostAction::Continue);
        }
        // SAFETY: alarm replaces the pending grace alarm with the rescue budget.
        unsafe { libc::alarm(RESCUE_S) };
        rescue(data);
        end_by(sig);
        Ok(PostAction::Remove)
    });
    match inserted {
        Ok(_) => WAKE.store(
            std::os::fd::IntoRawFd::into_raw_fd(write),
            Ordering::Release,
        ),
        Err(e) => log::warn!("no signal source: {e}"),
    }
}

/// Hangs up `socket` whoever else holds it: the compositor reads the end of the connection.
pub(crate) fn hang_up(socket: BorrowedFd) {
    SOCKET.store(-1, Ordering::Release);
    // SAFETY: plain syscall on an open fd.
    unsafe { libc::shutdown(socket.as_raw_fd(), libc::SHUT_RDWR) };
}

extern "C" fn on_ending(sig: libc::c_int) {
    let wake = WAKE.load(Ordering::Acquire);
    if wake >= 0
        && PENDING
            .compare_exchange(0, sig, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        // SIGALRM is taken only now, so the program's own use of it before an ending signal
        // keeps its handler.
        // SAFETY: write, sigemptyset, sigaction and alarm are async-signal-safe; the pipe is
        // non-blocking and the struct is zeroed and filled before sigaction reads it.
        unsafe {
            libc::write(wake, [1u8].as_ptr().cast(), 1);
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_alarm as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut sa.sa_mask);
            libc::sigaction(libc::SIGALRM, &sa, std::ptr::null_mut());
            libc::alarm(GRACE_S);
        }
        return;
    }
    end_by(sig);
}

/// Ends the program by the routed signal its loop did not get to.
extern "C" fn on_alarm(_: libc::c_int) {
    match PENDING.load(Ordering::Acquire) {
        0 => end_by(libc::SIGALRM),
        sig => end_by(sig),
    }
}

/// Hangs up and ends the program as `sig`'s default action would. Async-signal-safe.
fn end_by(sig: libc::c_int) {
    let fd = SOCKET.load(Ordering::Acquire);
    // SAFETY: shutdown, sigaction and raise are async-signal-safe; the struct is zeroed, which is
    // SIG_DFL with an empty mask. In the handler the raise lands once it returns.
    unsafe {
        if fd >= 0 {
            libc::shutdown(fd, libc::SHUT_RDWR);
        }
        let dfl: libc::sigaction = std::mem::zeroed();
        libc::sigaction(sig, &dfl, std::ptr::null_mut());
        libc::raise(sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn ends(mut peer: UnixStream) -> bool {
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        matches!(peer.read(&mut [0; 8]), Ok(0))
    }

    #[test]
    fn a_hang_up_ends_the_connection_while_another_fd_still_holds_it() {
        let (ours, peer) = UnixStream::pair().unwrap();
        let held = ours.try_clone().unwrap();
        hang_up(ours.as_fd());
        assert!(ends(peer), "the peer reads the end of the connection");
        drop(held);
    }

    #[test]
    fn a_program_ended_by_sigterm_hangs_up_though_its_socket_outlives_it() {
        let (ours, peer) = UnixStream::pair().unwrap();
        // SAFETY: the child calls only sigaction, raise and _exit before it ends.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            on_signals(ours.as_fd());
            // SAFETY: raise and _exit in the forked child.
            unsafe {
                libc::raise(libc::SIGTERM);
                libc::_exit(0);
            }
        }
        let mut status = 0;
        // SAFETY: waits on the child forked above.
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(
            libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGTERM,
            "the child ends by SIGTERM as before: status {status:#x}"
        );
        assert!(
            ends(peer),
            "the peer reads the end while this process still holds the child's socket"
        );
        drop(ours);
    }

    const CHILD: &str = "REPRISE_HANGUP_CHILD";

    #[test]
    fn a_sigterm_runs_the_rescue_on_the_loop_then_ends_the_program_by_it() {
        use smithay_client_toolkit::reexports::calloop::EventLoop;
        if std::env::var_os(CHILD).is_some() {
            let (ours, _peer) = UnixStream::pair().unwrap();
            let mut event_loop: EventLoop<Vec<&str>> = EventLoop::try_new().unwrap();
            on_signals(ours.as_fd());
            route_to(&event_loop.handle(), |saved: &mut Vec<&str>| {
                saved.push("rescued");
                println!("RESCUED {saved:?}");
            });
            let mut saved = vec!["typed"];
            // SAFETY: raise on this process.
            unsafe { libc::raise(libc::SIGTERM) };
            for _ in 0..50 {
                event_loop
                    .dispatch(Duration::from_millis(20), &mut saved)
                    .unwrap();
            }
            println!("SURVIVED");
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hangup::tests::a_sigterm_runs_the_rescue_on_the_loop_then_ends_the_program_by_it",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(r#"RESCUED ["typed", "rescued"]"#),
            "the rescue runs with the loop's data: {stdout}"
        );
        assert!(!stdout.contains("SURVIVED"), "the program ends: {stdout}");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            out.status.signal(),
            Some(libc::SIGTERM),
            "the program ends by SIGTERM: {:?}",
            out.status
        );
    }

    #[test]
    fn a_sigterm_ends_the_program_when_its_loop_never_runs() {
        use smithay_client_toolkit::reexports::calloop::EventLoop;
        if std::env::var_os(CHILD).is_some() {
            let (ours, _peer) = UnixStream::pair().unwrap();
            let event_loop: EventLoop<()> = EventLoop::try_new().unwrap();
            on_signals(ours.as_fd());
            route_to(&event_loop.handle(), |_| println!("RESCUED"));
            // SAFETY: raise on this process.
            unsafe { libc::raise(libc::SIGTERM) };
            std::thread::sleep(Duration::from_secs(10));
            println!("SURVIVED");
            return;
        }
        let started = std::time::Instant::now();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hangup::tests::a_sigterm_ends_the_program_when_its_loop_never_runs",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains("SURVIVED"), "the program ends: {stdout}");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(out.status.signal(), Some(libc::SIGTERM), "{:?}", out.status);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it ends 3 s after the signal: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_rescue_longer_than_the_grace_runs_to_its_end() {
        use smithay_client_toolkit::reexports::calloop::EventLoop;
        if std::env::var_os(CHILD).is_some() {
            let (ours, _peer) = UnixStream::pair().unwrap();
            let mut event_loop: EventLoop<()> = EventLoop::try_new().unwrap();
            on_signals(ours.as_fd());
            route_to(&event_loop.handle(), |_| {
                std::thread::sleep(Duration::from_secs(4));
                println!("RESCUE DONE");
            });
            // SAFETY: raise on this process.
            unsafe { libc::raise(libc::SIGTERM) };
            for _ in 0..50 {
                event_loop
                    .dispatch(Duration::from_millis(20), &mut ())
                    .unwrap();
            }
            println!("SURVIVED");
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hangup::tests::a_rescue_longer_than_the_grace_runs_to_its_end",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("RESCUE DONE"),
            "the rescue finishes: {stdout}"
        );
        assert!(!stdout.contains("SURVIVED"), "the program ends: {stdout}");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(out.status.signal(), Some(libc::SIGTERM), "{:?}", out.status);
    }

    #[test]
    fn a_routed_sigterm_restarts_a_worker_threads_blocking_read() {
        use smithay_client_toolkit::reexports::calloop::EventLoop;
        use std::os::unix::thread::JoinHandleExt;
        if std::env::var_os(CHILD).is_some() {
            let (ours, _peer) = UnixStream::pair().unwrap();
            let mut event_loop: EventLoop<()> = EventLoop::try_new().unwrap();
            on_signals(ours.as_fd());
            route_to(&event_loop.handle(), |_| {});
            let mut fds = [-1; 2];
            // SAFETY: pipe fills `fds` on success.
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            let worker = std::thread::spawn(move || {
                let mut b = [0u8; 1];
                // SAFETY: reads one byte into `b` from the pipe opened above.
                let n = unsafe { libc::read(fds[0], b.as_mut_ptr().cast(), 1) };
                println!("READ {n}");
            });
            std::thread::sleep(Duration::from_millis(200));
            // SAFETY: the worker is alive and blocked in read; write goes to our own pipe.
            unsafe {
                libc::pthread_kill(worker.as_pthread_t(), libc::SIGTERM);
                std::thread::sleep(Duration::from_millis(200));
                libc::write(fds[1], [1u8].as_ptr().cast(), 1);
            }
            worker.join().unwrap();
            for _ in 0..50 {
                event_loop
                    .dispatch(Duration::from_millis(20), &mut ())
                    .unwrap();
            }
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hangup::tests::a_routed_sigterm_restarts_a_worker_threads_blocking_read",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("READ 1"),
            "the read gets its byte, not EINTR: {stdout}"
        );
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(out.status.signal(), Some(libc::SIGTERM), "{:?}", out.status);
    }

    #[test]
    fn a_sigalrm_before_any_ending_signal_reaches_the_programs_own_handler() {
        use smithay_client_toolkit::reexports::calloop::EventLoop;
        use std::sync::atomic::AtomicBool;
        static RANG: AtomicBool = AtomicBool::new(false);
        extern "C" fn own(_: libc::c_int) {
            RANG.store(true, Ordering::Release);
        }
        if std::env::var_os(CHILD).is_some() {
            // SAFETY: installs a handler that only stores an atomic.
            unsafe { libc::signal(libc::SIGALRM, own as *const () as libc::sighandler_t) };
            let (ours, _peer) = UnixStream::pair().unwrap();
            let event_loop: EventLoop<()> = EventLoop::try_new().unwrap();
            on_signals(ours.as_fd());
            route_to(&event_loop.handle(), |_| {});
            // SAFETY: raise on this process.
            unsafe { libc::raise(libc::SIGALRM) };
            println!("RANG {}", RANG.load(Ordering::Acquire));
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hangup::tests::a_sigalrm_before_any_ending_signal_reaches_the_programs_own_handler",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("RANG true"), "{stdout} {:?}", out.status);
        assert!(out.status.success(), "{:?}", out.status);
    }

    #[test]
    fn a_program_started_with_sighup_ignored_survives_it() {
        let (ours, peer) = UnixStream::pair().unwrap();
        // SAFETY: the child calls only signal, sigaction, raise and _exit before it ends.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            // SAFETY: signal, raise and _exit in the forked child.
            unsafe { libc::signal(libc::SIGHUP, libc::SIG_IGN) };
            on_signals(ours.as_fd());
            // SAFETY: as above.
            unsafe {
                libc::raise(libc::SIGHUP);
                libc::_exit(7);
            }
        }
        let mut status = 0;
        // SAFETY: waits on the child forked above.
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 7,
            "the child exits by itself after the SIGHUP: status {status:#x}"
        );
        drop(ours);
        drop(peer);
    }
}
