use crate::access::{self, Tree, accesskit};
use crate::classic::widgets;
use crate::controls::arrange;
use crate::controls::calendar;
use crate::controls::colorbutton;
use crate::controls::combo;
use crate::controls::hotkey;
use crate::controls::iconlist;
use crate::controls::listbox;
use crate::controls::multiedit;
use crate::controls::timefield;
use crate::controls::treeview;
use crate::controls::updown;
use crate::raster::Rect;
use crate::text::TextRenderer;
use crate::trackbar;
use accesskit::{NodeId, Role};

use super::field::{masked, visible_from};
use super::layout::{
    check_box, check_label_rect, glyph_dy, icon_list_layout, radio_box, radio_label_rect,
};
use super::{Control, Form, Memo};

impl Form {
    /// The form's `access` part: one node per control under `parent`, each inside the group box
    /// whose rect holds it. A field without a label is labelled by the static text before it.
    /// Decoration is left out. Returns the node that has the focus.
    pub fn access(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        text: &TextRenderer,
        px: u32,
    ) -> Option<NodeId> {
        let mut groups: Vec<(Rect, NodeId)> = Vec::new();
        let mut label: Option<NodeId> = None;
        let mut focus = None;
        let cm = self.rendered.controls.get();
        let text = text.dialog(self.font);
        for (i, it) in self.items.iter().enumerate() {
            let r = self.at(it);
            let within = |g: &Rect| {
                g.contains(r.x, r.y) && r.right() <= g.right() && r.bottom() <= g.bottom()
            };
            let up = groups
                .iter()
                .rev()
                .find(|(g, _)| within(g))
                .map_or(parent, |(_, id)| *id);
            let key = (it.id, i);
            let (target, bare) = match &it.control {
                Control::Label {
                    text: t, enabled, ..
                } => {
                    label = Some(tree.push(up, key, access::label(r, t, *enabled)));
                    continue;
                }
                Control::Lines(lines) | Control::GreyedLines(lines) => {
                    let live = matches!(it.control, Control::Lines(_));
                    label = Some(tree.push(up, key, access::label(r, &lines.join(" "), live)));
                    continue;
                }
                Control::GroupTitle(t) => {
                    label = Some(tree.push(up, key, access::label(r, t, true)));
                    continue;
                }
                Control::Group(title) => {
                    let id = tree.push(up, key, access::named(Role::Group, r, title));
                    groups.push((r, id));
                    label = None;
                    continue;
                }
                Control::Separator
                | Control::Rule
                | Control::Blank
                | Control::Picture { .. }
                | Control::Monitor(_)
                | Control::Spectrum
                | Control::Sample { .. }
                | Control::Clock(_) => continue,
                Control::Check {
                    label: l,
                    checked,
                    enabled,
                    mixed,
                } => {
                    let lw = text.measure(&crate::text::split_mnemonic(l).0, false, px);
                    let cm = self.rendered.controls.get();
                    let b = check_box(r, &cm, glyph_dy(it, r)).union(&check_label_rect(
                        r,
                        it.indent,
                        &cm,
                        lw + 2,
                    ));
                    let state = match (mixed, checked) {
                        (Some(true), _) => accesskit::Toggled::Mixed,
                        (_, c) => (*c).into(),
                    };
                    let n = access::toggle(Role::CheckBox, b, l, state, *enabled);
                    (tree.push(up, key, n), None)
                }
                Control::Radio {
                    label: l,
                    checked,
                    enabled,
                    ..
                } => {
                    let lw = text.measure(&crate::text::split_mnemonic(l).0, false, px);
                    let cm = self.rendered.controls.get();
                    let b =
                        radio_box(r, &cm, glyph_dy(it, r)).union(&radio_label_rect(r, &cm, lw + 2));
                    let n = access::toggle(Role::RadioButton, b, l, (*checked).into(), *enabled);
                    (tree.push(up, key, n), None)
                }
                Control::Button {
                    label: l,
                    enabled,
                    default,
                } => {
                    let n = access::button(r, l, *enabled, *default);
                    (tree.push(up, key, n), None)
                }
                Control::IconList(l) => {
                    let lay = icon_list_layout(it.grows, r, &cm);
                    let id = iconlist::access(l, &lay, tree, up, key);
                    (id, Some(id))
                }
                Control::Arrange(a) => {
                    let lay = arrange::layout(r, a);
                    let id = arrange::access(a, &lay, tree, up, key);
                    (id, Some(id))
                }
                Control::Memo(memo) => {
                    let m = Memo::metrics(px, &cm, self.units());
                    let lay = multiedit::layout(text, &m, &memo.edit, r, 1);
                    let id = multiedit::access(
                        &memo.edit,
                        self.memo_view_1x(memo),
                        &lay,
                        &m,
                        text,
                        tree,
                        up,
                        key,
                    );
                    (id, Some(id))
                }
                Control::Edit {
                    edit,
                    first,
                    enabled,
                } => {
                    // A masked field reads out its bullets, never the value.
                    let shown = if edit.is_secret() {
                        masked(edit)
                    } else {
                        edit.clone()
                    };
                    let from = visible_from(&shown, first);
                    let id = widgets::access_edit(
                        &shown,
                        from,
                        edit.is_secret(),
                        r,
                        *enabled,
                        text,
                        px,
                        tree,
                        up,
                        key,
                    );
                    (id, Some(id))
                }
                Control::Combo(c) => {
                    let lay = combo::layout(c, r, &self.rendered.controls.get());
                    let target = combo::access(c, &lay, text, px, tree, up, key);
                    (target, Some(Tree::id_of(up, key)))
                }
                Control::ListBox { list, .. } => {
                    let lay = listbox::layout(list, r, &self.rendered.controls.get());
                    let target = listbox::access(list, &lay, tree, up, key);
                    (target, Some(Tree::id_of(up, key)))
                }
                Control::Tree(tv) => {
                    let lay = treeview::layout_without_text(tv, r, &self.rendered.controls.get());
                    let target = treeview::access(tv, &lay, text, px, tree, up, key);
                    (target, Some(Tree::id_of(up, key)))
                }
                Control::UpDown(u) => {
                    let cm = self.rendered.controls.get();
                    let id = updown::access(
                        u,
                        &updown::layout_for(u, r, &cm, self.units()),
                        tree,
                        up,
                        key,
                    );
                    (id, Some(id))
                }
                Control::TrackBar(t) => {
                    let lay = trackbar::layout(t, r.x, r.y, r.width);
                    let id = trackbar::access(t, &lay, tree, up, key);
                    (id, Some(id))
                }
                Control::Color(b) => {
                    let id = colorbutton::access(b, r, tree, up, key);
                    (id, Some(id))
                }
                Control::Calendar(cal) => {
                    let lay = calendar::layout(r, &self.rendered.controls.get());
                    let id = calendar::access(cal, &lay, text, px, tree, up, key);
                    (id, Some(id))
                }
                Control::Time(tf) => {
                    let cm = self.rendered.controls.get();
                    let lay = timefield::layout(r, &cm);
                    let id = timefield::access(tf, &lay, text, px, tree, up, key);
                    (id, Some(id))
                }
                Control::HotKey(h) => {
                    let focused = self.focus == Some(i);
                    let id = hotkey::access(h, hotkey::layout(r), focused, text, px, tree, up, key);
                    (id, Some(id))
                }
            };
            if let Some(n) = bare.and_then(|b| tree.get_mut(b)) {
                match (it.name, label) {
                    (Some(name), _) => n.set_label(name),
                    (None, Some(l)) => n.push_labelled_by(l),
                    (None, None) => {}
                }
            }
            label = None;
            if self.focus == Some(i) {
                focus = Some(target);
            }
        }
        focus
    }
}
