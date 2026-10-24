use crate::access::{
    self, Tree,
    accesskit::{Action, NodeId, Role},
};
use crate::controls::combo::{self, Combo};
use crate::raster::Rect;
use crate::text::TextRenderer;

use super::layout::Layout;
use super::{ALL_ICONS, Focus, TrayCustomize, Wording};

impl TrayCustomize {
    /// Names the combo just pushed under `list` for entry `i` of `total`, gives it the
    /// entry's place, which a reader's scroll-in counts by ([`Published::reveal`]), and makes
    /// the entry's row `row` control it, so a reader's focus on the row lands on the combo
    /// ([`Published::route`]).
    ///
    /// [`Published::reveal`]: crate::access::Published::reveal
    /// [`Published::route`]: crate::access::Published::route
    fn name_combo(tree: &mut Tree, list: NodeId, row: NodeId, name: &str, i: usize, total: usize) {
        let Some(combo) = tree.get(list).and_then(|l| l.children().last().copied()) else {
            return;
        };
        if let Some(n) = tree.get_mut(combo) {
            n.set_label(name);
            n.set_position_in_set(i + 1);
            n.set_size_of_set(total);
        }
        if let Some(n) = tree.get_mut(row) {
            n.set_controls([combo]);
        }
    }

    /// The text, the list with its rows (each named by the item and
    /// described by its behaviour, which AT-SPI reads where it drops a row's value), the combo,
    /// the box and the buttons; the focused part gets the focus.
    pub fn access(&self, tree: &mut Tree, lay: &Layout, text: &TextRenderer, px: u32) -> NodeId {
        let root = access::ROOT;
        let w = self.wording;
        let live = self.list_enabled();
        tree.push(root, "intro", access::label(lay.intro, w.intro(), true));
        let prompt = if self.is_page() {
            Wording::HEADING
        } else {
            w.prompt()
        };
        tree.push(root, "prompt", access::label(lay.prompt, prompt, true));
        let mut ln = access::node(Role::ListBox, lay.body);
        access::focusable(&mut ln, live);
        let list = tree.push(root, "list", ln);
        let mut focus_row = list;
        let mut row_ids = Vec::with_capacity(self.entries.len());
        let rows = self.row_rects(lay);
        for (i, e) in self.entries.iter().enumerate() {
            let mut n = access::node(Role::ListBoxOption, Rect::default());
            if let Some(r) = rows.get(i).copied().flatten() {
                n.set_bounds(access::row_bounds(lay.body, r));
            }
            n.set_label(e.name.as_str());
            n.set_description(w.label(w.choices()[w.index(e.behaviour, e.passive)]));
            n.set_selected(self.selected == Some(i));
            let id = tree.push(list, ("row", i), n);
            row_ids.push(id);
            if self.is_page()
                && let Some(n) = tree.get_mut(id)
            {
                // A page row is picked through its combo; it offers no action of its own.
                n.remove_action(Action::Focus);
                n.remove_action(Action::Click);
            }
            if self.selected == Some(i) {
                focus_row = id;
            }
        }
        let mut combo_focus = None;
        if let (Some((i, cl)), true) = (&lay.combo, live) {
            combo_focus = Some(combo::access(
                &self.combo,
                cl,
                text,
                px,
                tree,
                list,
                "combo",
            ));
            if self.is_page() {
                let name = &self.entries[*i].name;
                Self::name_combo(tree, list, row_ids[*i], name, *i, self.entries.len());
            }
        }
        if self.is_page() && live {
            let labels: Vec<&str> = w.choices().iter().map(|&b| w.label(b)).collect();
            for &(i, field) in &lay.fields {
                if lay.combo.as_ref().is_some_and(|(j, _)| *j == i) {
                    continue;
                }
                let e = &self.entries[i];
                let row = Combo::new(&labels, w.index(e.behaviour, e.passive), labels.len());
                let cl = combo::layout(&row, field, &self.metrics);
                combo::access(&row, &cl, text, px, tree, list, ("combo", i));
                Self::name_combo(tree, list, row_ids[i], &e.name, i, self.entries.len());
            }
        }
        let all = match (lay.check, lay.check_label, self.all_icons) {
            (Some(b), Some(l), Some(on)) => Some(tree.push(
                root,
                "all",
                access::toggle(Role::CheckBox, b.union(&l), ALL_ICONS, on.into(), true),
            )),
            _ => None,
        };
        let restore = tree.push(
            root,
            "restore",
            access::button(lay.restore, w.restore(), live, false),
        );
        let ok = tree.push(root, "ok", access::button(lay.ok, "OK", true, true));
        let cancel = tree.push(
            root,
            "cancel",
            access::button(lay.cancel, "Cancel", true, false),
        );
        let focus = match self.focus {
            Focus::List => focus_row,
            Focus::Combo => combo_focus.unwrap_or(list),
            Focus::AllIcons => all.unwrap_or(ok),
            Focus::Restore => restore,
            Focus::Ok => ok,
            Focus::Cancel => cancel,
        };
        tree.set_focus(focus);
        focus
    }
}
