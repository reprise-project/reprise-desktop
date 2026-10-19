//! AccessKit trees: a control's `access` part adds nodes for its rect to a [`Tree`]; the host
//! publishes one tree per surface through [`Published`], which sends only what changed.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

pub use accesskit;
use accesskit::{Action, Node, NodeId, Role, TextPosition, TextSelection, TreeId, TreeInfo};

use crate::raster::Rect;
use crate::text::TextRenderer;

pub const ROOT: NodeId = NodeId(1);

#[derive(Clone, Debug)]
pub struct Tree {
    nodes: Vec<(NodeId, Node)>,
    index: HashMap<NodeId, usize>,
    focus: NodeId,
}

impl Tree {
    pub fn new(root: Node) -> Tree {
        Tree {
            nodes: vec![(ROOT, root)],
            index: HashMap::from([(ROOT, 0)]),
            focus: ROOT,
        }
    }

    pub fn window(title: &str, size: (i32, i32)) -> Tree {
        let mut root = node(Role::Window, Rect::new(0, 0, size.0, size.1));
        root.set_label(title);
        Tree::new(root)
    }

    pub fn id_of(parent: NodeId, key: impl Hash) -> NodeId {
        let mut h = DefaultHasher::new();
        parent.0.hash(&mut h);
        key.hash(&mut h);
        NodeId(h.finish())
    }

    /// Add `node` as the last child of `parent` and return its id. A
    /// second child under the same key gets an id of its own derived from
    /// the first, so ids stay unique even when a caller repeats a key.
    /// Panics if `parent` is not in the tree.
    pub fn push(&mut self, parent: NodeId, key: impl Hash, node: Node) -> NodeId {
        let mut id = Tree::id_of(parent, key);
        while id == ROOT || self.index.contains_key(&id) {
            id = Tree::id_of(id, 0u8);
        }
        let p = self.index[&parent];
        self.nodes[p].1.push_child(id);
        self.index.insert(id, self.nodes.len());
        self.nodes.push((id, node));
        id
    }

    pub fn set_focus(&mut self, id: NodeId) {
        self.focus = id;
    }

    pub fn focus(&self) -> NodeId {
        self.focus
    }

    /// Where the focused node lies in the space the root is placed in:
    /// its bounds through every transform from the root down to it, a
    /// row's cut to the part in its list, or the list's when the row is
    /// wholly out of it. None when it
    /// has no bounds or hangs nowhere under the root. The magnifier pans
    /// to it.
    pub fn focus_bounds(&self) -> Option<accesskit::Rect> {
        fn down(
            t: &Tree,
            id: NodeId,
            want: NodeId,
            at: accesskit::Affine,
            list: Option<accesskit::Rect>,
        ) -> Option<accesskit::Rect> {
            let n = t.get(id)?;
            let at = n.transform().map_or(at, |a| at * *a);
            let b = n.bounds().map(|b| at.transform_rect_bbox(b));
            if id == want {
                return match (b, list) {
                    (Some(b), Some(f)) => Some(in_view(b, f)),
                    _ => b,
                };
            }
            let list = if is_list(n.role()) { b } else { list };
            n.children()
                .iter()
                .find_map(|&c| down(t, c, want, at, list))
        }
        down(self, ROOT, self.focus, accesskit::Affine::IDENTITY, None)
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.index.get(&id).map(|&i| &self.nodes[i].1)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.index.get(&id).map(|&i| &mut self.nodes[i].1)
    }

    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().map(|(id, n)| (*id, n))
    }

    pub fn find(&self, role: Role, label: &str) -> Option<NodeId> {
        self.nodes()
            .find(|(_, n)| n.role() == role && n.label() == Some(label))
            .map(|(id, _)| id)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Published {
    nodes: HashMap<NodeId, Node>,
    parents: HashMap<NodeId, NodeId>,
    live: bool,
}

impl Published {
    /// The update that brings an adapter holding what was published last
    /// to `tree`: the new and changed nodes (a parent whose children
    /// changed is one of them, which is how a removed node leaves), and
    /// always the focus.
    pub fn update(&mut self, tree: Tree) -> accesskit::TreeUpdate {
        let first = !self.live;
        self.live = true;
        let focus = tree.focus;
        let mut out = Vec::new();
        let mut next = HashMap::with_capacity(tree.nodes.len());
        self.parents.clear();
        for (id, n) in tree.nodes {
            if first || self.nodes.get(&id) != Some(&n) {
                out.push((id, n.clone()));
            }
            self.parents.extend(n.children().iter().map(|&c| (c, id)));
            next.insert(id, n);
        }
        self.nodes = next;
        accesskit::TreeUpdate {
            nodes: out,
            tree: first.then(|| TreeInfo {
                root: ROOT,
                toolkit_name: Some("reprisekit".into()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    /// Node `id` as last published, aimed at the middle of its part inside its list ([`in_view`])
    /// and placed by the root's transform. Only a root carries a transform here; the nodes under
    /// it lay out in its space. None for a node not in the last tree, without bounds or scrolled
    /// out of its list.
    pub fn target(&self, id: NodeId) -> Option<Target> {
        let n = self.nodes.get(&id)?;
        if self.scrolled_out(id).is_some() {
            return None;
        }
        let b = n.bounds()?;
        let list = self.list_of(id).and_then(|l| self.nodes.get(&l)?.bounds());
        Some(Target {
            role: n.role(),
            at: self.place(list.map_or(b, |f| in_view(b, f))),
        })
    }

    /// What a reader's ScrollIntoView on node `id` scrolls: a row lying outside its list's
    /// bounds ([`Published::scrolled_out`]), counted by its position in set when it has one (a
    /// list publishing only the rows near its view), else among the list's rows; in a scroll
    /// view, the row is its child holding the node. None for a row in view, or a node that is
    /// not a row of a list.
    pub fn reveal(&self, id: NodeId) -> Option<Reveal> {
        let list = self.scrolled_out(id)?;
        let node = self.nodes.get(&id)?;
        if let Some(pos) = node.position_in_set() {
            return Some(Reveal {
                at: self.middle(list)?,
                row: pos.checked_sub(1)?,
            });
        }
        let children = self.nodes.get(&list)?.children();
        if self.nodes.get(&list)?.role() == Role::ScrollView {
            let mut child = id;
            while self.parents.get(&child)? != &list {
                child = *self.parents.get(&child)?;
            }
            return Some(Reveal {
                at: self.middle(list)?,
                row: children.iter().position(|&c| c == child)?,
            });
        }
        let role = node.role();
        let mut row = 0;
        let mut stack: Vec<NodeId> = children.iter().rev().copied().collect();
        while let Some(at) = stack.pop() {
            if at == id {
                return Some(Reveal {
                    at: self.middle(list)?,
                    row,
                });
            }
            let Some(n) = self.nodes.get(&at) else {
                continue;
            };
            if n.role() == role {
                row += 1;
            }
            stack.extend(n.children().iter().rev());
        }
        None
    }

    /// The list `id` is a row of, when the row lies out of the list's bounds ([`out_of`]).
    fn scrolled_out(&self, id: NodeId) -> Option<NodeId> {
        let b = self.nodes.get(&id)?.bounds()?;
        let list = self.list_of(id)?;
        out_of(b, self.nodes.get(&list)?.bounds()?).then_some(list)
    }

    /// The nearest list above node `id`.
    fn list_of(&self, id: NodeId) -> Option<NodeId> {
        let mut up = *self.parents.get(&id)?;
        loop {
            if is_list(self.nodes.get(&up)?.role()) {
                return Some(up);
            }
            up = *self.parents.get(&up)?;
        }
    }

    /// The middle of node `id`'s bounds, placed by the root's transform.
    fn middle(&self, id: NodeId) -> Option<(f64, f64)> {
        Some(self.place(self.nodes.get(&id)?.bounds()?))
    }

    /// The middle of `b`, placed by the root's transform.
    fn place(&self, b: accesskit::Rect) -> (f64, f64) {
        let centre = accesskit::Point::new((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
        let root = self.nodes.get(&ROOT).and_then(|r| r.transform().copied());
        let p = root.map_or(centre, |t| t * centre);
        (p.x, p.y)
    }

    pub fn reset(&mut self) {
        self.nodes.clear();
        self.parents.clear();
        self.live = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg<K> {
    Activate(K),
    Deactivate(K),
    Act(K, Request),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub node: NodeId,
    pub act: Act,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Click,
    Focus,
    ContextMenu,
    ScrollIntoView,
}

impl Act {
    fn of(a: Action) -> Option<Act> {
        Some(match a {
            Action::Click => Act::Click,
            Action::Focus => Act::Focus,
            Action::ShowContextMenu => Act::ContextMenu,
            Action::ScrollIntoView => Act::ScrollIntoView,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub role: Role,
    pub at: (f64, f64),
}

/// A list to scroll so its row `row` (counted in the order the list's rows are published, a
/// tree's in its visible order, a scroll view's by its children) shows whole, without picking
/// it; `at` is the list's middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reveal {
    pub at: (f64, f64),
    pub row: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Click((f64, f64)),
    RightClick((f64, f64)),
    Hover((f64, f64)),
    /// Give the control at the point the keyboard, as Tab would, without acting on it.
    Focus((f64, f64)),
}

/// How a host carries out `act` on `target`: a click for Click, a right
/// click for ContextMenu. Focus is a click where a click does no more
/// than Tab or the arrows would (a field, a list row, a tab) and a hover
/// on a menu row; on anything else (a button, a check box) a click would
/// act, so the host moves the keyboard there instead ([`Gesture::Focus`]).
/// ScrollIntoView is no gesture: the host scrolls the list
/// [`Published::reveal`] names.
pub fn gesture(act: Act, target: Target) -> Option<Gesture> {
    let at = target.at;
    match act {
        Act::Click => Some(Gesture::Click(at)),
        Act::ContextMenu => Some(Gesture::RightClick(at)),
        Act::Focus => match target.role {
            Role::TextInput
            | Role::MultilineTextInput
            | Role::PasswordInput
            | Role::SearchInput
            | Role::EditableComboBox
            | Role::ListItem
            | Role::ListBoxOption
            | Role::TreeItem
            | Role::Row
            | Role::Cell
            | Role::Tab => Some(Gesture::Click(at)),
            Role::MenuItem | Role::MenuItemCheckBox | Role::MenuItemRadio => {
                Some(Gesture::Hover(at))
            }
            _ => Some(Gesture::Focus(at)),
        },
        Act::ScrollIntoView => None,
    }
}

pub struct Handler<K, P> {
    key: K,
    post: P,
}

impl<K: Copy, P: Fn(Msg<K>)> Handler<K, P> {
    pub fn new(key: K, post: P) -> Handler<K, P> {
        Handler { key, post }
    }
}

impl<K: Copy, P: Fn(Msg<K>)> accesskit::ActivationHandler for Handler<K, P> {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        (self.post)(Msg::Activate(self.key));
        None
    }
}

impl<K: Copy, P: Fn(Msg<K>)> accesskit::DeactivationHandler for Handler<K, P> {
    fn deactivate_accessibility(&mut self) {
        (self.post)(Msg::Deactivate(self.key));
    }
}

impl<K: Copy, P: Fn(Msg<K>)> accesskit::ActionHandler for Handler<K, P> {
    fn do_action(&mut self, req: accesskit::ActionRequest) {
        if let Some(act) = Act::of(req.action) {
            (self.post)(Msg::Act(
                self.key,
                Request {
                    node: req.target_node,
                    act,
                },
            ));
        }
    }
}

/// A role whose rows scroll inside it.
fn is_list(role: Role) -> bool {
    matches!(
        role,
        Role::ListBox
            | Role::Tree
            | Role::List
            | Role::ListGrid
            | Role::TreeGrid
            | Role::ScrollView
    )
}

/// A row with bounds `b` lies out of its list's bounds `f`: partly above or below them, or
/// wholly beside them (a row cut by a side edge, as a deep tree row is, stays in).
fn out_of(b: accesskit::Rect, f: accesskit::Rect) -> bool {
    b.y0 < f.y0 || b.y1 > f.y1 || b.x1 <= f.x0 || b.x0 >= f.x1
}

/// The part of a row with bounds `b` inside its list's bounds `f`, or `f` when none of it is.
fn in_view(b: accesskit::Rect, f: accesskit::Rect) -> accesskit::Rect {
    let v = b.intersect(f);
    if v.width() > 0.0 && v.height() > 0.0 {
        v
    } else {
        f
    }
}

pub fn bounds(r: Rect) -> accesskit::Rect {
    accesskit::Rect {
        x0: r.x.into(),
        y0: r.y.into(),
        x1: r.right().into(),
        y1: r.bottom().into(),
    }
}

/// The bounds of `row` in a list whose rows show in `view`: the row itself while any of it
/// shows, else a row of its size just past the edge it went out by. Every row out on one side
/// shares that place, so a scroll changes only the rows crossing the edge, and
/// [`Published::reveal`] still finds each outside the list.
pub fn row_bounds(view: Rect, row: Rect) -> accesskit::Rect {
    let r = if row.bottom() <= view.y {
        Rect::new(row.x, view.y - row.height, row.width, row.height)
    } else if row.y >= view.bottom() {
        Rect::new(row.x, view.bottom(), row.width, row.height)
    } else if row.right() <= view.x {
        Rect::new(view.x - row.width, row.y, row.width, row.height)
    } else if row.x >= view.right() {
        Rect::new(view.right(), row.y, row.width, row.height)
    } else {
        row
    };
    bounds(r)
}

pub fn node(role: Role, r: Rect) -> Node {
    let mut n = Node::new(role);
    n.set_bounds(bounds(r));
    n
}

pub fn named(role: Role, r: Rect, label: &str) -> Node {
    titled(role, r, &crate::text::split_mnemonic(label).0)
}

/// [`named`] for a label drawn as it is, with no access key: an `&` in it is read.
pub fn titled(role: Role, r: Rect, label: &str) -> Node {
    let mut n = node(role, r);
    if !label.is_empty() {
        n.set_label(label);
    }
    n
}

/// A control the keyboard can reach: offering `Focus` is what makes a
/// node focusable to an assistive tool, and `Click` gives it the AT-SPI
/// action a reader runs (the host clicks it, see [`gesture`]). A greyed
/// one is marked disabled instead.
pub fn focusable(n: &mut Node, enabled: bool) {
    if enabled {
        n.add_action(Action::Focus);
        n.add_action(Action::Click);
    } else {
        n.set_disabled();
    }
}

pub fn button(r: Rect, label: &str, enabled: bool, default: bool) -> Node {
    let role = if default {
        Role::DefaultButton
    } else {
        Role::Button
    };
    let mut n = named(role, r, label);
    focusable(&mut n, enabled);
    n
}

pub fn toggle(role: Role, r: Rect, label: &str, state: accesskit::Toggled, enabled: bool) -> Node {
    let mut n = named(role, r, label);
    n.set_toggled(state);
    focusable(&mut n, enabled);
    n
}

pub fn label(r: Rect, text: &str, enabled: bool) -> Node {
    let mut n = node(Role::Label, r);
    n.set_value(crate::text::split_mnemonic(text).0);
    if !enabled {
        n.set_disabled();
    }
    n
}

pub fn range(role: Role, r: Rect, value: f64, min: f64, max: f64, enabled: bool) -> Node {
    let mut n = node(role, r);
    n.set_numeric_value(value);
    n.set_min_numeric_value(min);
    n.set_max_numeric_value(max);
    n.set_numeric_value_step(1.0);
    focusable(&mut n, enabled);
    n
}

/// One line of an edit's text as drawn: `text` starts at byte `start` of
/// the value and is drawn with its left edge at `rect.x`. `hard_break`
/// says a newline ends it in the value; the run then carries that `\n` as
/// its last character, as AccessKit wants.
#[derive(Clone, Copy, Debug)]
pub struct Line<'a> {
    pub text: &'a str,
    pub start: usize,
    pub rect: Rect,
    pub hard_break: bool,
}

pub fn text_runs(
    tree: &mut Tree,
    parent: NodeId,
    lines: &[Line],
    text: &TextRenderer,
    px: u32,
    anchor: usize,
    caret: usize,
) {
    let mut ids = Vec::with_capacity(lines.len());
    let mut prev: Option<NodeId> = None;
    for (k, line) in lines.iter().enumerate() {
        let mut n = node(Role::TextRun, line.rect);
        let mut value = String::with_capacity(line.text.len() + 1);
        value.push_str(line.text);
        let mut lengths = Vec::with_capacity(line.text.len() + 1);
        let mut positions = Vec::with_capacity(line.text.len() + 1);
        let mut widths = Vec::with_capacity(line.text.len() + 1);
        let mut words = Vec::new();
        let mut x = 0i32;
        let mut prev_space = true;
        for (i, ch) in line.text.chars().enumerate() {
            let w = text.advance(ch, false, px);
            lengths.push(ch.len_utf8() as u8);
            positions.push(x as f32);
            widths.push(w as f32);
            let space = ch.is_whitespace();
            if prev_space && !space {
                words.push(i as u8);
            }
            prev_space = space;
            x += w;
        }
        if line.hard_break {
            value.push('\n');
            lengths.push(1);
            positions.push(x as f32);
            widths.push(0.0);
        }
        n.set_value(value);
        n.set_character_lengths(lengths);
        n.set_character_positions(positions);
        n.set_character_widths(widths);
        // Word starts are u8 character indices; a run too long for them
        // leaves them out, and a reader then walks it by character.
        if line.text.chars().count() <= usize::from(u8::MAX) {
            n.set_word_starts(words);
        }
        if let Some(p) = prev {
            n.set_previous_on_line(p);
        }
        let id = tree.push(parent, k, n);
        if let Some(p) = prev
            && !lines[k - 1].hard_break
            && let Some(pn) = tree.get_mut(p)
        {
            pn.set_next_on_line(id);
        }
        prev = (!line.hard_break).then_some(id);
        ids.push(id);
    }
    if ids.is_empty() {
        let mut n = node(Role::TextRun, Rect::default());
        n.set_value("");
        n.set_character_lengths(Vec::<u8>::new());
        ids.push(tree.push(parent, 0usize, n));
    }
    let at = |b: usize| position(lines, &ids, b);
    let selection = TextSelection {
        anchor: at(anchor),
        focus: at(caret),
    };
    if let Some(p) = tree.get_mut(parent) {
        p.set_text_selection(selection);
    }
}

pub fn value_run(
    tree: &mut Tree,
    id: NodeId,
    r: Rect,
    value: &str,
    selected: std::ops::Range<usize>,
    text: &TextRenderer,
    px: u32,
) {
    if let Some(n) = tree.get_mut(id) {
        n.set_read_only();
    }
    let line = Line {
        text: value,
        start: 0,
        rect: r,
        hard_break: false,
    };
    text_runs(tree, id, &[line], text, px, selected.start, selected.end);
}

/// The run and character a byte offset of the value falls on. The end of
/// a line is its line break when it has one; at a soft wrap it is the
/// start of the next line. An offset past every line is the end of the
/// last.
fn position(lines: &[Line], ids: &[NodeId], b: usize) -> TextPosition {
    for (k, line) in lines.iter().enumerate() {
        let end = line.start + line.text.len();
        let last = k + 1 == lines.len();
        if b >= line.start && (b < end || (b == end && (line.hard_break || last))) {
            let within = floor_boundary(line.text, b - line.start);
            return TextPosition {
                node: ids[k],
                character_index: line.text[..within].chars().count(),
            };
        }
    }
    TextPosition {
        node: *ids.last().expect("at least one run"),
        character_index: lines.last().map_or(0, |l| l.text.chars().count()),
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_focus_bounds_go_through_every_transform_above_the_node() {
        let mut root = node(Role::Window, Rect::new(0, 0, 400, 300));
        root.set_transform(accesskit::Affine::translate((100.0, 50.0)));
        let mut t = Tree::new(root);
        let mut pane = node(Role::Pane, Rect::new(10, 10, 200, 100));
        pane.set_transform(accesskit::Affine::translate((10.0, 10.0)));
        let pane = t.push(ROOT, "pane", pane);
        let field = t.push(
            pane,
            "field",
            node(Role::TextInput, Rect::new(5, 6, 80, 20)),
        );
        let b = t.focus_bounds().unwrap();
        assert_eq!(
            (b.x0, b.y0, b.x1, b.y1),
            (100.0, 50.0, 500.0, 350.0),
            "the root"
        );
        t.set_focus(field);
        let b = t.focus_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (115.0, 66.0, 195.0, 86.0));
        t.set_focus(NodeId(999));
        assert_eq!(t.focus_bounds(), None, "a focus not in the tree");
    }

    #[test]
    fn a_focused_row_scrolled_out_of_its_list_shows_as_the_list() {
        let mut root = node(Role::Window, Rect::new(0, 0, 400, 300));
        root.set_transform(accesskit::Affine::translate((100.0, 50.0)));
        let mut t = Tree::new(root);
        let list = t.push(
            ROOT,
            "list",
            node(Role::ListBox, Rect::new(10, 10, 100, 40)),
        );
        let shown = t.push(
            list,
            0,
            node(Role::ListBoxOption, Rect::new(10, 10, 100, 10)),
        );
        let gone = t.push(
            list,
            1,
            node(Role::ListBoxOption, Rect::new(10, 50, 100, 10)),
        );
        t.set_focus(shown);
        let b = t.focus_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (110.0, 60.0, 210.0, 70.0));
        t.set_focus(gone);
        let b = t.focus_bounds().unwrap();
        assert_eq!(
            (b.x0, b.y0, b.x1, b.y1),
            (110.0, 60.0, 210.0, 100.0),
            "the list"
        );
    }

    #[test]
    fn a_focused_row_cut_by_its_list_edge_shows_as_its_part_in_the_list() {
        let mut t = Tree::new(node(Role::Window, Rect::new(0, 0, 400, 300)));
        let list = t.push(
            ROOT,
            "list",
            node(Role::ListBox, Rect::new(10, 10, 100, 40)),
        );
        let cut = t.push(
            list,
            0,
            node(Role::ListBoxOption, Rect::new(10, 44, 100, 10)),
        );
        let above = t.push(
            list,
            1,
            node(Role::ListBoxOption, Rect::new(10, 4, 100, 10)),
        );
        t.set_focus(cut);
        let b = t.focus_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 44.0, 110.0, 50.0));
        t.set_focus(above);
        let b = t.focus_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 10.0, 110.0, 14.0));
    }

    #[test]
    fn the_handler_posts_activation_and_answers_with_no_tree() {
        use accesskit::{ActivationHandler, DeactivationHandler};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut h = Handler::new(7u8, move |m| tx.send(m).unwrap());
        assert!(
            h.request_initial_tree().is_none(),
            "the tree is built on the host's loop, not the adapter's thread"
        );
        h.deactivate_accessibility();
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [Msg::Activate(7), Msg::Deactivate(7)]
        );
    }

    #[test]
    fn the_handler_posts_an_action_on_its_node_and_drops_the_rest() {
        use accesskit::{ActionHandler, ActionRequest, TreeId};
        let (tx, rx) = std::sync::mpsc::channel();
        let mut h = Handler::new(7u8, move |m| tx.send(m).unwrap());
        let req = |action| ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: NodeId(42),
            data: None,
        };
        h.do_action(req(Action::Click));
        h.do_action(req(Action::ShowContextMenu));
        h.do_action(req(Action::Expand));
        let on = |act| {
            Msg::Act(
                7,
                Request {
                    node: NodeId(42),
                    act,
                },
            )
        };
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            [on(Act::Click), on(Act::ContextMenu)],
            "Expand has no Act and is dropped"
        );
    }

    #[test]
    fn a_target_is_its_nodes_centre_placed_by_the_root() {
        let mut root = node(Role::Window, Rect::new(0, 0, 800, 30));
        root.set_transform(accesskit::Affine::translate((0.0, 570.0)));
        let mut t = Tree::new(root);
        let ok = t.push(
            ROOT,
            "ok",
            button(Rect::new(10, 4, 20, 22), "OK", true, false),
        );
        let mut p = Published::default();
        p.update(t);
        assert_eq!(
            p.target(ok),
            Some(Target {
                role: Role::Button,
                at: (20.0, 585.0)
            })
        );
        assert_eq!(p.target(NodeId(9)), None, "a node not published");
    }

    #[test]
    fn a_cell_cut_by_its_lists_side_edge_is_aimed_at_its_part_in_the_list() {
        let mut root = node(Role::Window, Rect::new(0, 0, 200, 100));
        root.set_transform(accesskit::Affine::translate((100.0, 50.0)));
        let mut t = Tree::new(root);
        let list = t.push(
            ROOT,
            "list",
            node(Role::ListBox, Rect::new(10, 10, 100, 40)),
        );
        let row = t.push(
            list,
            "row",
            node(Role::ListBoxOption, Rect::new(12, 12, 96, 10)),
        );
        let cut = t.push(
            row,
            "cut",
            node(Role::ListBoxOption, Rect::new(90, 12, 60, 10)),
        );
        let mut p = Published::default();
        p.update(t);
        assert_eq!(
            p.target(cut).map(|t| t.at),
            Some((200.0, 67.0)),
            "the middle of x 90..110, not of the whole cell's 90..150"
        );
        assert_eq!(p.target(row).map(|t| t.at), Some((160.0, 67.0)));
    }

    #[test]
    fn a_tree_row_out_of_view_is_revealed_by_its_place_in_the_visible_order() {
        let mut root = node(Role::Window, Rect::new(0, 0, 200, 100));
        root.set_transform(accesskit::Affine::translate((100.0, 50.0)));
        let mut t = Tree::new(root);
        let tree = t.push(ROOT, "tree", node(Role::Tree, Rect::new(10, 10, 100, 40)));
        let row = |y| node(Role::TreeItem, Rect::new(12, y, 90, 10));
        let a = t.push(tree, "a", row(12));
        t.push(a, "edit", node(Role::TextInput, Rect::new(14, 12, 40, 10)));
        let b = t.push(a, "b", row(22));
        t.push(tree, "c", row(32));
        let d = t.push(tree, "d", row(52));
        t.push(d, "e", row(62));
        let mut p = Published::default();
        p.update(t);
        assert_eq!(p.reveal(b), None, "a row in view");
        assert_eq!(
            p.reveal(d),
            Some(Reveal {
                at: (160.0, 80.0),
                row: 3
            }),
            "the fourth row in tree order, other roles not counted"
        );
        assert_eq!(p.target(d), None);
        assert_eq!(p.reveal(tree), None, "the tree itself is no row");
    }

    #[test]
    fn a_node_out_of_a_scroll_view_is_revealed_by_the_child_holding_it() {
        let mut t = Tree::new(node(Role::Window, Rect::new(0, 0, 200, 100)));
        let view = t.push(
            ROOT,
            "view",
            node(Role::ScrollView, Rect::new(0, 0, 100, 80)),
        );
        let group = |x| node(Role::Group, Rect::new(x, 0, 50, 80));
        let slider = |x| node(Role::Slider, Rect::new(x + 10, 10, 20, 60));
        let a = t.push(view, "a", group(0));
        let shown = t.push(a, "slider", slider(0));
        let b = t.push(view, "b", group(50));
        t.push(b, "slider", slider(50));
        let c = t.push(view, "c", group(100));
        let beyond = t.push(c, "slider", slider(100));
        let mut p = Published::default();
        p.update(t);
        assert_eq!(p.reveal(shown), None, "a slider in view");
        assert_eq!(
            p.reveal(beyond),
            Some(Reveal {
                at: (50.0, 40.0),
                row: 2
            }),
            "the third column, whatever the slider's role"
        );
        assert_eq!(p.reveal(c).map(|r| r.row), Some(2));
        assert_eq!(p.target(beyond), None);
    }

    #[test]
    fn a_cell_wholly_beside_a_list_or_cut_by_its_bottom_is_revealed() {
        let mut t = Tree::new(node(Role::Window, Rect::new(0, 0, 200, 100)));
        let list = t.push(
            ROOT,
            "list",
            node(Role::ListBox, Rect::new(10, 10, 100, 40)),
        );
        let cell = |x| node(Role::ListBoxOption, Rect::new(x, 12, 40, 10));
        let shown = t.push(list, 0, cell(12));
        let cut = t.push(list, 1, cell(90));
        let left = t.push(list, 2, cell(-40));
        let low = t.push(
            list,
            3,
            node(Role::ListBoxOption, Rect::new(12, 45, 40, 10)),
        );
        let mut p = Published::default();
        p.update(t);
        assert_eq!(p.reveal(shown), None);
        assert_eq!(p.reveal(cut), None, "a cell cut by a side edge");
        assert_eq!(
            p.reveal(left).map(|r| r.row),
            Some(2),
            "a cell left of the list"
        );
        assert_eq!(
            p.reveal(low).map(|r| r.row),
            Some(3),
            "a row cut by the bottom edge"
        );
    }

    #[test]
    fn rows_out_of_view_share_a_place_past_the_edge_they_left_by() {
        let view = Rect::new(10, 10, 100, 40);
        let at = |x, y| row_bounds(view, Rect::new(x, y, 50, 10));
        assert_eq!(at(10, -100), at(10, 0), "every row above shares one place");
        assert_eq!(at(10, 0), bounds(Rect::new(10, 0, 50, 10)));
        assert_eq!(
            at(10, 5),
            bounds(Rect::new(10, 5, 50, 10)),
            "a cut row keeps its own"
        );
        assert_eq!(at(10, 300), bounds(Rect::new(10, 50, 50, 10)));
        assert_eq!(at(-300, 20), bounds(Rect::new(-40, 20, 50, 10)));
        assert_eq!(at(300, 20), bounds(Rect::new(110, 20, 50, 10)));
    }

    #[test]
    fn a_readers_focus_clicks_only_what_a_click_just_picks() {
        let at = (5.0, 6.0);
        let on = |role| Target { role, at };
        assert_eq!(
            gesture(Act::Click, on(Role::Button)),
            Some(Gesture::Click(at))
        );
        assert_eq!(
            gesture(Act::ContextMenu, on(Role::Button)),
            Some(Gesture::RightClick(at))
        );
        assert_eq!(
            gesture(Act::Focus, on(Role::Button)),
            Some(Gesture::Focus(at))
        );
        assert_eq!(
            gesture(Act::Focus, on(Role::CheckBox)),
            Some(Gesture::Focus(at))
        );
        assert_eq!(
            gesture(Act::Focus, on(Role::TextInput)),
            Some(Gesture::Click(at))
        );
        assert_eq!(
            gesture(Act::Focus, on(Role::MenuItem)),
            Some(Gesture::Hover(at))
        );
        assert_eq!(gesture(Act::ScrollIntoView, on(Role::ListItem)), None);
    }

    #[test]
    fn a_pushed_node_keeps_its_id_from_one_tree_to_the_next() {
        let build = || {
            let mut t = Tree::window("Title", (100, 50));
            let a = t.push(ROOT, "ok", button(Rect::new(1, 2, 3, 4), "OK", true, true));
            (t, a)
        };
        let (_, a) = build();
        let (t, b) = build();
        assert_eq!(a, b);
        assert_eq!(t.get(ROOT).unwrap().children(), &[a]);
    }

    #[test]
    fn a_repeated_key_still_gets_a_unique_id() {
        let mut t = Tree::window("T", (1, 1));
        let a = t.push(ROOT, "x", Node::new(Role::Label));
        let b = t.push(ROOT, "x", Node::new(Role::Label));
        assert_ne!(a, b);
        assert_eq!(t.get(ROOT).unwrap().children(), &[a, b]);
    }

    #[test]
    fn a_button_is_named_without_its_access_key_marker() {
        let n = button(Rect::new(0, 0, 75, 23), "&Apply", true, false);
        assert_eq!(n.label(), Some("Apply"));
        assert_eq!(n.role(), Role::Button);
        assert!(n.supports_action(Action::Focus));
        assert!(n.supports_action(Action::Click), "a reader can press it");
        let n = button(Rect::new(0, 0, 75, 23), "&Apply", false, false);
        assert!(n.is_disabled());
        assert!(!n.supports_action(Action::Focus));
        assert!(!n.supports_action(Action::Click));
    }

    #[test]
    fn the_second_update_carries_only_what_changed() {
        let tree = |label: &str| {
            let mut t = Tree::window("T", (10, 10));
            t.push(ROOT, "a", named(Role::Button, Rect::new(0, 0, 5, 5), "A"));
            t.push(ROOT, "b", named(Role::Button, Rect::new(5, 0, 5, 5), label));
            t
        };
        let mut p = Published::default();
        let first = p.update(tree("B"));
        assert_eq!(first.nodes.len(), 3);
        assert!(first.tree.is_some());
        let second = p.update(tree("B2"));
        assert_eq!(second.nodes.len(), 1);
        assert_eq!(second.nodes[0].1.label(), Some("B2"));
        assert!(second.tree.is_none());
        p.reset();
        assert_eq!(p.update(tree("B2")).nodes.len(), 3);
    }

    #[test]
    fn a_caret_maps_onto_its_run_and_the_line_break() {
        let value = "ab\ncd";
        let lines = [
            Line {
                text: "ab",
                start: 0,
                rect: Rect::new(0, 0, 20, 13),
                hard_break: true,
            },
            Line {
                text: "cd",
                start: 3,
                rect: Rect::new(0, 13, 20, 13),
                hard_break: false,
            },
        ];
        assert_eq!(value.len(), 5);
        let ids = [NodeId(10), NodeId(11)];
        assert_eq!(position(&lines, &ids, 2).node, NodeId(10));
        assert_eq!(position(&lines, &ids, 2).character_index, 2);
        assert_eq!(position(&lines, &ids, 3).node, NodeId(11));
        assert_eq!(position(&lines, &ids, 3).character_index, 0);
        assert_eq!(position(&lines, &ids, 5).character_index, 2);
    }

    #[test]
    fn runs_carry_their_characters_and_the_selection() {
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let mut t = Tree::window("T", (100, 100));
        let edit = t.push(
            ROOT,
            "e",
            node(Role::MultilineTextInput, Rect::new(0, 0, 100, 100)),
        );
        let lines = [
            Line {
                text: "hi",
                start: 0,
                rect: Rect::new(2, 2, 90, 13),
                hard_break: true,
            },
            Line {
                text: "",
                start: 3,
                rect: Rect::new(2, 15, 90, 13),
                hard_break: false,
            },
        ];
        text_runs(&mut t, edit, &lines, &text, 11, 0, 3);
        let e = t.get(edit).unwrap();
        let runs = e.children().to_vec();
        assert_eq!(runs.len(), 2);
        let first = t.get(runs[0]).unwrap();
        assert_eq!(first.value(), Some("hi\n"));
        assert_eq!(first.character_lengths(), &[1, 1, 1]);
        let sel = e.text_selection().unwrap();
        assert_eq!(sel.anchor.node, runs[0]);
        assert_eq!(sel.focus.node, runs[1]);
        assert_eq!(sel.focus.character_index, 0);
    }
}
