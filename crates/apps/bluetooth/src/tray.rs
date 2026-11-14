//! The Bluetooth icon in the notification area: what reprise-bluetooth shows on
//! its `reprisetray` item and what a click means. Hidden while there is no
//! adapter. The D-Bus side is `reprisetray`'s.

use reprisekit::art::bticon;
use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

use crate::bz::{self, Device, Snapshot};

pub const BUS_NAME: &str = "reprise.bluetooth";

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    Show,
    Add,
    TogglePower,
    /// A connected device's row, by its index into [`connected`]: opens its Properties.
    Device(usize),
    /// Button `i` of the received-file balloon `tag`.
    Balloon(u32, usize),
    Airplane,
    /// The greyed row under Airplane Mode while a hardware switch holds every radio off.
    AirplaneNote,
    Exit,
}

/// The paired devices connected now, in the snapshot's order.
pub fn connected(s: &Snapshot) -> Vec<&Device> {
    s.devices
        .iter()
        .filter(|d| d.paired && d.connected)
        .collect()
}

/// What `msg` means while the menu shows `s`'s rows.
pub fn translate(msg: Msg, s: &Snapshot) -> Option<TrayMsg> {
    match msg {
        Msg::Activate(..) => Some(TrayMsg::Show),
        Msg::BalloonButton(tag, i) => Some(TrayMsg::Balloon(tag, i)),
        msg => menu(s).translate(msg),
    }
}

/// The connected devices with their battery while the radio is on; Show and Exit always; Add
/// while the radio is on; the power row while there is a radio to turn; Airplane Mode while
/// the machine has a radio switch.
pub fn menu(s: &Snapshot) -> Rows<TrayMsg> {
    let mut rows = Rows::default();
    if s.adapter.as_ref().is_some_and(|a| a.powered) {
        let devices = connected(s);
        for (i, d) in devices.iter().enumerate() {
            rows.add(TrayMsg::Device(i), |id| Node::row(id, bz::battery_row(d)));
        }
        if !devices.is_empty() {
            rows.separator();
        }
    }
    rows.add(TrayMsg::Show, |id| Node::row(id, "Show Bluetooth Devices"));
    if let Some(a) = &s.adapter {
        if a.powered {
            rows.add(TrayMsg::Add, |id| Node::row(id, "Add a Bluetooth Device"));
        }
        let word = if a.powered {
            "Turn Bluetooth Off"
        } else {
            "Turn Bluetooth On"
        };
        rows.separator()
            .add(TrayMsg::TogglePower, |id| Node::row(id, word));
    }
    if let Some(mode) = s.airplane {
        rows.airplane(TrayMsg::Airplane, TrayMsg::AirplaneNote, mode);
    }
    rows.separator()
        .add(TrayMsg::Exit, |id| Node::row(id, "Exit"));
    rows
}

pub fn menu_nodes(s: &Snapshot) -> Vec<Node> {
    menu(s).into_nodes()
}

pub fn tooltip(s: &Snapshot) -> String {
    match &s.adapter {
        None => "No Bluetooth adapter".into(),
        Some(a) if !a.powered => "Bluetooth is off".into(),
        Some(_) => {
            let devices = connected(s);
            let head = match devices.len() {
                0 => return "Bluetooth: no devices connected".into(),
                1 => "Bluetooth: 1 device connected".to_string(),
                n => format!("Bluetooth: {n} devices connected"),
            };
            devices
                .iter()
                .fold(head, |tip, d| tip + "\n" + &bz::battery_row(d))
        }
    }
}

pub fn shown(s: &Snapshot) -> Shown {
    let on = s.adapter.as_ref().is_some_and(|a| a.powered);
    Shown {
        icon: Icon::Pixmap {
            size: bticon::SIZE as i32,
            argb: bticon::draw(on).to_argb_be(),
        },
        tip: tooltip(s),
        active: s.adapter.is_some(),
        nodes: menu_nodes(s),
    }
}

/// Bring the tray item up, passive until the first snapshot shows an adapter.
pub fn start(waker: repriseapp::Waker) -> reprisetray::Started {
    let config = Config {
        bus_name: BUS_NAME,
        id: "reprise-bluetooth",
        title: "Bluetooth",
        notify_icon: "bluetooth",
        raise: Raise::MenuRow(Rows::id(&TrayMsg::Show)),
    };
    reprisetray::start(
        config,
        shown(&Snapshot::default()),
        std::sync::Arc::new(move || waker.wake()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::{Adapter, Device};
    use reprisebus::airplane::Mode;

    fn snap(powered: bool, connected: usize) -> Snapshot {
        Snapshot {
            adapter: Some(Adapter {
                path: "/a".into(),
                powered,
                ..Adapter::default()
            }),
            devices: (0..connected)
                .map(|i| Device {
                    path: format!("/a/{i}"),
                    alias: format!("Dev {i}"),
                    paired: true,
                    connected: true,
                    ..Device::default()
                })
                .collect(),
            airplane: None,
        }
    }

    fn labels(s: &Snapshot) -> Vec<String> {
        menu_nodes(s).into_iter().map(|n| n.label).collect()
    }

    #[test]
    fn tooltip_counts_connected_devices() {
        assert_eq!(tooltip(&Snapshot::default()), "No Bluetooth adapter");
        assert_eq!(tooltip(&snap(false, 0)), "Bluetooth is off");
        assert_eq!(tooltip(&snap(true, 0)), "Bluetooth: no devices connected");
        assert_eq!(
            tooltip(&snap(true, 1)),
            "Bluetooth: 1 device connected\nDev 0"
        );
        assert_eq!(
            tooltip(&snap(true, 3)),
            "Bluetooth: 3 devices connected\nDev 0\nDev 1\nDev 2"
        );
    }

    #[test]
    fn connected_devices_show_their_battery_in_the_tooltip_and_the_menu() {
        let mut s = snap(true, 2);
        s.devices[0].battery = Some(80);
        s.devices.push(Device {
            path: "/a/off".into(),
            alias: "Away".into(),
            paired: true,
            battery: Some(50),
            ..Device::default()
        });
        assert_eq!(
            tooltip(&s),
            "Bluetooth: 2 devices connected\nDev 0 \u{2014} 80%\nDev 1"
        );
        assert_eq!(
            labels(&s)[..3],
            ["Dev 0 \u{2014} 80%", "Dev 1", ""],
            "a row per connected device, then a separator"
        );
        let click = |row| translate(Rows::clicked(&row), &s);
        assert_eq!(click(TrayMsg::Device(1)), Some(TrayMsg::Device(1)));
        assert_eq!(connected(&s)[1].path, "/a/1");
        assert_eq!(
            labels(&snap(false, 2))[0],
            "Show Bluetooth Devices",
            "no device rows while the radio is off"
        );
    }

    #[test]
    fn the_menu_follows_the_radio() {
        assert_eq!(
            labels(&snap(true, 0)),
            [
                "Show Bluetooth Devices",
                "Add a Bluetooth Device",
                "",
                "Turn Bluetooth Off",
                "",
                "Exit"
            ]
        );
        assert_eq!(
            labels(&snap(false, 0)),
            [
                "Show Bluetooth Devices",
                "",
                "Turn Bluetooth On",
                "",
                "Exit"
            ]
        );
        assert_eq!(
            labels(&Snapshot::default()),
            ["Show Bluetooth Devices", "", "Exit"]
        );
    }

    #[test]
    fn airplane_mode_is_a_check_row_under_the_power_row_on_a_machine_with_a_radio_switch() {
        let mut s = snap(false, 0);
        s.airplane = Some(Mode::On);
        let nodes = menu_nodes(&s);
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Show Bluetooth Devices",
                "",
                "Turn Bluetooth On",
                "Airplane Mode",
                "",
                "Exit"
            ]
        );
        assert_eq!(nodes[3].toggle, Some(true));
        s.airplane = Some(Mode::Off);
        assert_eq!(menu_nodes(&s)[3].toggle, Some(false));
        let click = translate(Rows::clicked(&TrayMsg::Airplane), &s);
        assert_eq!(click, Some(TrayMsg::Airplane));
    }

    #[test]
    fn under_a_hardware_kill_the_airplane_row_greys_with_a_note_under_it() {
        let mut s = snap(false, 0);
        s.airplane = Some(Mode::Killed);
        let nodes = menu_nodes(&s);
        assert_eq!(nodes[3].label, "Airplane Mode");
        assert_eq!(nodes[3].toggle, Some(true));
        assert!(!nodes[3].enabled, "the row greys");
        assert_eq!(nodes[4].label, "Turned off by a hardware switch");
        assert!(!nodes[4].enabled);
        assert!(nodes[5].separator);
    }

    #[test]
    fn clicks_map_to_the_rows_and_no_adapter_hides_the_icon() {
        let on = snap(true, 0);
        let click = |row| translate(Rows::clicked(&row), &on);
        assert_eq!(translate(Msg::Activate(0, 0), &on), Some(TrayMsg::Show));
        assert_eq!(click(TrayMsg::Add), Some(TrayMsg::Add));
        assert_eq!(click(TrayMsg::TogglePower), Some(TrayMsg::TogglePower));
        assert_eq!(click(TrayMsg::Exit), Some(TrayMsg::Exit));
        assert_eq!(translate(Msg::Scroll(1), &on), None);
        assert!(!shown(&Snapshot::default()).active);
        assert!(shown(&snap(false, 0)).active);
    }
}
