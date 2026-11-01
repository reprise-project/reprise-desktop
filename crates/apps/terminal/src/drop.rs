//! A drag dropped on a terminal window types into its open tab, as a paste: files as their
//! shell-quoted paths and web links as they are, each followed by a space; text as it came.

use repriseapp::{Accept, DragAction, DropKind, Dropped, Event, Offered, Reply, Uri};
use reprisekit::textedit::paths_line;

/// What a drag offering `o` is read as: a file manager's or browser's URI list keeps its files
/// and links apart; anything else with text is its text.
fn kind_for(o: Offered) -> Option<DropKind> {
    if o.files || o.link {
        Some(DropKind::Uris)
    } else if o.text {
        Some(DropKind::Text)
    } else {
        None
    }
}

/// The answer to a drag entering the window.
pub(crate) fn enter(ev: &Event) -> Option<Accept> {
    let Event::DragEnter {
        offered, source, ..
    } = ev
    else {
        return None;
    };
    kind_for(*offered).map(|k| repriseapp::fielddrop::accept_as(k, *source))
}

/// The text a drop of `data` types.
pub(crate) fn typed(data: &Dropped) -> String {
    let words = |items: Vec<String>| items.into_iter().map(|w| w + " ").collect();
    match data {
        Dropped::Text(t) => t.clone(),
        Dropped::Files(paths) => words(paths.iter().map(|p| paths_line(&[p])).collect()),
        Dropped::Uris(uris) => words(
            uris.iter()
                .map(|u| match u {
                    Uri::File(p) => paths_line(&[p]),
                    Uri::Url { url, .. } => url.clone(),
                })
                .collect(),
        ),
    }
}

/// The reply that closes a drop: an `Ask` is answered as a copy.
pub(crate) fn answered(mut reply: Reply, action: DragAction) -> Reply {
    if action == DragAction::Ask {
        reply.answer_drop = Some(Some(DragAction::Copy));
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriseapp::Actions;
    use std::path::PathBuf;

    #[test]
    fn files_type_as_quoted_paths_each_followed_by_a_space() {
        let files = Dropped::Files(vec![
            PathBuf::from("/home/u/notes.txt"),
            PathBuf::from("/home/u/My Files"),
            PathBuf::from("/home/u/it's"),
        ]);
        assert_eq!(
            typed(&files),
            "/home/u/notes.txt '/home/u/My Files' '/home/u/it'\\''s' "
        );
    }

    #[test]
    fn a_uri_list_types_its_files_as_paths_and_its_links_as_they_are() {
        let list = "# a comment\r\nfile:///home/u/a%20b.txt\r\nfile://localhost/home/u/c\r\n\
                    file://elsewhere/home/u/d\r\nhttps://example.com/?q=1&r=2\r\n";
        let uris =
            repriseapp::dnd::decode_drop(list.as_bytes(), DropKind::Uris, repriseapp::URI_LIST);
        assert_eq!(
            typed(&uris),
            "'/home/u/a b.txt' /home/u/c https://example.com/?q=1&r=2 ",
            "another host's file is not here and is left out"
        );
    }

    #[test]
    fn text_types_as_it_came() {
        assert_eq!(typed(&Dropped::Text("echo hi".into())), "echo hi");
    }

    #[test]
    fn files_and_links_are_read_as_a_uri_list_and_text_as_text() {
        let offered = |files, link, text| Offered { files, link, text };
        assert_eq!(kind_for(offered(true, false, true)), Some(DropKind::Uris));
        assert_eq!(kind_for(offered(false, true, true)), Some(DropKind::Uris));
        assert_eq!(kind_for(offered(false, false, true)), Some(DropKind::Text));
        assert_eq!(kind_for(offered(false, false, false)), None);
        let ev = Event::DragEnter {
            x: 0,
            y: 0,
            offered: offered(true, false, false),
            source: Actions {
                copy: true,
                move_: true,
                ask: false,
            },
            action: None,
            own: false,
        };
        let a = enter(&ev).unwrap();
        assert!(a.actions.copy && !a.actions.move_, "files are never moved");
    }
}
