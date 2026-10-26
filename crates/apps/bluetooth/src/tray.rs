//! The Bluetooth icon in the notification area: what reprise-bluetooth shows on
//! its `reprisetray` item and what a click means. Hidden while there is no
//! adapter. The D-Bus side is `reprisetray`'s.

use reprisekit::art::bticon;
use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

use crate::bz::Snapshot;

pub const BUS_NAME: &str = "reprise.bluetooth";

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    Show,
    Add,
    TogglePower,
    Exit,
}

/// What `msg` means while the menu shows `s`'s rows.
pub fn translate(msg: Msg, s: &Snapshot) -> Option<TrayMsg> {
    match msg {
        Msg::Activate(..) => Some(TrayMsg::Show),
        msg => menu(s).translate(msg),
    }
}

/// Show and Exit always; Add while the radio is on; the power row while
/// there is a radio to turn.
pub fn menu(s: &Snapshot) -> Rows<TrayMsg> {
    let mut rows = Rows::default();
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
        Some(_) => match s.devices.iter().filter(|d| d.paired && d.connected).count() {
            0 => "Bluetooth: no devices connected".into(),
            1 => "Bluetooth: 1 device connected".into(),
            n => format!("Bluetooth: {n} devices connected"),
        },
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
                    paired: true,
                    connected: true,
                    ..Device::default()
                })
                .collect(),
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
        assert_eq!(tooltip(&snap(true, 1)), "Bluetooth: 1 device connected");
        assert_eq!(tooltip(&snap(true, 3)), "Bluetooth: 3 devices connected");
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
