//! Whether a window's client still answers pings.
//!
//! One ping is out per xdg client (`ShellClient::send_ping`) or X11 window (`_NET_WM_PING`) at a
//! time, timed by `PING_TIMEOUT`. A ping no longer out that never timed out was answered.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::reexports::calloop::{
    RegistrationToken,
    timer::{TimeoutAction, Timer},
};
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

    /// The client showed it is running (it committed a new frame).
    pub fn alive(&mut self) {
        self.hung = false;
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
/// has no window in `live`.
fn orphaned(dialogs: &[Option<i32>], live: &[i32]) -> Vec<(usize, i32)> {
    dialogs
        .iter()
        .enumerate()
        .filter_map(|(i, pid)| pid.filter(|p| !live.contains(p)).map(|p| (i, p)))
        .collect()
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
        if let Some(h) = self.hang.windows.get_mut(&w) {
            h.liveness.alive();
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
        match self.hang.pings.remove(&target) {
            Some(t) => self.handle.remove(t),
            None => {
                for w in self.windows_of(target) {
                    self.window_hang(w).liveness.alive();
                }
            }
        }
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
        for w in self.windows_of(target) {
            self.window_hang(w).liveness.timed_out();
            self.offer_end(w);
        }
    }

    /// Whether End Program may kill `pid`: never the compositor itself or the Xwayland server under
    /// every X11 window.
    pub fn may_end(&self, pid: i32) -> bool {
        may_end(pid, std::process::id() as i32, self.xwayland.pid())
    }

    /// Only a window asked to close gets End Program, once per close.
    fn offer_end(&mut self, w: WindowId) {
        let Some(title) = self.win(w).filter(|x| x.mapped).map(|x| x.title()) else {
            return;
        };
        let h = self.window_hang(w);
        if !h.close_asked {
            return;
        }
        h.close_asked = false;
        if self.ending.is_some() {
            log::info!("window {title:?} is not responding; the session end lists it");
            return;
        }
        let Some(pid) = self.client_pid(w).filter(|&p| self.may_end(p)) else {
            log::info!("window {title:?} is not responding; no pid to end");
            return;
        };
        log::info!("window {title:?} (pid {pid}) is not responding");
        let name = program_name(pid).unwrap_or(title);
        self.open_dialog(Dialog::end_program(&name, pid));
    }

    /// A window went away: close each End Program whose program has no window left.
    pub fn close_ended_programs(&mut self) {
        let live: Vec<i32> = self
            .windows
            .iter()
            .filter_map(|w| self.client_pid(w.id))
            .collect();
        let pids: Vec<Option<i32>> = self
            .dialogs
            .views
            .iter()
            .map(|v| v.dialog.end_program_pid())
            .collect();
        for (i, pid) in orphaned(&pids, &live).into_iter().rev() {
            log::info!("end program: pid {pid} has no window left; closing");
            self.close_dialog_at(i);
        }
    }

    /// End Program's End Now: SIGKILL `pid`, as long as a window still belongs to it. The program
    /// may have gone and its pid been reused while the dialog waited; a window holding the pid
    /// means it is still the one that stopped responding.
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
        assert_eq!(orphaned(&dialogs, &[20, 30]), vec![(0, 10), (3, 10)]);
        assert_eq!(orphaned(&dialogs, &[10, 20]), vec![]);
        assert_eq!(orphaned(&dialogs, &[]), vec![(0, 10), (2, 20), (3, 10)]);
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
