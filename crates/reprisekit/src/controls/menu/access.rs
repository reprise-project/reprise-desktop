use super::MenuItem;
use super::layout::MenuLayout;
use super::popup::PopupMenu;
use crate::access::{self, Tree, accesskit};
use crate::raster::Rect;
use accesskit::{Node, NodeId, Role};

pub fn access_row(item: &MenuItem, r: Rect, open: bool) -> Option<Node> {
    let MenuItem::Entry {
        label,
        enabled,
        checked,
        radio,
        submenu,
        shortcut,
        ..
    } = item
    else {
        return None;
    };
    let role = match (radio, checked) {
        (true, _) => Role::MenuItemRadio,
        (false, true) => Role::MenuItemCheckBox,
        (false, false) => Role::MenuItem,
    };
    let mut n = access::node(role, r);
    n.set_label(label.as_str());
    if role != Role::MenuItem {
        n.set_toggled((*checked).into());
    }
    if *submenu {
        n.set_has_popup(accesskit::HasPopup::Menu);
        n.set_expanded(open);
    }
    if let Some(s) = shortcut {
        n.set_keyboard_shortcut(s.as_str());
    }
    access::focusable(&mut n, *enabled);
    Some(n)
}

/// Returns the menu's id, the id each row got (the menu's own for a separator, so a submenu hung there has a parent), and the highlighted row's id.
#[allow(clippy::too_many_arguments)]
pub fn access_menu(
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
    items: &[MenuItem],
    layout: &MenuLayout,
    rect: Rect,
    hovered: Option<usize>,
    open: Option<usize>,
) -> (NodeId, Vec<NodeId>, Option<NodeId>) {
    let menu = tree.push(parent, key, access::node(Role::Menu, rect));
    let mut rows = Vec::with_capacity(items.len());
    let mut focus = None;
    for (i, item) in items.iter().enumerate() {
        let r = layout.rows.get(i).copied().unwrap_or_default();
        let r = Rect::new(r.x + rect.x, r.y + rect.y, r.width, r.height);
        let Some(n) = access_row(item, r, open == Some(i)) else {
            rows.push(menu);
            continue;
        };
        let id = tree.push(menu, i, n);
        rows.push(id);
        if hovered == Some(i) {
            focus = Some(id);
        }
    }
    (menu, rows, focus)
}

impl PopupMenu {
    pub fn access(&self, tree: &mut Tree, parent: NodeId, key: impl std::hash::Hash) -> NodeId {
        let mut up = parent;
        let mut focus = parent;
        let mut rows: Vec<NodeId> = Vec::new();
        for (k, level) in self.levels.iter().enumerate() {
            if k > 0
                && let Some(&row) = level.path.last().and_then(|&i| rows.get(i))
            {
                up = row;
            }
            let next = self.levels.get(k + 1).and_then(|l| l.path.last().copied());
            let (menu, ids, hot) = access_menu(
                tree,
                up,
                (&key, k),
                &level.items,
                &level.layout,
                level.rect(),
                level.hovered,
                next,
            );
            focus = hot.unwrap_or(menu);
            rows = ids;
        }
        focus
    }
}
