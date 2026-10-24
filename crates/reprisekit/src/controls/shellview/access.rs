use super::details::DetailsRows;
use super::rename::rename_rect;
use super::{Layout, Mode, ShellView};
use crate::access::{self, Tree, accesskit};
use crate::classic::widgets;
use crate::controls::listview;
use crate::raster::Rect;
use crate::text::TextRenderer;
use accesskit::{NodeId, Role};

/// The part of `lay.body` the cells show in: below the header in Details.
fn cells_area(sv: &ShellView, lay: &Layout) -> Rect {
    let b = lay.body;
    match sv.mode {
        Mode::Details => {
            let head = lay.controls.list_header;
            Rect::new(b.x, b.y + head, b.width, (b.height - head).max(0))
        }
        _ => b,
    }
}

/// The shell view's `access` part, the same in every mode: a list with one
/// option per entry, named by its name and described by its Details columns
/// ("Size: 2 KB, Type: Text Document"), the selected ones marked. The list's
/// bounds are the cells' area, so an entry scrolled out lies outside them,
/// where a reader can ask to scroll it in ([`reveal`]). A rename in progress is a text
/// input. Returns the node that has the focus: the rename field, else the focused entry, else the list.
pub fn access(
    sv: &ShellView,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let area = cells_area(sv, lay);
    let mut n = access::node(Role::ListBox, area);
    n.set_multiselectable();
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let rows = DetailsRows(&sv.entries, &sv.fields);
    let mut focus = id;
    for (i, e) in sv.entries.iter().enumerate() {
        let mut o = accesskit::Node::new(Role::ListBoxOption);
        o.set_label(e.name.as_str());
        let rest: Vec<String> = sv
            .columns
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(k, c)| {
                let v = listview::Rows::cell(&rows, i, k);
                (!v.is_empty()).then(|| format!("{}: {v}", c.title))
            })
            .collect();
        if !rest.is_empty() {
            o.set_description(rest.join(", "));
        }
        o.set_selected(sv.selection.is(i));
        if let Some(cell) = lay.cells.get(i) {
            o.set_bounds(access::row_bounds(area, *cell));
        }
        let oid = tree.push(id, e.id, o);
        if sv.selection.focus == Some(i) {
            focus = oid;
        }
        if let Some((k, edit)) = &sv.edit
            && *k == i
        {
            let r = rename_rect(sv, lay, i, text, px).unwrap_or_default();
            focus = widgets::access_edit(edit, 0, false, r, true, text, px, tree, oid, "rename");
        }
    }
    focus
}
