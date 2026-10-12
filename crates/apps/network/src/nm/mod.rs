//! NetworkManager state folded into the model the windows draw: a pure fold of the property maps `bus` collected, testable without a daemon.
//! `bus` holds the thread, subscriptions and commands; its `watch` and `NmHandle` are re-exported here.

mod bus;

pub use bus::{NmHandle, watch};

use std::collections::{HashMap, HashSet};
use std::time::Instant;
use zbus::zvariant::OwnedValue;
use zeroize::Zeroizing;

pub type Props = HashMap<String, OwnedValue>;

/// The two device kinds the applet shows; everything else NM manages
/// (bridges, tunnels, loopback) is left out of the snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Wired,
    Wifi,
}

/// A device's state, the folder's vocabulary rather than NM's twelve codes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    Unmanaged,
    #[default]
    Unavailable,
    Disconnected,
    Connecting,
    Connected,
    Failed,
}

impl State {
    /// NM's `NM_DEVICE_STATE`: 10 unmanaged, 20 unavailable, 30
    /// disconnected, 40..=90 the connecting steps, 100 activated, 110
    /// deactivating (still busy, so connecting), 120 failed.
    pub fn from_code(code: u32) -> State {
        match code {
            10 => State::Unmanaged,
            30 => State::Disconnected,
            40..=90 | 110 => State::Connecting,
            100 => State::Connected,
            120 => State::Failed,
            _ => State::Unavailable,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ip4 {
    pub addr: String,
    pub prefix: u8,
    pub gateway: String,
    pub dns: Vec<String>,
    pub dhcp: bool,
}

pub fn subnet_mask(prefix: u8) -> String {
    let bits: u32 = match prefix {
        0 => 0,
        p if p >= 32 => u32::MAX,
        p => u32::MAX << (32 - p),
    };
    format!(
        "{}.{}.{}.{}",
        bits >> 24,
        (bits >> 16) & 0xff,
        (bits >> 8) & 0xff,
        bits & 0xff
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Security {
    #[default]
    Open,
    Psk,
    Sae,
    Enterprise,
}

impl Security {
    /// `Flags` (bit 0 privacy), `WpaFlags` and `RsnFlags` (0x100 PSK,
    /// 0x200 802.1X, 0x400 SAE). WEP, privacy with neither WPA nor RSN,
    /// needs a key the applet cannot ask for, so it counts as enterprise
    /// and sends the user to the connection editor.
    pub fn from_flags(flags: u32, wpa: u32, rsn: u32) -> Security {
        let both = wpa | rsn;
        if both & 0x200 != 0 {
            Security::Enterprise
        } else if both & 0x100 != 0 {
            Security::Psk
        } else if rsn & 0x400 != 0 {
            Security::Sae
        } else if flags & 1 != 0 {
            Security::Enterprise
        } else {
            Security::Open
        }
    }
}

/// A network's name as NetworkManager keeps it: raw bytes, which need not
/// be UTF-8. Two SSIDs are the same network only when their bytes are;
/// `Display` is a lossy rendering for the windows, `Debug` an escaped one
/// for the log.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ssid(pub Vec<u8>);

impl std::fmt::Debug for Ssid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ssid(\"{}\")", self.0.escape_ascii())
    }
}

impl Ssid {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Display for Ssid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.0))
    }
}

impl From<&str> for Ssid {
    fn from(s: &str) -> Ssid {
        Ssid(s.as_bytes().to_vec())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccessPoint {
    pub path: String,
    pub ssid: Ssid,
    pub strength: u8,
    pub security: Security,
    pub known: bool,
    /// The saved connection has no key NetworkManager can use on its
    /// own: its `psk-flags` leave the passphrase to a secret agent, the
    /// way plasma-nm and nm-applet keep it in a wallet. Connecting asks
    /// for the key and writes it into the profile.
    pub keyless: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wifi {
    pub active_ssid: Option<String>,
    pub strength: u8,
    pub aps: Vec<AccessPoint>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub path: String,
    pub iface: String,
    pub kind: Kind,
    /// The display name shown in the folder: "Local Area Connection",
    /// "Wireless Network Connection 2", ...
    pub shell_name: String,
    /// NM publishes no vendor string, so this is the driver and the
    /// interface: "e1000e (enp3s0)".
    pub product: String,
    pub state: State,
    pub managed: bool,
    pub carrier: bool,
    pub speed_mbit: u32,
    /// When the device last entered `Connected`; the status window's
    /// Duration counts from here.
    pub connected_since: Option<Instant>,
    pub ip4: Option<Ip4>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub wifi: Option<Wifi>,
    pub hotspot: Option<String>,
    /// Devices joined to the hotspot; None when that is not known.
    pub hotspot_clients: Option<usize>,
}

impl Device {
    /// "Hotspot: Desk (2 devices)" while the hotspot is up, without the count when it is not
    /// known.
    pub fn hotspot_line(&self) -> Option<String> {
        let ssid = self.hotspot.as_ref()?;
        Some(match self.hotspot_clients {
            None => format!("Hotspot: {ssid}"),
            Some(0) => format!("Hotspot: {ssid} (no devices)"),
            Some(1) => format!("Hotspot: {ssid} (1 device)"),
            Some(n) => format!("Hotspot: {ssid} ({n} devices)"),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vpn {
    pub conn_path: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
}

/// The name shown for a VPN kind: NM's plugin service name without its
/// `org.freedesktop.NetworkManager.` prefix, or `wireguard`.
pub fn vpn_kind_label(kind: &str) -> String {
    match kind {
        "openvpn" => "OpenVPN",
        "wireguard" => "WireGuard",
        "openconnect" => "OpenConnect",
        "vpnc" => "Cisco VPN (vpnc)",
        "l2tp" => "L2TP",
        "pptp" => "PPTP",
        "sstp" => "SSTP",
        "fortisslvpn" => "Fortinet SSL VPN",
        "strongswan" => "IPsec (strongSwan)",
        "libreswan" | "openswan" => "IPsec (Libreswan)",
        "ssh" => "SSH",
        "iodine" => "Iodine DNS tunnel",
        other => other,
    }
    .into()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connectivity {
    #[default]
    Unknown,
    None,
    Portal,
    Limited,
    Full,
}

impl Connectivity {
    /// NM's `NMConnectivityState`: 0 unknown, 1 none, 2 portal, 3
    /// limited, 4 full.
    pub fn from_code(code: u32) -> Connectivity {
        match code {
            1 => Connectivity::None,
            2 => Connectivity::Portal,
            3 => Connectivity::Limited,
            4 => Connectivity::Full,
            _ => Connectivity::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub vpns: Vec<Vpn>,
    pub primary: Option<String>,
    pub nm_running: bool,
    pub connectivity: Connectivity,
    /// The address NetworkManager's connectivity check fetches; empty when
    /// it has none. A browser sent there on a network with a sign-in page
    /// is redirected to that page.
    pub check_uri: String,
}

/// The display names shown in the folder ("Local Area Connection",
/// "Wireless Network Connection"), numbered per kind, the first one
/// unnumbered.
pub fn shell_names(kinds: &[Kind]) -> Vec<String> {
    let (mut wired, mut wifi) = (0, 0);
    kinds
        .iter()
        .map(|k| {
            let (stem, n) = match k {
                Kind::Wired => {
                    wired += 1;
                    ("Local Area Connection", wired)
                }
                Kind::Wifi => {
                    wifi += 1;
                    ("Wireless Network Connection", wifi)
                }
            };
            if n == 1 {
                stem.to_string()
            } else {
                format!("{stem} {n}")
            }
        })
        .collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeviceProps {
    pub path: String,
    pub device: Props,
    pub wired: Option<Props>,
    pub wireless: Option<Props>,
    pub ip4: Option<Props>,
    pub stats: Option<Props>,
    pub aps: Vec<(String, Props)>,
    /// The saved SSIDs, each with whether NetworkManager holds its key
    /// (see [`AccessPoint::keyless`]).
    pub known_ssids: HashMap<Ssid, bool>,
    pub active_ap: Option<String>,
    /// The device's `Dhcp4Config` is a real object, so the address came
    /// from DHCP rather than from a static profile.
    pub dhcp: bool,
    pub hotspot: Option<String>,
}

fn text(props: &Props, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

fn uint(props: &Props, key: &str) -> u32 {
    props
        .get(key)
        .and_then(|v| u32::try_from(v.clone()).ok())
        .unwrap_or(0)
}

fn big(props: &Props, key: &str) -> u64 {
    props
        .get(key)
        .and_then(|v| u64::try_from(v.clone()).ok())
        .unwrap_or(0)
}

fn flag(props: &Props, key: &str) -> bool {
    props
        .get(key)
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false)
}

/// An `aa{sv}` property: NM's `AddressData` and `NameserverData`.
fn dicts(props: &Props, key: &str) -> Vec<Props> {
    props
        .get(key)
        .and_then(|v| Vec::<Props>::try_from(v.clone()).ok())
        .unwrap_or_default()
}

fn ssid_of(props: &Props, key: &str) -> Ssid {
    Ssid(
        props
            .get(key)
            .and_then(|v| Vec::<u8>::try_from(v.clone()).ok())
            .unwrap_or_default(),
    )
}

/// One device's property maps folded into a `Device`. `shell_name` is left
/// empty: the numbering needs every device, so the caller fills it in
/// from `shell_names`, as it fills `connected_since` from the device's last
/// state.
pub fn fold_device(props: &DeviceProps) -> Device {
    let iface = text(&props.device, "Interface");
    let kind = if uint(&props.device, "DeviceType") == 2 {
        Kind::Wifi
    } else {
        Kind::Wired
    };
    let driver = text(&props.device, "Driver");
    let product = if driver.is_empty() {
        iface.clone()
    } else {
        format!("{driver} ({iface})")
    };
    let wired = props.wired.clone().unwrap_or_default();
    let wireless = props.wireless.clone().unwrap_or_default();
    let speed_mbit = match kind {
        Kind::Wired => uint(&wired, "Speed"),
        Kind::Wifi => uint(&wireless, "Bitrate") / 1000,
    };
    let stats = props.stats.clone().unwrap_or_default();
    Device {
        path: props.path.clone(),
        iface,
        kind,
        shell_name: String::new(),
        product,
        state: State::from_code(uint(&props.device, "State")),
        managed: flag(&props.device, "Managed"),
        carrier: kind == Kind::Wifi || flag(&wired, "Carrier"),
        speed_mbit,
        connected_since: None,
        ip4: props.ip4.as_ref().and_then(|m| fold_ip4(m, props.dhcp)),
        rx_bytes: big(&stats, "RxBytes"),
        tx_bytes: big(&stats, "TxBytes"),
        wifi: (kind == Kind::Wifi).then(|| fold_wifi(props)),
        hotspot: props.hotspot.clone(),
        hotspot_clients: None,
    }
}

fn fold_ip4(props: &Props, dhcp: bool) -> Option<Ip4> {
    let addresses = dicts(props, "AddressData");
    let first = addresses.first()?;
    Some(Ip4 {
        addr: text(first, "address"),
        prefix: uint(first, "prefix").min(32) as u8,
        gateway: text(props, "Gateway"),
        dns: dicts(props, "NameserverData")
            .iter()
            .map(|d| text(d, "address"))
            .collect(),
        dhcp,
    })
}

fn fold_wifi(props: &DeviceProps) -> Wifi {
    let mut aps: Vec<AccessPoint> = props
        .aps
        .iter()
        .map(|(path, ap)| {
            // A hidden AP beacons no SSID, so it folds to an empty one and no
            // saved profile claims it.
            let ssid = ssid_of(ap, "Ssid");
            let has_key = (!ssid.is_empty())
                .then(|| props.known_ssids.get(&ssid).copied())
                .flatten();
            AccessPoint {
                known: has_key.is_some(),
                keyless: has_key == Some(false),
                active: props.active_ap.as_deref() == Some(path.as_str()),
                path: path.clone(),
                ssid,
                strength: ap
                    .get("Strength")
                    .and_then(|v| u8::try_from(v.clone()).ok())
                    .unwrap_or(0),
                security: Security::from_flags(
                    uint(ap, "Flags"),
                    uint(ap, "WpaFlags"),
                    uint(ap, "RsnFlags"),
                ),
            }
        })
        .collect();
    sort_aps(&mut aps);
    let active = aps.iter().find(|a| a.active);
    Wifi {
        active_ssid: active.map(|a| a.ssid.to_string()),
        strength: active.map(|a| a.strength).unwrap_or(0),
        aps,
    }
}

pub fn sort_aps(aps: &mut [AccessPoint]) {
    aps.sort_by(|a, b| b.strength.cmp(&a.strength).then(a.ssid.cmp(&b.ssid)));
}

/// A secret: a Wi-Fi passphrase or a WireGuard key, zeroized on drop.
/// `Debug` redacts it so a logged `Cmd` never carries it into the session
/// log.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Psk(Zeroizing<String>);

impl Psk {
    pub fn new(key: &str) -> Psk {
        Psk(Zeroizing::new(key.to_string()))
    }
}

impl std::fmt::Debug for Psk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Psk(<redacted>)")
    }
}

impl std::ops::Deref for Psk {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Join `ssid` on `device` through access point `ap`; `psk` is the
    /// passphrase the user typed, None for an open or already saved one.
    Connect {
        device: String,
        ap: String,
        ssid: Ssid,
        psk: Option<Psk>,
    },
    Disconnect(String),
    Disable(String),
    Enable(String),
    Repair(String),
    VpnUp(String),
    VpnDown(String),
    LoadVpn(String),
    SaveWireGuard {
        conn_path: Option<String>,
        cfg: Box<crate::wg::Config>,
    },
    SavePluginVpn {
        conn_path: String,
        name: String,
        data: Vec<(String, String)>,
    },
    DeleteVpn(String),
    Import {
        path: std::path::PathBuf,
        kinds: Vec<String>,
    },
    Hotspot {
        device: String,
        ssid: String,
        psk: Psk,
        wpa3: bool,
        band: Band,
    },
    Rescan(String),
    Stats {
        device: String,
        on: bool,
    },
    Quit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Band {
    #[default]
    Auto,
    Bg,
    A,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VpnEdit {
    WireGuard {
        conn_path: String,
        cfg: Box<crate::wg::Config>,
    },
    Plugin {
        conn_path: String,
        name: String,
        kind: String,
        data: Vec<(String, String)>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum NmMsg {
    Snapshot(Snapshot),
    Vpn(VpnEdit),
    Failed { title: String, body: String },
}

pub fn known_and_vpns(
    saved: &[(String, HashMap<String, Props>)],
) -> (HashMap<Ssid, bool>, Vec<Vpn>) {
    let mut ssids = HashMap::new();
    let mut vpns = vec![];
    for (path, s) in saved {
        let meta = s.get("connection").cloned().unwrap_or_default();
        let kind = match text(&meta, "type").as_str() {
            "802-11-wireless" => {
                // A profile for a hidden network can carry no SSID at
                // all; an empty one would match every hidden AP.
                let ssid = s.get("802-11-wireless").map(|w| ssid_of(w, "ssid"));
                if let Some(ssid) = ssid.filter(|s| !s.is_empty()) {
                    let has_key = s.get("802-11-wireless-security").is_none_or(psk_in_profile);
                    ssids.insert(ssid, has_key);
                }
                continue;
            }
            // NM names the VPN plugin by its bus name; the last component
            // is the kind ("org.freedesktop.NetworkManager.openvpn").
            "vpn" => s
                .get("vpn")
                .map(|v| {
                    let service = text(v, "service-type");
                    service.rsplit('.').next().unwrap_or_default().to_string()
                })
                .unwrap_or_default(),
            "wireguard" => "wireguard".to_string(),
            _ => continue,
        };
        vpns.push(Vpn {
            conn_path: path.clone(),
            name: text(&meta, "id"),
            kind,
            active: false,
        });
    }
    vpns.sort_by(|a, b| a.name.cmp(&b.name));
    (ssids, vpns)
}

pub fn hotspot_profiles(saved: &[(String, HashMap<String, Props>)]) -> HashMap<String, String> {
    saved
        .iter()
        .filter_map(|(path, s)| {
            let w = s.get("802-11-wireless")?;
            (text(w, "mode") == "ap").then(|| (path.clone(), ssid_of(w, "ssid").to_string()))
        })
        .collect()
}

pub fn plugin_vpn(conn_path: &str, s: &HashMap<String, Props>) -> VpnEdit {
    let empty = Props::new();
    let vpn = s.get("vpn").unwrap_or(&empty);
    let mut data: Vec<(String, String)> = vpn
        .get("data")
        .and_then(|v| HashMap::<String, String>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .collect();
    data.sort();
    VpnEdit::Plugin {
        conn_path: conn_path.to_string(),
        name: s
            .get("connection")
            .map(|c| text(c, "id"))
            .unwrap_or_default(),
        kind: text(vpn, "service-type")
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_string(),
        data,
    }
}

pub fn owned_to_values(current: HashMap<String, Props>) -> crate::wg::Settings {
    current
        .into_iter()
        .map(|(group, entries)| {
            let entries = entries
                .into_iter()
                .map(|(k, v)| (k, zbus::zvariant::Value::from(v)))
                .collect();
            (group, entries)
        })
        .collect()
}

/// `current` with every group of `fresh` in place of its own, except
/// `connection`, which takes `fresh`'s keys over its own so the UUID
/// stays.
pub fn merge_groups(
    current: HashMap<String, Props>,
    fresh: crate::wg::Settings,
) -> crate::wg::Settings {
    let mut out = owned_to_values(current);
    for (group, entries) in fresh {
        if group == "connection" {
            out.entry(group).or_default().extend(entries);
        } else {
            out.insert(group, entries);
        }
    }
    out
}

/// `current` (a plugin VPN's `GetSettings`) renamed to `name` with its
/// `vpn.data` replaced by `data`; every other setting stays. No secret
/// goes back, so NetworkManager keeps the stored ones.
pub fn plugin_update(
    current: HashMap<String, Props>,
    name: &str,
    data: &[(String, String)],
) -> crate::wg::Settings {
    let mut out = owned_to_values(current);
    out.entry("connection".into())
        .or_default()
        .insert("id".into(), name.trim().to_string().into());
    let dict: HashMap<String, String> = data.iter().cloned().collect();
    out.entry("vpn".into())
        .or_default()
        .insert("data".into(), dict.into());
    out
}

/// `psk-flags` bit 0 (agent-owned) and bit 1 (not saved) both hand the
/// passphrase to a secret agent, which this desktop has none of.
pub const PSK_FLAGS_AGENT: u32 = 0x1 | 0x2;

/// Whether a profile's security group leaves NetworkManager a key it can
/// dial with: an open profile, or a PSK one whose `psk-flags` keep the
/// passphrase in the profile.
pub fn psk_in_profile(security: &Props) -> bool {
    match text(security, "key-mgmt").as_str() {
        "wpa-psk" | "sae" => uint(security, "psk-flags") & PSK_FLAGS_AGENT == 0,
        _ => true,
    }
}

/// What `Cmd::Connect` should do about the profile, decided before any
/// call goes out. A saved profile is never duplicated: a passphrase
/// typed for one that already exists rewrites its key instead. The plan
/// borrows the passphrase, so the `Cmd`'s copy is the only one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectPlan<'a> {
    Activate(String),
    Rekey { conn_path: String, psk: &'a Psk },
    Add { psk: Option<&'a Psk> },
}

/// The plan for an SSID: `known` is the saved connection carrying it, if
/// there is one, and `psk` what the user typed, if anything.
pub fn connect_plan<'a>(known: Option<&str>, psk: Option<&'a Psk>) -> ConnectPlan<'a> {
    match (known, psk) {
        (Some(conn_path), Some(psk)) => ConnectPlan::Rekey {
            conn_path: conn_path.to_string(),
            psk,
        },
        (Some(conn_path), None) => ConnectPlan::Activate(conn_path.to_string()),
        (None, psk) => ConnectPlan::Add { psk },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    /// The daemon logs a failed `Cmd` with `{:?}`; the key must not be in it.
    #[test]
    fn logged_connect_does_not_carry_the_passphrase() {
        let cmd = Cmd::Connect {
            device: "/dev/0".into(),
            ap: "/ap/1".into(),
            ssid: "home".into(),
            psk: Some(Psk::new("hunter2")),
        };
        let logged = format!("{cmd:?}");
        assert!(!logged.contains("hunter2"), "{logged}");
        assert!(logged.contains("home"));
        assert_eq!(&*Psk::new("hunter2"), "hunter2");
    }

    fn props(entries: Vec<(&str, Value<'static>)>) -> Props {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.try_to_owned().unwrap()))
            .collect()
    }

    #[test]
    fn state_codes_map() {
        assert_eq!(State::from_code(10), State::Unmanaged);
        assert_eq!(State::from_code(30), State::Disconnected);
        assert_eq!(State::from_code(50), State::Connecting);
        assert_eq!(State::from_code(100), State::Connected);
        assert_eq!(State::from_code(120), State::Failed);
        assert_eq!(State::from_code(110), State::Connecting);
    }

    #[test]
    fn security_from_flags() {
        assert_eq!(Security::from_flags(0, 0, 0), Security::Open);
        assert_eq!(Security::from_flags(1, 0, 0x100), Security::Psk);
        assert_eq!(Security::from_flags(1, 0, 0x400), Security::Sae);
        assert_eq!(
            Security::from_flags(1, 0, 0x500),
            Security::Psk,
            "PSK beside SAE is Psk"
        );
        assert_eq!(
            Security::from_flags(1, 0x100, 0x400),
            Security::Psk,
            "WPA1 PSK beside SAE is Psk"
        );
        assert_eq!(Security::from_flags(1, 0, 0x200), Security::Enterprise);
        assert_eq!(Security::from_flags(1, 0, 0), Security::Enterprise);
    }

    #[test]
    fn shell_names_number_per_kind() {
        assert_eq!(
            shell_names(&[Kind::Wired, Kind::Wifi, Kind::Wired]),
            vec![
                "Local Area Connection".to_string(),
                "Wireless Network Connection".to_string(),
                "Local Area Connection 2".to_string(),
            ]
        );
    }

    #[test]
    fn subnet_mask_from_prefix() {
        assert_eq!(subnet_mask(24), "255.255.255.0");
        assert_eq!(subnet_mask(16), "255.255.0.0");
        assert_eq!(subnet_mask(22), "255.255.252.0");
    }

    #[test]
    fn fold_device_reads_a_wired_device() {
        let address = props(vec![
            ("address", Value::from("10.0.0.5")),
            ("prefix", Value::from(24u32)),
        ]);
        let d = fold_device(&DeviceProps {
            path: "/org/freedesktop/NetworkManager/Devices/2".into(),
            device: props(vec![
                ("Interface", Value::from("enp3s0")),
                ("DeviceType", Value::from(1u32)),
                ("State", Value::from(100u32)),
                ("Managed", Value::from(true)),
                ("Driver", Value::from("e1000e")),
            ]),
            wired: Some(props(vec![
                ("Speed", Value::from(1000u32)),
                ("Carrier", Value::from(true)),
            ])),
            wireless: None,
            ip4: Some(props(vec![
                ("AddressData", Value::from(vec![address])),
                ("Gateway", Value::from("10.0.0.1")),
            ])),
            stats: Some(props(vec![("RxBytes", Value::from(1234u64))])),
            aps: vec![],
            known_ssids: HashMap::new(),
            active_ap: None,
            dhcp: true,
            hotspot: None,
        });
        assert_eq!(d.iface, "enp3s0");
        assert_eq!(d.kind, Kind::Wired);
        assert_eq!(d.state, State::Connected);
        assert_eq!(d.product, "e1000e (enp3s0)");
        assert!(d.managed);
        assert!(d.carrier);
        assert_eq!(d.speed_mbit, 1000);
        assert_eq!(d.rx_bytes, 1234);
        assert_eq!(d.tx_bytes, 0);
        assert_eq!(
            d.ip4,
            Some(Ip4 {
                addr: "10.0.0.5".into(),
                prefix: 24,
                gateway: "10.0.0.1".into(),
                dns: vec![],
                dhcp: true,
            })
        );
        assert_eq!(d.wifi, None);
    }

    #[test]
    fn fold_device_reads_wifi_aps() {
        let ap = |ssid: &str, strength: u8, rsn: u32| {
            props(vec![
                ("Ssid", Value::from(ssid.as_bytes().to_vec())),
                ("Strength", Value::from(strength)),
                ("Flags", Value::from(1u32)),
                ("WpaFlags", Value::from(0u32)),
                ("RsnFlags", Value::from(rsn)),
            ])
        };
        let d = fold_device(&DeviceProps {
            path: "/org/freedesktop/NetworkManager/Devices/3".into(),
            device: props(vec![
                ("Interface", Value::from("wlan0")),
                ("DeviceType", Value::from(2u32)),
                ("State", Value::from(100u32)),
                ("Managed", Value::from(true)),
                ("Driver", Value::from("iwlwifi")),
            ]),
            wired: None,
            wireless: Some(props(vec![("Bitrate", Value::from(130000u32))])),
            ip4: None,
            stats: None,
            aps: vec![
                ("/ap/1".to_string(), ap("weak", 30, 0x100)),
                ("/ap/2".to_string(), ap("strong", 80, 0x200)),
            ],
            known_ssids: HashMap::from([("weak".into(), false)]),
            active_ap: Some("/ap/2".to_string()),
            dhcp: false,
            hotspot: Some("Desk".into()),
        });
        assert_eq!(d.kind, Kind::Wifi);
        assert!(d.carrier);
        assert_eq!(d.speed_mbit, 130);
        assert_eq!(d.hotspot.as_deref(), Some("Desk"));
        let wifi = d.wifi.expect("a Wi-Fi device has a Wifi block");
        assert_eq!(wifi.active_ssid.as_deref(), Some("strong"));
        assert_eq!(wifi.strength, 80);
        assert_eq!(
            wifi.aps,
            vec![
                AccessPoint {
                    path: "/ap/2".into(),
                    ssid: "strong".into(),
                    strength: 80,
                    security: Security::Enterprise,
                    known: false,
                    keyless: false,
                    active: true,
                },
                AccessPoint {
                    path: "/ap/1".into(),
                    ssid: "weak".into(),
                    strength: 30,
                    security: Security::Psk,
                    known: true,
                    keyless: true,
                    active: false,
                },
            ]
        );
    }

    #[test]
    fn the_hotspot_line_counts_its_devices_when_known() {
        let mut d = Device::default();
        assert_eq!(d.hotspot_line(), None);
        d.hotspot = Some("Desk".into());
        assert_eq!(d.hotspot_line().as_deref(), Some("Hotspot: Desk"));
        let mut line = |n| {
            d.hotspot_clients = Some(n);
            d.hotspot_line().unwrap()
        };
        assert_eq!(line(0), "Hotspot: Desk (no devices)");
        assert_eq!(line(1), "Hotspot: Desk (1 device)");
        assert_eq!(line(3), "Hotspot: Desk (3 devices)");
    }

    #[test]
    fn vpn_kind_labels() {
        assert_eq!(vpn_kind_label("openvpn"), "OpenVPN");
        assert_eq!(vpn_kind_label("wireguard"), "WireGuard");
        assert_eq!(vpn_kind_label("openconnect"), "OpenConnect");
        assert_eq!(vpn_kind_label("vpnc"), "Cisco VPN (vpnc)");
        assert_eq!(vpn_kind_label("l2tp"), "L2TP");
        assert_eq!(vpn_kind_label("pptp"), "PPTP");
        assert_eq!(vpn_kind_label("sstp"), "SSTP");
        assert_eq!(vpn_kind_label("fortisslvpn"), "Fortinet SSL VPN");
        assert_eq!(vpn_kind_label("strongswan"), "IPsec (strongSwan)");
        assert_eq!(vpn_kind_label("libreswan"), "IPsec (Libreswan)");
        assert_eq!(vpn_kind_label("someplugin"), "someplugin");
    }

    #[test]
    fn known_and_vpns_reads_the_saved_connections() {
        let saved = vec![
            (
                "/Settings/1".to_string(),
                HashMap::from([
                    (
                        "connection".to_string(),
                        props(vec![
                            ("type", Value::from("802-11-wireless")),
                            ("id", Value::from("Intrepid 5")),
                        ]),
                    ),
                    (
                        "802-11-wireless".to_string(),
                        props(vec![("ssid", Value::from(b"Intrepid 5".to_vec()))]),
                    ),
                    (
                        "802-11-wireless-security".to_string(),
                        props(vec![
                            ("key-mgmt", Value::from("wpa-psk")),
                            ("psk-flags", Value::from(0u32)),
                        ]),
                    ),
                ]),
            ),
            (
                // Another desktop's profile: its applet keeps the key in
                // a wallet and NetworkManager never sees it.
                "/Settings/6".to_string(),
                HashMap::from([
                    (
                        "connection".to_string(),
                        props(vec![
                            ("type", Value::from("802-11-wireless")),
                            ("id", Value::from("Office")),
                        ]),
                    ),
                    (
                        "802-11-wireless".to_string(),
                        props(vec![("ssid", Value::from(b"Office".to_vec()))]),
                    ),
                    (
                        "802-11-wireless-security".to_string(),
                        props(vec![
                            ("key-mgmt", Value::from("wpa-psk")),
                            ("psk-flags", Value::from(1u32)),
                        ]),
                    ),
                ]),
            ),
            (
                "/Settings/2".to_string(),
                HashMap::from([(
                    "connection".to_string(),
                    props(vec![("type", Value::from("802-11-wireless"))]),
                )]),
            ),
            (
                "/Settings/3".to_string(),
                HashMap::from([
                    (
                        "connection".to_string(),
                        props(vec![
                            ("type", Value::from("vpn")),
                            ("id", Value::from("Work")),
                        ]),
                    ),
                    (
                        "vpn".to_string(),
                        props(vec![(
                            "service-type",
                            Value::from("org.freedesktop.NetworkManager.openvpn"),
                        )]),
                    ),
                ]),
            ),
            (
                "/Settings/4".to_string(),
                HashMap::from([(
                    "connection".to_string(),
                    props(vec![
                        ("type", Value::from("wireguard")),
                        ("id", Value::from("Away")),
                    ]),
                )]),
            ),
            (
                "/Settings/5".to_string(),
                HashMap::from([(
                    "connection".to_string(),
                    props(vec![("type", Value::from("802-3-ethernet"))]),
                )]),
            ),
        ];
        let (ssids, vpns) = known_and_vpns(&saved);
        assert_eq!(
            ssids,
            HashMap::from([("Intrepid 5".into(), true), ("Office".into(), false)])
        );
        assert_eq!(
            vpns,
            vec![
                Vpn {
                    conn_path: "/Settings/4".into(),
                    name: "Away".into(),
                    kind: "wireguard".into(),
                    active: false,
                },
                Vpn {
                    conn_path: "/Settings/3".into(),
                    name: "Work".into(),
                    kind: "openvpn".into(),
                    active: false,
                },
            ]
        );
    }

    #[test]
    fn connectivity_codes_map() {
        assert_eq!(Connectivity::from_code(0), Connectivity::Unknown);
        assert_eq!(Connectivity::from_code(1), Connectivity::None);
        assert_eq!(Connectivity::from_code(2), Connectivity::Portal);
        assert_eq!(Connectivity::from_code(3), Connectivity::Limited);
        assert_eq!(Connectivity::from_code(4), Connectivity::Full);
    }

    #[test]
    fn hotspot_profiles_are_the_access_point_ones() {
        let wifi = |mode: &str, ssid: &str| {
            HashMap::from([(
                "802-11-wireless".to_string(),
                props(vec![
                    ("mode", Value::from(mode.to_string())),
                    ("ssid", Value::from(ssid.as_bytes().to_vec())),
                ]),
            )])
        };
        let saved = vec![
            ("/Settings/1".to_string(), wifi("infrastructure", "Home")),
            ("/Settings/2".to_string(), wifi("ap", "Desk")),
        ];
        assert_eq!(
            hotspot_profiles(&saved),
            HashMap::from([("/Settings/2".to_string(), "Desk".to_string())])
        );
    }

    #[test]
    fn a_plugin_vpn_edit_renames_and_rewrites_its_data_only() {
        let data = HashMap::from([
            ("remote".to_string(), "vpn.example.org".to_string()),
            ("dev".to_string(), "tun".to_string()),
        ]);
        let current = HashMap::from([
            (
                "connection".to_string(),
                props(vec![
                    ("id", Value::from("Work")),
                    ("uuid", Value::from("abcd")),
                ]),
            ),
            (
                "vpn".to_string(),
                props(vec![
                    (
                        "service-type",
                        Value::from("org.freedesktop.NetworkManager.openvpn"),
                    ),
                    ("data", Value::from(data)),
                ]),
            ),
        ]);
        assert_eq!(
            plugin_vpn("/Settings/3", &current),
            VpnEdit::Plugin {
                conn_path: "/Settings/3".into(),
                name: "Work".into(),
                kind: "openvpn".into(),
                data: vec![
                    ("dev".into(), "tun".into()),
                    ("remote".into(), "vpn.example.org".into()),
                ],
            }
        );
        let s = plugin_update(current, "Office", &[("remote".into(), "b.example".into())]);
        assert_eq!(s["connection"]["id"], Value::from("Office"));
        assert_eq!(s["connection"]["uuid"], Value::from("abcd"));
        assert_eq!(
            s["vpn"]["data"],
            Value::from(HashMap::from([(
                "remote".to_string(),
                "b.example".to_string()
            )]))
        );
        assert_eq!(
            s["vpn"]["service-type"],
            Value::from("org.freedesktop.NetworkManager.openvpn")
        );
    }

    #[test]
    fn connect_never_duplicates_a_saved_profile() {
        let psk = Psk::new("hunter2");
        assert_eq!(
            connect_plan(Some("/Settings/1"), None),
            ConnectPlan::Activate("/Settings/1".into())
        );
        assert_eq!(
            connect_plan(Some("/Settings/1"), Some(&psk)),
            ConnectPlan::Rekey {
                conn_path: "/Settings/1".into(),
                psk: &psk,
            }
        );
        assert_eq!(
            connect_plan(None, Some(&psk)),
            ConnectPlan::Add { psk: Some(&psk) }
        );
        assert_eq!(connect_plan(None, None), ConnectPlan::Add { psk: None });
    }
}
