//! The printer icon in the notification area, shown while the user has
//! documents waiting, printing or aborted and kept by CUPS.

use reprisetray::{Config, Icon, Msg, Node, Raise, Rows, Shown};

use crate::cups::{Job, JobState, Snapshot};

pub const BUS_NAME: &str = "reprise.printers";
pub const TITLE: &str = "Printers";
const ICON: &str = "printer";
const ERROR_ICON: &str = "printer-error";

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum TrayMsg {
    Folder,
    Open,
    Queue(usize),
    Balloon(u32),
}

/// What `msg` means while the menu shows `user`'s printers in `s`.
pub fn translate(msg: Msg, s: &Snapshot, user: &str) -> Option<TrayMsg> {
    Some(match msg {
        Msg::Activate(..) => TrayMsg::Open,
        Msg::Clicked(_) => return menu(s, user).translate(msg),
        Msg::BalloonClicked(tag) => TrayMsg::Balloon(tag),
        Msg::SecondaryActivate | Msg::Scroll(_) | Msg::BalloonButton(..) => return None,
    })
}

/// The printers holding `user`'s documents, in folder order.
pub fn busy_printers(s: &Snapshot, user: &str) -> Vec<String> {
    s.printers
        .iter()
        .filter(|p| s.documents_of(&p.name).any(|j| j.owner == user))
        .map(|p| p.name.clone())
        .collect()
}

fn failed<'a>(s: &'a Snapshot, user: &'a str) -> impl Iterator<Item = &'a Job> {
    s.aborted.iter().filter(move |j| j.owner == user)
}

/// Tip text: which of `user`'s documents could not print, else how many
/// wait and what prints.
pub fn tip(s: &Snapshot, user: &str) -> String {
    let failed: Vec<_> = failed(s, user).collect();
    match failed.as_slice() {
        [] => {}
        [one] => return format!("{} could not be printed", one.name),
        jobs => return format!("{} documents could not be printed", jobs.len()),
    }
    let mine: Vec<_> = s.jobs.iter().filter(|j| j.owner == user).collect();
    match mine.as_slice() {
        [] => TITLE.into(),
        [one] if one.state == JobState::Printing => format!("Printing {}", one.name),
        jobs => {
            let n = jobs.len();
            let docs = if n == 1 { "document" } else { "documents" };
            format!("{n} {docs} pending for {user}")
        }
    }
}

/// The printers holding `user`'s documents, the first the bold default, then Printers.
pub fn menu(s: &Snapshot, user: &str) -> Rows<TrayMsg> {
    let mut rows = Rows::default();
    let busy = busy_printers(s, user);
    for (i, p) in busy.iter().enumerate() {
        rows.add(TrayMsg::Queue(i), |id| {
            let n = Node::row(id, p.clone());
            if i == 0 { n.as_default() } else { n }
        });
    }
    if !busy.is_empty() {
        rows.separator();
    }
    rows.add(TrayMsg::Folder, |id| Node::row(id, TITLE));
    rows
}

pub fn shown(s: &Snapshot, user: &str) -> Shown {
    let icon = if failed(s, user).next().is_some() {
        ERROR_ICON
    } else {
        ICON
    };
    Shown {
        icon: Icon::Name(icon.into()),
        tip: tip(s, user),
        active: !busy_printers(s, user).is_empty(),
        nodes: menu(s, user).into_nodes(),
    }
}

/// A second instance raises the folder through the Printers row.
pub fn start(waker: repriseapp::Waker, first: Shown) -> reprisetray::Started {
    reprisetray::start(
        Config {
            bus_name: BUS_NAME,
            id: "reprise-printers",
            title: TITLE,
            notify_icon: ICON,
            raise: Raise::MenuRow(Rows::id(&TrayMsg::Folder)),
        },
        first,
        std::sync::Arc::new(move || waker.wake()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cups::{Job, Printer};

    fn snap() -> Snapshot {
        let printer = |n: &str| Printer {
            name: n.into(),
            ..Default::default()
        };
        let job = |id, p: &str, owner: &str, state| Job {
            id,
            printer: p.into(),
            name: format!("doc{id}"),
            owner: owner.into(),
            state,
            ..Default::default()
        };
        Snapshot {
            running: true,
            printers: vec![printer("Inkjet"), printer("Laser"), printer("Office")],
            default: None,
            jobs: vec![
                job(1, "Laser", "ann", JobState::Printing),
                job(2, "Office", "bob", JobState::Pending),
                job(3, "Inkjet", "ann", JobState::Pending),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn the_icon_is_up_for_the_users_own_documents() {
        let s = snap();
        assert_eq!(busy_printers(&s, "ann"), ["Inkjet", "Laser"]);
        let sh = shown(&s, "ann");
        assert!(sh.active);
        let labels: Vec<&str> = sh.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, ["Inkjet", "Laser", "", TITLE]);
        assert!(sh.nodes[0].default);
        assert_eq!(sh.tip, "2 documents pending for ann");
        assert!(!shown(&s, "carol").active, "nothing of carol's waits");
        assert_eq!(tip(&s, "bob"), "1 document pending for bob");
        let mut one = s.clone();
        one.jobs.retain(|j| j.id == 1);
        assert_eq!(tip(&one, "ann"), "Printing doc1");
    }

    #[test]
    fn an_aborted_document_cups_keeps_shows_the_error_icon() {
        let mut s = snap();
        s.aborted = vec![Job {
            id: 7,
            printer: "Office".into(),
            name: "doc7".into(),
            owner: "carol".into(),
            state: JobState::Aborted,
            preserved: true,
            ..Default::default()
        }];
        assert_eq!(busy_printers(&s, "carol"), ["Office"]);
        let sh = shown(&s, "carol");
        assert!(sh.active);
        assert_eq!(sh.icon, Icon::Name(ERROR_ICON.into()));
        assert_eq!(sh.tip, "doc7 could not be printed");
        assert_eq!(shown(&s, "ann").icon, Icon::Name(ICON.into()));
        s.aborted.push(Job {
            id: 8,
            ..s.aborted[0].clone()
        });
        assert_eq!(tip(&s, "carol"), "2 documents could not be printed");
    }

    #[test]
    fn clicks_map_to_rows() {
        let s = snap();
        let translate = |msg| translate(msg, &s, "ann");
        assert_eq!(translate(Msg::Activate(0, 0)), Some(TrayMsg::Open));
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::Folder)),
            Some(TrayMsg::Folder)
        );
        assert_eq!(
            translate(Rows::clicked(&TrayMsg::Queue(1))),
            Some(TrayMsg::Queue(1))
        );
        assert_eq!(translate(Msg::BalloonClicked(4)), Some(TrayMsg::Balloon(4)));
        assert_eq!(translate(Msg::Scroll(1)), None);
    }
}
