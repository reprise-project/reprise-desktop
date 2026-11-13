//! The battery alarms as an edge detector: `Low` once when the charge falls to the low threshold while discharging, `Critical` once at the critical one; each re-arms when the battery charges or climbs `REARM_MARGIN` points above its line.
//! [`Saver`] spans the same low stretch: on from the low line until the alarm would re-arm.

use crate::upower::{Power, State};

/// Points above a threshold the charge must reach to re-arm; UPower's percentage bounces a point as the load changes.
const REARM_MARGIN: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alarm {
    Low(u8),
    Critical(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alarms {
    low_armed: bool,
    critical_armed: bool,
}

impl Default for Alarms {
    fn default() -> Self {
        Alarms {
            low_armed: true,
            critical_armed: true,
        }
    }
}

impl Alarms {
    /// Feed a snapshot; `low` and `critical` are thresholds in percent, 0 for an alarm that is off. Critical wins when both would fire.
    pub fn feed(&mut self, p: &Power, low: u32, critical: u32) -> Option<Alarm> {
        let Some(b) = &p.battery else {
            self.low_armed = true;
            self.critical_armed = true;
            return None;
        };
        let pct = u32::from(b.percent);
        let discharging = b.state == State::Discharging;
        if !discharging {
            self.low_armed = true;
            self.critical_armed = true;
            return None;
        }
        if critical > 0 && pct <= critical {
            if self.critical_armed {
                self.critical_armed = false;
                // The low alarm is spent by the same fall.
                self.low_armed = false;
                return Some(Alarm::Critical(b.percent));
            }
        } else if pct >= critical + REARM_MARGIN {
            self.critical_armed = true;
        }
        if low > 0 && pct <= low {
            if self.low_armed {
                self.low_armed = false;
                return Some(Alarm::Low(b.percent));
            }
        } else if pct >= low + REARM_MARGIN {
            self.low_armed = true;
        }
        None
    }
}

/// Whether the low battery power saver should hold: the edges of the low alarm's stretch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Saver {
    held: bool,
}

impl Saver {
    /// Feed a snapshot; `on` is false while the saver is off or has no profile to hold. `Some` says to
    /// take (`true`) or let go of (`false`) the hold, once per crossing.
    pub fn feed(&mut self, p: &Power, low: u32, on: bool) -> Option<bool> {
        let want = match &p.battery {
            Some(b) if on && low > 0 && b.state == State::Discharging => {
                let pct = u32::from(b.percent);
                if pct <= low {
                    true
                } else if pct >= low + REARM_MARGIN {
                    false
                } else {
                    self.held
                }
            }
            _ => false,
        };
        (want != self.held).then(|| {
            self.held = want;
            want
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upower::Battery;

    fn at(percent: u8, state: State) -> Power {
        Power {
            on_battery: state == State::Discharging,
            battery: Some(Battery {
                state,
                percent,
                ..Battery::default()
            }),
            limit: None,
        }
    }

    #[test]
    fn low_fires_once_and_rearms_on_charge() {
        let mut a = Alarms::default();
        assert_eq!(a.feed(&at(50, State::Discharging), 10, 3), None);
        assert_eq!(
            a.feed(&at(10, State::Discharging), 10, 3),
            Some(Alarm::Low(10))
        );
        assert_eq!(a.feed(&at(9, State::Discharging), 10, 3), None);
        assert_eq!(a.feed(&at(30, State::Charging), 10, 3), None);
        assert_eq!(
            a.feed(&at(8, State::Discharging), 10, 3),
            Some(Alarm::Low(8))
        );
    }

    #[test]
    fn critical_fires_on_its_own_line() {
        let mut a = Alarms::default();
        assert_eq!(
            a.feed(&at(10, State::Discharging), 10, 3),
            Some(Alarm::Low(10))
        );
        assert_eq!(a.feed(&at(5, State::Discharging), 10, 3), None);
        assert_eq!(
            a.feed(&at(3, State::Discharging), 10, 3),
            Some(Alarm::Critical(3))
        );
        assert_eq!(a.feed(&at(2, State::Discharging), 10, 3), None);
    }

    #[test]
    fn a_straight_fall_to_critical_is_one_alarm() {
        let mut a = Alarms::default();
        assert_eq!(
            a.feed(&at(2, State::Discharging), 10, 3),
            Some(Alarm::Critical(2))
        );
        assert_eq!(a.feed(&at(1, State::Discharging), 10, 3), None);
    }

    #[test]
    fn zero_thresholds_never_fire() {
        let mut a = Alarms::default();
        assert_eq!(a.feed(&at(1, State::Discharging), 0, 0), None);
    }

    #[test]
    fn a_one_point_wobble_at_the_line_does_not_fire_again() {
        let mut a = Alarms::default();
        assert_eq!(
            a.feed(&at(3, State::Discharging), 10, 3),
            Some(Alarm::Critical(3))
        );
        assert_eq!(a.feed(&at(4, State::Discharging), 10, 3), None);
        assert_eq!(a.feed(&at(3, State::Discharging), 10, 3), None);
        let mut a = Alarms::default();
        assert_eq!(
            a.feed(&at(10, State::Discharging), 10, 0),
            Some(Alarm::Low(10))
        );
        assert_eq!(a.feed(&at(11, State::Discharging), 10, 0), None);
        assert_eq!(a.feed(&at(10, State::Discharging), 10, 0), None);
    }

    #[test]
    fn rising_two_points_above_the_line_rearms() {
        let mut a = Alarms::default();
        assert_eq!(
            a.feed(&at(10, State::Discharging), 10, 0),
            Some(Alarm::Low(10))
        );
        assert_eq!(a.feed(&at(12, State::Discharging), 10, 0), None);
        assert_eq!(
            a.feed(&at(10, State::Discharging), 10, 0),
            Some(Alarm::Low(10))
        );
    }

    #[test]
    fn the_saver_holds_from_the_low_line_until_charging() {
        let mut s = Saver::default();
        assert_eq!(s.feed(&at(30, State::Discharging), 10, true), None);
        assert_eq!(s.feed(&at(10, State::Discharging), 10, true), Some(true));
        assert_eq!(s.feed(&at(9, State::Discharging), 10, true), None);
        assert_eq!(s.feed(&at(9, State::Charging), 10, true), Some(false));
        assert_eq!(
            s.feed(&at(9, State::Discharging), 10, true),
            Some(true),
            "unplugged again"
        );
    }

    #[test]
    fn the_saver_lets_go_two_points_above_the_line() {
        let mut s = Saver::default();
        assert_eq!(s.feed(&at(10, State::Discharging), 10, true), Some(true));
        assert_eq!(s.feed(&at(11, State::Discharging), 10, true), None);
        assert_eq!(s.feed(&at(12, State::Discharging), 10, true), Some(false));
        assert_eq!(s.feed(&at(11, State::Discharging), 10, true), None);
    }

    #[test]
    fn a_saver_turned_off_or_without_an_alarm_lets_go() {
        let mut s = Saver::default();
        assert_eq!(s.feed(&at(5, State::Discharging), 10, false), None);
        assert_eq!(s.feed(&at(5, State::Discharging), 0, true), None);
        assert_eq!(s.feed(&at(5, State::Discharging), 10, true), Some(true));
        assert_eq!(s.feed(&at(5, State::Discharging), 10, false), Some(false));
        assert_eq!(s.feed(&Power::default(), 10, true), None, "no battery");
    }
}
