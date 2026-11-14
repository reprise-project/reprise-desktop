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

    fn without(self, other: Radios) -> Radios {
        Radios {
            wifi: self.wifi && !other.wifi,
            wwan: self.wwan && !other.wwan,
            bluetooth: self.bluetooth && !other.bluetooth,
        }
    }

    fn with(self, other: Radios) -> Radios {
        Radios {
            wifi: self.wifi || other.wifi,
            wwan: self.wwan || other.wwan,
            bluetooth: self.bluetooth || other.bluetooth,
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

/// Airplane mode as the radios have it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    On,
    /// Every radio is held off by a hardware switch: no turn of airplane mode changes anything.
    Killed,
}

/// What the check box and the tray rows say under [`Mode::Killed`].
pub const KILLED: &str = "Turned off by a hardware switch";

impl Mode {
    /// Whether the check box reads checked: every radio is off.
    pub fn on(self) -> bool {
        self != Mode::Off
    }

    /// The turn a click asks for: on while off, off while on, none under a hardware kill.
    pub fn flipped(self) -> Option<bool> {
        match self {
            Mode::Off => Some(true),
            Mode::On => Some(false),
            Mode::Killed => None,
        }
    }
}

/// Airplane mode from NetworkManager's `WirelessEnabled`, `WwanEnabled` and `RadioFlags` and the
/// rfkill switches; None on a machine with no radio switch.
pub fn mode(
    wifi: bool,
    wwan: bool,
    radio_flags: Option<u32>,
    switches: Vec<Switch>,
) -> Option<Mode> {
    Now {
        wifi,
        wwan,
        radio_flags,
        switches,
    }
    .mode()
}

const KINDS: [Kind; 3] = [Kind::Wlan, Kind::Wwan, Kind::Bluetooth];

/// NetworkManager's two switches and `RadioFlags` and the rfkill switches, read together.
struct Now {
    wifi: bool,
    wwan: bool,
    radio_flags: Option<u32>,
    switches: Vec<Switch>,
}

impl Now {
    /// Without NetworkManager, Wi-Fi and mobile broadband are left alone.
    async fn read(conn: &Connection) -> Now {
        let switches = rfkill::switches();
        match nm(conn).await {
            Ok(p) => Now {
                wifi: p.get_property(WIFI_KEY).await.unwrap_or(false),
                wwan: p.get_property(WWAN_KEY).await.unwrap_or(false),
                radio_flags: p.get_property(RADIO_FLAGS_KEY).await.ok(),
                switches,
            },
            Err(_) => Now {
                wifi: false,
                wwan: false,
                radio_flags: Some(0),
                switches,
            },
        }
    }

    fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Switch> {
        self.switches.iter().filter(move |s| s.kind == kind)
    }

    /// Whether `kind` has a radio: Wi-Fi and mobile broadband as `RadioFlags` says (a modem
    /// may have no switch), else where a switch of that kind is.
    fn has(&self, kind: Kind) -> bool {
        let bit = match kind {
            Kind::Wlan => WLAN_AVAILABLE,
            Kind::Wwan => WWAN_AVAILABLE,
            Kind::Bluetooth => 0,
        };
        match self.radio_flags {
            Some(flags) if bit != 0 => flags & bit != 0,
            _ => self.of_kind(kind).next().is_some(),
        }
    }

    /// Whether a hardware switch holds a radio of `kind` off.
    fn held(&self, kind: Kind) -> bool {
        self.of_kind(kind).any(|s| s.hard)
    }

    /// Whether `kind` is soft-unblocked, hard blocks aside: what airplane mode turns off, so a
    /// radio held off in hardware stays off once that lifts. NetworkManager reports Wi-Fi and
    /// mobile broadband off under any hard block, so there the switches' soft bits tell.
    fn soft_on(&self, kind: Kind) -> bool {
        let nm = match kind {
            Kind::Wlan => Some(self.wifi),
            Kind::Wwan => Some(self.wwan),
            Kind::Bluetooth => None,
        };
        self.has(kind)
            && match nm {
                Some(nm) if !self.held(kind) => nm,
                _ => self.of_kind(kind).any(|s| !s.soft),
            }
    }

    /// Wi-Fi as the Wi-Fi key turns it: NetworkManager's switch, or the soft block while a
    /// hardware switch holds it.
    fn wifi_on(&self) -> bool {
        if self.held(Kind::Wlan) {
            self.soft_on(Kind::Wlan)
        } else {
            self.wifi
        }
    }

    fn soft(&self) -> Radios {
        Radios {
            wifi: self.soft_on(Kind::Wlan),
            wwan: self.soft_on(Kind::Wwan),
            bluetooth: self.soft_on(Kind::Bluetooth),
        }
    }

    /// Every radio there is has switches, and every one of them is hard-blocked.
    fn killed(&self) -> bool {
        let mut kinds = KINDS.into_iter().filter(|&k| self.has(k)).peekable();
        kinds.peek().is_some()
            && kinds.all(|k| self.of_kind(k).next().is_some() && self.of_kind(k).all(|s| s.hard))
    }

    /// On while every radio is soft-blocked, whatever holds it off in hardware.
    fn mode(&self) -> Option<Mode> {
        if self.switches.is_empty() {
            None
        } else if self.killed() {
            Some(Mode::Killed)
        } else if self.soft().any_on() {
            Some(Mode::Off)
        } else {
            Some(Mode::On)
        }
    }
}

/// Airplane mode now; None on a machine with no radio switch. While it is off, a kept radio
/// that is on again was brought back by hand and is no longer kept.
pub async fn read(conn: &Connection) -> Option<Mode> {
    let now = Now::read(conn).await;
    let mode = now.mode();
    if mode == Some(Mode::Off) {
        brought_back(&memory_path(), now.soft());
    }
    mode
}

/// Turn airplane mode on or off; Err with what failed. Nothing changes under a hardware kill.
pub async fn set(conn: &Connection, on: bool) -> Result<(), String> {
    let now = Now::read(conn).await;
    settle(&now, on, &memory_path(), |c| exec(conn, c)).await
}

/// Toggle airplane mode; the mode it is in afterwards.
pub async fn toggle(conn: &Connection) -> Result<Mode, String> {
    let now = Now::read(conn).await;
    if now.killed() {
        return Ok(Mode::Killed);
    }
    let on = !now.soft().any_on();
    settle(&now, !on, &memory_path(), |c| exec(conn, c)).await?;
    Ok(if on { Mode::Off } else { Mode::On })
}

/// The changes turning airplane mode on or off makes from `now`, keeping or recalling the radios
/// in `memory`.
fn turn(now: &Now, on: bool, memory: &Path) -> Vec<Change> {
    if now.killed() {
        return Vec::new();
    }
    let soft = now.soft();
    let want = if on {
        keep(memory, soft);
        Radios::default()
    } else {
        recall(memory)
    };
    (changes(soft, want, on).into_iter())
        .flat_map(|(kind, on)| radio(now, kind, on))
        .collect()
}

/// Turning one radio kind on or off: Bluetooth by its soft block, Wi-Fi and mobile broadband by
/// NetworkManager's switch, after their soft block while a hardware switch holds them.
fn radio(now: &Now, kind: Kind, on: bool) -> Vec<Change> {
    match kind {
        Kind::Bluetooth => vec![Change::Rfkill(kind, on)],
        _ if now.held(kind) => vec![Change::Rfkill(kind, on), Change::Nm(kind, on)],
        _ => vec![Change::Nm(kind, on)],
    }
}

/// Turn airplane mode on or off from `now` through `exec`; Err with what failed. A radio that
/// failed to come back on stays kept.
async fn settle<F>(
    now: &Now,
    on: bool,
    memory: &Path,
    exec: impl FnMut(Change) -> F,
) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
{
    let failed = run(turn(now, on, memory), exec).await;
    if !on {
        let has = |k| failed.iter().any(|(f, _)| *f == k);
        keep(
            memory,
            Radios {
                wifi: has(Kind::Wlan),
                wwan: has(Kind::Wwan),
                bluetooth: has(Kind::Bluetooth),
            },
        );
    }
    failures(failed)
}

/// Turn Wi-Fi off while it is on and on while it is off; whether it is on afterwards.
pub async fn toggle_wifi(conn: &Connection) -> Result<bool, String> {
    let now = Now::read(conn).await;
    let on = !now.wifi_on();
    failures(run(radio(&now, Kind::Wlan, on), |c| exec(conn, c)).await)?;
    Ok(on)
}

/// A radio kind switched on or off through NetworkManager or the rfkill soft block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    Nm(Kind, bool),
    Rfkill(Kind, bool),
}

/// What turns `now` into `want`. Turning airplane mode on only turns radios off and turning it
/// off only turns them on, so a radio switched by hand in between keeps its state.
fn changes(now: Radios, want: Radios, on: bool) -> Vec<(Kind, bool)> {
    [
        (now.wifi, want.wifi, Kind::Wlan),
        (now.wwan, want.wwan, Kind::Wwan),
        (now.bluetooth, want.bluetooth, Kind::Bluetooth),
    ]
    .into_iter()
    .filter(|(now, want, _)| now != want && *now == on)
    .map(|(_, want, kind)| (kind, want))
    .collect()
}

fn radio_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Wlan => "Wi-Fi",
        Kind::Wwan => "Mobile broadband",
        Kind::Bluetooth => "Bluetooth",
    }
}

/// Run `changes` through `exec`; the kinds that failed, with why. Once a change of a kind
/// fails, the kind's later changes are skipped, so its soft block and NetworkManager's switch
/// never disagree.
async fn run<F>(changes: Vec<Change>, mut exec: impl FnMut(Change) -> F) -> Vec<(Kind, String)>
where
    F: Future<Output = Result<(), String>>,
{
    let mut failed: Vec<(Kind, String)> = Vec::new();
    for change in changes {
        if failed.iter().any(|(k, _)| *k == change.kind()) {
            continue;
        }
        if let Err(e) = exec(change).await {
            log::warn!("airplane mode: {change:?} failed: {e}");
            failed.push((change.kind(), e));
        }
    }
    failed
}

fn failures(failed: Vec<(Kind, String)>) -> Result<(), String> {
    if failed.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = (failed.into_iter())
        .map(|(kind, e)| format!("{}: {e}", radio_name(kind)))
        .collect();
    Err(lines.join("\n"))
}

async fn exec(conn: &Connection, change: Change) -> Result<(), String> {
    match change {
        Change::Nm(kind, on) => {
            let key = if kind == Kind::Wlan {
                WIFI_KEY
            } else {
                WWAN_KEY
            };
            set_nm(conn, key, on).await
        }
        Change::Rfkill(kind, on) => rfkill::set_blocked(kind, !on).map_err(|e| e.to_string()),
    }
}

impl Change {
    fn kind(self) -> Kind {
        match self {
            Change::Nm(kind, _) | Change::Rfkill(kind, _) => kind,
        }
    }
}

#[cfg(test)]
impl Change {
    fn on(self) -> bool {
        match self {
            Change::Nm(_, on) | Change::Rfkill(_, on) => on,
        }
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
    set.map_err(|e| crate::error::message(&e).map_or_else(|| e.to_string(), str::to_owned))
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

/// Airplane mode goes on with `now` on: keep them for turning off, beside those an earlier turn
/// on kept that no turn off has brought back yet.
fn keep(path: &Path, now: Radios) {
    let kept = std::fs::read_to_string(path).map(|t| Radios::parse(&t));
    let keep = now.with(kept.unwrap_or_default());
    if keep.any_on() {
        remember(path, keep);
    }
}

/// Forget the kept radios that `now` has on.
fn brought_back(path: &Path, now: Radios) {
    let Ok(kept) = std::fs::read_to_string(path).map(|t| Radios::parse(&t)) else {
        return;
    };
    let left = kept.without(now);
    if !left.any_on() {
        let _ = std::fs::remove_file(path);
    } else if left != kept {
        remember(path, left);
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

    const WLAN_WWAN: u32 = WLAN_AVAILABLE | WWAN_AVAILABLE;

    fn switch(kind: Kind, soft: bool, hard: bool) -> Switch {
        Switch { kind, soft, hard }
    }

    /// A machine's radios as NetworkManager and the kernel keep them.
    struct Machine {
        user_wifi: bool,
        user_wwan: bool,
        radio_flags: u32,
        switches: Vec<Switch>,
    }

    impl Machine {
        /// `WirelessEnabled` or `WwanEnabled`: the user's switch while no switch of `kind` is
        /// blocked either way.
        fn reports(&self, kind: Kind, user: bool) -> bool {
            user && (self.switches.iter())
                .filter(|s| s.kind == kind)
                .all(|s| !s.soft && !s.hard)
        }

        fn now(&self) -> Now {
            Now {
                wifi: self.reports(Kind::Wlan, self.user_wifi),
                wwan: self.reports(Kind::Wwan, self.user_wwan),
                radio_flags: Some(self.radio_flags),
                switches: self.switches.clone(),
            }
        }

        fn soft_block(&mut self, kind: Kind, blocked: bool) {
            for s in self.switches.iter_mut().filter(|s| s.kind == kind) {
                s.soft = blocked;
            }
        }

        /// NetworkManager's `_rfkill_update_from_user`: a turn writes the soft block only when
        /// the radio's state with the hardware switches counted changes.
        fn apply(&mut self, changes: &[Change]) {
            for &change in changes {
                match change {
                    Change::Nm(kind, on) => {
                        let was = self.reports(kind, self.user(kind));
                        *self.user_mut(kind) = on;
                        let hw = self.switches.iter().all(|s| s.kind != kind || !s.hard);
                        if (on && hw) != was {
                            self.soft_block(kind, !(on && hw));
                        }
                    }
                    Change::Rfkill(kind, on) => self.soft_block(kind, !on),
                }
            }
        }

        fn user(&self, kind: Kind) -> bool {
            if kind == Kind::Wlan {
                self.user_wifi
            } else {
                self.user_wwan
            }
        }

        fn user_mut(&mut self, kind: Kind) -> &mut bool {
            if kind == Kind::Wlan {
                &mut self.user_wifi
            } else {
                &mut self.user_wwan
            }
        }

        fn lift(&mut self) {
            for s in &mut self.switches {
                s.hard = false;
            }
        }
    }

    fn machine(switches: Vec<Switch>) -> Machine {
        Machine {
            user_wifi: true,
            user_wwan: false,
            radio_flags: WLAN_AVAILABLE,
            switches,
        }
    }

    #[test]
    fn bluetooth_is_soft_on_while_any_of_its_switches_is_soft_unblocked() {
        let soft_on = |switches| machine(switches).now().soft().bluetooth;
        assert!(!soft_on(vec![switch(Kind::Wlan, false, false)]));
        let blocked = switch(Kind::Bluetooth, true, false);
        assert!(!soft_on(vec![blocked, blocked]));
        assert!(soft_on(vec![
            blocked,
            switch(Kind::Bluetooth, false, false)
        ]));
        assert!(soft_on(vec![switch(Kind::Bluetooth, false, true)]));
        assert!(!soft_on(vec![switch(Kind::Bluetooth, true, true)]));
    }

    #[test]
    fn a_hardware_kill_on_every_radio_reads_killed() {
        let mut m = machine(vec![
            switch(Kind::Wlan, false, true),
            switch(Kind::Wwan, false, true),
            switch(Kind::Bluetooth, false, true),
        ]);
        m.radio_flags = WLAN_WWAN;
        assert!(
            !m.now().wifi,
            "NetworkManager reports Wi-Fi off under a hard block"
        );
        assert_eq!(m.now().mode(), Some(Mode::Killed));
        m.switches[2].hard = false;
        assert_eq!(m.now().mode(), Some(Mode::Off));
        assert_eq!(
            machine(vec![]).now().mode(),
            None,
            "no switch, no airplane mode"
        );
    }

    #[test]
    fn a_modem_without_a_switch_keeps_a_hardware_kill_from_reading_killed() {
        let mut m = machine(vec![
            switch(Kind::Wlan, true, true),
            switch(Kind::Bluetooth, true, true),
        ]);
        m.radio_flags = WLAN_WWAN;
        m.user_wwan = true;
        assert_eq!(m.now().mode(), Some(Mode::Off));
        m.user_wwan = false;
        assert_eq!(m.now().mode(), Some(Mode::On));
        m.radio_flags = WLAN_AVAILABLE;
        assert_eq!(m.now().mode(), Some(Mode::Killed));
    }

    #[test]
    fn a_radio_without_its_hardware_counts_as_off() {
        let now = |radio_flags| Now {
            wifi: true,
            wwan: true,
            radio_flags,
            switches: vec![switch(Kind::Wlan, false, false)],
        };
        let r = now(Some(WLAN_AVAILABLE)).soft();
        assert_eq!((r.wifi, r.wwan), (true, false));
        let r = now(Some(WWAN_AVAILABLE)).soft();
        assert_eq!((r.wifi, r.wwan), (false, true));
        let r = now(None).soft();
        assert_eq!(
            (r.wifi, r.wwan),
            (true, false),
            "no RadioFlags: where a switch is"
        );
    }

    #[test]
    fn a_soft_unblocked_bluetooth_under_a_hard_block_keeps_airplane_mode_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let mut m = machine(vec![
            switch(Kind::Wlan, true, false),
            switch(Kind::Bluetooth, false, true),
        ]);
        m.user_wifi = false;
        assert_eq!(m.now().mode(), Some(Mode::Off));
        let on = turn(&m.now(), true, &path);
        assert_eq!(on, [Change::Rfkill(Kind::Bluetooth, false)]);
        m.apply(&on);
        assert_eq!(m.now().mode(), Some(Mode::On));
    }

    #[test]
    fn airplane_on_soft_blocks_a_hard_blocked_bluetooth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let mut m = machine(vec![
            switch(Kind::Wlan, false, false),
            switch(Kind::Bluetooth, false, true),
        ]);
        let on = turn(&m.now(), true, &path);
        assert_eq!(
            on,
            [
                Change::Nm(Kind::Wlan, false),
                Change::Rfkill(Kind::Bluetooth, false)
            ]
        );
        m.apply(&on);
        m.lift();
        assert_eq!(m.now().mode(), Some(Mode::On), "Bluetooth stays off");
        assert_eq!(
            turn(&m.now(), false, &path),
            [
                Change::Nm(Kind::Wlan, true),
                Change::Rfkill(Kind::Bluetooth, true)
            ]
        );
    }

    #[test]
    fn airplane_on_soft_blocks_a_wifi_a_hardware_switch_holds_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let mut m = machine(vec![
            switch(Kind::Wlan, false, true),
            switch(Kind::Bluetooth, true, false),
        ]);
        assert_eq!(m.now().mode(), Some(Mode::Off), "Wi-Fi is soft-unblocked");
        let on = turn(&m.now(), true, &path);
        assert_eq!(
            on,
            [
                Change::Rfkill(Kind::Wlan, false),
                Change::Nm(Kind::Wlan, false)
            ]
        );
        m.apply(&on);
        assert_eq!(m.now().mode(), Some(Mode::On));
        m.lift();
        assert!(!m.now().wifi, "Wi-Fi stays off once the switch lifts");
        assert_eq!(m.now().mode(), Some(Mode::On));
        let off = turn(&m.now(), false, &path);
        assert_eq!(off, [Change::Nm(Kind::Wlan, true)]);
        m.apply(&off);
        assert!(m.now().wifi);
    }

    #[test]
    fn airplane_off_under_a_wifi_hard_block_unblocks_it_for_when_the_switch_lifts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let mut m = machine(vec![
            switch(Kind::Wlan, false, true),
            switch(Kind::Bluetooth, false, false),
        ]);
        let on = turn(&m.now(), true, &path);
        m.apply(&on);
        let off = turn(&m.now(), false, &path);
        assert_eq!(
            off,
            [
                Change::Rfkill(Kind::Wlan, true),
                Change::Nm(Kind::Wlan, true),
                Change::Rfkill(Kind::Bluetooth, true)
            ]
        );
        m.apply(&off);
        m.lift();
        assert!(m.now().wifi);
    }

    /// Runs each change on `m`, failing the Wi-Fi soft block as a missing `/dev/rfkill` would.
    fn without_wifi_rfkill(
        m: &mut Machine,
    ) -> impl FnMut(Change) -> std::future::Ready<Result<(), String>> + '_ {
        move |c| {
            std::future::ready(if c == Change::Rfkill(Kind::Wlan, c.on()) {
                Err("No such file or directory".into())
            } else {
                m.apply(&[c]);
                Ok(())
            })
        }
    }

    #[test]
    fn a_failed_soft_block_leaves_networkmanager_and_the_kept_radios_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let held = || {
            machine(vec![
                switch(Kind::Wlan, false, true),
                switch(Kind::Bluetooth, false, false),
            ])
        };
        let mut m = held();
        let now = m.now();
        let r = zbus::block_on(settle(&now, true, &path, without_wifi_rfkill(&mut m)));
        assert_eq!(r, Err("Wi-Fi: No such file or directory".into()));
        assert!(m.user_wifi, "NetworkManager's switch stays on");
        assert_eq!(m.now().mode(), Some(Mode::Off), "Wi-Fi is still on");
        brought_back(&path, m.now().soft());

        let mut m = held();
        let now = m.now();
        let all = |c| {
            m.apply(&[c]);
            std::future::ready(Ok(()))
        };
        zbus::block_on(settle(&now, true, &path, all)).unwrap();
        let now = m.now();
        let r = zbus::block_on(settle(&now, false, &path, without_wifi_rfkill(&mut m)));
        assert!(r.is_err());
        assert!(!m.user_wifi, "NetworkManager's switch stays off");
        assert!(m.now().soft().bluetooth);
        brought_back(&path, m.now().soft());
        let wifi = Radios {
            wifi: true,
            ..Radios::default()
        };
        assert_eq!(recall(&path), wifi, "Wi-Fi stays kept");
    }

    #[test]
    fn the_wifi_key_under_a_hard_block_turns_the_soft_block_with_networkmanager() {
        let mut m = machine(vec![switch(Kind::Wlan, true, true)]);
        m.user_wifi = false;
        let now = m.now();
        assert!(!now.wifi_on());
        let on = radio(&now, Kind::Wlan, true);
        assert_eq!(
            on,
            [
                Change::Rfkill(Kind::Wlan, true),
                Change::Nm(Kind::Wlan, true)
            ]
        );
        m.apply(&on);
        m.lift();
        assert!(m.now().wifi, "Wi-Fi comes on once the switch lifts");

        let free = machine(vec![switch(Kind::Wlan, false, false)]).now();
        assert!(free.wifi_on());
        assert_eq!(
            radio(&free, Kind::Wlan, false),
            [Change::Nm(Kind::Wlan, false)]
        );
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
            [(Kind::Wlan, false), (Kind::Bluetooth, false)]
        );
    }

    #[test]
    fn under_a_hardware_kill_airplane_mode_takes_no_turn_and_keeps_the_kept_radios() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let now = machine(vec![
            switch(Kind::Wlan, false, true),
            switch(Kind::Bluetooth, false, true),
        ])
        .now();
        assert_eq!(now.mode(), Some(Mode::Killed));
        assert!(Mode::Killed.on());
        assert_eq!(Mode::Killed.flipped(), None);
        let wifi = Radios {
            wifi: true,
            ..Radios::default()
        };
        remember(&path, wifi);
        assert_eq!(turn(&now, false, &path), []);
        assert_eq!(turn(&now, true, &path), []);
        assert_eq!(recall(&path), wifi, "the kept radios stay kept");
    }

    #[test]
    fn turning_off_switches_on_only_what_was_kept() {
        let kept = Radios {
            wifi: true,
            ..Radios::default()
        };
        assert_eq!(
            changes(Radios::default(), kept, false),
            [(Kind::Wlan, true)]
        );
        let bluetooth_by_hand = Radios {
            bluetooth: true,
            ..Radios::default()
        };
        assert_eq!(
            changes(bluetooth_by_hand, kept, false),
            [(Kind::Wlan, true)],
            "a radio turned on by hand stays on"
        );
    }

    #[test]
    fn a_second_turn_on_keeps_what_the_first_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let wifi_and_bt = Radios {
            wifi: true,
            wwan: false,
            bluetooth: true,
        };
        keep(&path, wifi_and_bt);
        let bt_by_hand = Radios {
            bluetooth: true,
            ..Radios::default()
        };
        keep(&path, bt_by_hand);
        assert_eq!(recall(&path), wifi_and_bt);
        keep(&path, Radios::default());
        assert!(!path.exists(), "nothing on, nothing kept");
    }

    #[test]
    fn a_kept_radio_brought_back_by_hand_is_not_turned_on_by_a_later_turn_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reprise/airplane-mode");
        let wifi = Radios {
            wifi: true,
            ..Radios::default()
        };
        let bt = Radios {
            bluetooth: true,
            ..Radios::default()
        };
        keep(&path, wifi.with(bt));
        brought_back(&path, wifi);
        brought_back(&path, wifi.with(bt));
        assert!(!path.exists(), "every kept radio came back by hand");
        keep(&path, wifi);
        assert_eq!(
            recall(&path),
            wifi,
            "Bluetooth, turned off since, stays off"
        );

        keep(&path, wifi.with(bt));
        brought_back(&path, bt);
        keep(&path, bt);
        assert_eq!(recall(&path), wifi.with(bt), "Wi-Fi never came back");
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
