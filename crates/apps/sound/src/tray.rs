//! The speaker in the notification area: what reprise-sound shows on its
//! `reprisetray` item (icon name, tooltip, the mute toggle in the menu) and
//! what a click on it means: one click opens the volume popup, two inside the
//! double-click interval open the sheet. The D-Bus side is `reprisetray`'s.

use std::time::{Duration, Instant};

use reprisekit::pointer::DOUBLE_CLICK_SLOP;
use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

pub const BUS_NAME: &str = "reprise.sound";

/// What the tray asks the app to do.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    /// Left click at its click point (`Msg::Activate`): the volume popup,
    /// or with a second one the sheet.
    Activate(i32, i32),
    /// Adjust Audio Properties: the sheet.
    Properties,
    /// Open Volume Control: the mixer window.
    VolumeControl,
    ToggleMute,
    /// Wheel over the icon; positive is up.
    Scroll(i32),
    Exit,
}

/// The most wheel steps one message counts for.
const WHEEL_MAX: i32 = 20;

/// What a tray event means here. A middle click toggles mute without
/// opening the popup.
pub fn translate(msg: Msg) -> Option<TrayMsg> {
    Some(match msg {
        Msg::Activate(x, y) => TrayMsg::Activate(x, y),
        Msg::SecondaryActivate => TrayMsg::ToggleMute,
        // Any bus peer can send a delta; a full sweep of the slider is
        // twenty 5 % steps, so nothing past that means anything.
        Msg::Scroll(delta) => TrayMsg::Scroll(delta.clamp(-WHEEL_MAX, WHEEL_MAX)),
        Msg::Clicked(_) => return menu(false).translate(msg),
        Msg::BalloonClicked(_) | Msg::BalloonButton(..) => return None,
    })
}

/// The tray menu: the mixer window, the bold default, then the
/// properties sheet, the mute toggle and Exit.
pub fn menu(muted: bool) -> Rows<TrayMsg> {
    let mut rows = Rows::default();
    rows.add(TrayMsg::VolumeControl, |id| {
        Node::row(id, "Open Volume Control").as_default()
    })
    .add(TrayMsg::Properties, |id| {
        Node::row(id, "Adjust Audio Properties")
    })
    .add(TrayMsg::ToggleMute, |id| Node::toggle(id, "Mute", muted))
    .separator()
    .add(TrayMsg::Exit, |id| Node::row(id, "Exit"));
    rows
}

pub fn menu_nodes(muted: bool) -> Vec<Node> {
    menu(muted).into_nodes()
}

/// The one line the host shows when the pointer rests on the icon.
pub fn tooltip_text(volume: f64, mute: bool) -> String {
    if mute {
        "Volume: Muted".to_string()
    } else {
        format!("Volume: {}%", (volume * 100.0).round() as i64)
    }
}

/// Everything the item shows for an icon name, tooltip and mute state.
pub fn shown(icon: &str, tip: String, muted: bool) -> Shown {
    Shown {
        icon: Icon::Name(icon.to_string()),
        tip,
        active: true,
        nodes: menu_nodes(muted),
    }
}

/// Tells a single click on the icon from a double one. The first press is
/// held for the double-click interval; a second inside it and within
/// [`DOUBLE_CLICK_SLOP`] of it makes a double, and when none comes `due`
/// hands the first back. A second press too far away starts over.
#[derive(Default)]
pub struct ClickGate {
    first: Option<(Instant, (i32, i32))>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// Wait the interval out, then `due` gives the point back.
    First,
    Double,
}

impl ClickGate {
    pub fn press(&mut self, now: Instant, at: (i32, i32), interval: Duration) -> Press {
        match self.first.take() {
            Some((t, first))
                if now.duration_since(t) < interval
                    && (at.0 - first.0).abs() <= DOUBLE_CLICK_SLOP
                    && (at.1 - first.1).abs() <= DOUBLE_CLICK_SLOP =>
            {
                Press::Double
            }
            _ => {
                self.first = Some((now, at));
                Press::First
            }
        }
    }

    /// The first press's point, once the interval has passed without a
    /// second.
    pub fn due(&mut self, now: Instant, interval: Duration) -> Option<(i32, i32)> {
        match self.first {
            Some((t, at)) if now.duration_since(t) >= interval => {
                self.first = None;
                Some(at)
            }
            _ => None,
        }
    }
}

/// Bring the tray item up; see `reprisetray::start`.
pub fn start(waker: repriseapp::Waker) -> reprisetray::Started {
    let config = Config {
        bus_name: BUS_NAME,
        id: "reprise-sound",
        title: "Volume",
        notify_icon: "audio-volume-high",
        // A second instance raises the sheet through the Properties row:
        // Activate would wait out a double click and open the popup.
        raise: Raise::MenuRow(Rows::id(&TrayMsg::Properties)),
    };
    reprisetray::start(
        config,
        shown("audio-volume-high", "Volume".to_string(), false),
        std::sync::Arc::new(move || waker.wake()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_has_five_rows() {
        let nodes = menu_nodes(false);
        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes[0].id, Rows::id(&TrayMsg::VolumeControl));
        assert_eq!(nodes[0].label, "Open Volume Control");
        assert!(nodes[0].default, "Open Volume Control is the bold default");
        assert_eq!(nodes.iter().filter(|n| n.default).count(), 1);
        assert_eq!(nodes[1].id, Rows::id(&TrayMsg::Properties));
        assert_eq!(nodes[1].label, "Adjust Audio Properties");
        assert_eq!(nodes[2].id, Rows::id(&TrayMsg::ToggleMute));
        assert_eq!(nodes[2].label, "Mute");
        assert_eq!(nodes[2].toggle, Some(false));
        assert!(nodes[3].separator);
        assert_eq!(nodes[4].id, Rows::id(&TrayMsg::Exit));
        assert_eq!(nodes[4].label, "Exit");
    }

    #[test]
    fn mute_row_follows_the_state() {
        assert_eq!(menu_nodes(true)[2].toggle, Some(true));
        assert_eq!(menu_nodes(false)[2].toggle, Some(false));
    }

    #[test]
    fn clicks_map_to_the_rows() {
        assert_eq!(
            translate(Msg::Activate(3, 4)),
            Some(TrayMsg::Activate(3, 4))
        );
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::Properties)),
            Some(TrayMsg::Properties)
        );
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::VolumeControl)),
            Some(TrayMsg::VolumeControl)
        );
        assert_eq!(translate(Msg::SecondaryActivate), Some(TrayMsg::ToggleMute));
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::ToggleMute)),
            Some(TrayMsg::ToggleMute)
        );
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::Exit)),
            Some(TrayMsg::Exit)
        );
        assert_eq!(translate(Msg::Clicked(menu_nodes(false)[3].id)), None);
        assert_eq!(translate(Msg::Scroll(-1)), Some(TrayMsg::Scroll(-1)));
        // A peer's absurd delta is a full sweep, not an overflow later.
        assert_eq!(translate(Msg::Scroll(i32::MAX)), Some(TrayMsg::Scroll(20)));
        assert_eq!(translate(Msg::Scroll(i32::MIN)), Some(TrayMsg::Scroll(-20)));
    }

    const MS500: Duration = Duration::from_millis(500);

    #[test]
    fn a_lone_press_opens_the_popup_once_the_interval_passes() {
        let t0 = Instant::now();
        let mut g = ClickGate::default();
        assert_eq!(g.press(t0, (5, 6), MS500), Press::First);
        assert_eq!(
            g.due(t0 + MS500 / 2, MS500),
            None,
            "still waiting for a second press"
        );
        assert_eq!(g.due(t0 + MS500, MS500), Some((5, 6)));
        assert_eq!(g.due(t0 + MS500 * 2, MS500), None, "and only once");
    }

    #[test]
    fn a_second_press_cancels_the_popup() {
        let t0 = Instant::now();
        let mut g = ClickGate::default();
        g.press(t0, (5, 6), MS500);
        assert_eq!(g.press(t0 + MS500 / 2, (5, 6), MS500), Press::Double);
        assert_eq!(g.due(t0 + MS500 * 2, MS500), None);
    }

    #[test]
    fn a_late_second_press_starts_over() {
        let t0 = Instant::now();
        let mut g = ClickGate::default();
        g.press(t0, (5, 6), MS500);
        assert_eq!(g.press(t0 + MS500 * 2, (7, 8), MS500), Press::First);
        assert_eq!(g.due(t0 + MS500 * 3, MS500), Some((7, 8)));
    }

    #[test]
    fn a_second_press_past_the_slop_starts_over() {
        let t0 = Instant::now();
        let mut g = ClickGate::default();
        g.press(t0, (5, 6), MS500);
        let near = (5 + DOUBLE_CLICK_SLOP, 6 - DOUBLE_CLICK_SLOP);
        assert_eq!(g.press(t0 + MS500 / 4, near, MS500), Press::Double);
        g.press(t0 + MS500, (5, 6), MS500);
        let far = (5 + DOUBLE_CLICK_SLOP + 1, 6);
        assert_eq!(g.press(t0 + MS500 * 5 / 4, far, MS500), Press::First);
        assert_eq!(g.due(t0 + MS500 * 9 / 4, MS500), Some(far));
    }

    #[test]
    fn tooltip_reads_as_a_percentage() {
        assert_eq!(tooltip_text(0.64, false), "Volume: 64%");
        assert_eq!(tooltip_text(1.0, false), "Volume: 100%");
        assert_eq!(tooltip_text(0.0, false), "Volume: 0%");
        assert_eq!(tooltip_text(0.645, true), "Volume: Muted");
    }
}
