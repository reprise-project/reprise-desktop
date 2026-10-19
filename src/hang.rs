//! Whether a window's client still answers pings.
//!
//! One ping is out per xdg client (`ShellClient::send_ping`) or X11 window (`_NET_WM_PING`) at a
//! time, timed by `PING_TIMEOUT`. A ping no longer out that never timed out was answered.

use std::collections::HashMap;
use std::os::fd::{FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    Interest, Mode, PostAction, RegistrationToken,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use smithay::reexports::wayland_server::Resource;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::PingError;
use smithay::wayland::shell::xdg::ShellClient;
use smithay::xwayland::xwm::PingError as X11PingError;

use crate::compositor::State;
use crate::dialog::Dialog;
use crate::window::{Kind, WindowId};

/// Least time between two pings input sends to one window.
pub const PING_EVERY: Duration = Duration::from_secs(2);

/// How long a client has to answer a ping (sent by input or by a close) before it counts as hung
/// and a close asked of it opens End Program.
pub const PING_TIMEOUT: Duration = Duration::from_secs(5);

/// One window's ping record.
#[derive(Debug, Default)]
pub struct Liveness {
    waiting: bool,
    last_ping: Option<Instant>,
    hung: bool,
}

impl Liveness {
    /// Input reached the window at `now`: whether to ping its client.
    pub fn input(&mut self, now: Instant, outstanding: bool) -> bool {
        self.settle(outstanding);
        !outstanding
            && self
                .last_ping
                .is_none_or(|t| now.saturating_duration_since(t) >= PING_EVERY)
    }

    /// A ping went out at `now`.
    pub fn pinged(&mut self, now: Instant) {
        self.waiting = true;
        self.last_ping = Some(now);
    }

    /// A ping went unanswered past its timeout.
    pub fn timed_out(&mut self) {
        self.waiting = false;
        self.hung = true;
    }

    /// The client showed it is running (it answered a ping or committed a new frame): whether it
    /// was hung until now.
    pub fn alive(&mut self) -> bool {
        self.waiting = false;
        std::mem::take(&mut self.hung)
    }

    /// The window is being asked to close: whether its client is already known not to answer, so
    /// End Program opens at once.
    pub fn hung(&mut self, outstanding: bool) -> bool {
        self.settle(outstanding);
        self.hung
    }

    fn settle(&mut self, outstanding: bool) {
        if self.waiting && !outstanding {
            self.waiting = false;
            self.hung = false;
        }
    }
}

fn may_end(pid: i32, own: i32, xwayland: i32) -> bool {
    pid > 1 && pid != own && pid != xwayland
}

/// The index and pid of each End Program among `dialogs` (their `end_program_pid`s) whose pid
/// `ends` picks.
fn end_programs(dialogs: &[Option<i32>], ends: impl Fn(i32) -> bool) -> Vec<(usize, i32)> {
    dialogs
        .iter()
        .enumerate()
        .filter_map(|(i, pid)| pid.filter(|&p| ends(p)).map(|p| (i, p)))
        .collect()
}

/// Whether `pid`'s main thread has exited (`/proc/<pid>/stat` state Z or X); other threads of
/// it may still run.
fn leader_exited(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|s| matches!(stat_state(&s), Some('Z' | 'X')))
}

/// The state letter of a `/proc/<pid>/stat` line; the command name before it may hold `)`.
fn stat_state(stat: &str) -> Option<char> {
    stat.rsplit_once(')')?.1.trim_start().chars().next()
}

/// Whether `pid` is being torn down: a thread of it has SIGKILL pending, as a fatal signal and
/// `exit_group` leave on every thread they end. A thread stuck on a dead mount keeps it pending.
fn killed(pid: i32) -> bool {
    std::fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|t| {
            std::fs::read_to_string(t.path().join("status")).is_ok_and(|s| sigkill_pending(&s))
        })
}

/// Whether a `/proc/<pid>/task/<tid>/status` holds SIGKILL in its pending masks.
fn sigkill_pending(status: &str) -> bool {
    let bit = 1u64 << (libc::SIGKILL - 1);
    status
        .lines()
        .filter_map(|l| {
            l.strip_prefix("SigPnd:")
                .or_else(|| l.strip_prefix("ShdPnd:"))
        })
        .filter_map(|v| u64::from_str_radix(v.trim(), 16).ok())
        .any(|mask| mask & bit != 0)
}

/// Threads of a program watched for exit; one other than a stuck thread is enough.
const WATCHED_THREADS: usize = 4;

/// Pidfds that turn readable when a thread of `pid` other than its main one exits
/// (`PIDFD_THREAD`, Linux 6.9+), for its `WATCHED_THREADS` oldest. A main thread's pidfd stays
/// quiet while any other thread lives, so it is not taken.
fn thread_pidfds(pid: i32) -> Vec<(i32, OwnedFd)> {
    let mut tids: Vec<i32> = std::fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|t| t.file_name().to_str()?.parse().ok())
        .filter(|&t| t != pid)
        .collect();
    tids.sort_unstable();
    tids.into_iter()
        .take(WATCHED_THREADS)
        .filter_map(|tid| match thread_pidfd(tid) {
            Ok(fd) => Some((tid, fd)),
            Err(e) => {
                log::debug!("pidfd_open {tid}: {e}");
                None
            }
        })
        .collect()
}

fn thread_pidfd(tid: i32) -> std::io::Result<OwnedFd> {
    // SAFETY: pidfd_open takes a tid and flags, no pointers.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, tid, libc::PIDFD_THREAD) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a descriptor pidfd_open just returned; nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

fn program_name(pid: i32) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()
        .and_then(|exe| exe_name(&exe.to_string_lossy()))
        .or_else(|| {
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
            Some(comm.trim_end().to_string()).filter(|c| !c.is_empty())
        })
}

fn exe_name(link: &str) -> Option<String> {
    let path = link.strip_suffix(" (deleted)").unwrap_or(link);
    let name = path.rsplit('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

#[derive(Default)]
pub struct Hang {
    windows: HashMap<WindowId, WindowHang>,
    pings: HashMap<Target, RegistrationToken>,
    /// The threads watched per client pid with a window.
    threads: HashMap<i32, Threads>,
    next_key: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
/// An xdg client answers for all its windows; an X11 window answers for itself.
enum Target {
    Client(u64),
    X11(u32),
}

#[derive(Default)]
struct WindowHang {
    liveness: Liveness,
    close_asked: bool,
}

#[derive(Clone, Copy)]
struct ClientKey(u64);

#[derive(Default)]
struct Threads {
    /// The pidfd source per watched tid still running.
    watched: Vec<(i32, RegistrationToken)>,
    /// Its connections were dropped when the first thread left on the kill.
    dropped: bool,
}

impl State {
    fn client_key(&mut self, client: &ShellClient) -> Option<u64> {
        let next = &mut self.hang.next_key;
        client
            .with_data(|d| {
                d.insert_if_missing(|| {
                    *next += 1;
                    ClientKey(*next)
                });
                d.get::<ClientKey>().map(|k| k.0)
            })
            .ok()
            .flatten()
    }

    fn ping_target(&mut self, w: WindowId) -> Option<Target> {
        let win = self.win(w)?;
        match &win.kind {
            Kind::Xdg(t) => {
                let client = t.client();
                self.client_key(&client).map(Target::Client)
            }
            Kind::X11(x) if !win.override_redirect => Some(Target::X11(x.window_id())),
            Kind::X11(_) => None,
        }
    }

    fn ping_outstanding(&mut self, w: WindowId) -> bool {
        self.ping_target(w)
            .is_some_and(|t| self.hang.pings.contains_key(&t))
    }

    fn window_hang(&mut self, w: WindowId) -> &mut WindowHang {
        let windows = &self.windows;
        self.hang
            .windows
            .retain(|id, _| windows.iter().any(|x| x.id == *id));
        self.hang.windows.entry(w).or_default()
    }

    /// Ask window `w`'s client to close it, and ping it: a client that does not answer within
    /// `PING_TIMEOUT` gets End Program. A ping input sent earlier and still out stands in for the
    /// close's own, so its timeout comes sooner.
    pub fn ping_on_close(&mut self, w: WindowId) {
        let outstanding = self.ping_outstanding(w);
        let h = self.window_hang(w);
        h.close_asked = true;
        let hung = h.liveness.hung(outstanding);
        if !outstanding {
            self.ping(w);
        }
        if hung {
            self.offer_end(w);
        }
    }

    /// Input reached window `w`: ping its client when one is due (`Liveness::input`), so a client
    /// that stops answering is known before the window is asked to close.
    pub fn input_reached(&mut self, w: WindowId) {
        if !self.win(w).is_some_and(|x| x.mapped) {
            return;
        }
        let outstanding = self.ping_outstanding(w);
        if self
            .window_hang(w)
            .liveness
            .input(Instant::now(), outstanding)
        {
            self.ping(w);
        }
    }

    /// Window `w` committed: its client is running.
    pub fn window_alive(&mut self, w: WindowId) {
        self.recovered(&[w]);
    }

    /// Windows `ws` showed their client is running: close End Program for each that was hung.
    fn recovered(&mut self, ws: &[WindowId]) {
        let mut pids = Vec::new();
        for &w in ws {
            let was_hung = self
                .hang
                .windows
                .get_mut(&w)
                .is_some_and(|h| h.liveness.alive());
            if was_hung && let Some(pid) = self.client_pid(w) {
                pids.push(pid);
            }
        }
        if !pids.is_empty() {
            self.close_end_programs(|p| pids.contains(&p), "is responding again");
        }
    }

    /// Ping window `w`'s client and note it when one went out. An X11 client without
    /// `_NET_WM_PING` gets none, so never End Program. One whose last ping timed out still has it
    /// out (Smithay sends one at a time); the wait starts again on that one.
    fn ping(&mut self, w: WindowId) {
        let Some(target) = self.ping_target(w) else {
            return;
        };
        let Some(win) = self.win(w) else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let sent = match &win.kind {
            Kind::Xdg(t) => match t.client().send_ping(serial) {
                Ok(()) | Err(PingError::PingAlreadyPending(_)) => true,
                Err(PingError::DeadSurface) => false,
            },
            // The timestamp must not be 0 (CurrentTime).
            Kind::X11(x) => match x.send_ping(u32::from(serial).max(1)) {
                Ok(()) | Err(X11PingError::PingAlreadyPending(_)) => true,
                Err(e) => {
                    log::debug!("x11 ping: {e}");
                    false
                }
            },
        };
        if !sent {
            return;
        }
        let timer =
            self.handle
                .insert_source(Timer::from_duration(PING_TIMEOUT), move |_, _, state| {
                    state.ping_timed_out(target);
                    TimeoutAction::Drop
                });
        match timer {
            Ok(t) => {
                if let Some(old) = self.hang.pings.insert(target, t) {
                    self.handle.remove(old);
                }
            }
            Err(e) => log::warn!("ping timer: {}", e.error),
        }
        self.window_hang(w).liveness.pinged(Instant::now());
    }

    /// `client` answered its ping.
    pub fn pong(&mut self, client: ShellClient) {
        if let Some(key) = self.client_key(&client) {
            self.answered(Target::Client(key));
        }
    }

    /// X11 window `window` answered its `_NET_WM_PING`.
    pub fn x11_pong(&mut self, window: u32) {
        self.answered(Target::X11(window));
    }

    fn answered(&mut self, target: Target) {
        if let Some(t) = self.hang.pings.remove(&target) {
            self.handle.remove(t);
        }
        let ws = self.windows_of(target);
        self.recovered(&ws);
    }

    fn windows_of(&mut self, target: Target) -> Vec<WindowId> {
        let ids: Vec<WindowId> = self.windows.iter().map(|w| w.id).collect();
        ids.into_iter()
            .filter(|&w| self.ping_target(w) == Some(target))
            .collect()
    }

    /// Its windows are hung until it answers a ping or commits a frame.
    fn ping_timed_out(&mut self, target: Target) {
        self.hang.pings.remove(&target);
        let ws = self.windows_of(target);
        if ws.first().is_some_and(|&w| self.drop_exited(w)) {
            return;
        }
        for w in ws {
            self.window_hang(w).liveness.timed_out();
            self.offer_end(w);
        }
    }

    /// A program whose main thread has exited never answers again, but a thread of it stuck on a
    /// dead mount keeps its socket open: disconnect the client behind `w` so its windows go.
    fn drop_exited(&mut self, w: WindowId) -> bool {
        let Some(pid) = self
            .client_pid(w)
            .filter(|&p| self.may_end(p) && leader_exited(p))
        else {
            return false;
        };
        self.disconnect(w, pid)
    }

    /// Watch threads of mapped window `w`'s program (`thread_pidfds`), so a kill that leaves a
    /// thread stuck is known when the others exit rather than when a ping times out.
    pub fn watch_threads(&mut self, w: WindowId) {
        let Some(pid) = self.client_pid(w).filter(|&p| self.may_end(p)) else {
            return;
        };
        if self.hang.threads.contains_key(&pid) {
            return;
        }
        let mut threads = Threads::default();
        for (tid, fd) in thread_pidfds(pid) {
            let source = Generic::new(fd, Interest::READ, Mode::Level);
            match self.handle.insert_source(source, move |_, _, state| {
                state.thread_left(pid, tid);
                Ok(PostAction::Remove)
            }) {
                Ok(t) => threads.watched.push((tid, t)),
                Err(e) => log::warn!("pidfd source: {}", e.error),
            }
        }
        self.hang.threads.insert(pid, threads);
    }

    /// Thread `tid` of `pid` exited. A program being killed loses its connections now; a thread
    /// that ended by itself changes nothing.
    fn thread_left(&mut self, pid: i32, tid: i32) {
        let Some(threads) = self.hang.threads.get_mut(&pid) else {
            return;
        };
        threads.watched.retain(|&(t, _)| t != tid);
        if threads.dropped || !killed(pid) {
            return;
        }
        threads.dropped = true;
        log::info!("pid {pid}: killed, thread {tid} exited");
        let ws: Vec<WindowId> = self
            .windows
            .iter()
            .map(|w| w.id)
            .filter(|&w| self.client_pid(w) == Some(pid))
            .collect();
        let mut seen = Vec::new();
        for w in ws {
            if let Some(target) = self.ping_target(w).filter(|t| !seen.contains(t)) {
                seen.push(target);
                self.disconnect(w, pid);
            }
        }
    }

    /// Disconnect the client behind window `w`, program `pid`, which has exited or is being killed.
    fn disconnect(&mut self, w: WindowId, pid: i32) -> bool {
        let Some(win) = self.win(w) else {
            return false;
        };
        log::info!(
            "window {w:?}: pid {pid} has exited or is being killed; dropping its connection"
        );
        match &win.kind {
            Kind::Xdg(t) => {
                if let Some(c) = t.wl_surface().client() {
                    self.drop_client(c.id());
                }
            }
            Kind::X11(x) => self.xwayland.kill_owner(x.window_id(), pid as u32),
        }
        true
    }

    /// Whether End Program may kill `pid`: never the compositor itself or the Xwayland server under
    /// every X11 window.
    pub fn may_end(&self, pid: i32) -> bool {
        may_end(pid, std::process::id() as i32, self.xwayland.pid())
    }

    /// Only a window asked to close gets End Program, once per close.
    fn offer_end(&mut self, w: WindowId) {
        let Some((title, app_id)) = self
            .win(w)
            .filter(|x| x.mapped)
            .map(|x| (x.title(), x.app_id()))
        else {
            return;
        };
        let h = self.window_hang(w);
        if !h.close_asked {
            return;
        }
        h.close_asked = false;
        if self.ending.is_some() {
            log::info!("window {w:?} ({app_id:?}) is not responding; the session end lists it");
            return;
        }
        let Some(pid) = self.client_pid(w).filter(|&p| self.may_end(p)) else {
            log::info!("window {w:?} ({app_id:?}) is not responding; no pid to end");
            return;
        };
        log::info!("window {w:?} ({app_id:?}, pid {pid}) is not responding");
        let name = program_name(pid).unwrap_or(title);
        self.open_dialog(Dialog::end_program(&name, pid));
    }

    /// A window went away: close each End Program and thread watch whose program has no window
    /// left.
    pub fn close_ended_programs(&mut self) {
        let live: Vec<i32> = self
            .windows
            .iter()
            .filter_map(|w| self.client_pid(w.id))
            .collect();
        let gone: Vec<i32> = self
            .hang
            .threads
            .keys()
            .copied()
            .filter(|p| !live.contains(p))
            .collect();
        for pid in gone {
            for (_, t) in self.hang.threads.remove(&pid).unwrap_or_default().watched {
                self.handle.remove(t);
            }
        }
        self.close_end_programs(|p| !live.contains(&p), "has no window left");
    }

    fn close_end_programs(&mut self, ends: impl Fn(i32) -> bool, why: &str) {
        let pids: Vec<Option<i32>> = self
            .dialogs
            .views
            .iter()
            .map(|v| v.dialog.end_program_pid())
            .collect();
        for (i, pid) in end_programs(&pids, ends).into_iter().rev() {
            log::info!("end program: pid {pid} {why}; closing");
            self.close_dialog_at(i);
        }
    }

    /// End Program's End Now: SIGKILL `pid`, as long as a window still belongs to it. The program
    /// may have gone and its pid been reused while the dialog waited; a window holding the pid
    /// means it is still the one that stopped responding. Its Wayland connections are dropped too:
    /// a thread of it blocked on a dead mount outlives the kill and keeps their sockets open.
    pub fn end_program(&mut self, pid: i32) {
        let owns = self
            .windows
            .iter()
            .any(|w| self.client_pid(w.id) == Some(pid));
        if !owns || !self.may_end(pid) {
            log::info!("end program: pid {pid} has no window left");
            return;
        }
        log::info!("end program: killing pid {pid}");
        // SAFETY: plain syscall on a pid, no pointers.
        if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
            log::warn!(
                "end program: kill {pid}: {}",
                std::io::Error::last_os_error()
            );
        }
        self.drop_clients_of(pid);
    }

    /// Disconnects the Wayland clients behind `pid`'s xdg windows; X11 windows share Xwayland's.
    fn drop_clients_of(&mut self, pid: i32) {
        let mut clients = Vec::new();
        for w in self.windows.iter() {
            if let Kind::Xdg(t) = &w.kind
                && self.client_pid(w.id) == Some(pid)
                && let Some(c) = t.wl_surface().client()
                && !clients.contains(&c.id())
            {
                clients.push(c.id());
            }
        }
        for id in clients {
            self.drop_client(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn the_first_input_pings_and_later_input_waits_two_seconds() {
        let t0 = Instant::now();
        let mut l = Liveness::default();
        assert!(l.input(t0, false), "first input pings");
        l.pinged(t0);
        assert!(!l.input(at(t0, 100), true), "no ping while one is out");
        assert!(
            !l.input(at(t0, 1500), false),
            "answered, but 1.5 s is too soon"
        );
        assert!(l.input(at(t0, 2000), false), "pings again after 2 s");
    }

    #[test]
    fn an_unanswered_ping_marks_the_window_hung_until_the_next_is_answered() {
        let t0 = Instant::now();
        let mut l = Liveness::default();
        l.pinged(t0);
        assert!(!l.hung(true), "a ping still out is not hung yet");
        l.timed_out();
        assert!(l.hung(false), "hung after the timeout");
        assert!(
            l.input(at(t0, 5000), false),
            "input pings a hung window again"
        );
        l.pinged(at(t0, 5000));
        assert!(l.hung(true), "still hung while the new ping is out");
        assert!(!l.hung(false), "the new ping answered clears it");
    }

    #[test]
    fn the_stat_state_follows_the_last_parenthesis() {
        assert_eq!(stat_state("42 (a) b) Z 1 42"), Some('Z'));
        assert_eq!(stat_state("42 (foot) S 1 42"), Some('S'));
        assert_eq!(stat_state("garbage"), None);
    }

    #[test]
    fn a_child_left_unreaped_has_exited_and_a_running_one_has_not() {
        let mut running = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        assert!(!leader_exited(running.id() as i32), "running sleep");
        running.kill().unwrap();
        running.wait().unwrap();

        let mut exited = std::process::Command::new("true").spawn().unwrap();
        let pid = exited.id() as i32;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !leader_exited(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(leader_exited(pid), "unreaped true is a zombie");
        exited.wait().unwrap();
    }

    #[test]
    fn sigkill_shows_in_either_pending_mask() {
        let status = |sig: &str, shd: &str| format!("Name:\tx\nSigPnd:\t{sig}\nShdPnd:\t{shd}\n");
        assert!(sigkill_pending(&status(
            "0000000000000100",
            "0000000000000000"
        )));
        assert!(sigkill_pending(&status(
            "0000000000000000",
            "0000000000000100"
        )));
        assert!(!sigkill_pending(&status(
            "0000000000004002",
            "0000000000000000"
        )));
        assert!(!sigkill_pending("Name:\tx\n"));
    }

    fn readable(fd: &OwnedFd, ms: i32) -> bool {
        use std::os::fd::AsRawFd;
        let mut p = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        unsafe { libc::poll(&mut p, 1, ms) == 1 }
    }

    #[test]
    fn a_thread_pidfd_fires_when_its_thread_exits_and_a_lone_exit_is_no_kill() {
        extern "C" fn sleeper(_: *mut libc::c_void) -> *mut libc::c_void {
            // SAFETY: plain libc calls in the forked child.
            unsafe { libc::sleep(30) };
            std::ptr::null_mut()
        }
        extern "C" fn leaver(go: *mut libc::c_void) -> *mut libc::c_void {
            let mut b = 0u8;
            // SAFETY: reads one byte into a live buffer from the fd passed as the argument.
            unsafe { libc::read(go as i32, (&raw mut b).cast(), 1) };
            std::ptr::null_mut()
        }
        let mut go = [0; 2];
        // SAFETY: `go` holds two fds.
        assert_eq!(unsafe { libc::pipe(go.as_mut_ptr()) }, 0);
        let (ready_r, ready_w) = {
            let mut r = [0; 2];
            // SAFETY: `r` holds two fds.
            assert_eq!(unsafe { libc::pipe(r.as_mut_ptr()) }, 0);
            (r[0], r[1])
        };
        // SAFETY: the child only makes libc calls and never returns into the test.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // SAFETY: as above.
            unsafe {
                let mut t = 0;
                libc::pthread_create(&mut t, std::ptr::null(), sleeper, std::ptr::null_mut());
                libc::pthread_create(&mut t, std::ptr::null(), leaver, go[0] as *mut _);
                libc::write(ready_w, [1u8].as_ptr().cast(), 1);
                libc::pause();
                libc::_exit(0);
            }
        }
        let mut b = 0u8;
        // SAFETY: reads one byte into a live buffer.
        unsafe { libc::read(ready_r, (&raw mut b).cast(), 1) };
        let fds = thread_pidfds(pid);
        let reap = || {
            // SAFETY: plain syscalls on our own child and pipes.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
                for fd in [go[0], go[1], ready_r, ready_w] {
                    libc::close(fd);
                }
            }
        };
        if fds.is_empty() {
            reap();
            eprintln!("no PIDFD_THREAD on this kernel");
            return;
        }
        assert_eq!(
            fds.len(),
            2,
            "both threads besides the main one are watched"
        );
        assert!(
            fds.iter().all(|(_, fd)| !readable(fd, 50)),
            "quiet while all run"
        );
        // SAFETY: writes one byte from a live buffer.
        unsafe { libc::write(go[1], [1u8].as_ptr().cast(), 1) };
        let fired = fds.iter().filter(|(_, fd)| readable(fd, 2000)).count();
        let ended = fds.iter().filter(|(_, fd)| readable(fd, 0)).count();
        assert_eq!(
            (fired, ended),
            (1, 1),
            "only the thread that returned fires"
        );
        assert!(!killed(pid), "a thread ending by itself is no kill");
        reap();
    }

    #[test]
    fn a_new_frame_clears_hung() {
        let mut l = Liveness::default();
        l.pinged(Instant::now());
        l.timed_out();
        l.alive();
        assert!(!l.hung(false), "not hung after a commit");
    }

    #[test]
    fn a_window_that_never_pinged_is_not_hung() {
        let mut l = Liveness::default();
        assert!(!l.hung(false), "not hung without a ping");
        assert!(!l.hung(true), "another window's ping out is not a timeout");
    }

    /// End Program never kills init, the compositor or the Xwayland server; any other live pid is
    /// fair game.
    #[test]
    fn end_program_spares_the_session() {
        assert!(may_end(4321, 100, 200));
        assert!(!may_end(0, 100, 200));
        assert!(!may_end(1, 100, 200));
        assert!(!may_end(100, 100, 200), "never the compositor");
        assert!(!may_end(200, 100, 200), "never the Xwayland server");
        assert!(may_end(200, 100, 0), "no Xwayland spares no pid");
    }

    #[test]
    fn end_program_closes_once_its_program_has_no_window() {
        let dialogs = [Some(10), None, Some(20), Some(10)];
        let orphaned = |live: &[i32]| end_programs(&dialogs, |p| !live.contains(&p));
        assert_eq!(orphaned(&[20, 30]), vec![(0, 10), (3, 10)]);
        assert_eq!(orphaned(&[10, 20]), vec![]);
        assert_eq!(orphaned(&[]), vec![(0, 10), (2, 20), (3, 10)]);
    }

    #[test]
    fn a_hung_window_answering_again_reports_its_recovery_once() {
        let mut l = Liveness::default();
        assert!(!l.alive(), "a window never hung did not recover");
        l.pinged(Instant::now());
        l.timed_out();
        assert!(l.alive(), "the first answer after a timeout is a recovery");
        assert!(!l.alive(), "a second answer is not");
        assert!(!l.hung(false), "not hung after recovering");
    }

    /// The caption names the executable, without the kernel's " (deleted)" mark on a binary
    /// replaced since it started.
    #[test]
    fn exe_name_is_the_file_name() {
        assert_eq!(
            exe_name("/app/Signal/signal-desktop").as_deref(),
            Some("signal-desktop")
        );
        assert_eq!(exe_name("/usr/bin/foot (deleted)").as_deref(), Some("foot"));
        assert_eq!(exe_name("/"), None);
    }
}
