//! The battery alarms as an edge detector: `Low` once when the charge falls to the low threshold while discharging, `Critical` once at the critical one; each re-arms when the battery charges or climbs `REARM_MARGIN` points above its line.

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
}
