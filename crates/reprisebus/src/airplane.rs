//! Airplane mode: Wi-Fi and mobile broadband through NetworkManager, Bluetooth through rfkill.
//! The radios that were on wait in `$XDG_STATE_HOME/reprise/airplane-mode` until it is turned off.

use std::path::{Path, PathBuf};
use zbus::proxy::CacheProperties;
use zbus::{Connection, Proxy};

use crate::rfkill::{self, Kind, Switch};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
pub const WIFI_KEY: &str = "WirelessEnabled";
pub const WWAN_KEY: &str = "WwanEnabled";
pub const RADIO_FLAGS_KEY: &str = "RadioFlags";

/// `NM_RADIO_FLAG_WLAN_AVAILABLE` and `NM_RADIO_FLAG_WWAN_AVAILABLE`.
const WLAN_AVAILABLE: u32 = 1;
const WWAN_AVAILABLE: u32 = 2;

/// Which radios are switched on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Radios {
    pub wifi: bool,
    pub wwan: bool,
    pub bluetooth: bool,
}

impl Radios {
    const ALL: Radios = Radios {
        wifi: true,
        wwan: true,
        bluetooth: true,
    };

    /// The radios NetworkManager's two switches and the rfkill `switches` leave on. A Bluetooth
    /// radio is on while any of its switches is not soft-blocked.
    pub fn of(wifi: bool, wwan: bool, switches: &[Switch]) -> Radios {
        Radios {
            wifi,
            wwan,
            bluetooth: switches
                .iter()
                .any(|s| s.kind == Kind::Bluetooth && !s.soft),
        }
    }

    pub fn any_on(self) -> bool {
        self.wifi || self.wwan || self.bluetooth
    }

    fn names(self) -> String {
        [
            (self.wifi, "wifi"),
            (self.wwan, "wwan"),
            (self.bluetooth, "bluetooth"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| format!("{n}\n"))
        .collect()
    }

    fn parse(text: &str) -> Radios {
        let has = |n: &str| text.lines().any(|l| l.trim() == n);
        Radios {
            wifi: has("wifi"),
            wwan: has("wwan"),
            bluetooth: has("bluetooth"),
        }
    }
}

/// Whether airplane mode is on: every radio off. None on a machine with no radio switch.
pub fn state(radios: Radios, switches: &[Switch]) -> Option<bool> {
    (!switches.is_empty()).then_some(!radios.any_on())
}

/// [`Radios::of`] for NetworkManager's `WirelessEnabled`, `WwanEnabled` and `RadioFlags`. A
/// radio counts only where its hardware is: `WwanEnabled` stays on without a modem. A daemon
/// too old for `RadioFlags` counts both.
pub fn from_nm(wifi: bool, wwan: bool, radio_flags: Option<u32>, switches: &[Switch]) -> Radios {
    let has = |bit| radio_flags.is_none_or(|f| f & bit != 0);
    Radios::of(
        wifi && has(WLAN_AVAILABLE),
        wwan && has(WWAN_AVAILABLE),
        switches,
    )
}

/// The radios as they are now. Without NetworkManager, Wi-Fi and mobile broadband read off.
pub async fn radios(conn: &Connection) -> Radios {
    let switches = rfkill::switches();
    let Ok(p) = nm(conn).await else {
        return Radios::of(false, false, &switches);
    };
    from_nm(
        p.get_property(WIFI_KEY).await.unwrap_or(false),
        p.get_property(WWAN_KEY).await.unwrap_or(false),
        p.get_property(RADIO_FLAGS_KEY).await.ok(),
        &switches,
    )
}

/// Whether airplane mode is on now; None on a machine with no radio switch.
pub async fn read(conn: &Connection) -> Option<bool> {
    state(radios(conn).await, &rfkill::switches())
}

/// Turn airplane mode on or off; Err with what failed.
pub async fn set(conn: &Connection, on: bool) -> Result<(), String> {
    let memory = memory_path();
    let now = radios(conn).await;
    let want = if on {
        if now.any_on() {
            remember(&memory, now);
        }
        Radios::default()
    } else {
        recall(&memory)
    };
    apply(conn, changes(now, want, on)).await
}

/// Toggle airplane mode; the state it is in afterwards.
pub async fn toggle(conn: &Connection) -> Result<bool, String> {
    let on = !radios(conn).await.any_on();
    set(conn, !on).await.map(|()| !on)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Wifi(bool),
    Wwan(bool),
    Bluetooth(bool),
}

/// What turns `now` into `want`. Turning airplane mode on only turns radios off and turning it
/// off only turns them on, so a radio switched by hand in between keeps its state.
fn changes(now: Radios, want: Radios, on: bool) -> Vec<Change> {
    [
        (now.wifi, want.wifi, Change::Wifi as fn(bool) -> Change),
        (now.wwan, want.wwan, Change::Wwan),
        (now.bluetooth, want.bluetooth, Change::Bluetooth),
    ]
    .into_iter()
    .filter(|(now, want, _)| now != want && *now == on)
    .map(|(_, want, change)| change(want))
    .collect()
}

async fn apply(conn: &Connection, changes: Vec<Change>) -> Result<(), String> {
    let mut failed = Vec::new();
    for change in changes {
        let r = match change {
            Change::Wifi(on) => set_nm(conn, WIFI_KEY, on).await,
            Change::Wwan(on) => set_nm(conn, WWAN_KEY, on).await,
            Change::Bluetooth(on) => {
                rfkill::set_blocked(Kind::Bluetooth, !on).map_err(|e| format!("Bluetooth: {e}"))
            }
        };
        if let Err(e) = r {
            log::warn!("airplane mode: {change:?} failed: {e}");
            failed.push(e);
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("\n"))
    }
}

async fn nm(conn: &Connection) -> zbus::Result<Proxy<'static>> {
    zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(NM)
        .path(NM_PATH)
        .interface(NM)
        .cache_properties(CacheProperties::No)
        .build()
        .await
}

async fn set_nm(conn: &Connection, key: &str, on: bool) -> Result<(), String> {
    let set = match nm(conn).await {
        Ok(p) => p.set_property(key, on).await.map_err(zbus::Error::from),
        Err(e) => Err(e),
    };
    set.map_err(|e| {
        let why = crate::error::message(&e).map_or_else(|| e.to_string(), str::to_owned);
        let radio = if key == WIFI_KEY {
            "Wi-Fi"
        } else {
            "Mobile broadband"
        };
        format!("{radio}: {why}")
    })
}

fn memory_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    state.join("reprise/airplane-mode")
}

fn remember(path: &Path, radios: Radios) {
    let write = || {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, radios.names())
    };
    if let Err(e) = write() {
        log::warn!("airplane mode: could not keep the radios in {path:?}: {e}");
    }
}

/// The radios to turn back on, forgotten as they are read; every radio when none was kept.
fn recall(path: &Path) -> Radios {
    let kept = std::fs::read_to_string(path).map(|t| Radios::parse(&t));
    let _ = std::fs::remove_file(path);
    kept.ok().filter(|r| r.any_on()).unwrap_or(Radios::ALL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bt(soft: bool) -> Switch {
        Switch {
            kind: Kind::Bluetooth,
            soft,
            hard: false,
        }
    }

    const WLAN: Switch = Switch {
        kind: Kind::Wlan,
        soft: false,
        hard: false,
    };

    #[test]
    fn bluetooth_is_on_while_any_of_its_switches_is_unblocked() {
        assert!(!Radios::of(false, false, &[WLAN]).bluetooth);
        assert!(!Radios::of(false, false, &[bt(true), bt(true)]).bluetooth);
        assert!(Radios::of(false, false, &[bt(true), bt(false)]).bluetooth);
    }

    #[test]
    fn a_radio_without_its_hardware_counts_as_off() {
        let r = from_nm(true, true, Some(WLAN_AVAILABLE), &[]);
        assert_eq!((r.wifi, r.wwan), (true, false));
        let r = from_nm(true, true, Some(WWAN_AVAILABLE), &[]);
        assert_eq!((r.wifi, r.wwan), (false, true));
        let r = from_nm(true, false, None, &[]);
        assert_eq!((r.wifi, r.wwan), (true, false), "no RadioFlags: as enabled");
    }

    #[test]
    fn airplane_mode_is_every_radio_off_and_absent_without_a_switch() {
        let off = Radios::default();
        assert_eq!(state(off, &[WLAN]), Some(true));
        assert_eq!(state(Radios { wwan: true, ..off }, &[WLAN]), Some(false));
        assert_eq!(state(off, &[]), None);
    }

    #[test]
    fn turning_on_switches_off_only_what_is_on() {
        let now = Radios {
            wifi: true,
            wwan: false,
            bluetooth: true,
        };
        assert_eq!(
            changes(now, Radios::default(), true),
            [Change::Wifi(false), Change::Bluetooth(false)]
        );
    }

    #[test]
    fn turning_off_switches_on_only_what_was_kept() {
        let kept = Radios {
            wifi: true,
            ..Radios::default()
        };
        assert_eq!(
            changes(Radios::default(), kept, false),
            [Change::Wifi(true)]
        );
        let bluetooth_by_hand = Radios {
            bluetooth: true,
            ..Radios::default()
        };
        assert_eq!(
            changes(bluetooth_by_hand, kept, false),
            [Change::Wifi(true)],
            "a radio turned on by hand stays on"
        );
    }

    #[test]
    fn the_kept_radios_come_back_once_then_every_radio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let kept = Radios {
            wifi: true,
            wwan: false,
            bluetooth: true,
        };
        remember(&path, kept);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "wifi\nbluetooth\n");
        assert_eq!(recall(&path), kept);
        assert!(!path.exists(), "read once");
        assert_eq!(recall(&path), Radios::ALL);
    }
}
