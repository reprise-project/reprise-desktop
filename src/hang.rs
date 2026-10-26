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

/// How often programs with a window are checked for [`dying`] while a kill is suspected (a
/// thread left, a ping timed out, End Now), for one whose kill nothing else reports.
const DYING_EVERY: Duration = Duration::from_secs(2);

/// How long the [`dying`] check runs after the last suspicion that armed it.
const DYING_FOR: Duration = Duration::from_secs(30);

/// How long a program found [`killed`] when a thread left has to finish exiting before its
/// connections are dropped. `exit_group` also leaves SIGKILL pending on the threads it ends, and a
/// clean exit closes the socket well within this, so its last requests are still read.
const KILL_SETTLE: Duration = Duration::from_millis(250);

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

/// Whether `pid` never answers again: its main thread has SIGKILL pending, which a main thread
/// blocked on a dead mount keeps, or has exited (state Z or X) while the program is torn down
/// ([`killed`]) or has no other thread. A main thread that exited by itself while others run
/// (`pthread_exit`) leaves a program that still answers.
fn dying(pid: i32) -> bool {
    dying_by(
        &std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default(),
        || killed(pid) || threads(pid) <= 1,
    )
}

/// [`dying`] from a `/proc/<pid>/stat` line, with `torn_down` read only for an exited main
/// thread. Its `signal` field is the thread's own pending mask, where a fatal signal or
/// `exit_group` puts SIGKILL on every thread it ends.
fn dying_by(stat: &str, torn_down: impl FnOnce() -> bool) -> bool {
    let state = stat_fields(stat).next().and_then(|s| s.chars().next());
    stat_signal(stat).is_some_and(|m| m & sigkill_bit() != 0)
        || (matches!(state, Some('Z' | 'X')) && torn_down())
}

fn threads(pid: i32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/task")).map_or(0, |d| d.count())
}

/// The `signal` field (31) of a `/proc/<pid>/stat` line; `state` is field 3.
fn stat_signal(stat: &str) -> Option<u64> {
    stat_fields(stat).nth(28)?.parse().ok()
}

/// The fields of a `/proc/<pid>/stat` line from `state` on; the command name before them may
/// hold `)`.
fn stat_fields(stat: &str) -> impl Iterator<Item = &str> {
    stat.rsplit_once(')')
        .map_or("", |(_, rest)| rest)
        .split_ascii_whitespace()
}

fn sigkill_bit() -> u64 {
    1 << (libc::SIGKILL - 1)
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
    let bit = sigkill_bit();
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
/// (`PIDFD_THREAD`, Linux 6.9+), for its `n` oldest not in `known`. A main thread's pidfd stays
/// quiet while any other thread lives, so it is not taken.
fn thread_pidfds(pid: i32, known: &[i32], n: usize) -> Vec<(i32, OwnedFd)> {
    let mut tids: Vec<i32> = std::fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|t| t.file_name().to_str()?.parse().ok())
        .filter(|&t| t != pid && !known.contains(&t))
        .collect();
    tids.sort_unstable();
    tids.into_iter()
        .take(n)
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
    pidfd(tid, libc::PIDFD_THREAD)
}

/// A pidfd that turns readable once every thread of process `pid` has exited.
fn process_pidfd(pid: i32) -> std::io::Result<OwnedFd> {
    pidfd(pid, 0)
}

fn pidfd(pid: i32, flags: libc::c_uint) -> std::io::Result<OwnedFd> {
    // SAFETY: pidfd_open takes a pid and flags, no pointers.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, flags) };
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
    /// The timer of the [`dying`] check and when it stops, while a kill is suspected and a pid in
    /// `threads` is not yet dropped.
    dying_check: Option<(RegistrationToken, Instant)>,
    next_key: u64,
}

/// The pids the periodic [`dying`] check reads: those with a window not yet dropped. None stops
/// the timer.
fn dying_scan(threads: &HashMap<i32, Threads>) -> Vec<i32> {
    threads
        .iter()
        .filter(|(_, t)| !t.dropped)
        .map(|(&p, _)| p)
        .collect()
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
    /// The process pidfd source, until the program exits.
    exit: Option<RegistrationToken>,
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
    /// close's own, so its timeout comes sooner. A [`dying`] program's client is dropped instead.
    pub fn ping_on_close(&mut self, w: WindowId) {
        if self.drop_exited(w) {
            return;
        }
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
    /// that stops answering is known before the window is asked to close; a [`dying`] one is
    /// dropped instead.
    pub fn input_reached(&mut self, w: WindowId) {
        if !self.win(w).is_some_and(|x| x.mapped) {
            return;
        }
        let outstanding = self.ping_outstanding(w);
        if self
            .window_hang(w)
            .liveness
            .input(Instant::now(), outstanding)
            && !self.drop_exited(w)
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
    /// out (Smithay sends one at a time); the wait starts again on that one. Threads started since
    /// the last ping join the watch.
    fn ping(&mut self, w: WindowId) {
        let Some(target) = self.ping_target(w) else {
            return;
        };
        self.watch_threads(w);
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
        self.arm_dying_check();
        let ws = self.windows_of(target);
        if ws.first().is_some_and(|&w| self.drop_exited(w)) {
            return;
        }
        for w in ws {
            self.window_hang(w).liveness.timed_out();
            self.offer_end(w);
        }
    }

    /// A [`dying`] program never answers again, but a thread of it stuck on a dead mount keeps its
    /// socket open: disconnect the client behind `w` so its windows go.
    fn drop_exited(&mut self, w: WindowId) -> bool {
        let Some(pid) = self.client_pid(w).filter(|&p| self.may_end(p) && dying(p)) else {
            return false;
        };
        self.disconnect(w, pid)
    }

    /// Watch threads of window `w`'s program (`thread_pidfds`), up to `WATCHED_THREADS` running,
    /// so a kill that leaves a thread stuck is known when the others exit rather than when a ping
    /// times out, and the program's own exit (`process_pidfd`) for a socket another process
    /// still holds. Called at map and again at each ping for threads started since.
    pub fn watch_threads(&mut self, w: WindowId) {
        let Some(pid) = self.client_pid(w).filter(|&p| self.may_end(p)) else {
            return;
        };
        let mut threads = self.hang.threads.remove(&pid).unwrap_or_default();
        if threads.exit.is_none() && !threads.dropped {
            threads.exit = self.watch_exit(pid);
        }
        let known: Vec<i32> = threads.watched.iter().map(|&(t, _)| t).collect();
        let n = WATCHED_THREADS.saturating_sub(known.len());
        let fresh = if threads.dropped || n == 0 {
            Vec::new()
        } else {
            thread_pidfds(pid, &known, n)
        };
        for (tid, fd) in fresh {
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

    /// A source that drops `pid`'s connections once it has exited, after `KILL_SETTLE` so the
    /// last requests of a socket it shared with a child are still read.
    fn watch_exit(&mut self, pid: i32) -> Option<RegistrationToken> {
        let fd = process_pidfd(pid)
            .inspect_err(|e| log::debug!("pidfd_open {pid}: {e}"))
            .ok()?;
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        self.handle
            .insert_source(source, move |_, _, state| {
                if let Some(t) = state.hang.threads.get_mut(&pid) {
                    t.exit = None;
                }
                state.settle_then_drop(pid, "exited", |_| true);
                Ok(PostAction::Remove)
            })
            .inspect_err(|e| log::warn!("pidfd source: {}", e.error))
            .ok()
    }

    /// Check for [`dying`] programs every `DYING_EVERY` for the next `DYING_FOR`.
    fn arm_dying_check(&mut self) {
        let until = Instant::now() + DYING_FOR;
        if let Some((_, u)) = self.hang.dying_check.as_mut() {
            *u = until;
            return;
        }
        if dying_scan(&self.hang.threads).is_empty() {
            return;
        }
        let timer = self
            .handle
            .insert_source(Timer::from_duration(DYING_EVERY), |_, _, state| {
                state.dying_check()
            });
        match timer {
            Ok(t) => self.hang.dying_check = Some((t, until)),
            Err(e) => log::warn!("dying check timer: {}", e.error),
        }
    }

    /// Drops each program with a window found [`dying`]; the timer ends once none is left to read
    /// or `DYING_FOR` passed.
    fn dying_check(&mut self) -> TimeoutAction {
        for pid in dying_scan(&self.hang.threads) {
            if dying(pid) {
                log::info!("pid {pid}: dying, found by the periodic check");
                self.drop_program(pid);
            }
        }
        let until = self.hang.dying_check.map(|(_, u)| u);
        if dying_scan(&self.hang.threads).is_empty() || until.is_none_or(|u| Instant::now() >= u) {
            self.hang.dying_check = None;
            TimeoutAction::Drop
        } else {
            TimeoutAction::ToDuration(DYING_EVERY)
        }
    }

    /// Thread `tid` of `pid` exited. A program still [`killed`] after `KILL_SETTLE` loses its
    /// connections; a thread that ended by itself changes nothing.
    fn thread_left(&mut self, pid: i32, tid: i32) {
        let Some(threads) = self.hang.threads.get_mut(&pid) else {
            return;
        };
        threads.watched.retain(|&(t, _)| t != tid);
        if threads.dropped || !killed(pid) {
            return;
        }
        self.arm_dying_check();
        self.settle_then_drop(pid, &format!("killed, thread {tid} exited"), killed);
    }

    /// After `KILL_SETTLE`, drop `pid`'s connections if it still has some and `still(pid)`.
    fn settle_then_drop(&mut self, pid: i32, why: &str, still: fn(i32) -> bool) {
        let why = why.to_string();
        let timer =
            self.handle
                .insert_source(Timer::from_duration(KILL_SETTLE), move |_, _, state| {
                    if state.hang.threads.get(&pid).is_some_and(|t| !t.dropped) && still(pid) {
                        log::info!("pid {pid}: {why}");
                        state.drop_program(pid);
                    }
                    TimeoutAction::Drop
                });
        if let Err(e) = timer {
            log::warn!("kill settle timer: {}", e.error);
        }
    }

    /// Disconnect every client behind `pid`'s windows, once.
    fn drop_program(&mut self, pid: i32) {
        let Some(threads) = self.hang.threads.get_mut(&pid).filter(|t| !t.dropped) else {
            return;
        };
        threads.dropped = true;
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
            let threads = self.hang.threads.remove(&pid).unwrap_or_default();
            for t in threads
                .watched
                .into_iter()
                .map(|(_, t)| t)
                .chain(threads.exit)
            {
                self.handle.remove(t);
            }
        }
        if dying_scan(&self.hang.threads).is_empty()
            && let Some((t, _)) = self.hang.dying_check.take()
        {
            self.handle.remove(t);
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
        self.arm_dying_check();
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
    fn the_stat_fields_follow_the_last_parenthesis() {
        assert_eq!(stat_fields("42 (a) b) Z 1 42").next(), Some("Z"));
        assert_eq!(stat_fields("42 (foot) S 1 42").next(), Some("S"));
        assert_eq!(stat_fields("garbage").next(), None);
    }

    #[test]
    fn a_child_left_unreaped_has_exited_and_a_running_one_has_not() {
        let mut running = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        assert!(!dying(running.id() as i32), "running sleep");
        running.kill().unwrap();
        running.wait().unwrap();

        let mut exited = std::process::Command::new("true").spawn().unwrap();
        let pid = exited.id() as i32;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dying(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(dying(pid), "unreaped true is a zombie");
        exited.wait().unwrap();
    }

    #[test]
    fn a_main_thread_stuck_with_sigkill_pending_is_dying() {
        // Fields 4..=30 as 1, then `signal` (31) and four more.
        let stat = |state: char, signal: u64| {
            format!("42 (a) b) {state}{} {signal} 0 0 0 0", " 1".repeat(27))
        };
        let never = || -> bool { panic!("read only for an exited main thread") };
        assert!(
            dying_by(&stat('D', 256), never),
            "stuck on a dead mount, killed"
        );
        assert!(!dying_by(&stat('D', 0), never), "stuck, not killed");
        assert!(
            !dying_by(&stat('S', 0x4002), never),
            "running, other signals pending"
        );
        assert!(dying_by(&stat('Z', 0), || true), "exited, torn down");
        assert!(
            !dying_by(&stat('Z', 0), || false),
            "main thread exited by itself, others run"
        );
        assert!(!dying_by("", never), "gone");
    }

    #[test]
    fn a_program_whose_main_thread_exited_by_itself_is_not_dying_until_killed() {
        extern "C" fn worker(_: *mut libc::c_void) -> *mut libc::c_void {
            // SAFETY: plain libc call in the forked child.
            unsafe { libc::sleep(30) };
            std::ptr::null_mut()
        }
        // SAFETY: the child only makes libc calls and never returns into the test.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // SAFETY: as above; the exit syscall ends only the calling thread, as pthread_exit
            // does without its unwinding.
            unsafe {
                let mut t = 0;
                libc::pthread_create(&mut t, std::ptr::null(), worker, std::ptr::null_mut());
                libc::syscall(libc::SYS_exit, 0);
                libc::_exit(0);
            }
        }
        let state = || {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            stat_fields(&stat).next().and_then(|s| s.chars().next())
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while (state() != Some('Z') || threads(pid) < 2) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let exited = (state(), threads(pid));
        let alive = dying(pid);
        // SAFETY: plain syscall on our own child.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dying(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let killed = dying(pid);
        // SAFETY: reaps our own child.
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) };
        assert_eq!(exited, (Some('Z'), 2), "main thread exited, worker runs");
        assert!(!alive, "a worker still runs and answers");
        assert!(killed, "killed");
    }

    #[test]
    fn a_signal_pending_on_a_thread_shows_in_its_stat_signal_field() {
        let mut ready = [0; 2];
        // SAFETY: `ready` holds two fds.
        assert_eq!(unsafe { libc::pipe(ready.as_mut_ptr()) }, 0);
        // SAFETY: the child only makes libc calls and never returns into the test.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // SAFETY: as above; a live sigset on the stack.
            unsafe {
                let mut set = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGUSR1);
                libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
                libc::write(ready[1], [1u8].as_ptr().cast(), 1);
                libc::pause();
                libc::_exit(0);
            }
        }
        let mut b = 0u8;
        // SAFETY: reads one byte into a live buffer; then plain syscalls on our own child.
        unsafe {
            libc::read(ready[0], (&raw mut b).cast(), 1);
            libc::syscall(libc::SYS_tgkill, pid, pid, libc::SIGUSR1);
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        // SAFETY: plain syscalls on our own child and pipe.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
            libc::close(ready[0]);
            libc::close(ready[1]);
        }
        assert_eq!(stat_signal(&stat), Some(1 << (libc::SIGUSR1 - 1)));
        assert!(!dying_by(&stat, || false), "a pending SIGUSR1 is no kill");
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
    fn a_process_pidfd_fires_once_the_program_exits() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let fd = process_pidfd(child.id() as i32).unwrap();
        assert!(!readable(&fd, 0), "a running program");
        child.kill().unwrap();
        assert!(readable(&fd, 5000), "an exited program");
        child.wait().unwrap();
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
        let fds = thread_pidfds(pid, &[], WATCHED_THREADS);
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
        let rest = thread_pidfds(pid, &[fds[0].0], WATCHED_THREADS);
        assert_eq!(
            rest.iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            [fds[1].0],
            "a re-scan takes only the threads not yet watched"
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
    fn the_dying_check_reads_only_programs_not_yet_dropped_and_stops_without_one() {
        let threads = |dropped: &[(i32, bool)]| -> HashMap<i32, Threads> {
            dropped
                .iter()
                .map(|&(p, dropped)| {
                    (
                        p,
                        Threads {
                            dropped,
                            ..Threads::default()
                        },
                    )
                })
                .collect()
        };
        let mut scan = dying_scan(&threads(&[(10, false), (20, true), (30, false)]));
        scan.sort_unstable();
        assert_eq!(scan, [10, 30], "a dropped program is not read again");
        assert!(
            dying_scan(&threads(&[(20, true)])).is_empty(),
            "only dropped programs left: the timer stops"
        );
        assert!(
            dying_scan(&HashMap::new()).is_empty(),
            "no program with a window: no timer"
        );
    }

    #[test]
    fn the_dying_check_runs_only_once_a_ping_timed_out() {
        const PID: i32 = i32::MAX - 811;
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut c = h.connect_as(PID);
        let t = h.map_toplevel(&mut c, "app", 200, 100);
        assert!(
            h.state.hang.threads.contains_key(&PID),
            "the map watches the program"
        );
        assert!(
            h.state.hang.dying_check.is_none(),
            "a mapped window polls nothing"
        );
        let target = h.state.ping_target(t.id).expect("ping target");
        h.state.ping_timed_out(target);
        assert!(
            h.state.hang.dying_check.is_some(),
            "a timed-out ping arms the check"
        );
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
