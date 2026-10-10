//! The tree view, Explorer's Folders pane: 17 px rows indented 19 px a level,
//! each an expander box (a 9 px square with + or -), a 16 px icon and the
//! label, the selected label on the highlight colour with the dotted focus
//! rectangle, scrollbars when the rows or labels outgrow the box, inside a
//! 2 px sunken client edge. The host builds the nodes and expands them
//! (lazily, for a file system) when asked.
//!
//! `Style::Classic` keeps dotted connector lines and gives roots their own
//! expander column. `Style::Explorer` draws no connector lines, no box on a
//! root and no edge of its own: it is flat white inside the Explorer bar's
//! frame. A root's icon sits 4 px in, each level indents its icon 19 px
//! further with the expander box 15 px left of it, and a label starts 21 px
//! right of its icon's left edge. `Style::Picker`, the Move/Copy picker's
//! tree, is `Style::Explorer` a px further left and inside the sunken edge:
//! its root's icon sits 3 px in. In every style the highlight band runs from
//! 2 px before the label to 5 px past its text, and the label's capitals start
//! 5 px below the row's top.
//!
//! A label renames in place in a box the row's height, its 1 px black border
//! 5 px left of the label, the text where the label was and 15 px of white
//! after it (`TreeView::edit`).
//!
//! The shell's trees (`Style::Explorer` and `Style::Picker`) track the
//! pointer: the label under it is underlined, in the hot-tracking colour
//! unless selected, and a click on a row opens it as it selects it, closing
//! the branch the old selection was in (`TreeView::click_row`).
//!
//! A look that boxes its items (`look::widgets::item`) boxes the row across
//! the body instead of the label's band, the selection grey without focus,
//! the row under the pointer ([`TreeView::track`]) lit; it draws no lines,
//! and its triangle expanders show only while the pointer is in the tree or
//! the tree has focus.

use crate::access::{self, accesskit};
use crate::classic::widgets::{self, Axis, ScrollHit, axis_lit};
use crate::classic::widgets::{
    BarHit, ScrollParts, bar_hit, draw_focus_ring, scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::listview;
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar;
use crate::look::Paint;
use crate::look::widgets::item::{self, Mark};
use crate::look::widgets::{draw_field_frame, draw_hscrollbar, draw_scrollbar};
use crate::pointer::Lit;
use crate::pointer::Pointer;
use crate::raster::{Canvas, Rect};
use crate::selection::{scroll_into_view, step_index};
use crate::text::{Align, TextRenderer};
use crate::textedit::TextEdit;
use crate::theme::Color;
use accesskit::{NodeId, Role};
use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// The row pitch; the expander boxes land at row.y + 4, which `(ROW_H - BOX) / 2` gives at 17.
pub const ROW_H: i32 = 17;
pub const INDENT: i32 = 19;
pub const ICON_PX: i32 = 16;
/// The expander box's size and where its left edge sits in the level's indent.
const BOX: i32 = 9;
const BOX_X: i32 = 4;
const LABEL_GAP: i32 = 2;
/// `Style::Explorer`'s gap: the label starts `ICON_PX + 5` (21 px) right of the icon's left edge.
const EXPLORER_LABEL_GAP: i32 = 5;
const ROOT_ICON_X: i32 = 4;
const PICKER_ROOT_ICON_X: i32 = 3;
/// `Style::Plain`'s row pitch, icon inset and label gap, for the Open With dialog's list.
const PLAIN_ROW_H: i32 = 18;
const PLAIN_ICON_X: i32 = 3;
const PLAIN_LABEL_GAP: i32 = 5;
/// How far the selection band runs left of a label and right of its text.
const BAND_BEFORE: i32 = 2;
const BAND_AFTER: i32 = 5;
/// Slack kept right of the longest label before the horizontal scrollbar appears.
const ROW_PAD: i32 = 2;
/// The rename box's left border, left of the label's text origin.
const EDIT_BACK: i32 = 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Style {
    /// Dotted connector lines, every level (roots included) with its own expander column.
    #[default]
    Classic,
    /// No lines, no box on a root, roots' icons 4 px in, no edge: the Explorer bar frames it.
    Explorer,
    /// The Move/Copy picker's tree: `Explorer`'s, roots' icons 3 px in.
    Picker,
    /// The Open With dialog's list: no boxes or lines, 18 px rows, icon 3 px in, label 5 px after it.
    Plain,
}

impl Style {
    fn flat(self) -> bool {
        self == Style::Explorer
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub label: String,
    pub icon: Option<Canvas>,
    /// Drawn instead of `icon` while the node is expanded.
    pub open_icon: Option<Canvas>,
    /// Whether it can expand (a folder); an unexpanded node with empty `children` shows a + and asks the host to fill it.
    pub has_children: bool,
    pub expanded: bool,
    pub children: Vec<Node>,
    /// The host's handle for this node, to match a row back to what it stands for.
    pub id: u64,
}

impl Node {
    pub fn new(label: &str, has_children: bool) -> Node {
        Node {
            label: label.into(),
            has_children,
            ..Default::default()
        }
    }

    pub fn shown_icon(&self) -> Option<&Canvas> {
        match (self.expanded, &self.open_icon) {
            (true, Some(ic)) => Some(ic),
            _ => self.icon.as_ref(),
        }
    }
}

pub type Path = Vec<usize>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleRow {
    pub path: Path,
    pub depth: usize,
    pub last: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TreeView {
    /// The scrollbar part the pointer rests on or holds, tracked by the host and lit by `draw`.
    pub bars: Pointer<(Axis, ScrollHit)>,
    pub roots: Vec<Node>,
    pub selected: Option<Path>,
    pub top: usize,
    /// Horizontal scroll offset in logical px; read through [`TreeView::left_in`], which holds it to the rows.
    pub left: i32,
    pub focus_ring: bool,
    /// The visible row a drag hovers, drawn like the selection.
    pub drop_hilight: Option<usize>,
    /// The visible row the pointer rests on, underlined in a tree that tracks it ([`TreeView::hover`]).
    pub hot: Option<usize>,
    /// Where the pointer is over the tree ([`TreeView::track`]), lit by a look that boxes its items.
    pub over: Over,
    /// The selection shows in the face colour, no ring, when the tree lacks focus.
    pub inactive: bool,
    pub style: Style,
    /// The node being renamed in place and its edit, drawn as a box over its
    /// label. The host opens it ([`TreeView::begin_edit`]), feeds it keys and
    /// takes it back to commit or drop.
    pub edit: Option<(Path, TextEdit)>,
    /// The visible rows as last built; `visible` checks them against `roots`
    /// and `expanded` (one walk, no allocation) and rebuilds only on a difference.
    rows: RefCell<Option<Rc<Vec<VisibleRow>>>>,
    /// `layout`'s `content_w` as last measured, keyed on a hash of its inputs,
    /// so an unchanged tree is not re-measured per pointer motion.
    content_w: Cell<Option<(u64, i32)>>,
}

/// Where the pointer is over a tree, for a look that boxes its items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Over {
    pub inside: bool,
    pub row: Option<usize>,
    pub expander: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Expander(usize),
    Row(usize),
    /// The vertical bar: a page is `rows`; `top_at` turns a thumb's grab and a pointer y into the row to scroll to.
    VBar(BarHit),
    /// The horizontal bar: a page is `body.width` px; `left_at` is its `top_at`.
    HBar(BarHit),
    Nothing,
}

impl Hit {
    pub fn bar(self) -> Option<(Axis, ScrollHit)> {
        match self {
            Hit::VBar(b) => Some((Axis::Vertical, b.part()?)),
            Hit::HBar(b) => Some((Axis::Horizontal, b.part()?)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub body: Rect,
    pub rows: usize,
    pub scrollbar: Option<Rect>,
    pub hscroll: Option<Rect>,
    /// The widest visible row, indent and icon included, what `left` scrolls through.
    pub content_w: i32,
    /// The look's shortest scrollbar thumb, which hit and drag lay the bar out with as `draw` does.
    pub thumb_min: i32,
}

impl TreeView {
    pub fn new(roots: Vec<Node>, style: Style) -> TreeView {
        TreeView {
            roots,
            style,
            ..Default::default()
        }
    }

    pub fn node(&self, path: &[usize]) -> Option<&Node> {
        let (first, rest) = path.split_first()?;
        let mut n = self.roots.get(*first)?;
        for &i in rest {
            n = n.children.get(i)?;
        }
        Some(n)
    }

    pub fn node_mut(&mut self, path: &[usize]) -> Option<&mut Node> {
        let (first, rest) = path.split_first()?;
        let mut n = self.roots.get_mut(*first)?;
        for &i in rest {
            n = n.children.get_mut(i)?;
        }
        Some(n)
    }

    /// The rows on show, depth first through the expanded nodes. Built once
    /// per tree shape: a deep Folders pane is thousands of rows.
    pub fn visible(&self) -> Rc<Vec<VisibleRow>> {
        /// Whether `rows[*k..]` starts with what `walk` builds for `nodes` under `parent`, moving `*k` past them.
        fn same(nodes: &[Node], parent: &[usize], rows: &[VisibleRow], k: &mut usize) -> bool {
            let n = nodes.len();
            for (i, node) in nodes.iter().enumerate() {
                let Some(row) = rows.get(*k) else {
                    return false;
                };
                if row.path.split_last() != Some((&i, parent))
                    || row.depth != parent.len()
                    || row.last != (i + 1 == n)
                {
                    return false;
                }
                *k += 1;
                if node.expanded && !same(&node.children, &row.path, rows, k) {
                    return false;
                }
            }
            true
        }
        fn walk(nodes: &[Node], path: &mut Path, depth: usize, out: &mut Vec<VisibleRow>) {
            let n = nodes.len();
            for (i, node) in nodes.iter().enumerate() {
                path.push(i);
                out.push(VisibleRow {
                    path: path.clone(),
                    depth,
                    last: i + 1 == n,
                });
                if node.expanded {
                    walk(&node.children, path, depth + 1, out);
                }
                path.pop();
            }
        }
        let mut cached = self.rows.borrow_mut();
        if let Some(rows) = cached.as_ref() {
            let mut k = 0;
            if same(&self.roots, &[], rows, &mut k) && k == rows.len() {
                return rows.clone();
            }
        }
        let mut out = Vec::new();
        walk(&self.roots, &mut Vec::new(), 0, &mut out);
        let rows = Rc::new(out);
        *cached = Some(rows.clone());
        rows
    }

    pub fn selected_row(&self, rows: &[VisibleRow]) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        rows.iter().position(|r| &r.path == sel)
    }

    pub fn follow(&mut self, lay: &Layout) {
        let rows = self.visible();
        let mut top = self.top;
        if let Some(i) = self.selected_row(&rows) {
            scroll_into_view(&mut top, i, lay.rows);
        }
        self.set_top(top.min(self.max_top(lay)));
    }

    /// Scroll to `top`. A hot row moves away from the pointer, so none is hot until it moves. True when it moved.
    fn set_top(&mut self, top: usize) -> bool {
        let moved = top != self.top;
        if moved {
            self.top = top;
            self.rows_moved();
        }
        moved
    }

    fn max_top(&self, lay: &Layout) -> usize {
        self.visible().len().saturating_sub(lay.rows)
    }

    /// Pull `top` back after rows went away so the view ends at the last row.
    fn clamp_top(&mut self, lay: &Layout) {
        self.set_top(self.top.min(self.max_top(lay)));
    }

    pub fn hit(&self, lay: &Layout, x: i32, y: i32) -> Hit {
        if let Some(sb) = lay.scrollbar.filter(|sb| sb.contains(x, y)) {
            let p = self.vparts(lay, sb);
            return Hit::VBar(bar_hit(&p, x, y, false));
        }
        if let Some(hb) = lay.hscroll.filter(|hb| hb.contains(x, y)) {
            let p = self.hparts(lay, hb);
            return Hit::HBar(bar_hit(&p, x, y, true));
        }
        if !lay.body.contains(x, y) {
            return Hit::Nothing;
        }
        let rows = self.visible();
        let row_h = self.row_h();
        let i = self.top + ((y - lay.body.y) / row_h) as usize;
        let Some(row) = rows.get(i) else {
            return Hit::Nothing;
        };
        let row_y = lay.body.y + (i - self.top) as i32 * row_h;
        let ix = lay.body.x - self.left_in(lay) + self.icon_x(row.depth);
        if self.has_box(row.depth) {
            let expander = Rect::new(ix - INDENT + BOX_X, row_y, BOX, row_h);
            if expander.contains(x, y) {
                let node = self.node(&row.path);
                if node.is_some_and(|n| n.has_children) {
                    return Hit::Expander(i);
                }
            }
        }
        if x >= ix {
            return Hit::Row(i);
        }
        Hit::Nothing
    }

    /// The row pitch: `Style::Plain`'s list is a px taller than the trees with expander boxes.
    pub fn row_h(&self) -> i32 {
        match self.style {
            Style::Plain => PLAIN_ROW_H,
            _ => ROW_H,
        }
    }

    /// Where a row's icon starts inside the body: `Classic` keeps an expander
    /// column left of the roots, `Explorer` does not (root icon at `body.x + 4`),
    /// `Picker` puts the root's at `body.x + 3`, `Plain` puts it 3 px in.
    fn icon_x(&self, depth: usize) -> i32 {
        match self.style {
            Style::Classic => (depth as i32 + 1) * INDENT,
            Style::Explorer => ROOT_ICON_X + depth as i32 * INDENT,
            Style::Picker => PICKER_ROOT_ICON_X + depth as i32 * INDENT,
            Style::Plain => PLAIN_ICON_X + depth as i32 * INDENT,
        }
    }

    /// How far a label's line box sits below centring in the row: a px in 17 px
    /// rows, none in `Plain`'s 18 px ones, so capitals start 5 px below the top in both.
    fn label_dy(&self) -> i32 {
        match self.style {
            Style::Plain => 0,
            _ => 1,
        }
    }

    fn label_x(&self, depth: usize) -> i32 {
        let gap = match self.style {
            Style::Classic => LABEL_GAP,
            Style::Explorer | Style::Picker => EXPLORER_LABEL_GAP,
            Style::Plain => PLAIN_LABEL_GAP,
        };
        self.icon_x(depth) + ICON_PX + gap
    }

    /// Whether this depth gets an expander column: Explorer draws no box on a root, `Plain` never does.
    fn has_box(&self, depth: usize) -> bool {
        match self.style {
            Style::Classic => true,
            Style::Explorer | Style::Picker => depth > 0,
            Style::Plain => false,
        }
    }

    /// Whether the rows answer the pointer as the shell's trees do: hot labels and a click that opens the row.
    pub fn tracks(&self) -> bool {
        matches!(self.style, Style::Explorer | Style::Picker)
    }

    /// The pointer moved to (`x`, `y`): the row whose icon or label it is on turns hot. True when drawing changed.
    pub fn hover(&mut self, lay: &Layout, text: &TextRenderer, px: u32, x: i32, y: i32) -> bool {
        let hot = match self.hit(lay, x, y) {
            Hit::Row(i) if self.tracks() => {
                let rows = self.visible();
                let label = self.node(&rows[i].path).map_or("", |n| n.label.as_str());
                let end = lay.body.x - self.left_in(lay)
                    + self.label_x(rows[i].depth)
                    + text.measure(label, false, px);
                (x < end).then_some(i)
            }
            _ => None,
        };
        std::mem::replace(&mut self.hot, hot) != hot
    }

    /// The pointer moved to (`x`, `y`): where it is over the tree
    /// ([`TreeView::over`]); a host under a boxing look calls this beside
    /// [`TreeView::hover`]. True when that changed.
    pub fn track(&mut self, lay: &Layout, x: i32, y: i32) -> bool {
        let row = lay
            .body
            .contains(x, y)
            .then(|| self.top + ((y - lay.body.y) / self.row_h()) as usize)
            .filter(|&i| i < self.visible().len());
        let over = Over {
            inside: lay.frame.contains(x, y),
            row,
            expander: matches!(self.hit(lay, x, y), Hit::Expander(_)),
        };
        std::mem::replace(&mut self.over, over) != over
    }

    /// The pointer left the tree: no row is hot. True when one was, or the pointer was over it.
    pub fn unhover(&mut self) -> bool {
        let over = std::mem::take(&mut self.over) != Over::default();
        self.hot.take().is_some() | over
    }

    /// The rows moved under the pointer: none is under it until it moves again.
    fn rows_moved(&mut self) {
        self.hot = None;
        self.over.row = None;
        self.over.expander = false;
    }

    /// A click on visible row `i`'s icon or label: select it, and in a tree
    /// that tracks the pointer open it too, closing the old selection and its
    /// ancestors up to one the new row sits under. Returns the path to fill
    /// when the row opened with no children loaded.
    pub fn click_row(&mut self, i: usize, lay: &Layout) -> Option<Path> {
        let rows = self.visible();
        let new = rows.get(i)?.path.clone();
        let old = self.selected.clone();
        let tracks = self.tracks();
        if tracks && let Some(old) = old.filter(|o| !new.starts_with(o)) {
            for depth in (1..=old.len()).rev() {
                let up = &old[..depth];
                if new.starts_with(up) {
                    break;
                }
                if let Some(n) = self.node_mut(up) {
                    n.expanded = false;
                }
            }
        }
        self.selected = Some(new.clone());
        if tracks {
            self.rows_moved();
        }
        let fill = match self.node_mut(&new) {
            Some(n) if tracks && n.has_children && !n.expanded => {
                n.expanded = true;
                n.children.is_empty().then(|| new.clone())
            }
            _ => None,
        };
        self.follow(lay);
        fill
    }

    /// Toggle the node at visible row `i`; returns its path when it just expanded with no children loaded.
    pub fn toggle(&mut self, i: usize, lay: &Layout) -> Option<Path> {
        let rows = self.visible();
        let path = rows.get(i)?.path.clone();
        let node = self.node_mut(&path)?;
        if !node.has_children {
            return None;
        }
        node.expanded = !node.expanded;
        let fill = (node.expanded && node.children.is_empty()).then_some(path);
        self.rows_moved();
        self.clamp_top(lay);
        fill
    }

    pub fn select(&mut self, i: usize, lay: &Layout) -> bool {
        let rows = self.visible();
        let Some(row) = rows.get(i) else {
            return false;
        };
        let changed = self.selected.as_ref() != Some(&row.path);
        self.selected = Some(row.path.clone());
        self.follow(lay);
        changed
    }

    /// Open the rename box on the node at `path`, its whole label selected.
    pub fn begin_edit(&mut self, path: Path) {
        let Some(n) = self.node(&path) else {
            return;
        };
        let ed = TextEdit::new(&n.label, true);
        self.edit = Some((path, ed));
    }

    /// The rename box, border included, sized to its text: `None` without one or while its row is scrolled out of the body.
    pub fn edit_rect(&self, lay: &Layout, text: &TextRenderer, px: u32) -> Option<Rect> {
        let (path, ed) = self.edit.as_ref()?;
        let rows = self.visible();
        let i = rows.iter().position(|r| &r.path == path)?;
        if !(self.top..self.top + lay.rows).contains(&i) {
            return None;
        }
        let row_h = self.row_h();
        let x = lay.body.x - self.left_in(lay) + self.label_x(rows[i].depth) - EDIT_BACK;
        let w = listview::EDIT_PAD_L + text.measure(&ed.text, false, px) + listview::EDIT_PAD_R;
        Some(Rect::new(
            x,
            lay.body.y + (i - self.top) as i32 * row_h,
            w,
            row_h,
        ))
    }

    /// The part of the rename box inside the body, where it answers the
    /// pointer and the access tree: a long name is cut off under the
    /// scrollbar, and the bar keeps its presses.
    pub fn edit_bounds(&self, lay: &Layout, text: &TextRenderer, px: u32) -> Option<Rect> {
        let r = self.edit_rect(lay, text, px)?;
        let right = r.right().min(lay.body.right());
        Some(Rect::new(r.x, r.y, (right - r.x).max(0), r.height))
    }

    /// Move the selection `step` visible rows (Home / End with a big step); turns the focus ring on.
    pub fn step(&mut self, step: i64, lay: &Layout) -> bool {
        let rows = self.visible();
        let Some(to) = step_index(self.selected_row(&rows), step, rows.len()) else {
            return false;
        };
        let changed = self.selected.as_ref() != Some(&rows[to].path) || !self.focus_ring;
        self.selected = Some(rows[to].path.clone());
        self.focus_ring = true;
        self.follow(lay);
        changed
    }

    /// Right expands the selected node (or steps into its first child when open); returns the path to fill when it expanded empty.
    pub fn expand_selected(&mut self, lay: &Layout) -> Option<Path> {
        let rows = self.visible();
        let i = self.selected_row(&rows)?;
        let node = self.node(&rows[i].path)?;
        if node.has_children && !node.expanded {
            return self.toggle(i, lay);
        }
        if node.expanded && !node.children.is_empty() {
            self.step(1, lay);
        }
        None
    }

    pub fn collapse_selected(&mut self, lay: &Layout) -> bool {
        let Some(path) = self.selected.clone() else {
            return false;
        };
        let Some(node) = self.node_mut(&path) else {
            return false;
        };
        if node.expanded {
            node.expanded = false;
            self.follow(lay);
            return true;
        }
        if path.len() > 1 {
            self.selected = Some(path[..path.len() - 1].to_vec());
            self.follow(lay);
            return true;
        }
        false
    }

    pub fn scroll(&mut self, by: i64, lay: &Layout) -> bool {
        let max = self.max_top(lay) as i64;
        let top = (self.top as i64 + by).clamp(0, max) as usize;
        self.set_top(top)
    }

    /// Scroll the rows sideways by `by` logical px, clamped to what overflows `body`.
    pub fn hscroll(&mut self, by: i32, lay: &Layout) -> bool {
        let from = self.left_in(lay);
        self.left = scrollbar::held(from + by, lay.body.width, lay.content_w);
        self.left != from
    }

    /// How far the rows are scrolled sideways under `lay`: `left` held to what
    /// overflows the body, since a collapsing branch or a widening pane can
    /// leave it past the end.
    pub fn left_in(&self, lay: &Layout) -> i32 {
        scrollbar::held(self.left, lay.body.width, lay.content_w)
    }

    fn vparts(&self, lay: &Layout, sb: Rect) -> ScrollParts {
        scrollbar_parts(
            sb,
            1,
            self.top,
            lay.rows,
            self.visible().len(),
            false,
            lay.thumb_min,
        )
    }

    fn hparts(&self, lay: &Layout, hb: Rect) -> ScrollParts {
        scrollbar_parts(
            hb,
            1,
            self.left_in(lay) as usize,
            lay.body.width.max(0) as usize,
            lay.content_w.max(0) as usize,
            true,
            lay.thumb_min,
        )
    }

    /// The top row a thumb grabbed `grab` px below its own top scrolls to when the pointer is at `y`.
    pub fn top_at(&self, lay: &Layout, y: i32, grab: i32) -> usize {
        match lay.scrollbar {
            Some(sb) => {
                let p = self.vparts(lay, sb);
                scrollbar_pos_at(&p, y - grab, lay.rows, self.visible().len(), false)
            }
            None => self.top,
        }
    }

    /// The offset a horizontal thumb grabbed `grab` px right of its left edge scrolls to when the pointer is at `x`.
    pub fn left_at(&self, lay: &Layout, x: i32, grab: i32) -> i32 {
        match lay.hscroll {
            Some(hb) => {
                let p = self.hparts(lay, hb);
                let visible = lay.body.width.max(0) as usize;
                let total = lay.content_w.max(0) as usize;
                scrollbar_pos_at(&p, x - grab, visible, total, true) as i32
            }
            None => self.left_in(lay),
        }
    }
}

fn row_width(tv: &TreeView, row: &VisibleRow, text: &TextRenderer, px: u32) -> i32 {
    let label = tv.node(&row.path).map(|n| n.label.as_str()).unwrap_or("");
    tv.label_x(row.depth) + text.measure(label, false, px) + ROW_PAD
}

pub fn layout(tv: &TreeView, text: &TextRenderer, px: u32, r: Rect, cm: &ControlMetrics) -> Layout {
    let visible = tv.visible();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (std::ptr::from_ref(text) as usize, px, tv.style).hash(&mut h);
    for row in visible.iter() {
        row.depth.hash(&mut h);
        tv.node(&row.path).map(|n| n.label.as_str()).hash(&mut h);
    }
    let key = h.finish();
    let content_w = match tv.content_w.get() {
        Some((at, w)) if at == key => w,
        _ => {
            let w = visible
                .iter()
                .map(|row| row_width(tv, row, text, px))
                .max()
                .unwrap_or(0);
            tv.content_w.set(Some((key, w)));
            w
        }
    };
    layout_with(tv, r, content_w, cm)
}

/// `layout` for a host with no `TextRenderer`: rows are taken to fit, so no
/// horizontal scrollbar. A tree on a `form::Form` gets this (the Open With
/// list never scrolls sideways).
pub fn layout_without_text(tv: &TreeView, r: Rect, cm: &ControlMetrics) -> Layout {
    layout_with(tv, r, 0, cm)
}

fn layout_with(tv: &TreeView, r: Rect, content_w: i32, cm: &ControlMetrics) -> Layout {
    let inner = if tv.style.flat() { r } else { r.inset(2) };
    let visible = tv.visible();
    // Reserving one bar can make the other necessary and neither is given back, so two rounds reach the fixed point (as in `listview`).
    let (mut vbar, mut hbar) = (false, false);
    for _ in 0..2 {
        let w = inner.width - if vbar { cm.scroll } else { 0 };
        let h = inner.height - if hbar { cm.scroll } else { 0 };
        hbar |= content_w > w;
        vbar |= visible.len() > (h / tv.row_h()).max(1) as usize;
    }
    let body = Rect::new(
        inner.x,
        inner.y,
        inner.width - if vbar { cm.scroll } else { 0 },
        inner.height - if hbar { cm.scroll } else { 0 },
    );
    Layout {
        frame: r,
        body,
        rows: (body.height / tv.row_h()).max(1) as usize,
        scrollbar: vbar.then(|| Rect::new(body.right(), body.y, cm.scroll, body.height)),
        hscroll: hbar.then(|| Rect::new(body.x, body.bottom(), body.width, cm.scroll)),
        content_w,
        thumb_min: cm.thumb_min,
    }
}

fn dotted_h(c: &mut Canvas, x0: i32, x1: i32, y: i32, s: i32, ink: Color) {
    let mut x = x0;
    while x < x1 {
        if ((x / s) + (y / s)) % 2 == 0 {
            c.fill_rect(Rect::new(x, y, s, s), ink);
        }
        x += s;
    }
}

fn dotted_v(c: &mut Canvas, x: i32, y0: i32, y1: i32, s: i32, ink: Color) {
    let mut y = y0;
    while y < y1 {
        if ((x / s) + (y / s)) % 2 == 0 {
            c.fill_rect(Rect::new(x, y, s, s), ink);
        }
        y += s;
    }
}

pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    tv: &TreeView,
    lay: &Layout,
    bars: Lit<(Axis, ScrollHit)>,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let frame = lay.frame.scaled(s);
    if tv.style.flat() {
        c.fill_rect(frame, cs.window);
    } else {
        c.fill_rect(frame, cs.button_face);
        draw_field_frame(c, p, frame, cs.window);
    }
    let body = lay.body.scaled(s);
    let rows = tv.visible();
    // The rows go into a canvas of their own so a `left` past 0 clips there instead of drawing over the frame or scrollbar.
    let mut bc = Canvas::with_scale(
        body.width.max(0) as u32,
        body.height.max(0) as u32,
        c.scale(),
    );
    let all = Rect::new(0, 0, body.width, body.height);
    bc.fill_rect(all, cs.window);
    let row_h = tv.row_h() * s;
    let left = tv.left_in(lay) * s;
    // The dotted lines and box rings are 128 grey, `button_shadow` in every stock scheme; a dark scheme's follows.
    let line = cs.button_shadow;
    // Which depths still have siblings below the shown rows, for vertical lines running past a row, seeded from the top row's ancestors so a scrolled tree keeps them.
    let mut open_levels: Vec<bool> = Vec::new();
    if let Some(row) = rows.get(tv.top) {
        for d in 0..row.depth {
            let siblings = match d {
                0 => tv.roots.len(),
                _ => tv.node(&row.path[..d]).map_or(0, |n| n.children.len()),
            };
            open_levels.push(row.path[d] + 1 < siblings);
        }
    }
    for k in 0..lay.rows {
        let i = tv.top + k;
        let Some(row) = rows.get(i) else {
            break;
        };
        let y = k as i32 * row_h;
        let mid = y + row_h / 2;
        let ix = -left + tv.icon_x(row.depth) * s;
        let Some(node) = tv.node(&row.path) else {
            continue;
        };
        open_levels.resize(row.depth + 1, false);
        open_levels[row.depth] = !row.last;
        let bx = ix - (INDENT - BOX_X) * s;
        let cx = bx + (BOX / 2) * s;
        let selected = tv.selected.as_ref() == Some(&row.path);
        let drop = tv.drop_hilight == Some(i);
        // A boxing look boxes the row across the body, 1 px in from either side, draws no lines, shows expanders only while the pointer is in the tree or it has focus, and leaves the label as is.
        let boxed = item::draw_item(
            &mut bc,
            p,
            Rect::new(s, y, all.width - 2 * s, row_h),
            Mark {
                selected: selected || drop,
                hot: tv.over.row == Some(i) || drop,
                dim: tv.inactive && !drop,
                ..Mark::default()
            },
        );
        if boxed {
            let shown = tv.over.inside || (!tv.inactive && p.active);
            if node.has_children && tv.has_box(row.depth) && shown {
                let hot = tv.over.row == Some(i) && tv.over.expander;
                item::draw_expander(&mut bc, p, (cx, mid), node.expanded, hot);
            }
        } else if tv.style == Style::Classic {
            // Lines: vertical through the ancestors that continue, this level's vertical to the box, the horizontal to the icon. Explorer's tree has none.
            for (d, cont) in open_levels.iter().enumerate().take(row.depth) {
                if *cont {
                    let lx = -left + (tv.icon_x(d) - INDENT + BOX_X + BOX / 2) * s;
                    dotted_v(&mut bc, lx, y, y + row_h, s, line);
                }
            }
            let top = if i == 0 && row.depth == 0 { mid } else { y };
            let bottom = if row.last { mid } else { y + row_h };
            dotted_v(&mut bc, cx, top, bottom, s, line);
            dotted_h(&mut bc, cx, ix, mid, s, line);
        }
        if node.has_children && tv.has_box(row.depth) && !boxed {
            let b = Rect::new(bx, mid - (BOX / 2) * s, BOX * s, BOX * s);
            bc.fill_rect(b, cs.window);
            bc.ring_t(b, line, line, s);
            let ink = cs.button_text;
            bc.fill_rect(Rect::new(b.x + 2 * s, mid, 5 * s, s), ink);
            if !node.expanded {
                bc.fill_rect(Rect::new(cx, b.y + 2 * s, s, 5 * s), ink);
            }
        }
        if let Some(ic) = node.shown_icon() {
            bc.blit_over(ix, y + (row_h - ic.height as i32) / 2, ic);
        }
        if tv.edit.as_ref().is_some_and(|(p, _)| p == &row.path) {
            continue;
        }
        let tx = -left + tv.label_x(row.depth) * s;
        let tw = (all.right() - tx - 2 * s).max(0);
        let shown = text.ellipsize(&node.label, tw, false, px);
        let tr = Rect::new(tx, y + tv.label_dy() * s, tw, row_h);
        if boxed {
            text.draw(&mut bc, tr, &shown, false, cs.button_text, Align::Left, px);
            continue;
        }
        // A hot label is underlined in its own ink, `hot_light` off the band, a px below the baseline.
        let hot = tv.tracks() && tv.hot == Some(i);
        let underline = |bc: &mut Canvas, ink: Color| {
            if let Some(base) = text.baseline(tr, false, px) {
                let w = text.measure(&shown, false, px);
                let ul = Rect::new(tx, base + s, w, s).intersect(&all);
                bc.fill_rect(ul, ink);
            }
        };
        if selected || tv.drop_hilight == Some(i) {
            // Without focus the pick reads in the face colour with no ring; a drop target (always focused while dragging) never dims.
            let unfocused = tv.inactive && selected && tv.drop_hilight != Some(i);
            let (fill, ink) = if unfocused {
                (cs.button_face, cs.button_text)
            } else {
                (cs.hilight, cs.hilight_text)
            };
            let w = text.measure(&shown, false, px);
            let hl = Rect::new(
                tx - BAND_BEFORE * s,
                y,
                w + (BAND_BEFORE + BAND_AFTER) * s,
                row_h,
            )
            .intersect(&all);
            bc.fill_rect(hl, fill);
            text.draw(&mut bc, tr, &shown, false, ink, Align::Left, px);
            if hot {
                underline(&mut bc, ink);
            }
            // The drop target wins over the ring on a row that is both.
            if tv.focus_ring && selected && tv.drop_hilight != Some(i) && !unfocused {
                draw_focus_ring(&mut bc, hl, s, cs.hilight_text);
            }
        } else if hot {
            text.draw(&mut bc, tr, &shown, false, cs.hot_light, Align::Left, px);
            underline(&mut bc, cs.hot_light);
        } else {
            text.draw(&mut bc, tr, &shown, false, cs.button_text, Align::Left, px);
        }
    }
    if let (Some(b), Some((_, ed))) = (tv.edit_rect(lay, text, px), &tv.edit) {
        let b = Rect::new(b.x - lay.body.x, b.y - lay.body.y, b.width, b.height).scaled(s);
        listview::draw_rename_box(&mut bc, text, p, ed, b, px);
    }
    c.blit_canvas(body.x, body.y, &bc);
    if let Some(sb) = lay.scrollbar {
        let lit = axis_lit(bars, Axis::Vertical);
        draw_scrollbar(c, p, sb.scaled(s), tv.top, lay.rows, rows.len(), lit);
    }
    if let Some(hb) = lay.hscroll {
        draw_hscrollbar(
            c,
            p,
            hb.scaled(s),
            tv.left_in(lay) as usize,
            lay.body.width.max(0) as usize,
            lay.content_w.max(0) as usize,
            axis_lit(bars, Axis::Horizontal),
        );
    }
}

/// The tree view's `access` part: a tree whose items nest as the visible rows
/// do, expanded or collapsed when they have children. Rows scrolled out of
/// view have no bounds. Returns the node that has the focus when the tree
/// does: the rename box while open, the selected row, else the tree.
pub fn access(
    tv: &TreeView,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut access::Tree,
    parent: NodeId,
    key: impl Hash,
) -> NodeId {
    let mut n = access::node(Role::Tree, lay.frame);
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    let rows = tv.visible();
    // The node of each depth's last row: a row hangs under the one at the depth above.
    let mut at_depth: Vec<NodeId> = Vec::new();
    let rh = tv.row_h();
    for (i, row) in rows.iter().enumerate() {
        let Some(node) = tv.node(&row.path) else {
            continue;
        };
        let mut item = accesskit::Node::new(Role::TreeItem);
        item.set_label(node.label.as_str());
        item.set_level(row.depth + 1);
        if node.has_children {
            item.set_expanded(node.expanded);
        }
        let selected = tv.selected.as_ref() == Some(&row.path);
        item.set_selected(selected);
        if (tv.top..tv.top + lay.rows).contains(&i) {
            let y = lay.body.y + (i - tv.top) as i32 * rh;
            let x = lay.body.x - tv.left_in(lay) + tv.label_x(row.depth);
            item.set_bounds(access::bounds(Rect::new(
                x,
                y,
                (lay.body.right() - x).max(0),
                rh,
            )));
        }
        let up = row
            .depth
            .checked_sub(1)
            .and_then(|d| at_depth.get(d))
            .copied()
            .unwrap_or(id);
        let item_id = tree.push(up, &row.path, item);
        at_depth.truncate(row.depth);
        at_depth.push(item_id);
        if selected {
            focus = item_id;
        }
        if let Some((p, ed)) = &tv.edit
            && p == &row.path
            && let Some(r) = tv.edit_bounds(lay, text, px)
        {
            focus = widgets::access_edit(ed, 0, false, r, true, text, px, tree, item_id, "rename");
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    const LINE_GRAY: Color = Color::rgb(128, 128, 128);

    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    fn tree() -> TreeView {
        let mut c = Node::new("Local Disk", true);
        c.children = vec![Node::new("Programs", true), Node::new("Users", true)];
        c.expanded = true;
        let mut desktop = Node::new("Desktop", true);
        desktop.children = vec![
            Node::new("My Documents", true),
            c,
            Node::new("Recycle Bin", false),
        ];
        desktop.expanded = true;
        TreeView {
            roots: vec![desktop],
            ..Default::default()
        }
    }

    /// The rows are built once per tree shape and shared until a node opens or closes.
    #[test]
    fn visible_rows_are_shared_until_the_shape_changes() {
        let mut tv = tree();
        let a = tv.visible();
        let b = tv.visible();
        assert!(Rc::ptr_eq(&a, &b), "the same rows twice");
        tv.roots[0].children[1].expanded = false;
        let c = tv.visible();
        assert!(!Rc::ptr_eq(&a, &c), "a collapse through the field rebuilds");
        assert!(c.len() < a.len());
        tv.roots[0].children.push(Node {
            label: "New".into(),
            ..Node::default()
        });
        assert!(tv.visible().len() == c.len() + 1, "a new sibling rebuilds");
    }

    /// Rows cached for another shape of the same size are rebuilt.
    #[test]
    fn visible_rebuilds_rows_left_from_another_shape() {
        let tv = tree();
        let want = tv.visible().as_ref().clone();
        let mut other = tree();
        other.roots[0].children[1].expanded = false;
        other.roots[0].children[0].expanded = true;
        other.roots[0].children[0].children = vec![Node::new("a", false), Node::new("b", false)];
        let stale = other.visible();
        assert_eq!(stale.len(), want.len());
        *tv.rows.borrow_mut() = Some(stale);
        assert_eq!(*tv.visible(), want, "the rows of the tree as it stands");
    }

    #[test]
    fn visible_rows_follow_expansion_and_keys_walk_them() {
        let mut tv = tree();
        let rows = tv.visible();
        let labels: Vec<&str> = rows
            .iter()
            .map(|r| tv.node(&r.path).unwrap().label.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "Desktop",
                "My Documents",
                "Local Disk",
                "Programs",
                "Users",
                "Recycle Bin"
            ]
        );
        assert_eq!(rows[2].depth, 1);
        assert!(rows[5].last && !rows[2].last);
        let text = TextRenderer::load(&FontConfig::default());
        let px = FontConfig::default().size_px;
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        tv.step(1, &lay);
        assert_eq!(tv.selected, Some(vec![0]));
        tv.step(2, &lay);
        assert_eq!(tv.selected, Some(vec![0, 1]));
        assert!(tv.collapse_selected(&lay));
        assert_eq!(tv.visible().len(), 4);
        assert!(tv.collapse_selected(&lay));
        assert_eq!(tv.selected, Some(vec![0]));
        tv.step(2, &lay);
        assert_eq!(tv.expand_selected(&lay), None, "children already loaded");
        assert_eq!(tv.visible().len(), 6);
        tv.step(1, &lay);
        assert_eq!(tv.expand_selected(&lay), Some(vec![0, 1, 0]));
        assert!(tv.node(&[0, 1, 0]).unwrap().expanded);
    }

    /// The Open With style: 18 px rows, no expander, the icon 3 px into its level.
    #[test]
    fn plain_style_has_18_px_rows_and_no_expanders() {
        let mut tv = tree();
        tv.style = Style::Plain;
        let text = TextRenderer::load(&FontConfig::default());
        let lay = layout(
            &tv,
            &text,
            11,
            Rect::new(0, 0, 200, 210),
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.rows, 11, "206 px of body holds 11 full rows");
        let b = lay.body;
        let y = b.y + 2 * PLAIN_ROW_H + 8;
        assert_eq!(tv.hit(&lay, b.x + INDENT + BOX_X + 4, y), Hit::Row(2));
        assert_eq!(tv.hit(&lay, b.x + INDENT + 2, y), Hit::Nothing);
        assert_eq!(tv.hit(&lay, b.x + INDENT + PLAIN_ICON_X, y), Hit::Row(2));
    }

    #[test]
    fn hits_expanders_and_rows() {
        let tv = tree();
        let text = TextRenderer::load(&FontConfig::default());
        let px = FontConfig::default().size_px;
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let b = lay.body;
        let y = b.y + 2 * ROW_H + 8;
        assert_eq!(tv.hit(&lay, b.x + INDENT + BOX_X + 4, y), Hit::Expander(2));
        assert_eq!(tv.hit(&lay, b.x + 2 * INDENT + 5, y), Hit::Row(2));
        assert_eq!(tv.hit(&lay, b.x + 2, y), Hit::Nothing);
        let y5 = b.y + 5 * ROW_H + 8;
        assert_eq!(tv.hit(&lay, b.x + INDENT + BOX_X + 4, y5), Hit::Nothing);
        assert_eq!(tv.hit(&lay, b.x + 5, b.y + 10 * ROW_H), Hit::Nothing);
    }

    #[test]
    fn draws_boxes_lines_and_the_selection() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let mut tv = tree();
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        tv.select(2, &lay);
        let mut c = Canvas::new(200, 300);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        let b = lay.body;
        let mid = b.y + ROW_H / 2;
        assert_eq!(c.get(b.x + BOX_X, mid - 4), LINE_GRAY.argb());
        assert_eq!(c.get(b.x + BOX_X + 4, mid), cs.button_text.argb());
        assert_eq!(
            c.get(b.x + BOX_X + 4, mid - 2),
            cs.window.argb(),
            "no + on an open node"
        );
        let mid4 = b.y + 4 * ROW_H + ROW_H / 2;
        assert_eq!(
            c.get(b.x + 2 * INDENT + BOX_X + 4, mid4 - 2),
            cs.button_text.argb()
        );
        let tx = b.x + 2 * INDENT + ICON_PX + LABEL_GAP;
        assert_eq!(c.get(tx - 1, b.y + 2 * ROW_H + 8), cs.hilight.argb());
    }

    /// An expanded node blits `open_icon`, a collapsed one `icon`, as different pixels.
    #[test]
    fn an_expanded_node_blits_its_open_icon() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let solid = |color: Color| {
            let mut ic = Canvas::new(ICON_PX as u32, ICON_PX as u32);
            ic.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), color);
            ic
        };
        let (shut, open) = (Color::rgb(255, 0, 0), Color::rgb(0, 255, 0));
        let mut tv = tree();
        for n in &mut tv.roots[0].children {
            n.icon = Some(solid(shut));
            n.open_icon = Some(solid(open));
        }
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let mut c = Canvas::new(200, 300);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        let b = lay.body;
        let icon_x = |row: &VisibleRow| b.x + (row.depth as i32 + 1) * INDENT + ICON_PX / 2;
        let rows = tv.visible();
        assert!(!tv.node(&rows[1].path).unwrap().expanded);
        assert!(tv.node(&rows[2].path).unwrap().expanded);
        assert_eq!(c.get(icon_x(&rows[1]), b.y + ROW_H + 8), shut.argb());
        assert_eq!(c.get(icon_x(&rows[2]), b.y + 2 * ROW_H + 8), open.argb());
    }

    #[test]
    fn a_drop_hilight_draws_an_unselected_row_selected() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let mut tv = tree();
        tv.focus_ring = true;
        tv.drop_hilight = Some(2);
        assert_eq!(tv.selected, None, "nothing is selected");
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let mut c = Canvas::new(200, 300);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        let b = lay.body;
        let tx = b.x + 2 * INDENT + ICON_PX + LABEL_GAP;
        assert_eq!(c.get(tx - 1, b.y + 2 * ROW_H + 8), cs.hilight.argb());
        assert_eq!(c.get(tx - 1, b.y + ROW_H + 8), cs.window.argb());
        assert_ne!(c.get(tx - 2, b.y + 2 * ROW_H), cs.hilight_text.argb());
    }

    /// A tree that lacks the focus shows its pick in the face colour with no ring.
    #[test]
    fn an_inactive_selection_draws_in_the_face_colour() {
        let (theme, text) = (Theme::default(), TextRenderer::load(&FontConfig::default()));
        let cs = &theme.colors;
        let mut tv = TreeView::new(vec![Node::new("Desktop", false)], Style::Classic);
        tv.selected = Some(vec![0]);
        tv.focus_ring = true;
        let r = Rect::new(0, 0, 200, 60);
        let lay = layout(&tv, &text, 11, r, &ControlMetrics::CLASSIC);
        // (fill, ring): fill is inside the highlight past "Desktop"'s glyphs; ring is the highlight's top-left corner, always a dot of `draw_focus_ring`'s pattern.
        let probe = |tv: &TreeView| {
            let mut c = Canvas::new(200, 60);
            draw(&mut c, &text, &classic_paint(cs, 1), tv, &lay, None, 11);
            (
                c.get(lay.body.x + 76, lay.body.y + 8),
                c.get(lay.body.x + 35, lay.body.y),
            )
        };
        tv.inactive = false;
        let (fill, ring) = probe(&tv);
        assert_eq!(
            fill,
            cs.hilight.argb(),
            "a focused tree's pick is the highlight"
        );
        assert_eq!(
            ring,
            cs.hilight_text.argb(),
            "a focused tree's pick keeps its focus ring"
        );
        tv.inactive = true;
        let (fill, ring) = probe(&tv);
        assert_eq!(
            fill,
            cs.button_face.argb(),
            "an inactive pick is the face colour"
        );
        assert_eq!(
            ring,
            cs.button_face.argb(),
            "an inactive pick draws no ring"
        );
    }

    /// Labels wider than the body bring the horizontal scrollbar, which hits
    /// and scrolls like `listview`'s; `layout` measures labels once and reuses
    /// the width until a label, the size or the rows change.
    #[test]
    fn layout_reuses_the_measured_width_until_a_label_changes() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut tv = tree();
        let r = Rect::new(0, 0, 200, 300);
        let w = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC).content_w;
        assert!(w > 0);
        let (key, _) = tv.content_w.get().expect("the width is cached");
        tv.content_w.set(Some((key, 999)));
        assert_eq!(
            layout(&tv, &text, px, r, &ControlMetrics::CLASSIC).content_w,
            999,
            "the cached width"
        );
        assert_ne!(
            layout(&tv, &text, px + 2, r, &ControlMetrics::CLASSIC).content_w,
            999,
            "a new size"
        );
        tv.content_w.set(Some((key, 999)));
        tv.roots[0].children[0].label = "Docs".into();
        assert_ne!(
            layout(&tv, &text, px, r, &ControlMetrics::CLASSIC).content_w,
            999,
            "a rename"
        );
    }

    /// A look with a 17 px scrollbar lays both bars out 17 px thick, and the body gives up 17 px to each.
    #[test]
    fn a_17_px_scrollbar_takes_17_px_off_the_body_each_way() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut tv = tree();
        tv.roots[0].children[0].label = "a folder with a very, very long name".into();
        let r = Rect::new(0, 0, 150, 60);
        let cm = ControlMetrics {
            scroll: 17,
            ..ControlMetrics::CLASSIC
        };
        let lay = layout(&tv, &text, px, r, &cm);
        let vb = lay.scrollbar.expect("the rows overflow the body");
        let hb = lay.hscroll.expect("the label overflows the body");
        assert_eq!(lay.body, Rect::new(2, 2, 129, 39));
        assert_eq!(vb, Rect::new(131, 2, 17, 39));
        assert_eq!(hb, Rect::new(2, 41, 129, 17));
    }

    /// A tree whose widest row collapses away or whose pane widens, scrolled
    /// to the far end, draws and hits as if scrolled to its new far end (0
    /// once the rows fit), with no blank past it.
    #[test]
    fn a_narrower_tree_holds_its_sideways_scroll_to_the_widest_row() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut tv = tree();
        tv.style = Style::Explorer;
        tv.roots[0].children[1].children[0].label = "a folder with a very, very long name".into();
        tv.roots[0].children[2].label = "a folder with a long name".into();
        let narrow = Rect::new(0, 0, 160, 300);
        let lay = |tv: &TreeView, r| layout(tv, &text, px, r, &ControlMetrics::CLASSIC);
        let shot = |tv: &TreeView, l: &Layout| {
            let mut c = Canvas::new(400, 300);
            draw(
                &mut c,
                &text,
                &classic_paint(&theme.colors, 1),
                tv,
                l,
                None,
                px,
            );
            c
        };
        let wide = lay(&tv, narrow);
        tv.hscroll(10_000, &wide);
        let far = tv.left;
        assert_eq!(far, wide.content_w - wide.body.width);

        tv.roots[0].children[1].expanded = false;
        let l = lay(&tv, narrow);
        let end = l.content_w - l.body.width;
        assert!((1..far).contains(&end), "the tree still overflows, by less");
        assert_eq!(tv.left_in(&l), end);
        let held = TreeView {
            left: end,
            ..tv.clone()
        };
        assert!(shot(&tv, &l) == shot(&held, &l), "drawn at the new far end");
        let (x, y) = (l.body.right() - 20, l.body.y + 3 * ROW_H + 8);
        assert_eq!(tv.hit(&l, x, y), held.hit(&l, x, y));
        assert!(!tv.hscroll(1, &l), "already at the far end");
        assert!(tv.hscroll(-1, &l));
        assert_eq!(tv.left, end - 1, "the arrow steps from the far end");

        tv.left = far;
        let roomy = lay(&tv, Rect::new(0, 0, 400, 300));
        assert!(roomy.hscroll.is_none());
        assert_eq!(tv.left_in(&roomy), 0);
        let home = TreeView {
            left: 0,
            ..tv.clone()
        };
        assert!(shot(&tv, &roomy) == shot(&home, &roomy));
    }

    #[test]
    fn wide_labels_bring_the_horizontal_scrollbar() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut tv = tree();
        let r = Rect::new(0, 0, 200, 300);
        assert!(
            layout(&tv, &text, px, r, &ControlMetrics::CLASSIC)
                .hscroll
                .is_none()
        );
        tv.roots[0].children[0].label = "a folder with a very, very long name".into();
        let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
        let hb = lay.hscroll.expect("the label overflows the body");
        assert_eq!(
            hb,
            Rect::new(
                lay.body.x,
                lay.body.bottom(),
                lay.body.width,
                ControlMetrics::CLASSIC.scroll
            )
        );
        assert!(lay.content_w > lay.body.width);
        assert_eq!(tv.hit(&lay, hb.x + 3, hb.y + 3), Hit::HBar(BarHit::Back));
        assert_eq!(
            tv.hit(&lay, hb.right() - 3, hb.y + 3),
            Hit::HBar(BarHit::Forward)
        );
        // The thumb reports its grab, the track past it pages, a drag to the track's end lands on the last page.
        let p = scrollbar_parts(
            hb,
            1,
            0,
            lay.body.width as usize,
            lay.content_w as usize,
            true,
            lay.thumb_min,
        );
        let thumb = p.thumb.expect("a thumb");
        assert_eq!(
            tv.hit(&lay, thumb.x + 2, hb.y + 3),
            Hit::HBar(BarHit::Thumb(2))
        );
        assert_eq!(
            tv.hit(&lay, thumb.right() + 2, hb.y + 3),
            Hit::HBar(BarHit::PageForward)
        );
        assert_eq!(
            tv.left_at(&lay, p.track.right(), 2),
            lay.content_w - lay.body.width
        );
        assert!(tv.hscroll(1000, &lay));
        assert_eq!(
            tv.hit(&lay, hb.x + ControlMetrics::CLASSIC.scroll + 2, hb.y + 3),
            Hit::HBar(BarHit::PageBack)
        );
        assert_eq!(tv.left, lay.content_w - lay.body.width);
        assert!(!tv.hscroll(1, &lay), "clamped at the right edge");
        // Scrolled a little, the depth-1 expander box moves left with it and the hit test follows.
        tv.left = 4;
        let y = lay.body.y + 2 * ROW_H + 8;
        let bx = lay.body.x + INDENT + BOX_X + 4 - tv.left;
        assert_eq!(tv.hit(&lay, bx, y), Hit::Expander(2));
        assert_eq!(tv.hit(&lay, bx + INDENT, y), Hit::Row(2));
    }

    /// A folder icon and its open twin, as solid blocks so a test can find their columns.
    fn icons() -> (Canvas, Canvas) {
        let mut shut = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        shut.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), Color::rgb(255, 224, 123));
        let mut open = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        open.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), Color::rgb(174, 218, 255));
        (shut, open)
    }

    /// The Folders tree is flat: no sunken edge, its whole rect white, rows from the rect's top. The connector-line tree keeps its 2 px edge.
    #[test]
    fn the_explorer_style_draws_no_edge() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let r = Rect::new(0, 0, 120, 80);
        for (style, edge) in [
            (Style::Explorer, cs.window),
            (Style::Classic, cs.button_shadow),
        ] {
            let tv = TreeView::new(vec![Node::new("Desktop", false)], style);
            let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
            let mut c = Canvas::new(120, 80);
            draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
            assert_eq!(c.get(0, 0), edge.argb(), "{style:?}'s corner");
            assert_eq!(c.get(0, 60), edge.argb(), "{style:?}'s left column");
        }
        let tv = TreeView::new(vec![Node::new("Desktop", false)], Style::Explorer);
        let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
        assert_eq!((lay.body.x, lay.body.y), (0, 0));
    }

    /// `Style::Explorer`'s pixels, client-local: the white body starts at x 1.
    /// "Desktop" inks its icon at x 5..20 with nothing left of it and its label
    /// from x 26. The selected "My Documents" at depth 1 has the 9 px box at
    /// x 9..17, its icon at x 24..39 and its band from x 43. "Downloads"
    /// (depth 2) inks its icon at x 43..58 and its label from x 64.
    #[test]
    fn explorer_style_lands_on_its_pixels() {
        const BODY_X: i32 = 1;
        const ROOT_ICON: i32 = 5;
        const ROOT_LABEL: i32 = 26;
        const BOX_L: i32 = 9;
        const BOX_R: i32 = 17;
        const D1_ICON: i32 = 24;
        const D1_HILIGHT: i32 = 43;
        const D2_ICON: i32 = 43;
        const D2_LABEL: i32 = 64;

        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let (shut, open) = icons();

        let mut docs = Node::new("My Documents", true);
        docs.children = vec![Node::new("Downloads", false)];
        docs.expanded = true;
        let mut desktop = Node::new("Desktop", true);
        desktop.children = vec![docs];
        desktop.expanded = true;
        let mut tv = TreeView {
            roots: vec![desktop],
            selected: Some(vec![0, 0]),
            style: Style::Explorer,
            ..Default::default()
        };
        fn paint(n: &mut Node, shut: &Canvas, open: &Canvas) {
            n.icon = Some(shut.clone());
            n.open_icon = Some(open.clone());
            for ch in &mut n.children {
                paint(ch, shut, open);
            }
        }
        paint(&mut tv.roots[0], &shut, &open);

        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let mut c = Canvas::new(200, 300);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        let b = lay.body;
        let band = |k: i32| (b.y + k * ROW_H, b.y + (k + 1) * ROW_H);
        let first = |argb: u32, k: i32, x0: i32, x1: i32| {
            let (y0, y1) = band(k);
            (x0..x1).find(|&x| (y0..y1).any(|y| c.get(x, y) == argb))
        };
        let any = |argb: u32, k: i32, x0: i32, x1: i32| first(argb, k, x0, x1).is_some();

        let icon_shut = Color::rgb(255, 224, 123).argb();
        let icon_open = Color::rgb(174, 218, 255).argb();
        assert_eq!(
            first(icon_open, 0, b.x, b.right()),
            Some(b.x + ROOT_ICON - BODY_X)
        );
        assert!(
            !any(LINE_GRAY.argb(), 0, b.x, b.right()),
            "a root has no expander box and Explorer draws no lines"
        );
        let mut scratch = Canvas::new(200, ROW_H as u32);
        scratch.fill_rect(scratch.bounds(), cs.window);
        let origin = Rect::new(0, 0, 200, ROW_H);
        text.draw(
            &mut scratch,
            origin,
            "Desktop",
            false,
            cs.button_text,
            Align::Left,
            px,
        );
        let bearing = (0..200)
            .find(|&x| (0..ROW_H).any(|y| scratch.get(x, y) == cs.button_text.argb()))
            .expect("the label inks something");
        assert_eq!(
            first(cs.button_text.argb(), 0, b.x, b.right()),
            Some(b.x + ROOT_LABEL - BODY_X + bearing)
        );

        let (y0, _) = band(1);
        let mid = y0 + ROW_H / 2;
        assert_eq!(
            first(LINE_GRAY.argb(), 1, b.x, b.right()),
            Some(b.x + BOX_L - BODY_X)
        );
        assert_eq!(c.get(b.x + BOX_R - BODY_X, mid), LINE_GRAY.argb());
        assert_eq!(
            first(icon_open, 1, b.x, b.right()),
            Some(b.x + D1_ICON - BODY_X)
        );
        assert_eq!(
            first(cs.hilight.argb(), 1, b.x, b.right()),
            Some(b.x + D1_HILIGHT - BODY_X)
        );

        assert_eq!(
            first(icon_shut, 2, b.x, b.right()),
            Some(b.x + D2_ICON - BODY_X)
        );
        assert!(
            !any(LINE_GRAY.argb(), 2, b.x, b.right()),
            "no lines run down to a leaf"
        );
        assert_eq!(
            first(cs.button_text.argb(), 2, b.x, b.right()),
            Some(b.x + D2_LABEL - BODY_X + bearing),
            "\"Downloads\" and \"Desktop\" both start on a D"
        );
    }

    /// `Style::Picker`'s pixels, body-relative: "Desktop" inks its icon from x
    /// 3 with no box and its label from x 24; "My Documents" at depth 1 has
    /// its 9 px box at x 7..15, icon from x 22, band from x 41, label from 43.
    #[test]
    fn picker_style_lands_on_its_pixels() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let (shut, open) = icons();
        let mut docs = Node::new("My Documents", true);
        docs.icon = Some(shut.clone());
        let mut stuff = Node::new("Stuff", false);
        stuff.icon = Some(shut.clone());
        let mut desktop = Node::new("Desktop", true);
        desktop.icon = Some(shut.clone());
        desktop.open_icon = Some(open.clone());
        desktop.children = vec![docs, stuff];
        desktop.expanded = true;
        let tv = TreeView {
            roots: vec![desktop],
            selected: Some(vec![0, 0]),
            style: Style::Picker,
            ..Default::default()
        };
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 100),
            &ControlMetrics::CLASSIC,
        );
        let mut c = Canvas::new(200, 100);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        let b = lay.body;
        let first = |argb: u32, k: i32| {
            (b.x..b.right())
                .find(|&x| (b.y + k * ROW_H..b.y + (k + 1) * ROW_H).any(|y| c.get(x, y) == argb))
                .map(|x| x - b.x)
        };
        let icon_open = Color::rgb(174, 218, 255).argb();
        let icon_shut = Color::rgb(255, 224, 123).argb();
        assert_eq!(first(icon_open, 0), Some(3), "the root's icon");
        assert_eq!(
            first(LINE_GRAY.argb(), 0),
            None,
            "no box or line on the root"
        );
        assert_eq!(
            first(cs.button_text.argb(), 0),
            Some(24),
            "the root's label"
        );
        assert_eq!(first(LINE_GRAY.argb(), 1), Some(7), "the box's left edge");
        assert_eq!(
            c.get(b.x + 15, b.y + ROW_H + 8),
            LINE_GRAY.argb(),
            "its right edge"
        );
        assert_eq!(first(icon_shut, 1), Some(22), "the depth 1 icon");
        assert_eq!(first(cs.hilight.argb(), 1), Some(41), "the band");
        let (y0, y1) = (b.y + ROW_H, b.y + 2 * ROW_H);
        let label = (b.x + 41..b.right())
            .find(|&x| (y0..y1).any(|y| c.get(x, y) == cs.hilight_text.argb()))
            .map(|x| x - b.x);
        assert_eq!(label, Some(43), "the label");
        assert_eq!(
            first(LINE_GRAY.argb(), 2),
            None,
            "no line runs down to a leaf"
        );
    }

    /// A picked "My Documents" at depth 1: the band covers the 17 px row from 2 px before the label to 5 px past its text, capitals 5 px below the top, in `Plain`'s 18 px row as in 17 px ones.
    #[test]
    fn the_band_runs_5_px_past_the_label_and_the_label_sits_5_px_down() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        for style in [Style::Explorer, Style::Picker, Style::Classic, Style::Plain] {
            let mut desktop = Node::new("Desktop", true);
            desktop.children = vec![Node::new("My Documents", false)];
            desktop.expanded = true;
            let tv = TreeView {
                roots: vec![desktop],
                selected: Some(vec![0, 0]),
                style,
                ..Default::default()
            };
            let lay = layout(
                &tv,
                &text,
                px,
                Rect::new(0, 0, 200, 100),
                &ControlMetrics::CLASSIC,
            );
            let mut c = Canvas::new(200, 100);
            draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
            let b = lay.body;
            let row_h = tv.row_h();
            let (y0, y1) = (b.y + row_h, b.y + 2 * row_h);
            let lx = b.x + tv.label_x(1);
            let w = text.measure("My Documents", false, px);
            let band: Vec<i32> = (b.x..b.right())
                .filter(|&x| c.get(x, y0) == cs.hilight.argb())
                .collect();
            assert_eq!(
                (band[0], *band.last().unwrap()),
                (lx - 2, lx + w + 4),
                "{style:?}: the band's columns"
            );
            assert_eq!(
                c.get(lx, y1 - 1),
                cs.hilight.argb(),
                "{style:?}: the band's last row"
            );
            let ink_top = (y0..y1)
                .find(|&y| (lx..lx + w).any(|x| c.get(x, y) == cs.hilight_text.argb()))
                .expect("the label inks");
            assert_eq!(ink_top - y0, 5, "{style:?}: the capitals' top");
        }
    }

    /// `Style::Classic` keeps its lines and root expander column; the `Style::Explorer` geometry is opt-in.
    #[test]
    fn the_classic_style_keeps_its_lines_and_root_box() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let tv = tree();
        assert_eq!(tv.style, Style::Classic, "the default");
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let mut c = Canvas::new(200, 300);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        let b = lay.body;
        let mid = b.y + ROW_H / 2;
        assert_eq!(c.get(b.x + BOX_X, mid - 4), LINE_GRAY.argb());
        let (y0, y1) = (b.y + 3 * ROW_H, b.y + 4 * ROW_H);
        assert!(
            (b.x + INDENT..b.x + 2 * INDENT)
                .any(|x| (y0..y1).any(|y| c.get(x, y) == LINE_GRAY.argb()))
        );
    }

    /// A click in the shell's trees selects and opens a row, closing the old selection and its ancestors up to the first the new row sits under, and asks for a never-opened row's children. A connector-line tree only selects.
    #[test]
    fn a_click_opens_the_row_and_closes_the_old_branch() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut tv = tree();
        tv.style = Style::Explorer;
        let lay = layout(
            &tv,
            &text,
            11,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        tv.roots[0].children[1].children[0].children = vec![Node::new("App", false)];
        tv.roots[0].children[1].children[0].expanded = true;
        tv.selected = Some(vec![0, 1, 0]);
        let row = |tv: &TreeView, path: &[usize]| {
            tv.visible()
                .iter()
                .position(|r| r.path == path)
                .expect("a visible row")
        };
        let i = row(&tv, &[0, 0]);
        assert_eq!(tv.click_row(i, &lay), Some(vec![0, 0]), "fill it");
        assert_eq!(tv.selected, Some(vec![0, 0]));
        let open = |tv: &TreeView, p: &[usize]| tv.node(p).unwrap().expanded;
        assert!(open(&tv, &[0, 0]), "My Documents opens");
        assert!(!open(&tv, &[0, 1, 0]), "Programs closes");
        assert!(!open(&tv, &[0, 1]), "and Local Disk above it");
        assert!(open(&tv, &[0]), "the Desktop holds both");
        tv.roots[0].children[0].children = vec![Node::new("Music", true)];
        let i = row(&tv, &[0, 0, 0]);
        assert_eq!(tv.click_row(i, &lay), Some(vec![0, 0, 0]));
        assert!(open(&tv, &[0, 0]), "the parent stays open");
        let i = row(&tv, &[0, 2]);
        assert_eq!(tv.click_row(i, &lay), None);
        assert_eq!(tv.selected, Some(vec![0, 2]));
        assert!(!open(&tv, &[0, 0]), "the old branch closed");

        let mut classic = tree();
        classic.selected = Some(vec![0, 1]);
        assert_eq!(classic.click_row(1, &lay), None);
        assert_eq!(classic.selected, Some(vec![0, 0]));
        assert!(!open(&classic, &[0, 0]), "a Classic row does not open");
        assert!(open(&classic, &[0, 1]), "nor the old one close");
    }

    /// A click that closes a long branch above the view scrolls back so the view ends at the last row.
    #[test]
    fn a_click_that_closes_a_long_branch_keeps_the_view_full() {
        let mut b = Node::new("B", true);
        b.expanded = true;
        b.children = (0..25)
            .map(|k| Node::new(&format!("b{k}"), false))
            .collect();
        let mut a = Node::new("A", true);
        a.expanded = true;
        a.children = vec![b, Node::new("C", false)];
        let mut tv = TreeView::new(vec![a], Style::Explorer);
        let lay = layout_without_text(&tv, Rect::new(0, 0, 200, 340), &ControlMetrics::CLASSIC);
        assert_eq!(lay.rows, 20);
        tv.top = 8;
        tv.selected = Some(vec![0, 0]);
        let c = tv.visible().iter().position(|r| r.path == [0, 1]).unwrap();
        assert_eq!(tv.click_row(c, &lay), None);
        assert!(!tv.node(&[0, 0]).unwrap().expanded, "B closes");
        assert_eq!(tv.visible().len(), 3);
        assert_eq!(tv.top, 0, "A, B and C all show");
    }

    /// The pointer on a row's icon or label makes it hot; past the label, off the rows or in a non-tracking tree, nothing is.
    #[test]
    fn the_icon_or_label_under_the_pointer_is_hot() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut tv = tree();
        tv.style = Style::Picker;
        let lay = layout(
            &tv,
            &text,
            11,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let b = lay.body;
        let y = b.y + ROW_H + 8;
        let end = b.x + 43 + text.measure("My Documents", false, 11);
        assert!(tv.hover(&lay, &text, 11, b.x + 23, y), "on the icon");
        assert_eq!(tv.hot, Some(1));
        assert!(!tv.hover(&lay, &text, 11, end - 1, y), "still row 1");
        assert!(tv.hover(&lay, &text, 11, end, y), "past the label");
        assert_eq!(tv.hot, None);
        tv.hover(&lay, &text, 11, b.x + 23, y);
        assert!(tv.unhover());
        tv.style = Style::Classic;
        tv.hover(&lay, &text, 11, b.x + 50, y);
        assert_eq!(tv.hot, None, "a connector-line tree does not track");
    }

    /// A scroll with the pointer at rest takes the hot row away: nothing is hot until it moves.
    #[test]
    fn a_scroll_leaves_no_row_hot() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut b = Node::new("B", true);
        b.expanded = true;
        b.children = (0..30)
            .map(|k| Node::new(&format!("b{k}"), false))
            .collect();
        let mut tv = TreeView::new(vec![b], Style::Explorer);
        let lay = layout(
            &tv,
            &text,
            11,
            Rect::new(0, 0, 200, 120),
            &ControlMetrics::CLASSIC,
        );
        let y = lay.body.y + ROW_H + 8;
        assert!(tv.hover(&lay, &text, 11, lay.body.x + 30, y));
        assert!(tv.scroll(1, &lay));
        assert_eq!(tv.hot, None);
    }

    /// A hot label is drawn in `hot_light` and underlined 2 px below its ink's lowest row, as wide as its text; on the selection's band the underline takes the band's ink.
    #[test]
    fn a_hot_label_is_underlined_in_the_hot_colour() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut tv = tree();
        tv.style = Style::Picker;
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let shot = |tv: &TreeView| {
            let mut c = Canvas::new(200, 300);
            draw(&mut c, &text, &classic_paint(cs, 1), tv, &lay, None, px);
            c
        };
        let b = lay.body;
        let (label, y) = (b.x + 43, b.y + 2 * ROW_H);
        let w = text.measure("Local Disk", false, px);
        let plain = shot(&tv);
        let l_w = text.advance('L', false, px);
        let l_bottom = (y..y + ROW_H)
            .filter(|&yy| (label..label + l_w).any(|x| plain.get(x, yy) == cs.button_text.argb()))
            .max()
            .expect("the L inks");
        tv.hot = Some(2);
        let c = shot(&tv);
        let ul = l_bottom + 2;
        let hot = cs.hot_light.argb();
        assert!(
            (label..label + w).all(|x| c.get(x, ul) == hot),
            "the underline at y {ul}"
        );
        assert_eq!(c.get(label + w, ul), cs.window.argb(), "no further");
        assert_eq!(c.get(label - 1, ul), cs.window.argb());
        assert!(
            (y..ul).any(|yy| (label..label + l_w).any(|x| c.get(x, yy) == hot)),
            "the label inks in the hot colour"
        );
        assert!(
            !(y..y + ROW_H)
                .any(|yy| (label..label + w).any(|x| c.get(x, yy) == cs.button_text.argb())),
            "and not in the plain one"
        );
        tv.selected = Some(vec![0, 1]);
        let c = shot(&tv);
        assert_eq!(c.get(label, ul), cs.hilight_text.argb(), "on the band");
    }

    /// The rename box over "Programs" (picker style, depth 2, label at
    /// body.x + 62): a 1 px black ring the row's 17 px tall from 5 px left of the
    /// label, the name on the highlight's 13 rows from the box's third, the
    /// caret after it and 15 px of white to the right edge.
    #[test]
    fn the_rename_box_rings_the_row_and_keeps_the_labels_place() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut tv = tree();
        tv.style = Style::Picker;
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let shot = |tv: &TreeView| {
            let mut c = Canvas::new(200, 300);
            draw(&mut c, &text, &classic_paint(cs, 1), tv, &lay, None, px);
            c
        };
        let plain = shot(&tv);
        tv.begin_edit(vec![0, 1, 0]);
        let c = shot(&tv);
        let b = lay.body;
        let (label, y) = (b.x + 62, b.y + 3 * ROW_H);
        let w = text.measure("Programs", false, px);
        let r = tv.edit_rect(&lay, &text, px).expect("the box is in sight");
        assert_eq!(r, Rect::new(label - 5, y, 5 + w + 15, ROW_H), "the box");
        let black = cs.window_frame.argb();
        assert_eq!(c.get(r.x, y + 8), black, "left ring");
        assert_eq!(c.get(r.right() - 1, y + 8), black, "right ring");
        assert_eq!(c.get(r.x + 3, y), black, "top ring on the row's top");
        assert_eq!(c.get(r.x + 3, y + ROW_H - 1), black, "bottom ring");
        assert_eq!(c.get(label - 1, y + 2), cs.window.argb(), "white before");
        assert_eq!(c.get(label, y + 2), cs.hilight.argb(), "highlight top");
        assert_eq!(c.get(label, y + 14), cs.hilight.argb(), "highlight bottom");
        assert_eq!(c.get(label, y + 15), cs.window.argb(), "white under it");
        assert_eq!(c.get(label + w, y + 8), black, "the caret after the name");
        assert_eq!(c.get(r.right() - 3, y + 8), cs.window.argb());
        let rows = |c: &Canvas, ink: u32| {
            (y + 2..y + 15)
                .filter(|&yy| (label..label + w).any(|x| c.get(x, yy) == ink))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(&c, cs.hilight_text.argb()),
            rows(&plain, cs.button_text.argb()),
            "the name inks the label's rows"
        );
    }

    /// Scrolled to "Programs", the first row still carries "Local Disk"'s line down to "Recycle Bin", 27 px into the body.
    #[test]
    fn a_scrolled_classic_tree_keeps_its_ancestors_lines() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut tv = tree();
        tv.top = 3;
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 300),
            &ControlMetrics::CLASSIC,
        );
        let mut c = Canvas::new(200, 300);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &tv,
            &lay,
            None,
            px,
        );
        let b = lay.body;
        let x = b.x + INDENT + BOX_X + BOX / 2;
        assert!(
            (b.y..b.y + ROW_H).any(|y| c.get(x, y) == LINE_GRAY.argb()),
            "a dotted line at x {x} through the first row"
        );
    }

    /// Writes target/treeview-preview.png: `Style::Explorer` left, `Style::Classic` right.
    #[test]
    #[ignore]
    fn render_treeview() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let (icon, open) = icons();
        let mut c = Canvas::new(410, 120);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        for (col, style) in [Style::Explorer, Style::Classic].into_iter().enumerate() {
            let mut tv = tree();
            tv.style = style;
            for n in &mut tv.roots[0].children {
                n.icon = Some(icon.clone());
                n.open_icon = Some(open.clone());
            }
            tv.roots[0].children[1].label = "Local Disk: a name long enough to overflow".into();
            tv.selected = Some(vec![0, 0]);
            tv.focus_ring = true;
            tv.drop_hilight = Some(3);
            let r = Rect::new(5 + col as i32 * 205, 5, 195, 110);
            let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
            draw(
                &mut c,
                &text,
                &classic_paint(&theme.colors, 1),
                &tv,
                &lay,
                None,
                px,
            );
        }
        crate::raster::write_preview(&c, "treeview-preview.png");
    }

    #[test]
    fn a_blue_trees_lit_bar_part_draws_hot() {
        use crate::classic::widgets::{Axis, ScrollHit};
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = theme.look.controls();
        let text = TextRenderer::load(&FontConfig::default());
        let mut tv = TreeView {
            roots: (0..40)
                .map(|i| Node::new(&format!("Folder {i}"), false))
                .collect(),
            ..Default::default()
        };
        let r = Rect::new(0, 0, 150, 120);
        let lay = layout(&tv, &text, 11, r, &cm);
        let sb = lay.scrollbar.expect("40 rows overflow the tree");
        let down = tv.hit(&lay, sb.x + 8, sb.bottom() - 3).bar();
        assert_eq!(down, Some((Axis::Vertical, ScrollHit::Forward)));
        tv.bars.motion(down);
        let mut c = Canvas::new(150, 120);
        draw(&mut c, &text, &theme.paint(), &tv, &lay, tv.bars.lit(), 11);
        assert_eq!(
            c.get(sb.x + 8, sb.bottom() - 17 + 2),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowDown,
                crate::style::State::Hot,
                8,
                2
            ),
            "the hot down arrow"
        );
    }
}
