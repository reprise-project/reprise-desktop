//! `[lock] away_device`: the session locks once the paired Bluetooth device, having been
//! connected, stays disconnected for `away_after_s`. It never unlocks; docs/features.md, Lock
//! When Away, says what the device must do.

use std::time::Duration;

use smithay::reexports::calloop::{
    RegistrationToken,
    channel::{self, Sender},
    timer::{TimeoutAction, Timer},
};
use zbus::Connection;

use crate::compositor::State;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    /// Not yet seen connected: a device away from the start never locks.
    #[default]
    Unknown,
    Near,
    /// Disconnected, the grace running.
    Away,
    /// Away past the grace and locked once; only coming back starts over.
    Locked,
}

/// What the grace timer does after a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    Arm,
    Disarm,
    Lock,
}

/// The device followed and where it stands, apart from the timer and the bus.
#[derive(Debug, Default)]
pub struct Watch {
    /// `[lock] away_device`; empty is off.
    target: String,
    phase: Phase,
}

impl Watch {
    /// Follow `want` from scratch. Whether it is another device than before.
    pub fn retarget(&mut self, want: &str) -> bool {
        if self.target == want {
            return false;
        }
        self.target = want.to_string();
        self.phase = Phase::Unknown;
        true
    }

    /// `target` is or is not connected. A report about another device than the one followed
    /// is from before a retarget and means nothing.
    pub fn seen(&mut self, target: &str, connected: bool) -> Step {
        if self.target.is_empty() || target != self.target {
            return Step::Nothing;
        }
        let was = self.phase;
        match (was, connected) {
            (Phase::Away, true) => {
                self.phase = Phase::Near;
                Step::Disarm
            }
            (_, true) => {
                self.phase = Phase::Near;
                Step::Nothing
            }
            (Phase::Near, false) => {
                self.phase = Phase::Away;
                Step::Arm
            }
            (_, false) => Step::Nothing,
        }
    }

    /// The grace ran out.
    pub fn grace_over(&mut self) -> Step {
        if self.phase != Phase::Away {
            return Step::Nothing;
        }
        self.phase = Phase::Locked;
        Step::Lock
    }
}

#[derive(Default)]
pub struct AwayLock {
    watch: Watch,
    timer: Option<RegistrationToken>,
    targets: Option<async_channel::Sender<String>>,
}

struct Seen {
    target: String,
    connected: bool,
}

/// Runs on its own thread: follows the latest target the channel sent until the next one.
async fn run(reports: Sender<Seen>, targets: async_channel::Receiver<String>) {
    let conn = match Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("away lock: no system bus ({e}); [lock] away_device does nothing");
            return;
        }
    };
    let mut target = String::new();
    loop {
        if !target.is_empty() {
            let t = target.clone();
            let follow = async {
                reprisebus::bluez::follow(&conn, &t, |connected| {
                    let _ = reports.send(Seen {
                        target: t.clone(),
                        connected,
                    });
                })
                .await;
                None
            };
            let next = async { Some(targets.recv().await) };
            match futures_lite::future::or(follow, next).await {
                Some(Ok(t)) => {
                    target = t;
                    continue;
                }
                Some(Err(_)) => return,
                None => log::info!("away lock: the system bus closed"),
            }
        }
        match targets.recv().await {
            Ok(t) => target = t,
            Err(_) => return,
        }
    }
}

pub fn setup(state: &mut State) {
    let (tx, rx) = channel::channel::<Seen>();
    if let Err(e) = state.handle.insert_source(rx, |event, _, state| {
        if let channel::Event::Msg(s) = event {
            seen(state, s);
        }
    }) {
        log::warn!("away lock: no report channel ({})", e.error);
        return;
    }
    let (target_tx, target_rx) = async_channel::unbounded::<String>();
    let thread = std::thread::Builder::new()
        .name("awaylock-dbus".into())
        .spawn(move || zbus::block_on(run(tx, target_rx)));
    if thread.is_ok() {
        state.awaylock.targets = Some(target_tx);
    }
    config_changed(state);
}

/// A new `[lock] away_device` starts over; a new grace applies from the next disconnect.
pub fn config_changed(state: &mut State) {
    let want = state.config.lock.away_device.clone();
    if !state.awaylock.watch.retarget(&want) {
        return;
    }
    disarm(state);
    if want.is_empty() {
        log::info!("away lock: off");
    } else {
        log::info!(
            "away lock: following {want}; it locks once the device, seen connected, stays disconnected"
        );
    }
    if let Some(tx) = &state.awaylock.targets {
        let _ = tx.try_send(want);
    }
}

fn seen(state: &mut State, s: Seen) {
    match state.awaylock.watch.seen(&s.target, s.connected) {
        Step::Arm => arm(state),
        Step::Disarm => {
            log::info!("away lock: {} is back", s.target);
            disarm(state);
        }
        Step::Nothing | Step::Lock => {}
    }
}

fn disarm(state: &mut State) {
    if let Some(t) = state.awaylock.timer.take() {
        state.handle.remove(t);
    }
}

fn arm(state: &mut State) {
    disarm(state);
    let grace = state.config.lock.away_after_s;
    log::info!(
        "away lock: {} disconnected; locking in {grace} s unless it comes back",
        state.config.lock.away_device
    );
    let timer = state.handle.insert_source(
        Timer::from_duration(Duration::from_secs(grace.into())),
        |_, _, state| {
            state.awaylock.timer = None;
            if state.awaylock.watch.grace_over() == Step::Lock {
                // Unlike the idle lock, an idle inhibitor (a video playing) does not hold this off.
                log::info!("away lock: the device stayed away; locking");
                crate::lockview::lock(state);
            }
            TimeoutAction::Drop
        },
    );
    match timer {
        Ok(t) => state.awaylock.timer = Some(t),
        Err(e) => log::warn!("away lock: grace timer: {}", e.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: &str = "AA:BB:CC:DD:EE:01";

    fn following() -> Watch {
        let mut w = Watch::default();
        assert!(w.retarget(PHONE));
        w
    }

    #[test]
    fn a_drop_arms_the_grace_and_its_end_locks_once() {
        let mut w = following();
        assert_eq!(w.seen(PHONE, true), Step::Nothing, "connect");
        assert_eq!(w.seen(PHONE, false), Step::Arm, "drop");
        assert_eq!(w.seen(PHONE, false), Step::Nothing, "still away");
        assert_eq!(w.grace_over(), Step::Lock);
        assert_eq!(w.grace_over(), Step::Nothing, "locks once");
        assert_eq!(
            w.seen(PHONE, false),
            Step::Nothing,
            "no second lock while away"
        );
    }

    #[test]
    fn coming_back_within_the_grace_disarms() {
        let mut w = following();
        w.seen(PHONE, true);
        assert_eq!(w.seen(PHONE, false), Step::Arm);
        assert_eq!(w.seen(PHONE, true), Step::Disarm);
        assert_eq!(
            w.grace_over(),
            Step::Nothing,
            "a timer firing late locks nothing"
        );
        assert_eq!(w.seen(PHONE, false), Step::Arm, "the next drop arms again");
    }

    #[test]
    fn coming_back_after_the_lock_starts_over() {
        let mut w = following();
        w.seen(PHONE, true);
        w.seen(PHONE, false);
        assert_eq!(w.grace_over(), Step::Lock);
        assert_eq!(w.seen(PHONE, true), Step::Nothing, "back: no unlock");
        assert_eq!(w.seen(PHONE, false), Step::Arm);
        assert_eq!(w.grace_over(), Step::Lock);
    }

    #[test]
    fn a_device_never_seen_connected_never_locks() {
        let mut w = following();
        assert_eq!(w.seen(PHONE, false), Step::Nothing);
        assert_eq!(w.grace_over(), Step::Nothing);
    }

    #[test]
    fn turning_it_off_or_another_device_ignores_old_reports() {
        let mut w = following();
        w.seen(PHONE, true);
        assert!(w.retarget(""));
        assert!(!w.retarget(""), "the same setting is no change");
        assert_eq!(w.seen(PHONE, false), Step::Nothing, "off");
        assert_eq!(w.seen("", false), Step::Nothing, "off");
        let other = "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_02";
        assert!(w.retarget(other));
        w.seen(other, true);
        assert_eq!(w.seen(PHONE, false), Step::Nothing, "the old device");
        assert_eq!(w.seen(other, false), Step::Arm);
        assert!(w.retarget(PHONE));
        assert_eq!(w.grace_over(), Step::Nothing, "a retarget drops the grace");
    }
}
