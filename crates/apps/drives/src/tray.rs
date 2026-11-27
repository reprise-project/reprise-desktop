//! The Safely Remove Hardware icon: what reprise-drives shows on its
//! `reprisetray` item and what a click means. A left click opens the menu as a
//! flyout; a right click gets the same rows through the dbusmenu. Up only while a
//! removable drive, phone or camera is mounted.

use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

use crate::model::Entry;

pub const BUS_NAME: &str = "reprise.drives";
/// The tooltip, which is also the program's title.
pub fn tip() -> &'static str {
    static TIP: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    TIP.get_or_init(|| reprisekit::tr!("tip"))
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    /// Left click at its click point (`Msg::Activate`): the menu.
    Open(i32, i32),
    /// The dbusmenu's row `i`: safely remove that drive or device.
    Remove(usize),
    /// A clickable balloon, by its tag.
    Balloon(u32),
    /// Button `i` of a balloon with buttons, by its tag.
    Button(u32, usize),
}

pub fn rows(entries: &[Entry]) -> Rows<TrayMsg> {
    let mut rows = Rows::default();
    for (i, e) in entries.iter().enumerate() {
        rows.add(TrayMsg::Remove(i), |id| Node::row(id, e.label.clone()));
    }
    rows
}

/// What `msg` means while the menu shows `entries`.
pub fn translate(msg: Msg, entries: &[Entry]) -> Option<TrayMsg> {
    Some(match msg {
        Msg::Activate(x, y) => TrayMsg::Open(x, y),
        Msg::Clicked(_) => return rows(entries).translate(msg),
        Msg::BalloonClicked(tag) => TrayMsg::Balloon(tag),
        Msg::BalloonButton(tag, i) => TrayMsg::Button(tag, i),
        Msg::SecondaryActivate | Msg::Scroll(_) => return None,
    })
}

pub fn shown(entries: &[Entry]) -> Shown {
    Shown {
        icon: Icon::Name("media-eject".into()),
        tip: tip().into(),
        active: !entries.is_empty(),
        nodes: rows(entries).into_nodes(),
    }
}

/// Bring the tray item up; see `reprisetray::start`.
pub fn start(waker: repriseapp::Waker) -> reprisetray::Started {
    reprisetray::start(
        Config {
            bus_name: BUS_NAME,
            id: "reprise-drives",
            title: tip().into(),
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
    use crate::model::Target;

    #[test]
    fn menu_rows_map_back_to_their_drive() {
        let entries = vec![
            Entry {
                target: Target::Disk("a".into()),
                label: "Safely remove A - X".into(),
            },
            Entry {
                target: Target::Disk("b".into()),
                label: "Safely remove B - Y".into(),
            },
        ];
        let n = rows(&entries).into_nodes();
        assert_eq!(n[1].label, "Safely remove B - Y");
        let translate = |msg| translate(msg, &entries);
        assert_eq!(translate(Msg::Clicked(n[1].id)), Some(TrayMsg::Remove(1)));
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
            target: Target::Disk("a".into()),
            label: "x".into(),
        }];
        assert!(shown(&one).active);
        assert_eq!(shown(&one).tip, "Safely Remove Hardware");
    }
}
