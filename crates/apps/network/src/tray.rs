//! The network icon in the notification area: the `reprisetray` item's icon,
//! tooltip and device-shaped menu, the balloons a link change sends, and what a
//! click means. It turns a snapshot into those; the D-Bus side is `reprisetray`'s.

use reprisetray::{Config, Icon, Msg, Node, Raise, Shown};

use crate::nm;
use reprisekit::art::neticon::{NetIcon, bars};

pub const BUS_NAME: &str = "reprise.network";

pub const ID_DISABLE: i32 = 1;
pub const ID_STATUS: i32 = 2;
pub const ID_REPAIR: i32 = 3;
pub const ID_SEP1: i32 = 4;
pub const ID_WIRELESS: i32 = 5;
pub const ID_SEP2: i32 = 6;
pub const ID_FOLDER: i32 = 7;
pub const ID_SEP3: i32 = 8;
pub const ID_EXIT: i32 = 9;
pub const ID_HOTSPOT: i32 = 10;
pub const ID_SEP4: i32 = 11;
pub const ID_VPN: i32 = 100;

pub const TAG_PORTAL: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayMsg {
    /// A VPN's row, by its index in the snapshot: connect, or disconnect when up.
    Vpn(usize),
    Hotspot,
    SignIn,
    /// The first row: the app decides from `managed` whether it disables or enables.
    Toggle,
    Status,
    Repair,
    Wireless,
    Folder,
    Exit,
}

pub fn translate(msg: Msg) -> Option<TrayMsg> {
    Some(match msg {
        Msg::Activate(..) | Msg::SecondaryActivate | Msg::Clicked(ID_STATUS) => TrayMsg::Status,
        Msg::Clicked(ID_DISABLE) => TrayMsg::Toggle,
        Msg::Clicked(ID_REPAIR) => TrayMsg::Repair,
        Msg::Clicked(ID_WIRELESS) => TrayMsg::Wireless,
        Msg::Clicked(ID_FOLDER) => TrayMsg::Folder,
        Msg::Clicked(ID_EXIT) => TrayMsg::Exit,
        Msg::Clicked(ID_HOTSPOT) => TrayMsg::Hotspot,
        Msg::Clicked(id) if id >= ID_VPN => TrayMsg::Vpn((id - ID_VPN) as usize),
        Msg::BalloonClicked(TAG_PORTAL) => TrayMsg::SignIn,
        Msg::Clicked(_) | Msg::Scroll(_) | Msg::BalloonClicked(_) => return None,
    })
}

/// What the rows depend on: the first row's wording, whether there is anything
/// to disable or any radio to list, the hotspot, and the saved VPNs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuShape {
    /// "Disable" or "Enable". The compositor's menu host strips `_`, not `&`, so
    /// these are plain words.
    pub enabled_label: &'static str,
    pub has_wifi: bool,
    pub has_device: bool,
    pub hotspot_on: bool,
    pub vpns: Vec<(String, bool)>,
}

impl Default for MenuShape {
    fn default() -> Self {
        MenuShape {
            enabled_label: "Disable",
            has_wifi: false,
            has_device: false,
            hotspot_on: false,
            vpns: vec![],
        }
    }
}

/// The tray menu: Disable, Status, Repair, the wireless list and hotspot, the
/// saved VPNs as check rows, the folder and Exit. Rows and their separators go
/// together, so two separators never meet.
pub fn menu_nodes(shape: &MenuShape) -> Vec<Node> {
    let row = Node::row;
    let separator = Node::separator;
    let mut nodes = vec![];
    if shape.has_device {
        nodes.push(row(ID_DISABLE, shape.enabled_label));
        nodes.push(row(ID_STATUS, "Status"));
        nodes.push(row(ID_REPAIR, "Repair"));
        nodes.push(separator(ID_SEP1));
    }
    if shape.has_wifi {
        nodes.push(row(ID_WIRELESS, "View Available Wireless Networks"));
        nodes.push(row(
            ID_HOTSPOT,
            if shape.hotspot_on {
                "Stop Hotspot"
            } else {
                "Create Hotspot..."
            },
        ));
        nodes.push(separator(ID_SEP2));
    }
    if !shape.vpns.is_empty() {
        for (i, (name, up)) in shape.vpns.iter().enumerate() {
            nodes.push(Node::toggle(ID_VPN + i as i32, name.clone(), *up));
        }
        nodes.push(separator(ID_SEP4));
    }
    nodes.push(row(ID_FOLDER, "Open Network Connections"));
    nodes.push(separator(ID_SEP3));
    nodes.push(row(ID_EXIT, "Exit"));
    nodes
}

/// The menu a snapshot asks for: the first row reads off the primary device's
/// `managed`; per-device rows go with the primary, the wireless row with any radio.
pub fn menu_shape(s: &nm::Snapshot) -> MenuShape {
    let primary = primary(s);
    MenuShape {
        enabled_label: match primary {
            Some(d) if !d.managed => "Enable",
            _ => "Disable",
        },
        has_wifi: wifi_device(s).is_some(),
        has_device: primary.is_some(),
        hotspot_on: wifi_device(s).is_some_and(|d| d.hotspot.is_some()),
        vpns: s.vpns.iter().map(|v| (v.name.clone(), v.active)).collect(),
    }
}

/// Whether the icon wears the limited badge: a sign-in page or no route past
/// the local network, or no connectivity while a device says it is connected.
pub fn limited(s: &nm::Snapshot) -> bool {
    match s.connectivity {
        nm::Connectivity::Portal | nm::Connectivity::Limited => true,
        nm::Connectivity::None => primary(s).is_some_and(|d| d.state == nm::State::Connected),
        nm::Connectivity::Unknown | nm::Connectivity::Full => false,
    }
}

/// The device the wireless list opens on: the primary one if it is a radio, else
/// the first radio. A managed radio wins, since an unmanaged one can list nothing.
pub fn wifi_device(s: &nm::Snapshot) -> Option<&nm::Device> {
    let radio = |d: &nm::Device| d.kind == nm::Kind::Wifi;
    primary(s)
        .filter(|d| radio(d) && d.managed)
        .or_else(|| s.devices.iter().find(|d| radio(d) && d.managed))
        .or_else(|| primary(s).filter(|d| radio(d)))
        .or_else(|| s.devices.iter().find(|d| radio(d)))
}

/// The device the icon and tooltip speak for: NM's primary connection, else
/// anything connected, else a radio, else whatever there is. An unmanaged device
/// is the last resort of each: its icon would only read "disabled".
pub fn primary(s: &nm::Snapshot) -> Option<&nm::Device> {
    s.primary
        .as_ref()
        .and_then(|p| s.devices.iter().find(|d| &d.path == p))
        .or_else(|| s.devices.iter().find(|d| d.state == nm::State::Connected))
        .or_else(|| {
            s.devices
                .iter()
                .find(|d| d.kind == nm::Kind::Wifi && d.managed)
        })
        .or_else(|| s.devices.iter().find(|d| d.managed))
        .or_else(|| s.devices.iter().find(|d| d.kind == nm::Kind::Wifi))
        .or_else(|| s.devices.first())
}

pub fn signal_word(strength: u8) -> &'static str {
    match strength {
        80..=u8::MAX => "Excellent",
        60..=79 => "Very Good",
        40..=59 => "Good",
        20..=39 => "Low",
        _ => "Very Low",
    }
}

/// A link speed as the tooltip writes it; empty when the driver reports none.
pub fn speed_text(mbit: u32) -> String {
    match mbit {
        0 => String::new(),
        m if m >= 1000 => format!("{:.1} Gbps", m as f32 / 1000.0),
        m => format!("{:.1} Mbps", m as f32),
    }
}

/// The tooltip lines: the primary device's, then a line on connectivity while
/// it is limited.
pub fn tooltip(s: &nm::Snapshot) -> String {
    let link = link_tip(s);
    if !limited(s) {
        return link;
    }
    let why = if s.connectivity == nm::Connectivity::Portal {
        "Sign in to the network"
    } else {
        "Limited or no connectivity"
    };
    format!("{link}\n{why}")
}

fn link_tip(s: &nm::Snapshot) -> String {
    if !s.nm_running {
        return "NetworkManager is not running".to_string();
    }
    let Some(d) = primary(s) else {
        return "No network adapters".to_string();
    };
    if let Some(ssid) = &d.hotspot {
        return format!("{}\nHotspot: {ssid}", d.shell_name);
    }
    if !d.managed {
        return format!("{}\nDisabled", d.shell_name);
    }
    if d.kind == nm::Kind::Wired && !d.carrier {
        return format!("{}\nA network cable is unplugged.", d.shell_name);
    }
    let speed = speed_text(d.speed_mbit);
    let ssid = d.wifi.as_ref().and_then(|w| w.active_ssid.clone());
    match (d.kind, d.state, ssid) {
        (nm::Kind::Wired, nm::State::Connected, _) => {
            let mut lines = vec![d.shell_name.clone()];
            if !speed.is_empty() {
                lines.push(format!("Speed: {speed}"));
            }
            lines.push("Status: Connected".to_string());
            lines.join("\n")
        }
        (nm::Kind::Wifi, nm::State::Connected, Some(ssid)) => {
            let strength = d.wifi.as_ref().map(|w| w.strength).unwrap_or(0);
            let mut lines = vec![format!("{} ({ssid})", d.shell_name)];
            if !speed.is_empty() {
                lines.push(format!("Speed: {speed}"));
            }
            lines.push(format!("Signal Strength: {}", signal_word(strength)));
            lines.join("\n")
        }
        _ => format!("{}\nNot connected", d.shell_name),
    }
}

/// The icon for the primary device, and whether a padlock goes over it. A wired
/// device neither connected nor connecting gets the red cross.
pub fn icon_for(s: &nm::Snapshot) -> (NetIcon, bool) {
    let vpn = s.vpns.iter().any(|v| v.active);
    let Some(d) = primary(s).filter(|_| s.nm_running) else {
        return (NetIcon::WiredUnplugged, vpn);
    };
    let icon = match (d.kind, d.state) {
        (nm::Kind::Wired, nm::State::Connected) => NetIcon::Wired,
        (nm::Kind::Wifi, nm::State::Connected) => {
            NetIcon::Wireless(bars(d.wifi.as_ref().map(|w| w.strength).unwrap_or(0)))
        }
        (_, nm::State::Connecting) => NetIcon::Connecting,
        (nm::Kind::Wired, _) => NetIcon::WiredUnplugged,
        (nm::Kind::Wifi, _) => NetIcon::WirelessOff,
    };
    (icon, vpn)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Balloon {
    pub summary: String,
    pub body: String,
    pub tag: Option<u32>,
}

impl Balloon {
    fn new(summary: String, body: String) -> Balloon {
        Balloon {
            summary,
            body,
            tag: None,
        }
    }
}

struct Seen {
    path: String,
    connected: bool,
    carrier: bool,
}

/// The link changes worth a balloon, found by comparing the last snapshot with
/// the new one. Nothing fires on the first snapshot, nor on the one after the
/// daemon comes back.
#[derive(Default)]
pub struct Edges {
    prev: Option<(bool, Vec<Seen>)>,
    connectivity: nm::Connectivity,
}

impl Edges {
    /// [`Edges::links`], and "Sign in to the network" each time the
    /// connectivity check lands on a sign-in page. That one fires on the
    /// first snapshot too: a page in the way is news whenever it is found.
    pub fn feed(&mut self, s: &nm::Snapshot) -> Vec<Balloon> {
        let was = std::mem::replace(&mut self.connectivity, s.connectivity);
        let mut out = self.links(s);
        if s.connectivity == nm::Connectivity::Portal && was != nm::Connectivity::Portal {
            out.push(Balloon {
                summary: "Sign in to the network".into(),
                body: "This network asks you to sign in before it lets you on the internet. \
                       Click here to open its sign-in page."
                    .into(),
                tag: Some(TAG_PORTAL),
            });
        }
        out
    }

    fn links(&mut self, s: &nm::Snapshot) -> Vec<Balloon> {
        let seen = s
            .devices
            .iter()
            .map(|d| Seen {
                path: d.path.clone(),
                connected: d.state == nm::State::Connected,
                carrier: d.carrier,
            })
            .collect();
        let Some((was_running, prev)) = self.prev.replace((s.nm_running, seen)) else {
            return vec![];
        };
        // A NetworkManager restart empties the snapshot and fills it
        // again; every device would read as freshly connected.
        if !was_running {
            return vec![];
        }
        let mut out = vec![];
        for d in &s.devices {
            let was = prev.iter().find(|p| p.path == d.path);
            // A device that only just appeared counts as having been
            // disconnected, so plugging a dongle in still says so. Its
            // carrier, though, has no previous value: a dongle arriving
            // with the cable out is not a cable coming out.
            let was_connected = was.is_some_and(|p| p.connected);
            let was_carrier = was.is_some_and(|p| p.carrier);
            if !was_connected && d.state == nm::State::Connected {
                let body = match d.kind {
                    nm::Kind::Wired => match speed_text(d.speed_mbit) {
                        s if s.is_empty() => "Connected".to_string(),
                        s => format!("Speed: {s}"),
                    },
                    nm::Kind::Wifi => {
                        let w = d.wifi.clone().unwrap_or_default();
                        format!(
                            "Connected to: {}\nSignal Strength: {}",
                            w.active_ssid.unwrap_or_default(),
                            signal_word(w.strength)
                        )
                    }
                };
                out.push(Balloon::new(
                    format!("{} is now connected", d.shell_name),
                    body,
                ));
            }
            if d.kind == nm::Kind::Wired && was_carrier && !d.carrier {
                out.push(Balloon::new(
                    d.shell_name.clone(),
                    "A network cable is unplugged.".to_string(),
                ));
            }
        }
        out
    }
}

pub fn shown(pixmap: Vec<u8>, tip: String, active: bool, menu: &MenuShape) -> Shown {
    Shown {
        icon: Icon::Pixmap {
            size: reprisekit::art::neticon::SIZE as i32,
            argb: pixmap,
        },
        tip,
        active,
        nodes: menu_nodes(menu),
    }
}

/// Bring the tray item up; see `reprisetray::start`. Passive until the first
/// snapshot, so a machine with no adapter never flashes the icon. A second
/// instance raises the first one's folder through the "Open Network Connections"
/// row, since SNI `Activate` means the primary connection's Status sheet.
pub fn start(waker: repriseapp::Waker) -> reprisetray::Started {
    let config = Config {
        bus_name: BUS_NAME,
        id: "reprise-network",
        title: "Network",
        notify_icon: "network-wired",
        raise: Raise::MenuRow(ID_FOLDER),
    };
    let initial = shown(
        reprisekit::art::neticon::draw(NetIcon::WiredUnplugged, false).to_argb_be(),
        "No network adapters".to_string(),
        false,
        &MenuShape::default(),
    );
    reprisetray::start(config, initial, std::sync::Arc::new(move || waker.wake()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wired(state: nm::State, carrier: bool, speed: u32) -> nm::Device {
        nm::Device {
            path: "/dev/1".into(),
            iface: "enp3s0".into(),
            kind: nm::Kind::Wired,
            shell_name: "Local Area Connection".into(),
            state,
            managed: true,
            carrier,
            speed_mbit: speed,
            ..Default::default()
        }
    }

    fn wifi(state: nm::State, ssid: Option<&str>, strength: u8) -> nm::Device {
        nm::Device {
            path: "/dev/2".into(),
            iface: "wlan0".into(),
            kind: nm::Kind::Wifi,
            shell_name: "Wireless Network Connection".into(),
            state,
            managed: true,
            carrier: true,
            speed_mbit: 54,
            wifi: Some(nm::Wifi {
                active_ssid: ssid.map(str::to_string),
                strength,
                aps: vec![],
            }),
            ..Default::default()
        }
    }

    fn snap(devices: Vec<nm::Device>) -> nm::Snapshot {
        nm::Snapshot {
            devices,
            vpns: vec![],
            primary: None,
            nm_running: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_menu_follows_the_primary_device() {
        // Nothing at all: no per-device rows, no wireless row.
        assert_eq!(
            menu_shape(&snap(vec![])),
            MenuShape {
                enabled_label: "Disable",
                has_wifi: false,
                has_device: false,
                ..MenuShape::default()
            }
        );
        // A wired device on its own: the rows, no wireless.
        let wire = snap(vec![wired(nm::State::Connected, true, 1000)]);
        assert_eq!(
            menu_shape(&wire),
            MenuShape {
                enabled_label: "Disable",
                has_wifi: false,
                has_device: true,
                ..MenuShape::default()
            }
        );
        assert!(wifi_device(&wire).is_none());
        // An unmanaged primary reads Enable.
        let mut off = wired(nm::State::Unmanaged, true, 0);
        off.managed = false;
        assert_eq!(menu_shape(&snap(vec![off])).enabled_label, "Enable");
        // A radio anywhere puts the wireless row up, even when the
        // primary is the wired device.
        let both = snap(vec![
            wired(nm::State::Connected, true, 1000),
            wifi(nm::State::Disconnected, None, 0),
        ]);
        assert!(menu_shape(&both).has_wifi);
        assert_eq!(
            wifi_device(&both).map(|d| d.path.as_str()),
            Some("/dev/2"),
            "the first radio when the primary has none"
        );
        // A Wi-Fi primary is the one the list opens on.
        let mut primary_wifi = snap(vec![
            wifi(nm::State::Connected, Some("Intrepid 5"), 70),
            wired(nm::State::Disconnected, false, 0),
        ]);
        primary_wifi.primary = Some("/dev/2".into());
        assert_eq!(
            wifi_device(&primary_wifi).map(|d| d.path.as_str()),
            Some("/dev/2")
        );
        // An unmanaged radio still puts the row up, since the list is how
        // the radio gets turned back on, but a managed one is preferred.
        let mut off_radio = wifi(nm::State::Unmanaged, None, 0);
        off_radio.managed = false;
        assert!(menu_shape(&snap(vec![off_radio.clone()])).has_wifi);
        assert_eq!(
            wifi_device(&snap(vec![off_radio.clone()])).map(|d| d.path.as_str()),
            Some("/dev/2")
        );
        let mut managed_radio = wifi(nm::State::Disconnected, None, 0);
        managed_radio.path = "/dev/3".into();
        assert_eq!(
            wifi_device(&snap(vec![off_radio, managed_radio])).map(|d| d.path.as_str()),
            Some("/dev/3"),
            "the managed radio, even though the unmanaged one comes first"
        );
    }

    #[test]
    fn tooltip_wording() {
        assert_eq!(
            tooltip(&snap(vec![wired(nm::State::Connected, true, 1000)])),
            "Local Area Connection\nSpeed: 1.0 Gbps\nStatus: Connected"
        );
        assert_eq!(
            tooltip(&snap(vec![wifi(
                nm::State::Connected,
                Some("Intrepid 5"),
                65
            )])),
            "Wireless Network Connection (Intrepid 5)\nSpeed: 54.0 Mbps\nSignal Strength: Very Good"
        );
        assert_eq!(
            tooltip(&snap(vec![wired(nm::State::Disconnected, false, 0)])),
            "Local Area Connection\nA network cable is unplugged."
        );
        assert_eq!(
            tooltip(&snap(vec![wired(nm::State::Disconnected, true, 0)])),
            "Local Area Connection\nNot connected"
        );
        assert_eq!(
            tooltip(&snap(vec![wifi(nm::State::Disconnected, None, 0)])),
            "Wireless Network Connection\nNot connected"
        );
        let mut disabled = wired(nm::State::Unmanaged, false, 0);
        disabled.managed = false;
        assert_eq!(
            tooltip(&snap(vec![disabled])),
            "Local Area Connection\nDisabled",
            "Disabled wins over the unplugged cable"
        );
        assert_eq!(tooltip(&snap(vec![])), "No network adapters");
        assert_eq!(
            tooltip(&nm::Snapshot::default()),
            "NetworkManager is not running"
        );
    }

    #[test]
    fn speed_text_units() {
        assert_eq!(speed_text(0), "");
        assert_eq!(speed_text(54), "54.0 Mbps");
        assert_eq!(speed_text(100), "100.0 Mbps");
        assert_eq!(speed_text(999), "999.0 Mbps");
        assert_eq!(speed_text(1000), "1.0 Gbps");
        assert_eq!(speed_text(2500), "2.5 Gbps");
    }

    #[test]
    fn signal_words_at_boundaries() {
        assert_eq!(signal_word(80), "Excellent");
        assert_eq!(signal_word(79), "Very Good");
        assert_eq!(signal_word(60), "Very Good");
        assert_eq!(signal_word(59), "Good");
        assert_eq!(signal_word(40), "Good");
        assert_eq!(signal_word(39), "Low");
        assert_eq!(signal_word(20), "Low");
        assert_eq!(signal_word(19), "Very Low");
        assert_eq!(signal_word(0), "Very Low");
    }

    fn rows(shape: MenuShape) -> Vec<(i32, String, bool)> {
        menu_nodes(&shape)
            .into_iter()
            .map(|n| (n.id, n.label, n.separator))
            .collect()
    }

    #[test]
    fn clicks_map_to_the_rows() {
        assert_eq!(translate(Msg::Activate(0, 0)), Some(TrayMsg::Status));
        assert_eq!(translate(Msg::Clicked(ID_DISABLE)), Some(TrayMsg::Toggle));
        assert_eq!(translate(Msg::Clicked(ID_FOLDER)), Some(TrayMsg::Folder));
        assert_eq!(translate(Msg::Clicked(ID_EXIT)), Some(TrayMsg::Exit));
        assert_eq!(translate(Msg::Clicked(ID_SEP1)), None);
        assert_eq!(translate(Msg::Scroll(1)), None);
    }

    #[test]
    fn menu_nodes_shape() {
        let row = |id, label: &str| (id, label.to_string(), false);
        let sep = |id| (id, String::new(), true);
        assert_eq!(
            rows(MenuShape {
                enabled_label: "Disable",
                has_wifi: true,
                has_device: true,
                ..MenuShape::default()
            }),
            vec![
                row(ID_DISABLE, "Disable"),
                row(ID_STATUS, "Status"),
                row(ID_REPAIR, "Repair"),
                sep(ID_SEP1),
                row(ID_WIRELESS, "View Available Wireless Networks"),
                row(ID_HOTSPOT, "Create Hotspot..."),
                sep(ID_SEP2),
                row(ID_FOLDER, "Open Network Connections"),
                sep(ID_SEP3),
                row(ID_EXIT, "Exit"),
            ]
        );
        // No radio: the wireless row and its separator both go, so two
        // separators never meet. The first row says what it does.
        assert_eq!(
            rows(MenuShape {
                enabled_label: "Enable",
                has_wifi: false,
                has_device: true,
                ..MenuShape::default()
            }),
            vec![
                row(ID_DISABLE, "Enable"),
                row(ID_STATUS, "Status"),
                row(ID_REPAIR, "Repair"),
                sep(ID_SEP1),
                row(ID_FOLDER, "Open Network Connections"),
                sep(ID_SEP3),
                row(ID_EXIT, "Exit"),
            ]
        );
        assert_eq!(
            rows(MenuShape::default()),
            vec![
                row(ID_FOLDER, "Open Network Connections"),
                sep(ID_SEP3),
                row(ID_EXIT, "Exit"),
            ]
        );
    }

    #[test]
    fn the_menu_lists_the_hotspot_and_every_saved_vpn() {
        let row = |id, label: &str| (id, label.to_string(), false);
        let sep = |id| (id, String::new(), true);
        let shape = MenuShape {
            has_wifi: true,
            hotspot_on: true,
            vpns: vec![("Away".into(), true), ("Work".into(), false)],
            ..MenuShape::default()
        };
        assert_eq!(
            rows(shape.clone()),
            vec![
                row(ID_WIRELESS, "View Available Wireless Networks"),
                row(ID_HOTSPOT, "Stop Hotspot"),
                sep(ID_SEP2),
                row(ID_VPN, "Away"),
                row(ID_VPN + 1, "Work"),
                sep(ID_SEP4),
                row(ID_FOLDER, "Open Network Connections"),
                sep(ID_SEP3),
                row(ID_EXIT, "Exit"),
            ]
        );
        let toggles: Vec<Option<bool>> = menu_nodes(&shape)
            .iter()
            .filter(|n| n.id >= ID_VPN)
            .map(|n| n.toggle)
            .collect();
        assert_eq!(
            toggles,
            [Some(true), Some(false)],
            "a VPN that is up is ticked"
        );
        assert_eq!(translate(Msg::Clicked(ID_VPN + 1)), Some(TrayMsg::Vpn(1)));
        assert_eq!(translate(Msg::Clicked(ID_HOTSPOT)), Some(TrayMsg::Hotspot));
        assert_eq!(
            translate(Msg::BalloonClicked(TAG_PORTAL)),
            Some(TrayMsg::SignIn)
        );
        let mut s = snap(vec![wifi(nm::State::Connected, Some("Desk"), 0)]);
        s.devices[0].hotspot = Some("Desk".into());
        assert!(menu_shape(&s).hotspot_on);
        assert_eq!(tooltip(&s), "Wireless Network Connection\nHotspot: Desk");
    }

    #[test]
    fn a_sign_in_page_badges_the_icon_and_says_so_once() {
        let mut s = snap(vec![wired(nm::State::Connected, true, 100)]);
        s.connectivity = nm::Connectivity::Full;
        assert!(!limited(&s));
        let mut edges = Edges::default();
        assert_eq!(edges.feed(&s), vec![]);
        s.connectivity = nm::Connectivity::Portal;
        assert!(limited(&s));
        assert_eq!(
            tooltip(&s),
            "Local Area Connection\nSpeed: 100.0 Mbps\nStatus: Connected\nSign in to the network"
        );
        let b = edges.feed(&s);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].summary, "Sign in to the network");
        assert_eq!(b[0].tag, Some(TAG_PORTAL));
        assert_eq!(edges.feed(&s), vec![], "once per sign-in page");
        s.connectivity = nm::Connectivity::Limited;
        assert!(tooltip(&s).ends_with("\nLimited or no connectivity"));
        s.connectivity = nm::Connectivity::None;
        assert!(limited(&s), "no connectivity on a connected wire");
        s.connectivity = nm::Connectivity::Unknown;
        assert!(!limited(&s), "an unchecked network is not limited");
        let mut down = snap(vec![wired(nm::State::Disconnected, true, 0)]);
        down.connectivity = nm::Connectivity::None;
        assert!(
            !limited(&down),
            "nothing connected is the cross, not the badge"
        );
    }

    #[test]
    fn primary_prefers_nm_primary_then_connected() {
        let devices = vec![
            wired(nm::State::Disconnected, true, 0),
            wifi(nm::State::Connected, Some("Intrepid 5"), 65),
        ];
        let mut s = snap(devices.clone());
        s.primary = Some("/dev/1".into());
        assert_eq!(primary(&s).map(|d| d.path.as_str()), Some("/dev/1"));
        // No primary: the connected one, whichever it is.
        assert_eq!(
            primary(&snap(devices.clone())).map(|d| d.path.as_str()),
            Some("/dev/2")
        );
        // Nothing connected: the radio before the wire.
        let idle = vec![
            wired(nm::State::Disconnected, true, 0),
            wifi(nm::State::Disconnected, None, 0),
        ];
        assert_eq!(
            primary(&snap(idle)).map(|d| d.path.as_str()),
            Some("/dev/2")
        );
        // Neither: whatever comes first.
        assert_eq!(
            primary(&snap(vec![wired(nm::State::Unavailable, false, 0)])).map(|d| d.path.as_str()),
            Some("/dev/1")
        );
        // An unmanaged radio loses to a managed wire, and to nothing else.
        let mut off_radio = wifi(nm::State::Unmanaged, None, 0);
        off_radio.managed = false;
        assert_eq!(
            primary(&snap(vec![
                off_radio.clone(),
                wired(nm::State::Disconnected, true, 0)
            ]))
            .map(|d| d.path.as_str()),
            Some("/dev/1")
        );
        assert_eq!(
            primary(&snap(vec![off_radio])).map(|d| d.path.as_str()),
            Some("/dev/2")
        );
        assert_eq!(primary(&snap(vec![])), None);
    }

    #[test]
    fn icon_for_states() {
        assert_eq!(
            icon_for(&snap(vec![wired(nm::State::Connected, true, 1000)])),
            (NetIcon::Wired, false)
        );
        assert_eq!(
            icon_for(&snap(vec![wifi(nm::State::Connected, Some("a"), 65)])),
            (NetIcon::Wireless(3), false)
        );
        assert_eq!(
            icon_for(&snap(vec![wired(nm::State::Unavailable, false, 0)])),
            (NetIcon::WiredUnplugged, false)
        );
        assert_eq!(
            icon_for(&snap(vec![wired(nm::State::Connecting, true, 0)])),
            (NetIcon::Connecting, false)
        );
        assert_eq!(
            icon_for(&snap(vec![wifi(nm::State::Disconnected, None, 0)])),
            (NetIcon::WirelessOff, false)
        );
        assert_eq!(icon_for(&snap(vec![])), (NetIcon::WiredUnplugged, false));
        let mut vpn_only = snap(vec![]);
        vpn_only.vpns = vec![nm::Vpn {
            conn_path: "/Settings/3".into(),
            name: "Work".into(),
            kind: "wireguard".into(),
            active: true,
        }];
        assert_eq!(
            icon_for(&vpn_only),
            (NetIcon::WiredUnplugged, true),
            "a tunnel over nothing still wears the padlock"
        );
        let mut vpn = snap(vec![wired(nm::State::Connected, true, 1000)]);
        vpn.vpns = vec![nm::Vpn {
            conn_path: "/Settings/3".into(),
            name: "Work".into(),
            kind: "openvpn".into(),
            active: true,
        }];
        assert_eq!(icon_for(&vpn), (NetIcon::Wired, true));
    }

    #[test]
    fn edges_fire_once_per_transition() {
        let mut edges = Edges::default();
        let idle = snap(vec![wired(nm::State::Disconnected, true, 0)]);
        assert_eq!(edges.feed(&idle), vec![], "nothing on the first snapshot");
        let up = snap(vec![wired(nm::State::Connected, true, 100)]);
        assert_eq!(
            edges.feed(&up),
            vec![Balloon {
                summary: "Local Area Connection is now connected".into(),
                body: "Speed: 100.0 Mbps".into(),
                tag: None,
            }]
        );
        assert_eq!(edges.feed(&up), vec![], "the same snapshot is no edge");
        let out = snap(vec![wired(nm::State::Unavailable, false, 0)]);
        assert_eq!(
            edges.feed(&out),
            vec![Balloon {
                summary: "Local Area Connection".into(),
                body: "A network cable is unplugged.".into(),
                tag: None,
            }]
        );
    }

    #[test]
    fn edges_name_the_wifi_network() {
        let mut edges = Edges::default();
        edges.feed(&snap(vec![wifi(nm::State::Disconnected, None, 0)]));
        assert_eq!(
            edges.feed(&snap(vec![wifi(
                nm::State::Connected,
                Some("Intrepid 5"),
                85
            )])),
            vec![Balloon {
                summary: "Wireless Network Connection is now connected".into(),
                body: "Connected to: Intrepid 5\nSignal Strength: Excellent".into(),
                tag: None,
            }]
        );
    }

    #[test]
    fn edges_ignore_a_daemon_restart() {
        let mut edges = Edges::default();
        edges.feed(&snap(vec![wired(nm::State::Connected, true, 100)]));
        // NetworkManager goes away: no devices, and nothing to say.
        let down = nm::Snapshot::default();
        assert_eq!(edges.feed(&down), vec![]);
        // It comes back with everything at once; that is not a link change.
        assert_eq!(
            edges.feed(&snap(vec![wired(nm::State::Connected, true, 100)])),
            vec![],
            "a service bounce is no connection event"
        );
    }

    #[test]
    fn edges_on_a_device_that_just_appeared() {
        let mut edges = Edges::default();
        edges.feed(&snap(vec![]));
        // A dongle arriving already up says so, with no speed to report.
        assert_eq!(
            edges.feed(&snap(vec![wired(nm::State::Connected, true, 0)])),
            vec![Balloon {
                summary: "Local Area Connection is now connected".into(),
                body: "Connected".into(),
                tag: None,
            }]
        );
        let mut edges = Edges::default();
        edges.feed(&snap(vec![]));
        // One arriving with the cable out has no carrier to have lost.
        assert_eq!(
            edges.feed(&snap(vec![wired(nm::State::Disconnected, false, 0)])),
            vec![]
        );
    }
}
