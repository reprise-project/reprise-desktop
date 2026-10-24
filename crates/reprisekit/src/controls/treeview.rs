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
    /// The row pitch, grown with the text ([`TreeView::row_h`]).
    pub row_h: i32,
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

    /// Scroll visible row `i` into view without picking it. True when it moved.
    pub fn reveal(&mut self, i: usize, lay: &Layout) -> bool {
        if i >= self.visible().len() {
            return false;
        }
        let mut top = self.top;
        scroll_into_view(&mut top, i, lay.rows);
        self.set_top(top.min(self.max_top(lay)))
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
        let row_h = lay.row_h;
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

    /// The row pitch with text grown by `cm.text`: `Style::Plain`'s list is a px taller than the
    /// trees with expander boxes.
    pub fn row_h(&self, cm: &ControlMetrics) -> i32 {
        cm.text.len(match self.style {
            Style::Plain => PLAIN_ROW_H,
            _ => ROW_H,
        })
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
            .then(|| self.top + ((y - lay.body.y) / lay.row_h) as usize)
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
        let row_h = lay.row_h;
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
        scrollbar::hparts(
            hb,
            self.left_in(lay),
            lay.body.width,
            lay.content_w,
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
                scrollbar::left_at(&p, x, grab, lay.body.width, lay.content_w)
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
    let row_h = tv.row_h(cm);
    // Reserving one bar can make the other necessary and neither is given back, so two rounds reach the fixed point (as in `listview`).
    let (mut vbar, mut hbar) = (false, false);
    for _ in 0..2 {
        let w = inner.width - if vbar { cm.scroll } else { 0 };
        let h = inner.height - if hbar { cm.scroll } else { 0 };
        hbar |= content_w > w;
        vbar |= visible.len() > (h / row_h).max(1) as usize;
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
        rows: (body.height / row_h).max(1) as usize,
        scrollbar: vbar.then(|| Rect::new(body.right(), body.y, cm.scroll, body.height)),
        hscroll: hbar.then(|| Rect::new(body.x, body.bottom(), body.width, cm.scroll)),
        content_w,
        thumb_min: cm.thumb_min,
        row_h,
    }
}

/// The dots of a dotted line `r`: every other `s`-px cell on a grid shared by all lines.
fn dotted(c: &mut Canvas, r: Rect, s: i32, ink: Color) {
    for y in (r.y..r.bottom()).step_by(s as usize) {
        for x in (r.x..r.right()).step_by(s as usize) {
            if ((x / s) + (y / s)) % 2 == 0 {
                c.fill_rect(Rect::new(x, y, s, s), ink);
            }
        }
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
    let row_h = lay.row_h * s;
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
                    dotted(&mut bc, Rect::new(lx, y, s, row_h), s, line);
                }
            }
            let top = if i == 0 && row.depth == 0 { mid } else { y };
            let bottom = if row.last { mid } else { y + row_h };
            dotted(&mut bc, Rect::new(cx, top, s, bottom - top), s, line);
            dotted(&mut bc, Rect::new(cx, mid, ix - cx, s), s, line);
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
/// view lie above or below the frame. Returns the node that has the focus when the tree
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
    let rh = lay.row_h;
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
        let y = lay.body.y + (i as i32 - tv.top as i32) * rh;
        let x = lay.body.x - tv.left_in(lay) + tv.label_x(row.depth);
        let r = Rect::new(x, y, (lay.body.right() - x).max(0), rh);
        item.set_bounds(access::row_bounds(lay.body, r));
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
mod tests;
