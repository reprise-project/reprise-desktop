//! The Safely Remove Hardware icon: what reprise-drives shows on its
//! `reprisetray` item and what a click means. A left click opens the menu as a
//! flyout; a right click gets the same rows through the dbusmenu. Up only while a
//! removable drive is mounted.

use reprisetray::{Config, Icon, Msg, Node, Raise, Shown};

use crate::model::Entry;

pub const BUS_NAME: &str = "reprise.drives";
pub const TIP: &str = "Safely Remove Hardware";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayMsg {
    /// Left click at its click point (`Msg::Activate`): the menu.
    Open(i32, i32),
    /// The dbusmenu's row `i`: safely remove that drive.
    Remove(usize),
    /// A clickable balloon, by its tag.
    Balloon(u32),
    /// Button `i` of a balloon with buttons, by its tag.
    Button(u32, usize),
}

/// Row ids are the entry's index plus one; 0 is the dbusmenu's root.
pub fn nodes(entries: &[Entry]) -> Vec<Node> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| Node::row(i as i32 + 1, e.label.clone()))
        .collect()
}

pub fn translate(msg: Msg) -> Option<TrayMsg> {
    Some(match msg {
        Msg::Activate(x, y) => TrayMsg::Open(x, y),
        Msg::Clicked(id) if id > 0 => TrayMsg::Remove(id as usize - 1),
        Msg::BalloonClicked(tag) => TrayMsg::Balloon(tag),
        Msg::BalloonButton(tag, i) => TrayMsg::Button(tag, i),
        Msg::Clicked(_) | Msg::SecondaryActivate | Msg::Scroll(_) => return None,
    })
}

pub fn shown(entries: &[Entry]) -> Shown {
    Shown {
        icon: Icon::Name("media-eject".into()),
        tip: TIP.into(),
        active: !entries.is_empty(),
        nodes: nodes(entries),
    }
}

/// Bring the tray item up; see `reprisetray::start`.
pub fn start(waker: repriseapp::Waker) -> reprisetray::Started {
    reprisetray::start(
        Config {
            bus_name: BUS_NAME,
            id: "reprise-drives",
            title: TIP,
            notify_icon: "drive-removable-media",
            // No window to raise: a second instance clicks the root,
            // which is no row, and exits.
            raise: Raise::MenuRow(0),
        },
        shown(&[]),
        std::sync::Arc::new(move || waker.wake()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_rows_map_back_to_their_drive() {
        let entries = vec![
            Entry {
                disk: "a".into(),
                label: "Safely remove A - X".into(),
            },
            Entry {
                disk: "b".into(),
                label: "Safely remove B - Y".into(),
            },
        ];
        let n = nodes(&entries);
        assert_eq!(n[1].id, 2);
        assert_eq!(n[1].label, "Safely remove B - Y");
        assert_eq!(translate(Msg::Clicked(2)), Some(TrayMsg::Remove(1)));
        assert_eq!(translate(Msg::Clicked(0)), None);
        assert_eq!(translate(Msg::Activate(5, 6)), Some(TrayMsg::Open(5, 6)));
        assert_eq!(translate(Msg::BalloonClicked(3)), Some(TrayMsg::Balloon(3)));
        assert_eq!(
            translate(Msg::BalloonButton(3, 1)),
            Some(TrayMsg::Button(3, 1))
        );
    }

    #[test]
    fn the_icon_shows_only_with_a_drive_to_remove() {
        assert!(!shown(&[]).active);
        let one = [Entry {
            disk: "a".into(),
            label: "x".into(),
        }];
        assert!(shown(&one).active);
        assert_eq!(shown(&one).tip, "Safely Remove Hardware");
    }
}
