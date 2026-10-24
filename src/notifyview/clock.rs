//! How long each balloon stays up, and the timer that takes it down.

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::time::{Duration, Instant};

use crate::compositor::State;

use super::motion::leave;
use super::reply::collapse_idle;
use super::stack::display_next;
use super::{NotifyView, REASON_EXPIRED, REPLY_IDLE, hovered, index_of, sheets};

/// A balloon's time on screen: running toward `due`, or paused with `left` to go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Clock {
    due: Option<Instant>,
    left: Option<Duration>,
}

impl Clock {
    /// `None` never runs out.
    pub(super) fn start(ms: Option<i32>, now: Instant) -> Clock {
        Clock {
            due: ms
                .filter(|&ms| ms > 0)
                .map(|ms| now + Duration::from_millis(ms as u64)),
            left: None,
        }
    }

    pub(super) fn pause(&mut self, now: Instant) {
        if let Some(due) = self.due.take() {
            self.left = Some(due.saturating_duration_since(now));
        }
    }

    pub(super) fn resume(&mut self, now: Instant) {
        if let Some(left) = self.left.take() {
            self.due = Some(now + left);
        }
    }

    pub(super) fn due(&self) -> Option<Instant> {
        self.due
    }
}

/// Balloon `i`'s clock runs `ms` from now, held while the pointer is on a stack of sheets.
pub(super) fn restart(v: &mut NotifyView, i: usize, ms: Option<i32>, sheets: bool) {
    let now = Instant::now();
    let paused = sheets && hovered(v);
    let Some(b) = v.stack.get_mut(i) else {
        return;
    };
    b.clock = Clock::start(ms, now);
    if paused {
        b.clock.pause(now);
    }
}

pub(super) fn reply_idle_ms() -> Option<i32> {
    Some(REPLY_IDLE.as_millis() as i32)
}

/// Holds every clock while the pointer is on a stack of sheets, else lets them run; true on a change.
pub(super) fn hold_clocks(state: &mut State) -> bool {
    let hold = sheets(state) && hovered(&state.notify);
    let now = Instant::now();
    let mut moved = false;
    for b in &mut state.notify.stack {
        let was = b.clock;
        if hold {
            b.clock.pause(now);
        } else {
            b.clock.resume(now);
        }
        moved |= b.clock != was;
    }
    moved
}

/// Arms `timer` for the earliest clock in the stack.
pub(super) fn rearm(state: &mut State) {
    if let Some(t) = state.notify.timer.take() {
        state.handle.remove(t);
    }
    let Some(next) = state
        .notify
        .stack
        .iter()
        .filter_map(|b| b.clock.due())
        .min()
    else {
        return;
    };
    let wait = next.saturating_duration_since(Instant::now());
    match state
        .handle
        .insert_source(Timer::from_duration(wait), |_, _, state| {
            state.notify.timer = None;
            due(state);
            TimeoutAction::Drop
        }) {
        Ok(t) => state.notify.timer = Some(t),
        Err(e) => log::warn!("notification timer: {}", e.error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    Expire,
    /// The reply row has sat untouched for `REPLY_IDLE`.
    Collapse,
    /// The reply row is open with the pointer in the balloon: another `REPLY_IDLE`.
    Wait,
}

pub(super) fn due_step(replying: bool, pointer_in: bool) -> Due {
    match (replying, pointer_in) {
        (false, _) => Due::Expire,
        (true, false) => Due::Collapse,
        (true, true) => Due::Wait,
    }
}

pub(super) fn due(state: &mut State) {
    let now = Instant::now();
    let ripe: Vec<u64> = state
        .notify
        .stack
        .iter()
        .filter(|b| b.clock.due().is_some_and(|d| d <= now))
        .map(|b| b.serial)
        .collect();
    let sheets = sheets(state);
    for serial in ripe {
        let Some(i) = index_of(&state.notify, serial) else {
            continue;
        };
        let b = &state.notify.stack[i];
        match due_step(b.reply.is_some(), b.pointer_in) {
            Due::Expire => expired(state, i),
            Due::Collapse => collapse_idle(state),
            Due::Wait => restart(&mut state.notify, i, reply_idle_ms(), sheets),
        }
    }
    rearm(state);
}

/// A transient notification closes with its balloon; any other stays open in the history.
fn expired(state: &mut State, i: usize) {
    if i >= state.notify.stack.len() {
        return;
    }
    let b = state.notify.stack.remove(i);
    let gone = b.item.note().filter(|n| n.transient).map(|n| n.id);
    leave(state, b);
    if let Some(id) = gone {
        crate::tray::notification_closed(state, id, REASON_EXPIRED);
    }
    display_next(state);
}
