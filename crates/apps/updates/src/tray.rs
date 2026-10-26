//! The updates icon in the notification area: shown on reprise-updates' `reprisetray` item only while there are updates to install, one is installing, or one waits on a restart. A left click opens Software Updates.

use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

use crate::model::TrayLook;

pub const BUS_NAME: &str = "reprise.updates";
pub const TITLE: &str = "Software Updates";

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    Open,
    Check,
}

pub fn translate(msg: Msg) -> Option<TrayMsg> {
    match msg {
        Msg::Activate(..) | Msg::BalloonClicked(_) => Some(TrayMsg::Open),
        msg => rows().translate(msg),
    }
}

pub fn rows() -> Rows<TrayMsg> {
    let mut rows = Rows::default();
    rows.add(TrayMsg::Open, |id| Node::row(id, "Review Updates"))
        .add(TrayMsg::Check, |id| Node::row(id, "Check for Updates"));
    rows
}

pub fn shown(look: &TrayLook) -> Shown {
    Shown {
        icon: Icon::Name(look.icon.into()),
        tip: look.tip.clone(),
        active: look.active,
        nodes: rows().into_nodes(),
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
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::Open)),
            Some(TrayMsg::Open)
        );
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::Check)),
            Some(TrayMsg::Check)
        );
        assert_eq!(translate(Msg::Clicked(0)), None);
        assert_eq!(translate(Msg::Scroll(1)), None);
    }
}
