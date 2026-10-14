//! The Start panel's `access` part: a menu filling the panel with an item per row, the search field,
//! and the Shut down button and its arrow. Results and a jump list hang in groups named by their headers.

use super::layout::Layout;
use super::{
    ALL_PROGRAMS, BACK, CLEAR, Mode, SEARCH_CUE, SHUT_DOWN, SHUT_DOWN_OPTIONS, Spot, StartPanel,
    TreeRow, USER_ACCOUNTS,
};
use crate::access::accesskit::{HasPopup, Node, NodeId, Role};
use crate::access::{self, Tree};
use crate::raster::Rect;

/// Add the panel's nodes under `parent`, in `l`'s coordinates (pass the unscaled layout for logical
/// bounds). Returns the focused node: the spot the keyboard walked to, else the keyboard's stop; the pointer's spot never takes it.
pub fn access(p: &StartPanel, l: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let menu = tree.push(parent, "start", access::node(Role::Menu, l.panel));
    let groups: Vec<(usize, NodeId)> = l
        .headers
        .iter()
        .map(|&(g, r)| {
            let title = match p.mode() {
                Mode::Jump(_) => p.jump_title.as_str(),
                _ => p.results.get(g).map_or("", |grp| grp.title.as_str()),
            };
            let bounds = l
                .spots
                .iter()
                .filter(|(s, _)| in_group(*s, g))
                .fold(r, |b, (_, sr)| b.union(sr));
            let node = access::named(Role::Group, bounds, title);
            (g, tree.push(menu, ("group", g), node))
        })
        .collect();
    let focus = p.walked().unwrap_or(p.focus());
    let mut focused = menu;
    for &(s, r) in &l.spots {
        let Some(n) = spot_node(p, s, r) else {
            continue;
        };
        let under = groups
            .iter()
            .find(|&&(g, _)| in_group(s, g))
            .map_or(menu, |&(_, id)| id);
        let id = tree.push(under, s, n);
        if s == focus {
            focused = id;
        }
    }
    focused
}

/// Whether `s` is a row of the group headed by header `g`: a result of group `g`, or a jump row (the jump list's one header).
fn in_group(s: Spot, g: usize) -> bool {
    match s {
        Spot::Result(sg, _) => sg == g,
        Spot::Jump(_) => true,
        _ => false,
    }
}

fn item(r: Rect, label: &str) -> Node {
    let mut n = access::node(Role::MenuItem, r);
    n.set_label(label);
    access::focusable(&mut n, true);
    n
}

/// The node for spot `s` at `r`.
fn spot_node(p: &StartPanel, s: Spot, r: Rect) -> Option<Node> {
    let n = match s {
        Spot::Program(i) => {
            let row = p.program(i)?;
            let mut n = item(r, &row.name);
            if row.jump {
                n.set_has_popup(HasPopup::Menu);
                n.set_expanded(p.mode() == Mode::Jump(i));
            }
            n
        }
        Spot::AllPrograms => item(
            r,
            if p.mode() == Mode::Programs {
                BACK
            } else {
                ALL_PROGRAMS
            },
        ),
        Spot::Tree(i) => match p.tree.get(i)? {
            TreeRow::Folder { name, open, .. } => {
                let mut n = item(r, name);
                n.set_expanded(*open);
                n
            }
            TreeRow::App { name, .. } => item(r, name),
        },
        Spot::Result(g, i) => item(r, &p.results.get(g)?.rows.get(i)?.name),
        Spot::Jump(k) => item(r, &p.jump.get(k)?.name),
        Spot::Place(i) => {
            let place = p.places.get(i)?;
            let mut n = item(r, &place.label);
            if place.submenu {
                n.set_has_popup(HasPopup::Menu);
            }
            n
        }
        Spot::Picture => access::button(r, USER_ACCOUNTS, true, false),
        Spot::Search => {
            let mut n = access::node(Role::SearchInput, r);
            n.set_value(p.query());
            n.set_placeholder(SEARCH_CUE);
            access::focusable(&mut n, true);
            n
        }
        Spot::Clear => access::button(r, CLEAR, true, false),
        Spot::Shutdown => access::button(r, SHUT_DOWN, true, false),
        Spot::ShutdownArrow => {
            let mut n = access::button(r, SHUT_DOWN_OPTIONS, true, false);
            n.set_has_popup(HasPopup::Menu);
            n
        }
    };
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::accesskit::Action;
    use crate::startpanel::layout::layout;
    use crate::startpanel::tests::{place, prow, row};
    use crate::startpanel::{Key, ResultGroup};
    use crate::textedit::EditKey;

    fn build(p: &StartPanel) -> (Tree, NodeId) {
        let mut t = Tree::window("", (600, 600));
        let f = access(p, &layout(p), &mut t, access::ROOT);
        (t, f)
    }

    #[test]
    fn home_lists_the_rows_the_field_and_shut_down_with_the_field_focused() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true)];
        p.places = vec![place("Documents")];
        let (t, focus) = build(&p);
        let n = t.get(focus).unwrap();
        assert_eq!(n.role(), Role::SearchInput);
        assert_eq!(n.placeholder(), Some(SEARCH_CUE));
        let notepad = t.find(Role::MenuItem, "Notepad").expect("the program row");
        assert_eq!(t.get(notepad).unwrap().has_popup(), Some(HasPopup::Menu));
        assert!(t.find(Role::MenuItem, ALL_PROGRAMS).is_some());
        assert!(t.find(Role::MenuItem, "Documents").is_some());
        assert!(t.find(Role::Button, SHUT_DOWN).is_some());
        assert!(t.find(Role::Button, SHUT_DOWN_OPTIONS).is_some());
        let picture = t
            .find(Role::Button, USER_ACCOUNTS)
            .expect("the user picture");
        assert!(t.get(picture).unwrap().supports_action(Action::Click));
        p.key(Key::Down);
        let (_, focus) = build(&p);
        assert_eq!(focus, notepad, "the lit row has the focus");
    }

    #[test]
    fn the_pointer_lights_a_row_but_leaves_the_focus_where_the_keyboard_is() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", false)];
        p.places = vec![place("Documents")];
        p.pointer(Some(Spot::Place(0)));
        let (t, focus) = build(&p);
        assert_eq!(t.get(focus).unwrap().role(), Role::SearchInput);
        p.pointer(None);
        p.key(Key::Down);
        p.pointer(Some(Spot::Place(0)));
        let (t, focus) = build(&p);
        assert_eq!(Some(focus), t.find(Role::MenuItem, "Notepad"));
    }

    #[test]
    fn results_hang_in_groups_named_by_their_headers() {
        let mut p = StartPanel::new();
        p.key(Key::Char(EditKey::Insert("no".into())));
        p.set_results(vec![ResultGroup {
            title: "Programs (1)".into(),
            rows: vec![row("Notepad")],
        }]);
        let (t, focus) = build(&p);
        let group = t.find(Role::Group, "Programs (1)").expect("the group");
        let notepad = t.find(Role::MenuItem, "Notepad").unwrap();
        assert_eq!(t.get(group).unwrap().children(), [notepad]);
        assert_eq!(focus, notepad, "the first result is lit");
        let field = t
            .nodes()
            .find(|(_, n)| n.role() == Role::SearchInput)
            .unwrap()
            .1;
        assert_eq!(field.value(), Some("no"));
        assert!(t.find(Role::Button, CLEAR).is_some());
    }
}
