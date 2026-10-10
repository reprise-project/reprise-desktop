//! The updates icon in the notification area: shown on reprise-updates' `reprisetray` item only while there are updates to install, one is installing, or one waits on a restart. A left click opens Software Updates.

use reprisetray::{Config, Icon, Msg, Node, Raise, Shown};

use crate::model::TrayLook;

pub const BUS_NAME: &str = "reprise.updates";
pub const TITLE: &str = "Software Updates";

/// Menu row ids, as they travel in `GetLayout` and come back in `Event`.
pub const ID_OPEN: i32 = 1;
pub const ID_CHECK: i32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayMsg {
    Open,
    Check,
}

pub fn translate(msg: Msg) -> Option<TrayMsg> {
    Some(match msg {
        Msg::Activate(..) | Msg::Clicked(ID_OPEN) | Msg::BalloonClicked(_) => TrayMsg::Open,
        Msg::Clicked(ID_CHECK) => TrayMsg::Check,
        Msg::Clicked(_) | Msg::SecondaryActivate | Msg::Scroll(_) => return None,
    })
}

pub fn menu_nodes() -> Vec<Node> {
    vec![
        Node::row(ID_OPEN, "Review Updates"),
        Node::row(ID_CHECK, "Check for Updates"),
    ]
}

pub fn shown(look: &TrayLook) -> Shown {
    Shown {
        icon: Icon::Name(look.icon.into()),
        tip: look.tip.clone(),
        active: look.active,
        nodes: menu_nodes(),
    }
}

/// Bring the tray item up (see `reprisetray::start`); passive until a check finds something.
pub fn start(waker: repriseapp::Waker, first: &TrayLook) -> reprisetray::Started {
    reprisetray::start(
        Config {
            bus_name: BUS_NAME,
            id: "reprise-updates",
            title: TITLE,
            notify_icon: "software-update-available",
            // A second instance opens the window.
            raise: Raise::Activate,
        },
        shown(first),
        std::sync::Arc::new(move || waker.wake()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_map_to_the_rows() {
        assert_eq!(translate(Msg::Activate(1, 2)), Some(TrayMsg::Open));
        assert_eq!(translate(Msg::BalloonClicked(0)), Some(TrayMsg::Open));
        assert_eq!(translate(Msg::Clicked(ID_OPEN)), Some(TrayMsg::Open));
        assert_eq!(translate(Msg::Clicked(ID_CHECK)), Some(TrayMsg::Check));
        assert_eq!(translate(Msg::Clicked(0)), None);
        assert_eq!(translate(Msg::Scroll(1)), None);
    }
}
