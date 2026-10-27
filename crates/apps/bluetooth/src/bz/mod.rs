//! What BlueZ reports, folded into the model the windows draw, and the messages to and from the bus thread.
//! Nothing here touches the bus: [`fold`] reads the maps `GetManagedObjects` returns.

mod agent;
mod bus;
mod rfkill;

pub use bus::{BzHandle, watch};

use std::collections::HashMap;

use reprisekit::art::devicon;
use zbus::{OwnedObjectPath, OwnedVariant};

pub type Props = HashMap<String, OwnedVariant>;
pub type Objects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

pub const ADAPTER_IFACE: &str = "org.bluez.Adapter1";
pub const DEVICE_IFACE: &str = "org.bluez.Device1";
pub const BATTERY_IFACE: &str = "org.bluez.Battery1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The first adapter in path order; None without bluetoothd or a
    /// radio.
    pub adapter: Option<Adapter>,
    pub devices: Vec<Device>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapter {
    pub path: String,
    pub powered: bool,
    pub blocked: bool,
    pub discoverable: bool,
    pub discovering: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Device {
    pub path: String,
    pub address: String,
    pub name: Option<String>,
    /// What the lists show: the user's rename, else the name, else the
    /// address.
    pub alias: String,
    pub kind: Kind,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub rssi: Option<i16>,
    pub battery: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Headphones,
    Headset,
    Speaker,
    Gamepad,
    Keyboard,
    Mouse,
    Phone,
    Computer,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Audio,
    Input,
    Phones,
    Computers,
    Other,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Audio => "Audio",
            Group::Input => "Input devices",
            Group::Phones => "Phones",
            Group::Computers => "Computers",
            Group::Other => "Other devices",
        }
    }
}

impl Kind {
    /// BlueZ's `Icon`, a freedesktop device icon name it derives from the
    /// class of device or the LE appearance.
    pub fn from_icon(icon: &str) -> Option<Kind> {
        Some(match icon {
            "audio-headphones" => Kind::Headphones,
            "audio-headset" => Kind::Headset,
            "audio-card" => Kind::Speaker,
            "input-gaming" => Kind::Gamepad,
            "input-keyboard" => Kind::Keyboard,
            "input-mouse" | "input-tablet" => Kind::Mouse,
            "phone" | "modem" => Kind::Phone,
            "computer" => Kind::Computer,
            _ => return None,
        })
    }

    /// The class of device: major class in bits 8..=12, minor in 2..=7.
    /// Audio minors 1 and 2 are headsets and 6 headphones; a peripheral's
    /// low minor bits 1 and 2 are a joystick and a gamepad, and its top
    /// two a keyboard (1), a pointer (2) or both (3).
    pub fn from_class(class: u32) -> Option<Kind> {
        let major = (class >> 8) & 0x1f;
        let minor = (class >> 2) & 0x3f;
        Some(match major {
            1 => Kind::Computer,
            2 => Kind::Phone,
            4 => match minor {
                1 | 2 => Kind::Headset,
                6 => Kind::Headphones,
                _ => Kind::Speaker,
            },
            5 => match (minor >> 4, minor & 0xf) {
                (_, 1 | 2) => Kind::Gamepad,
                (1 | 3, _) => Kind::Keyboard,
                (2, _) => Kind::Mouse,
                _ => return None,
            },
            _ => return None,
        })
    }

    pub fn group(self) -> Group {
        match self {
            Kind::Headphones | Kind::Headset | Kind::Speaker => Group::Audio,
            Kind::Gamepad | Kind::Keyboard | Kind::Mouse => Group::Input,
            Kind::Phone => Group::Phones,
            Kind::Computer => Group::Computers,
            Kind::Other => Group::Other,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Headphones => "Headphones",
            Kind::Headset => "Headset",
            Kind::Speaker => "Speaker",
            Kind::Gamepad => "Game controller",
            Kind::Keyboard => "Keyboard",
            Kind::Mouse => "Mouse",
            Kind::Phone => "Phone",
            Kind::Computer => "Computer",
            Kind::Other => "Bluetooth device",
        }
    }

    pub fn icon(self) -> devicon::Device {
        match self {
            Kind::Headphones => devicon::Device::Headphones,
            Kind::Headset => devicon::Device::Headset,
            Kind::Speaker => devicon::Device::Speaker,
            Kind::Gamepad => devicon::Device::Gamepad,
            Kind::Keyboard => devicon::Device::Keyboard,
            Kind::Mouse => devicon::Device::Mouse,
            Kind::Phone => devicon::Device::Phone,
            Kind::Computer => devicon::Device::Computer,
            Kind::Other => devicon::Device::Generic,
        }
    }
}

fn flag(p: &Props, key: &str) -> bool {
    p.get(key)
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false)
}

fn text(p: &Props, key: &str) -> Option<String> {
    p.get(key).and_then(|v| String::try_from(v.clone()).ok())
}

fn adapter_of(path: &str, p: &Props) -> Adapter {
    Adapter {
        path: path.to_string(),
        powered: flag(p, "Powered"),
        blocked: text(p, "PowerState").as_deref() == Some("off-blocked"),
        discoverable: flag(p, "Discoverable"),
        discovering: flag(p, "Discovering"),
    }
}

fn device_of(path: &str, p: &Props, battery: Option<&Props>) -> Device {
    let address = text(p, "Address").unwrap_or_default();
    let name = text(p, "Name").filter(|n| !n.is_empty());
    let alias = text(p, "Alias")
        .filter(|a| !a.is_empty())
        .or_else(|| name.clone())
        .unwrap_or_else(|| address.clone());
    let kind = text(p, "Icon")
        .and_then(|i| Kind::from_icon(&i))
        .or_else(|| {
            p.get("Class")
                .and_then(|v| u32::try_from(v.clone()).ok())
                .and_then(Kind::from_class)
        })
        .unwrap_or_default();
    Device {
        path: path.to_string(),
        address,
        name,
        alias,
        kind,
        paired: flag(p, "Paired"),
        trusted: flag(p, "Trusted"),
        connected: flag(p, "Connected"),
        rssi: p.get("RSSI").and_then(|v| i16::try_from(v.clone()).ok()),
        battery: battery
            .and_then(|b| b.get("Percentage"))
            .and_then(|v| u8::try_from(v.clone()).ok()),
    }
}

/// The first adapter in path order and the devices under it, sorted by
/// alias.
pub fn fold(objects: &Objects) -> Snapshot {
    let mut paths: Vec<&OwnedObjectPath> = objects.keys().collect();
    paths.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    let Some(adapter) = paths.iter().find_map(|p| {
        objects[*p]
            .get(ADAPTER_IFACE)
            .map(|a| adapter_of(p.as_str(), a))
    }) else {
        return Snapshot::default();
    };
    let prefix = format!("{}/", adapter.path);
    let mut devices: Vec<Device> = paths
        .iter()
        .filter(|p| p.as_str().starts_with(&prefix))
        .filter_map(|p| {
            let ifaces = &objects[*p];
            ifaces
                .get(DEVICE_IFACE)
                .map(|d| device_of(p.as_str(), d, ifaces.get(BATTERY_IFACE)))
        })
        .collect();
    devices.sort_by(|a, b| {
        a.alias
            .to_lowercase()
            .cmp(&b.alias.to_lowercase())
            .then(a.address.cmp(&b.address))
    });
    Snapshot {
        adapter: Some(adapter),
        devices,
    }
}

pub fn by_group<'a>(devs: impl IntoIterator<Item = &'a Device>) -> Vec<Device> {
    let mut out: Vec<Device> = devs.into_iter().cloned().collect();
    out.sort_by(|a, b| {
        a.kind
            .group()
            .cmp(&b.kind.group())
            .then(a.alias.to_lowercase().cmp(&b.alias.to_lowercase()))
            .then(a.address.cmp(&b.address))
    });
    out
}

pub fn status_line(d: &Device) -> String {
    let state = if d.connected {
        "Connected"
    } else {
        "Not connected"
    };
    match d.battery {
        Some(b) => format!("{state}, battery {b}%"),
        None => state.to_string(),
    }
}

/// What the windows ask the bus thread to do. Adapter and device fields
/// are object paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    SetPowered {
        adapter: String,
        on: bool,
    },
    SetDiscoverable {
        adapter: String,
        on: bool,
    },
    StartDiscovery(String),
    StopDiscovery(String),
    /// Pair, trust, then connect; answered by `BzMsg::Paired` or
    /// `BzMsg::PairFailed`.
    Pair(String),
    CancelPair(String),
    Connect(String),
    Disconnect(String),
    SetTrusted {
        device: String,
        on: bool,
    },
    SetAlias {
        device: String,
        alias: String,
    },
    Remove {
        adapter: String,
        device: String,
    },
    Quit,
}

#[derive(Debug)]
pub enum BzMsg {
    Snapshot(Snapshot),
    Failed {
        title: String,
        body: String,
    },
    /// A `Cmd::Connect` finished, which it does detached, so this can
    /// land after the user has moved on from that device. `error` is why
    /// it failed, if it did.
    Connected {
        device: String,
        error: Option<String>,
    },
    /// `Cmd::Pair` went through. `connect_error` is why the connect
    /// after it failed, if it did.
    Paired {
        device: String,
        connect_error: Option<String>,
    },
    PairFailed {
        device: String,
        reason: String,
    },
    Agent(AgentRequest),
    AgentCancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    PinCode,
    Passkey,
    ShowPin(String),
    ShowPasskey(u32),
    Confirm(u32),
    Authorize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Pin(String),
    Passkey(u32),
    Accept,
    Reject,
}

/// One agent call waiting on the user. `done` is None for the display
/// calls, which BlueZ does not wait on. Dropping the request unanswered
/// closes the channel, which the agent reads as Canceled.
#[derive(Debug)]
pub struct AgentRequest {
    pub device: String,
    pub ask: Ask,
    pub done: Option<async_channel::Sender<Answer>>,
}

impl AgentRequest {
    pub fn answer(&mut self, a: Answer) {
        if let Some(done) = self.done.take() {
            let _ = done.try_send(a);
        }
    }
}

pub fn failure_title(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::SetPowered { on: true, .. } => "Could not turn Bluetooth on",
        Cmd::SetPowered { on: false, .. } => "Could not turn Bluetooth off",
        Cmd::SetDiscoverable { .. } => "Could not change discovery",
        Cmd::StartDiscovery(_) => "Could not search for devices",
        Cmd::Connect(_) => "Could not connect",
        Cmd::Disconnect(_) => "Could not disconnect",
        Cmd::Remove { .. } => "Could not remove the device",
        Cmd::SetTrusted { .. } | Cmd::SetAlias { .. } => "Could not change the device",
        Cmd::StopDiscovery(_) | Cmd::Pair(_) | Cmd::CancelPair(_) | Cmd::Quit => "Bluetooth",
    }
}

/// A BlueZ error in plain words: the pairing failures by name, a
/// timed-out or refused connection and a lost pairing key by their
/// detail, the other common ones by name, the rest as BlueZ's own detail,
/// or the name when there is none.
pub fn plain_error(name: &str, detail: &str) -> String {
    let short = name.rsplit('.').next().unwrap_or(name);
    let unreachable = ["timeout", "refused"].iter().any(|d| detail.contains(d))
        || short == "ConnectionAttemptFailed";
    let words = match short {
        "AuthenticationFailed" | "AuthenticationRejected" | "AuthenticationTimeout" => {
            "The device did not accept the pairing."
        }
        "AuthenticationCanceled" => "Pairing was cancelled.",
        _ if unreachable => {
            "The device could not be reached. Make sure it is turned on and in range."
        }
        _ if detail.contains("key-missing") => {
            "The device no longer knows this computer. Remove it and add it again."
        }
        "NotReady" => "Bluetooth is not ready. Make sure it is turned on.",
        "InProgress" => "Bluetooth is busy with another request. Try again in a moment.",
        "AlreadyExists" => "This device has already been added.",
        "DoesNotExist" => "The device is no longer available.",
        "Blocked" => "Bluetooth is turned off by a switch or key on this computer.",
        _ if !detail.is_empty() => return detail.to_string(),
        _ => return name.to_string(),
    };
    words.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::{OwnedVariant, Str};

    fn s(v: &str) -> OwnedVariant {
        OwnedVariant::from(Str::from(v.to_string()))
    }

    fn props(pairs: Vec<(&str, OwnedVariant)>) -> Props {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn adapter(powered: bool, state: &str) -> HashMap<String, Props> {
        HashMap::from([(
            ADAPTER_IFACE.to_string(),
            props(vec![
                ("Powered", OwnedVariant::from(powered)),
                ("PowerState", s(state)),
                ("Discoverable", OwnedVariant::from(false)),
                ("Discovering", OwnedVariant::from(true)),
            ]),
        )])
    }

    fn device(name: Option<&str>, alias: &str, icon: &str, paired: bool) -> HashMap<String, Props> {
        let mut p = vec![
            ("Address", s("AA:BB:CC:DD:EE:01")),
            ("Alias", s(alias)),
            ("Icon", s(icon)),
            ("Paired", OwnedVariant::from(paired)),
            ("Trusted", OwnedVariant::from(paired)),
            ("Connected", OwnedVariant::from(false)),
            ("RSSI", OwnedVariant::from(-60i16)),
        ];
        if let Some(n) = name {
            p.push(("Name", s(n)));
        }
        HashMap::from([(DEVICE_IFACE.to_string(), props(p))])
    }

    fn path(p: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(p.to_string()).unwrap()
    }

    #[test]
    fn no_adapter_is_an_empty_snapshot() {
        assert_eq!(fold(&Objects::new()), Snapshot::default());
        let mut o = Objects::new();
        o.insert(path("/org/bluez"), HashMap::new());
        assert_eq!(fold(&o).adapter, None);
    }

    #[test]
    fn the_first_adapter_and_only_its_devices_are_read() {
        let mut o = Objects::new();
        o.insert(path("/org/bluez/hci1"), adapter(true, "on"));
        o.insert(path("/org/bluez/hci0"), adapter(false, "off-blocked"));
        o.insert(
            path("/org/bluez/hci0/dev_AA"),
            device(Some("Pad"), "Pad", "input-gaming", true),
        );
        o.insert(
            path("/org/bluez/hci1/dev_BB"),
            device(Some("Cans"), "Cans", "audio-headphones", true),
        );
        let snap = fold(&o);
        let a = snap.adapter.expect("hci0");
        assert_eq!(a.path, "/org/bluez/hci0");
        assert!(!a.powered && a.blocked && a.discovering);
        assert_eq!(snap.devices.len(), 1, "hci1's device is not hci0's");
        assert_eq!(snap.devices[0].kind, Kind::Gamepad);
    }

    #[test]
    fn alias_is_the_title_and_name_is_optional() {
        let mut o = Objects::new();
        o.insert(path("/org/bluez/hci0"), adapter(true, "on"));
        o.insert(
            path("/org/bluez/hci0/dev_AA"),
            device(None, "AA-BB-CC-DD-EE-01", "", false),
        );
        let d = &fold(&o).devices[0];
        assert_eq!(d.name, None);
        assert_eq!(d.alias, "AA-BB-CC-DD-EE-01");
        assert_eq!(d.kind, Kind::Other);
        assert_eq!(d.rssi, Some(-60));
    }

    #[test]
    fn battery_comes_from_its_own_interface() {
        let mut o = Objects::new();
        o.insert(path("/org/bluez/hci0"), adapter(true, "on"));
        let mut dev = device(Some("Cans"), "Cans", "audio-headphones", true);
        dev.insert(
            BATTERY_IFACE.into(),
            props(vec![("Percentage", OwnedVariant::from(80u8))]),
        );
        o.insert(path("/org/bluez/hci0/dev_AA"), dev);
        assert_eq!(fold(&o).devices[0].battery, Some(80));
    }

    #[test]
    fn kind_from_icon_then_class() {
        assert_eq!(Kind::from_icon("audio-headset"), Some(Kind::Headset));
        assert_eq!(Kind::from_icon("input-tablet"), Some(Kind::Mouse));
        assert_eq!(Kind::from_icon("modem"), Some(Kind::Phone));
        assert_eq!(Kind::from_icon("camera-photo"), None);
        assert_eq!(
            Kind::from_class(0x240404),
            Some(Kind::Headset),
            "wearable headset"
        );
        assert_eq!(Kind::from_class(0x240418), Some(Kind::Headphones));
        assert_eq!(
            Kind::from_class(0x240414),
            Some(Kind::Speaker),
            "loudspeaker"
        );
        assert_eq!(Kind::from_class(0x000508), Some(Kind::Gamepad));
        assert_eq!(Kind::from_class(0x000540), Some(Kind::Keyboard));
        assert_eq!(Kind::from_class(0x000580), Some(Kind::Mouse));
        assert_eq!(Kind::from_class(0x5a020c), Some(Kind::Phone));
        assert_eq!(Kind::from_class(0x10010c), Some(Kind::Computer));
        assert_eq!(Kind::from_class(0x000600), None, "imaging");
    }

    #[test]
    fn by_group_orders_groups_then_names() {
        let d = |alias: &str, kind| Device {
            alias: alias.into(),
            kind,
            ..Device::default()
        };
        let devs = [
            d("zed pad", Kind::Gamepad),
            d("Phone", Kind::Phone),
            d("buds", Kind::Headphones),
            d("Aux", Kind::Speaker),
            d("mouse", Kind::Mouse),
        ];
        let names: Vec<String> = by_group(&devs).into_iter().map(|d| d.alias).collect();
        assert_eq!(names, ["Aux", "buds", "mouse", "zed pad", "Phone"]);
    }

    #[test]
    fn status_line_says_connected_and_battery() {
        let mut d = Device::default();
        assert_eq!(status_line(&d), "Not connected");
        d.connected = true;
        assert_eq!(status_line(&d), "Connected");
        d.battery = Some(80);
        assert_eq!(status_line(&d), "Connected, battery 80%");
    }

    #[test]
    fn plain_error_words_the_common_failures() {
        assert_eq!(
            plain_error("org.bluez.Error.AuthenticationFailed", ""),
            "The device did not accept the pairing."
        );
        assert_eq!(
            plain_error("org.bluez.Error.Failed", "br-connection-page-timeout"),
            "The device could not be reached. Make sure it is turned on and in range."
        );
        assert_eq!(
            plain_error("org.bluez.Error.AuthenticationTimeout", "timeout"),
            "The device did not accept the pairing.",
            "a pairing failure's name outranks its detail"
        );
        assert_eq!(
            plain_error("org.bluez.Error.Failed", "br-connection-refused"),
            "The device could not be reached. Make sure it is turned on and in range."
        );
        assert_eq!(
            plain_error("org.bluez.Error.Failed", "br-connection-key-missing"),
            "The device no longer knows this computer. Remove it and add it again."
        );
        assert_eq!(
            plain_error(
                "org.bluez.Error.Failed",
                "br-connection-profile-unavailable"
            ),
            "br-connection-profile-unavailable",
            "any other connection detail reads as itself"
        );
        assert_eq!(plain_error("org.bluez.Error.Weird", "detail"), "detail");
        assert_eq!(
            plain_error("org.bluez.Error.Weird", ""),
            "org.bluez.Error.Weird"
        );
    }

    #[test]
    fn an_unanswered_request_reads_as_closed() {
        let (done, wait) = async_channel::bounded(1);
        let mut req = AgentRequest {
            device: "/d".into(),
            ask: Ask::PinCode,
            done: Some(done),
        };
        req.answer(Answer::Pin("0000".into()));
        assert_eq!(wait.try_recv(), Ok(Answer::Pin("0000".into())));
        let (done, wait) = async_channel::bounded::<Answer>(1);
        drop(AgentRequest {
            device: "/d".into(),
            ask: Ask::PinCode,
            done: Some(done),
        });
        assert!(
            wait.try_recv().is_err(),
            "a dropped request closes its channel"
        );
    }
}
