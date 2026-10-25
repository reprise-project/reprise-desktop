use super::{ClickKind, TrayCommand, send};
use crate::compositor::State;
use crate::menuview::{MenuFrom, MenuKind};
use crate::raster::Rect;
use crate::startmenu::{MenuItem, MenuStyle, place_submenu};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    pub id: i32,
    pub label: String,
    pub separator: bool,
    pub enabled: bool,
    pub visible: bool,
    pub checked: Option<bool>,
    pub radio: bool,
    pub has_children: bool,
    pub default: bool,
}

pub const DEFAULT_PROP: &str = "x-reprise-default";

/// Remove DBusMenu mnemonic markers: a single `_` is dropped, `__` becomes `_`.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Map DBusMenu nodes to menu items, skipping invisible ones. The second
/// vector runs parallel to the first and carries the node id to send back
/// on click (`None` for separators).
pub fn menu_items(nodes: &[MenuNode]) -> (Vec<MenuItem>, Vec<Option<i32>>) {
    let mut items = Vec::with_capacity(nodes.len());
    let mut ids = Vec::with_capacity(nodes.len());
    for n in nodes.iter().filter(|n| n.visible) {
        if n.separator {
            items.push(MenuItem::Separator);
            ids.push(None);
        } else {
            items.push(
                MenuItem::entry(strip_mnemonic(&n.label))
                    .enabled(n.enabled)
                    .checked(n.checked.unwrap_or(false))
                    .radio(n.radio)
                    .submenu(n.has_children)
                    .bold(n.default),
            );
            ids.push(Some(n.id));
        }
    }
    (items, ids)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmenuRequest {
    pub id: String,
    pub node: i32,
    pub depth: usize,
    pub row: usize,
}

/// Tray submenu bookkeeping: the request in flight, and the parent row every
/// open submenu hangs off (`open_rows[d]` is the row of the menu at depth `d`
/// that opened the menu at `d + 1`). Pure because the cases are races: a reply
/// landing after the menu was dismissed or the pointer moved on.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Submenus {
    pub pending: Option<SubmenuRequest>,
    open_rows: Vec<usize>,
}

impl Submenus {
    pub fn needs_request(&self, id: &str, depth: usize, row: usize) -> bool {
        if self.open_rows.get(depth) == Some(&row) {
            return false;
        }
        !matches!(&self.pending, Some(p) if p.id == id && p.depth == depth && p.row == row)
    }

    pub fn request(&mut self, req: SubmenuRequest) {
        self.pending = Some(req);
    }

    /// Match a `MenuLayout` reply against the request in flight and consume
    /// it: `Some((depth, row))` for the reply we asked for. A reply for
    /// another item, another node (the pointer moved on) or a menu that has
    /// since closed is dropped.
    pub fn accept(&mut self, id: &str, parent: i32) -> Option<(usize, usize)> {
        let p = self.pending.as_ref()?;
        if p.id != id || p.node != parent {
            return None;
        }
        let at = (p.depth, p.row);
        self.pending = None;
        Some(at)
    }

    pub fn opened(&mut self, depth: usize, row: usize) {
        self.open_rows.truncate(depth);
        // Never reached in practice (submenus open depth by depth); the
        // filler cannot collide with a real row.
        self.open_rows.resize(depth, usize::MAX);
        self.open_rows.push(row);
    }

    pub fn closed_above(&mut self, depth: usize) {
        self.open_rows.truncate(depth);
        if self.pending.as_ref().is_some_and(|p| p.depth >= depth) {
            self.pending = None;
        }
    }

    pub fn clear(&mut self) {
        self.pending = None;
        self.open_rows.clear();
    }

    pub fn forget_item(&mut self, id: &str) {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.pending = None;
        }
    }
}

pub(super) fn open_layout(state: &mut State, id: String, parent: i32, nodes: Vec<MenuNode>) {
    let (items, node_ids) = menu_items(&nodes);
    let pending_root = state.tray.as_mut().and_then(|t| {
        t.pending_menu
            .take_if(|(pid, ..)| *pid == id && parent == 0)
    });
    if let Some((_, output, icon, (x, y))) = pending_root {
        if !state.scene.outputs().contains(&output) || state.layout_box(Some(&output)).width <= 0 {
            return;
        }
        // The D-Bus side answers an unreadable menu with no rows: the
        // item gets `ContextMenu`, as one without a menu does. No menu
        // follows, so the bar the request held may hide again.
        if items.is_empty() {
            let kind = ClickKind::ContextMenu;
            let token = None;
            send(
                state,
                TrayCommand::Click {
                    id,
                    x,
                    y,
                    kind,
                    token,
                },
            );
            crate::autohideview::poke(state);
            return;
        }
        let usable = state.usable_area(Some(&output));
        let layout = MenuStyle::context_for(&state.config.theme, &items).layout_in(
            &state.config.theme,
            &state.text,
            &items,
            usable,
        );
        let (x, y) = crate::barplace::bar_popup(
            &state.bar_placement(&output),
            icon,
            (layout.width, layout.height),
            usable,
        );
        state.close_menu();
        state.push_menu(
            0,
            (x, y),
            layout,
            items,
            output,
            MenuKind::Tray { id, node_ids },
            MenuFrom::Bar,
        );
        state.redraw_menu(0);
        return;
    }
    let Some((depth, row)) = state
        .tray
        .as_mut()
        .and_then(|t| t.submenus.accept(&id, parent))
    else {
        return;
    };
    let Some(parent_menu) = state.menus.get(depth) else {
        return;
    };
    if !matches!(&parent_menu.kind, MenuKind::Tray { id: pid, .. } if *pid == id) {
        return;
    }
    let parent_rect = Rect::new(
        parent_menu.x,
        parent_menu.y,
        parent_menu.layout.width,
        parent_menu.layout.height,
    );
    let Some(&local_row) = parent_menu.layout.rows.get(row) else {
        return;
    };
    let row_rect = Rect::new(
        parent_menu.x + local_row.x,
        parent_menu.y + local_row.y,
        local_row.width,
        local_row.height,
    );
    let output = parent_menu.output.clone();
    let usable = state.usable_area(Some(&output));
    let layout = MenuStyle::context_for(&state.config.theme, &items).layout_in(
        &state.config.theme,
        &state.text,
        &items,
        usable,
    );
    let (x, y) = place_submenu(parent_rect, row_rect, (layout.width, layout.height), usable);
    state.close_menus_above(depth);
    state.push_menu(
        depth + 1,
        (x, y),
        layout,
        items,
        output,
        MenuKind::Tray { id, node_ids },
        MenuFrom::Bar,
    );
    if let Some(m) = state.menus.last_mut() {
        m.parent_row = Some(row);
    }
    if let Some(tray) = state.tray.as_mut() {
        tray.submenus.opened(depth, row);
    }
    state.redraw_menu(depth + 1);
}

/// Whether the open root menu is item `id`'s.
pub(super) fn menu_open_for(state: &State, id: &str) -> bool {
    matches!(
        state.menus.first().map(|m| &m.kind),
        Some(MenuKind::Tray { id: open, .. }) if *open == id
    )
}

pub fn menu_hover(state: &mut State, depth: usize, idx: Option<usize>) {
    let Some(menu) = state.menus.get(depth) else {
        return;
    };
    let MenuKind::Tray { id, node_ids } = &menu.kind else {
        return;
    };
    let Some(i) = idx else { return };
    let is_submenu = matches!(
        menu.items.get(i),
        Some(MenuItem::Entry {
            submenu: true,
            enabled: true,
            ..
        })
    );
    if !is_submenu {
        state.close_menus_above(depth);
        return;
    }
    let (id, node) = (id.clone(), node_ids.get(i).copied().flatten());
    let Some(node) = node else { return };
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    if !tray.submenus.needs_request(&id, depth, i) {
        return;
    }
    tray.submenus.request(SubmenuRequest {
        id: id.clone(),
        node,
        depth,
        row: i,
    });
    tray.send(TrayCommand::GetLayout { id, parent: node });
}

pub fn forget_output(state: &mut State) {
    let outputs = state.scene.outputs().to_vec();
    if let Some(tray) = state.tray.as_mut()
        && tray
            .pending_menu
            .as_ref()
            .is_some_and(|(_, o, ..)| !outputs.contains(o))
    {
        tray.pending_menu = None;
    }
}

/// Every menu closed: nothing in flight can still be wanted. A dropped
/// root request no longer holds its bar, so the bars hear of it.
pub fn menus_closed(state: &mut State) {
    let Some(tray) = state.tray.as_mut() else {
        return;
    };
    let dropped = tray.pending_menu.take().is_some();
    tray.submenus.clear();
    if dropped {
        crate::autohideview::poke(state);
    }
}

pub fn menus_closed_above(state: &mut State, depth: usize) {
    if let Some(tray) = state.tray.as_mut() {
        tray.submenus.closed_above(depth);
    }
}

pub fn menu_activate(state: &mut State, depth: usize, idx: usize) {
    let Some(menu) = state.menus.get(depth) else {
        return;
    };
    let MenuKind::Tray { id, node_ids } = &menu.kind else {
        return;
    };
    let enabled = matches!(
        menu.items.get(idx),
        Some(MenuItem::Entry {
            enabled: true,
            submenu: false,
            ..
        })
    );
    let node = node_ids.get(idx).copied().flatten();
    let id = id.clone();
    if let (true, Some(item)) = (enabled, node) {
        state.close_menu();
        let token = state.new_activation_token();
        send(state, TrayCommand::MenuEvent { id, item, token });
    }
}
