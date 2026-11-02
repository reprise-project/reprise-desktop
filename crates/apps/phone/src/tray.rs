//! The phone icon in the notification area: what reprise-phone shows on its `reprisetray` item
//! and what a click means. Up while kdeconnectd runs; a left click opens KDE Connect.

use reprisebus::kdeconnect::Device;
use reprisekit::art::icons;
use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

pub const BUS_NAME: &str = "reprise.phone";
pub const TITLE: &str = "Phone";

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    /// The greyed row of a device's name and battery, by device id.
    Status(String),
    NoDevices,
    Ring(String),
    Send(String),
    Settings,
}

/// "Pixel: 80%, charging", or the name alone before a charge is reported.
pub fn status(d: &Device) -> String {
    match d.battery {
        Some(b) if b.charging => format!("{}: {}%, charging", d.name, b.charge),
        Some(b) => format!("{}: {}%", d.name, b.charge),
        None => d.name.clone(),
    }
}

/// Per device its status, then Ring phone and Send files... where its plugins allow; the
/// settings row last, which a left click also opens.
pub fn rows(devices: &[Device]) -> Rows<TrayMsg> {
    let mut rows = Rows::default();
    if devices.is_empty() {
        rows.add(TrayMsg::NoDevices, |id| Node::inert(id, "No devices"));
    }
    for d in devices {
        rows.add(TrayMsg::Status(d.id.clone()), |id| {
            Node::inert(id, status(d))
        });
        if d.ring {
            rows.add(TrayMsg::Ring(d.id.clone()), |id| {
                Node::row(id, "Ring phone")
            });
        }
        if d.share {
            rows.add(TrayMsg::Send(d.id.clone()), |id| {
                Node::row(id, "Send files...")
            });
        }
        rows.separator();
    }
    if devices.is_empty() {
        rows.separator();
    }
    rows.add(TrayMsg::Settings, |id| {
        Node::row(id, "Open KDE Connect settings").as_default()
    });
    rows
}

/// What `msg` means while the menu shows `devices`; the greyed rows mean nothing.
pub fn translate(msg: Msg, devices: &[Device]) -> Option<TrayMsg> {
    match msg {
        Msg::Activate(..) => Some(TrayMsg::Settings),
        msg => rows(devices)
            .translate(msg)
            .filter(|m| !matches!(m, TrayMsg::Status(_) | TrayMsg::NoDevices)),
    }
}

pub fn tooltip(devices: &[Device]) -> String {
    if devices.is_empty() {
        return "KDE Connect: no devices connected".into();
    }
    let lines: Vec<String> = devices.iter().map(status).collect();
    lines.join("\n")
}

/// `devices` is None while kdeconnectd is not running, which hides the icon.
pub fn shown(devices: Option<&[Device]>) -> Shown {
    let list = devices.unwrap_or_default();
    Shown {
        icon: Icon::Pixmap {
            size: icons::PHONE.size().0 as i32,
            argb: icons::PHONE.canvas().to_argb_be(),
        },
        tip: tooltip(list),
        active: devices.is_some(),
        nodes: rows(list).into_nodes(),
    }
}

/// Bring the tray item up, passive until kdeconnectd is found.
pub fn start(waker: repriseapp::Waker) -> reprisetray::Started {
    let config = Config {
        bus_name: BUS_NAME,
        id: "reprise-phone",
        title: TITLE,
        notify_icon: "phone",
        raise: Raise::Activate,
    };
    reprisetray::start(
        config,
        shown(None),
        std::sync::Arc::new(move || waker.wake()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisebus::kdeconnect::Battery;

    fn phone(id: &str, name: &str, battery: Option<(u8, bool)>) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            kind: "smartphone".into(),
            battery: battery.map(|(charge, charging)| Battery { charge, charging }),
            ring: true,
            share: true,
        }
    }

    fn labels(devices: &[Device]) -> Vec<(String, bool)> {
        (rows(devices).into_nodes().into_iter())
            .map(|n| (n.label, n.enabled))
            .collect()
    }

    #[test]
    fn no_devices_says_so_above_the_settings_row() {
        assert_eq!(
            labels(&[]),
            [
                ("No devices".into(), false),
                (String::new(), true),
                ("Open KDE Connect settings".into(), true),
            ]
        );
        assert_eq!(tooltip(&[]), "KDE Connect: no devices connected");
    }

    #[test]
    fn each_device_gets_its_battery_and_the_rows_its_plugins_allow() {
        let mut tab = phone("t1", "Tab", None);
        tab.ring = false;
        let devices = [phone("p1", "Pixel", Some((80, true))), tab];
        assert_eq!(
            labels(&devices),
            [
                ("Pixel: 80%, charging".into(), false),
                ("Ring phone".into(), true),
                ("Send files...".into(), true),
                (String::new(), true),
                ("Tab".into(), false),
                ("Send files...".into(), true),
                (String::new(), true),
                ("Open KDE Connect settings".into(), true),
            ]
        );
        assert_eq!(tooltip(&devices), "Pixel: 80%, charging\nTab");
        assert_eq!(
            status(&phone("p", "Pixel", Some((15, false)))),
            "Pixel: 15%"
        );
    }

    #[test]
    fn clicks_map_to_their_device_and_status_rows_do_nothing() {
        let devices = [
            phone("p1", "Pixel", Some((80, false))),
            phone("p2", "Moto", None),
        ];
        let click = |row: TrayMsg| translate(Rows::clicked(&row), &devices);
        assert_eq!(
            click(TrayMsg::Ring("p2".into())),
            Some(TrayMsg::Ring("p2".into()))
        );
        assert_eq!(
            click(TrayMsg::Send("p1".into())),
            Some(TrayMsg::Send("p1".into()))
        );
        assert_eq!(click(TrayMsg::Settings), Some(TrayMsg::Settings));
        assert_eq!(click(TrayMsg::Status("p1".into())), None);
        assert_eq!(
            translate(Msg::Activate(0, 0), &devices),
            Some(TrayMsg::Settings)
        );
        assert_eq!(translate(Msg::Scroll(1), &devices), None);
        let settings = rows(&devices).into_nodes().into_iter().last().unwrap();
        assert!(settings.default, "a left click opens the settings");
    }

    #[test]
    fn the_icon_shows_while_kdeconnectd_runs() {
        assert!(!shown(None).active);
        let up = shown(Some(&[]));
        assert!(up.active);
        assert_eq!(up.nodes[0].label, "No devices");
        let Icon::Pixmap { size, argb } = up.icon else {
            panic!("a drawn icon");
        };
        assert_eq!(argb.len(), (size * size * 4) as usize);
    }
}
