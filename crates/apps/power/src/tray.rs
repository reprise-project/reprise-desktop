//! The battery in the notification area: what reprise-power shows on its `reprisetray` item (icon,
//! tooltip, menu rows) and what a click on it means. The D-Bus side is `reprisetray`'s.

use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

use crate::profiles::{self, Profiles};
use crate::upower::{Power, State};

pub const BUS_NAME: &str = "reprise.power";

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    OpenMeter,
    OpenProperties,
    Exit,
    Profile(usize),
}

/// What `msg` means while the menu shows `p`'s profiles.
pub fn translate(msg: Msg, p: &Profiles) -> Option<TrayMsg> {
    match msg {
        Msg::Activate(..) | Msg::SecondaryActivate => Some(TrayMsg::OpenMeter),
        msg => menu(p).translate(msg),
    }
}

/// The tray menu's rows: one radio row per power profile the daemon offers, the active one on, and a
/// separator under them (none while the daemon is missing); then Open Power Meter, Adjust Power Properties, and Exit.
pub fn menu(p: &Profiles) -> Rows<TrayMsg> {
    let mut rows = Rows::default();
    for (i, name) in p.available.iter().enumerate() {
        rows.add(TrayMsg::Profile(i), |id| {
            Node::radio(id, profiles::label(name), *name == p.active)
        });
    }
    if !p.available.is_empty() {
        rows.separator();
    }
    rows.add(TrayMsg::OpenMeter, |id| Node::row(id, "Open Power Meter"))
        .add(TrayMsg::OpenProperties, |id| {
            Node::row(id, "Adjust Power Properties")
        })
        .separator()
        .add(TrayMsg::Exit, |id| Node::row(id, "Exit"));
    rows
}

pub fn menu_nodes(p: &Profiles) -> Vec<Node> {
    menu(p).into_nodes()
}

pub fn clock(seconds: u64) -> String {
    let minutes = seconds / 60;
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

pub fn tooltip(p: &Power) -> String {
    let Some(b) = &p.battery else {
        return "On AC power".to_string();
    };
    match b.state {
        State::Full => format!("Fully charged ({}%)", b.percent),
        State::Charging => format!("Charging ({}%)", b.percent),
        State::Discharging if b.time_to_empty > 0 => {
            format!("{} remaining ({}%)", clock(b.time_to_empty), b.percent)
        }
        State::Discharging => format!("{}% remaining", b.percent),
        State::Unknown if p.on_battery => format!("{}% remaining", b.percent),
        State::Unknown => format!("On AC power ({}%)", b.percent),
    }
}

/// Everything the item shows for a drawn icon (16x16 ARGB big-endian), tooltip, status and power profiles.
pub fn shown(pixmap: Vec<u8>, tip: String, active: bool, profiles: &Profiles) -> Shown {
    Shown {
        icon: Icon::Pixmap {
            size: reprisekit::art::powericon::SIZE as i32,
            argb: pixmap,
        },
        tip,
        active,
        nodes: menu_nodes(profiles),
    }
}

/// Bring the tray item up; see `reprisetray::start`. Passive until the first snapshot says whether
/// there is a battery, so a desktop never flashes the icon.
pub fn start(waker: repriseapp::Waker) -> reprisetray::Started {
    let config = Config {
        bus_name: BUS_NAME,
        id: "reprise-power",
        title: "Power",
        notify_icon: "battery-caution",
        // A second instance raises the sheet on its first tab; Activate would open the Power Meter tab instead.
        raise: Raise::MenuRow(Rows::id(&TrayMsg::OpenProperties)),
    };
    let initial = shown(
        reprisekit::art::powericon::draw(reprisekit::art::powericon::Source::Ac).to_argb_be(),
        "On AC power".to_string(),
        false,
        &Profiles::default(),
    );
    reprisetray::start(config, initial, std::sync::Arc::new(move || waker.wake()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upower::Battery;

    fn power(state: State, percent: u8, empty: u64, on_battery: bool) -> Power {
        Power {
            on_battery,
            battery: Some(Battery {
                state,
                percent,
                time_to_empty: empty,
                time_to_full: 0,
            }),
        }
    }

    #[test]
    fn tooltip_names_the_power_state() {
        assert_eq!(tooltip(&Power::default()), "On AC power");
        assert_eq!(
            tooltip(&power(State::Full, 100, 0, false)),
            "Fully charged (100%)"
        );
        assert_eq!(
            tooltip(&power(State::Charging, 72, 0, false)),
            "Charging (72%)"
        );
        assert_eq!(
            tooltip(&power(State::Discharging, 45, 7800, true)),
            "2:10 remaining (45%)"
        );
        assert_eq!(
            tooltip(&power(State::Discharging, 45, 0, true)),
            "45% remaining"
        );
        assert_eq!(
            tooltip(&power(State::Unknown, 80, 0, false)),
            "On AC power (80%)"
        );
    }

    #[test]
    fn clock_is_hours_and_minutes() {
        assert_eq!(clock(59), "0:00");
        assert_eq!(clock(60), "0:01");
        assert_eq!(clock(7800), "2:10");
        assert_eq!(clock(36_000), "10:00");
    }

    #[test]
    fn menu_has_the_meter_properties_and_exit_rows() {
        let nodes = menu_nodes(&Profiles::default());
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Open Power Meter", "Adjust Power Properties", "", "Exit"]
        );
        assert!(nodes[2].separator);
    }

    #[test]
    fn menu_leads_with_the_power_profiles() {
        let p = Profiles {
            available: vec!["power-saver".into(), "balanced".into()],
            active: "balanced".into(),
        };
        let nodes = menu_nodes(&p);
        let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Power saver",
                "Balanced",
                "",
                "Open Power Meter",
                "Adjust Power Properties",
                "",
                "Exit"
            ]
        );
        assert!(nodes[..2].iter().all(|n| n.radio));
        assert_eq!(
            (nodes[0].toggle, nodes[1].toggle),
            (Some(false), Some(true)),
            "the active profile is on"
        );
        assert!(nodes[2].separator);
        let ids: Vec<i32> = nodes.iter().map(|n| n.id).collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "every row has its own id");
        assert_eq!(
            translate(Msg::Clicked(nodes[1].id), &p),
            Some(TrayMsg::Profile(1))
        );
    }

    #[test]
    fn clicks_map_to_the_rows() {
        let translate = |msg| translate(msg, &Profiles::default());
        assert_eq!(translate(Msg::Activate(0, 0)), Some(TrayMsg::OpenMeter));
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::OpenMeter)),
            Some(TrayMsg::OpenMeter)
        );
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::OpenProperties)),
            Some(TrayMsg::OpenProperties)
        );
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::Exit)),
            Some(TrayMsg::Exit)
        );
        assert_eq!(translate(Msg::Scroll(1)), None);
    }
}
