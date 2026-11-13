//! The low battery balloon: once when a connected device's battery drops to the warning level,
//! again only after it charges [`BT_BATTERY_REARM`] points past it.

use std::collections::HashSet;

use repriseconf::BT_BATTERY_REARM;

use crate::bz::{Kind, Snapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub summary: String,
    pub body: String,
    pub icon: &'static str,
}

/// The devices warned about, by object path, until they charge back past the re-arm level.
#[derive(Debug, Default)]
pub struct Warner {
    warned: HashSet<String>,
}

impl Warner {
    /// The balloons `s` calls for at warning level `level` (0 is off). A device that drops off
    /// the bus or loses its battery reading stays warned.
    pub fn check(&mut self, s: &Snapshot, level: u8) -> Vec<Warning> {
        let mut out = Vec::new();
        for d in &s.devices {
            let Some(b) = d.battery else {
                continue;
            };
            if b > level.saturating_add(BT_BATTERY_REARM) {
                self.warned.remove(&d.path);
            } else if level > 0 && d.connected && b <= level && self.warned.insert(d.path.clone()) {
                out.push(warning(&d.alias, d.kind, b));
            }
        }
        out
    }
}

fn warning(alias: &str, kind: Kind, percent: u8) -> Warning {
    Warning {
        summary: "Low Bluetooth battery".into(),
        body: format!("{alias} has {percent}% battery left."),
        icon: kind.icon_name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::Device;

    fn snap(battery: Option<u8>, connected: bool) -> Snapshot {
        Snapshot {
            adapter: None,
            devices: vec![Device {
                path: "/org/bluez/hci0/dev_AA".into(),
                alias: "Cans".into(),
                kind: Kind::Headphones,
                connected,
                battery,
                ..Device::default()
            }],
            airplane: None,
        }
    }

    fn warns(w: &mut Warner, battery: u8) -> usize {
        w.check(&snap(Some(battery), true), 15).len()
    }

    #[test]
    fn a_drop_to_the_level_warns_once_with_name_and_icon() {
        let mut w = Warner::default();
        assert_eq!(warns(&mut w, 40), 0);
        assert_eq!(
            w.check(&snap(Some(15), true), 15),
            [Warning {
                summary: "Low Bluetooth battery".into(),
                body: "Cans has 15% battery left.".into(),
                icon: "audio-headphones",
            }]
        );
        assert_eq!(warns(&mut w, 12), 0, "a further drop does not warn again");
        assert_eq!(warns(&mut w, 5), 0);
    }

    #[test]
    fn it_warns_again_only_after_charging_past_20() {
        let mut w = Warner::default();
        assert_eq!(warns(&mut w, 15), 1);
        assert_eq!(warns(&mut w, 20), 0, "20 is not past the re-arm level");
        assert_eq!(warns(&mut w, 15), 0);
        assert_eq!(warns(&mut w, 21), 0);
        assert_eq!(warns(&mut w, 14), 1);
    }

    #[test]
    fn only_a_connected_device_with_a_reading_warns_and_0_is_off() {
        let mut w = Warner::default();
        assert!(w.check(&snap(Some(10), false), 15).is_empty());
        assert!(w.check(&snap(None, true), 15).is_empty());
        assert!(w.check(&snap(Some(10), true), 0).is_empty());
        assert_eq!(w.check(&snap(Some(10), true), 15).len(), 1);
    }

    #[test]
    fn reconnecting_while_still_low_does_not_warn_again() {
        let mut w = Warner::default();
        assert_eq!(warns(&mut w, 10), 1);
        assert!(w.check(&snap(Some(10), false), 15).is_empty());
        assert!(w.check(&Snapshot::default(), 15).is_empty());
        assert_eq!(warns(&mut w, 10), 0);
    }
}
