//! The network speed the taskbar shows, read from sysfs once a second while the meter is on or a
//! tooltip carrying it is up or pending, and not at all otherwise.

use std::path::Path;
use std::time::{Duration, Instant};

use reprisefs::netspeed::{self, Meter, Rate};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;

const TICK: Duration = Duration::from_secs(1);

/// The `Id` of reprise-network's tray item, whose tooltip carries the speed.
pub const NETWORK_ITEM: &str = "reprise-network";

#[derive(Debug, Default)]
pub struct NetSpeed {
    meter: Meter,
    rate: Option<Rate>,
    timer: Option<RegistrationToken>,
    #[cfg(test)]
    reads: u32,
}

fn wanted(state: &State) -> bool {
    state.config.taskbar.network_meter || crate::tooltip::shows_speed(state)
}

/// Start or stop sampling to match what is shown; cheap, so callers need not check first.
pub fn sync(state: &mut State) {
    match (wanted(state), state.netspeed.timer.is_some()) {
        (true, false) => {
            sample(state);
            let timer = Timer::from_duration(TICK);
            match state.handle.insert_source(timer, |_, _, state| {
                tick(state);
                TimeoutAction::ToDuration(TICK)
            }) {
                Ok(t) => state.netspeed.timer = Some(t),
                Err(e) => log::warn!("network speed timer: {}", e.error),
            }
        }
        (false, true) => {
            if let Some(t) = state.netspeed.timer.take() {
                state.handle.remove(t);
            }
            state.netspeed.meter.clear();
            state.netspeed.rate = None;
        }
        _ => {}
    }
}

/// Read the counters now; whether the readings changed.
pub fn sample(state: &mut State) -> bool {
    #[cfg(test)]
    {
        state.netspeed.reads += 1;
    }
    let counters = netspeed::read(Path::new("/"));
    let Some(rate) = state.netspeed.meter.sample(Instant::now(), &counters) else {
        return false;
    };
    let changed = state.netspeed.rate.map(netspeed::lines) != Some(netspeed::lines(rate));
    state.netspeed.rate = Some(rate);
    changed
}

fn tick(state: &mut State) {
    if !sample(state) {
        return;
    }
    if state.config.taskbar.network_meter {
        state.redraw_taskbars();
    }
    crate::tooltip::speed_changed(state);
}

#[cfg(test)]
pub fn sampling(state: &State) -> bool {
    state.netspeed.timer.is_some()
}

/// The meter's two readings; None until two samples are in.
pub fn lines(state: &State) -> Option<[String; 2]> {
    state.netspeed.rate.map(netspeed::lines)
}

/// Both readings on one line, for a tooltip.
pub fn line(state: &State) -> Option<String> {
    state.netspeed.rate.map(netspeed::line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_read_while_the_meter_is_off_and_no_tip_is_up() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        assert!(!h.state.config.taskbar.network_meter);
        sync(&mut h.state);
        h.run_for(2500);
        assert_eq!(h.state.netspeed.reads, 0);
        assert!(h.state.netspeed.timer.is_none());
    }

    #[test]
    fn the_meter_reads_once_a_second_until_it_is_turned_off() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.config.taskbar.network_meter = true;
        sync(&mut h.state);
        h.run_for(2500);
        let reads = h.state.netspeed.reads;
        assert!((3..=4).contains(&reads), "{reads} reads in 2.5 s");
        assert!(h.state.netspeed.rate.is_some());
        h.state.config.taskbar.network_meter = false;
        sync(&mut h.state);
        assert!(h.state.netspeed.timer.is_none() && h.state.netspeed.rate.is_none());
        h.run_for(2500);
        assert_eq!(h.state.netspeed.reads, reads, "no reads once off");
    }
}
