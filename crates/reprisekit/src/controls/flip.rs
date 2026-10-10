//! The 3D switcher's `access` part; the model, layout and hit test are the root [`crate::flip`], re-exported here.

use crate::access::{self, Tree, accesskit};
use crate::raster::Rect;
use accesskit::{Node, NodeId, Role};

pub use crate::flip::*;

/// The stack's `access` part, in the coordinates `cards` were laid out in: every entry by title, the `selected` one selected; entries without a drawn card have no bounds. Returns the selected node, which has focus while the stack is up.
pub fn access<S: AsRef<str>>(
    titles: &[S],
    selected: usize,
    cards: &[Card],
    area: Rect,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let list = tree.push(parent, "entries", access::node(Role::ListBox, area));
    let mut focus = list;
    for (i, title) in titles.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(title.as_ref());
        if let Some(c) = cards.iter().find(|c| c.entry == i) {
            o.set_bounds(access::bounds(c.bounds));
        }
        o.set_selected(i == selected);
        let id = tree.push(list, i, o);
        if i == selected {
            focus = id;
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_entry_has_the_focus_and_undrawn_ones_no_bounds() {
        let area = Rect::new(0, 0, 1000, 800);
        let g = Geometry {
            shown: 2,
            ..GEOMETRY
        };
        let cards = layout(area, &[(400, 300); 4], 0.0, &g);
        let titles = ["One", "Two", "Three", "Four"];
        let mut t = Tree::window("", (area.width, area.height));
        let focus = access(&titles, 0, &cards, area, &mut t, access::ROOT);
        let n = t.get(focus).unwrap();
        assert_eq!(n.label(), Some("One"));
        assert_eq!(n.is_selected(), Some(true));
        assert_eq!(
            n.bounds(),
            Some(access::bounds(cards[1].bounds)),
            "One is the front card"
        );
        let bounds = |title| {
            t.get(t.find(Role::ListBoxOption, title).unwrap())
                .unwrap()
                .bounds()
        };
        assert!(bounds("Two").is_some(), "Two is drawn behind");
        assert_eq!(bounds("Four"), None, "Four is past the shown cards");
    }
}
