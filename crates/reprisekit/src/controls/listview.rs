//! The list view in Details mode (Explorer's right pane): a header of column buttons over 17 px rows
//! (16 px icon plus cells), scrollbars when rows or columns outgrow the box, inside a 2 px sunken edge.
//! The host owns the rows and the icons.
//!
//! - Header strip: the look's height (17 in Classic); title ink 6 px in (`HEADER_TEXT_X`).
//! - Sort arrow: 9 px past the title's advance. A right-aligned title moves left to make room, the
//!   arrow stopping `ControlMetrics::header_arrow_end` short of the column's end (7 px Classic, 2 px
//!   Blue). With `header_arrow_over` the arrow is centred over the column and the title stays put.
//!   Classic draws a solid `button_shadow` triangle, 7x4, apex up, 6 rows under the header's top.
//! - Horizontal scrollbar: `ControlMetrics::scroll` thick, flush under the body.
//! - In-place rename box: 146x17 for a 126 px name, 1 px `window_frame` border on `window`; the border
//!   sits 3 px left of the name's origin and the text 2 px right of it ([`ListView::edit_rect`]).
//! - A look that boxes its items (`look::widgets::item`) boxes a selected, hot ([`ListView::hot`]) or
//!   focused row from the first column to the body's right edge and one row past its foot, shared with
//!   the next row's box; text stays black. The lead row draws darker with focus; without it
//!   ([`ListView::inactive`]) the selection greys.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    Axis, BarHit, ScrollHit, ScrollParts, axis_lit, bar_hit, draw_edit, draw_focus_ring,
    edit_first_visible, scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar;
use crate::look::Paint;
use crate::look::widgets::header as header_widgets;
use crate::look::widgets::item::{self, Mark};
use crate::look::widgets::{draw_checkbox, draw_field_frame, draw_hscrollbar, draw_scrollbar};
use crate::pointer::{Lit, Pointer};
use crate::raster::{Canvas, Rect};
use crate::selection::{Mods, Selection, step_index};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, TextEdit};
use crate::theme::{Color, ColorScheme};
use accesskit::{Node, NodeId, Role};
use std::borrow::{Borrow, Cow};

/// The header strip's height in Classic; a look sets `ControlMetrics::list_header`.
pub const HEADER_H: i32 = 17;
pub const ROW_H: i32 = 17;
/// A group header's slot: the rule 18 px down, the group's first row 31 px down.
pub const GROUP_H: i32 = 31;
/// The band the title is centred in, the rule on its last row.
const GROUP_TITLE_H: i32 = 19;
pub const ICON_PX: i32 = 16;
/// The name column: icon 2 px in, text 2 px after it; other columns' text and the header text 6 px in.
const ICON_X: i32 = 2;
const NAME_X: i32 = ICON_X + ICON_PX + 2;
const CELL_X: i32 = 6;
const HEADER_TEXT_X: i32 = 6;
/// A check column's box: the look's check box side (13 px), 8 px from the frame edge, 3 px below the
/// row top; it replaces the icon and the first cell's text starts 7 px after it.
const CHECK_X: i32 = 8;
const CHECK_Y: i32 = 3;
const CHECK_TEXT_GAP: i32 = 7;

/// How far either side of a column's right edge the pointer grabs the divider.
pub const DIVIDER_GRAB: i32 = 4;
/// The narrowest a drag may leave a column, so an edge stays to grab.
pub const MIN_COL_W: i32 = 20;

/// How far the sorted column's body tint moves the window colour toward the window text, in 255ths
/// (white becomes 247 grey). Off by default: only Explorer's Details view tints.
const SORT_TINT: u8 = 8;

const ARROW_GAP: i32 = 9;

pub(crate) const EDIT_BACK: i32 = 3;
pub(crate) const EDIT_PAD_L: i32 = 5;
pub(crate) const EDIT_PAD_R: i32 = 15;
pub(crate) const EDIT_H: i32 = ROW_H;
/// How far the editable area sits inside the border on the top, bottom and right.
const EDIT_INSET: i32 = 2;

/// A group header's title inset from the body's left edge; bold, `window_text`.
const GROUP_TEXT_X: i32 = 14;
/// Px the title sits below a centred line in its band.
const GROUP_TEXT_DY: i32 = 1;
/// The group rule's left colour; fades linearly to white over `GROUP_LINE_W` px (not a scheme colour).
const GROUP_LINE: Color = Color::rgb(166, 202, 240);
const GROUP_LINE_W: i32 = 300;
pub const GROUP_RULE_Y: i32 = GROUP_TITLE_H - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub title: String,
    pub width: i32,
    pub right: bool,
}

impl Column {
    pub fn new(title: &str, width: i32) -> Column {
        Column {
            title: title.into(),
            width,
            right: false,
        }
    }
}

/// One row: its icon (16 px, already sized) and a cell per column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub icon: Option<std::rc::Rc<Canvas>>,
    pub cells: Vec<String>,
}

/// What the view reads of its rows. A host with its own rows implements this over a borrow instead of
/// copying icons and cells into `Row`s per call.
pub trait Rows {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn icon(&self, row: usize) -> Option<&Canvas>;
    fn cell(&self, row: usize, col: usize) -> Cow<'_, str>;
}

impl Rows for Vec<Row> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn icon(&self, row: usize) -> Option<&Canvas> {
        self.get(row).and_then(|r| r.icon.as_deref())
    }
    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        Cow::Borrowed(
            self.get(row)
                .and_then(|r| r.cells.get(col))
                .map_or("", String::as_str),
        )
    }
}

/// A run of rows under a heading. The header takes a [`GROUP_H`] slot before `first`; a list's groups
/// are sorted by `first`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub first: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Header(usize),
    Row(usize),
}

/// The view's state. `C` and `S` hold the columns and selection: owned by default, borrowed by a host
/// that only lays out, hit-tests or draws through a transient view (resizing and selecting need owned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListView<R: Rows = Vec<Row>, C = Vec<Column>, S = Selection> {
    pub columns: C,
    pub rows: R,
    pub selection: S,
    pub top: usize,
    /// Px the columns are scrolled left; read through [`ListView::left_in`], which clamps it.
    pub left: i32,
    /// The sorted column and whether ascending; it gets the arrow.
    pub sort: Option<(usize, bool)>,
    /// Whether the sorted column's body cells sit on the window colour tinted by [`SORT_TINT`].
    pub sort_tint: bool,
    /// Whether the dotted focus rectangle shows (once the keyboard has been used).
    pub focus_ring: bool,
    pub edit: Option<(usize, TextEdit)>,
    /// The row a drag hovers: highlighted like a selected one, never focused.
    pub drop_row: Option<usize>,
    /// A check box per row; `None` for a plain list.
    pub checks: Option<Vec<bool>>,
    /// The row whose check box the pointer holds: pressed while over it, toggles on release over it.
    pub held_check: Pointer<usize>,
    /// The groups, sorted by `first`; empty for an ungrouped list.
    pub groups: Vec<Group>,
    /// The 2 px sunken border. On by default; Network Connections' list is borderless.
    pub frame: bool,
    /// The scrollbar part the pointer rests on or holds; the host tracks it, `draw` lights it.
    pub bars: Pointer<(Axis, ScrollHit)>,
    /// The header column the pointer rests on or holds (see [`Hit::column`]); the host tracks it.
    pub header: Pointer<usize>,
    /// The row under the pointer ([`ListView::hover`]), lit by a look that boxes its items.
    pub hot: Option<usize>,
    /// The list lacks the focus: a boxing look greys the selection.
    pub inactive: bool,
}

impl<R: Rows + Default> Default for ListView<R> {
    fn default() -> ListView<R> {
        ListView {
            columns: Vec::new(),
            rows: R::default(),
            selection: Selection::default(),
            top: 0,
            left: 0,
            sort: None,
            sort_tint: false,
            focus_ring: false,
            edit: None,
            drop_row: None,
            checks: None,
            held_check: Pointer::new(),
            groups: Vec::new(),
            frame: true,
            bars: Pointer::new(),
            header: Pointer::new(),
            hot: None,
            inactive: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Header(usize),
    Divider(usize),
    Row(usize),
    Background,
    Check(usize),
    /// The vertical bar: a page is `rows`; `top_at` maps a thumb grab and pointer y to the row.
    VBar(BarHit),
    /// The horizontal bar: a page is `body.width` px; `left_at` maps a thumb grab and pointer x to the offset.
    HBar(BarHit),
    Nothing,
}

impl Hit {
    /// The header column this lands on ([`ListView::header`]): a title, not the divider beside it.
    pub fn column(self) -> Option<usize> {
        match self {
            Hit::Header(i) => Some(i),
            _ => None,
        }
    }

    pub fn bar(self) -> Option<(Axis, ScrollHit)> {
        match self {
            Hit::VBar(b) => Some((Axis::Vertical, b.part()?)),
            Hit::HBar(b) => Some((Axis::Horizontal, b.part()?)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub header: Rect,
    pub body: Rect,
    pub rows: usize,
    pub scrollbar: Option<Rect>,
    pub hscroll: Option<Rect>,
    pub total_w: i32,
    /// The look's shortest scrollbar thumb.
    pub thumb_min: i32,
    /// The look's check box side.
    pub check: i32,
}

pub fn layout<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>>(
    lv: &ListView<R, C, S>,
    r: Rect,
    cm: &ControlMetrics,
) -> Layout {
    let inner = if lv.frame { r.inset(2) } else { r };
    let header = Rect::new(inner.x, inner.y, inner.width, cm.list_header);
    let (full_w, full_h) = (inner.width, inner.height - cm.list_header);
    let total_w: i32 = lv.columns.as_ref().iter().map(|c| c.width).sum();
    let content_h: i32 = if lv.groups.is_empty() {
        lv.rows.len() as i32 * ROW_H
    } else {
        lv.slots_from(0).map(slot_h).sum()
    };
    // Reserving one bar can make the other necessary and neither is given back: two rounds converge.
    let (mut vbar, mut hbar) = (false, false);
    for _ in 0..2 {
        let w = full_w - if vbar { cm.scroll } else { 0 };
        let h = full_h - if hbar { cm.scroll } else { 0 };
        hbar |= total_w > w;
        vbar |= content_h > h.max(ROW_H);
    }
    let body = Rect::new(
        inner.x,
        header.bottom(),
        full_w - if vbar { cm.scroll } else { 0 },
        full_h - if hbar { cm.scroll } else { 0 },
    );
    let rows = lv.visible(body.height);
    Layout {
        frame: r,
        header,
        body,
        rows,
        scrollbar: vbar.then(|| Rect::new(body.right(), body.y, cm.scroll, body.height)),
        hscroll: hbar.then(|| Rect::new(body.x, body.bottom(), body.width, cm.scroll)),
        total_w,
        thumb_min: cm.thumb_min,
        check: cm.check,
    }
}

fn check_box(lay: &Layout, at: i32) -> Rect {
    Rect::new(
        lay.frame.x + CHECK_X,
        lay.body.y + at + CHECK_Y,
        lay.check,
        lay.check,
    )
}

/// How tall a slot is: [`GROUP_H`] for a header, [`ROW_H`] for a row.
fn slot_h(slot: Slot) -> i32 {
    match slot {
        Slot::Header(_) => GROUP_H,
        Slot::Row(_) => ROW_H,
    }
}

impl<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>> ListView<R, C, S> {
    /// The slots from row `from` to the last, in drawing order; a group's header precedes its first row.
    fn slots_from(&self, from: usize) -> impl Iterator<Item = Slot> + '_ {
        let mut g = self.groups.partition_point(|g| g.first < from);
        (from..self.rows.len()).flat_map(move |i| {
            let head = self
                .groups
                .get(g)
                .is_some_and(|x| x.first == i)
                .then_some(Slot::Header(g));
            while self.groups.get(g).is_some_and(|x| x.first <= i) {
                g += 1;
            }
            head.into_iter().chain(std::iter::once(Slot::Row(i)))
        })
    }

    /// The same slots with their y offset (px) from the first one's top.
    fn slots_at(&self, from: usize) -> impl Iterator<Item = (Slot, i32)> + '_ {
        let mut y = 0;
        self.slots_from(from).map(move |s| {
            let at = y;
            y += slot_h(s);
            (s, at)
        })
    }

    /// How many slot positions `height` px hold from `top`: the slots, then empty [`ROW_H`] positions
    /// once they run out.
    fn visible(&self, height: i32) -> usize {
        let mut slots = self.slots_from(self.top);
        let (mut y, mut n) = (0, 0);
        loop {
            let h = slots.next().map_or(ROW_H, slot_h);
            if y + h > height {
                return n.max(1);
            }
            y += h;
            n += 1;
        }
    }

    /// How many slots the list lays out: its rows plus one per group header.
    pub fn slots(&self) -> usize {
        self.rows.len()
            + self
                .groups
                .iter()
                .filter(|g| g.first < self.rows.len())
                .count()
    }

    /// How many slots lie above `top`: rows plus headers of groups starting before it. The vertical bar counts slots.
    pub fn top_slot(&self) -> usize {
        self.top + self.groups.iter().filter(|g| g.first < self.top).count()
    }

    /// Where `row` sits in the body (px, negative above its top). A group starting exactly at `top`
    /// pushes its first row down by its header.
    fn slot_y(&self, row: usize) -> i32 {
        let heads = |lo: usize, hi: usize| {
            self.groups
                .iter()
                .filter(|g| (lo..=hi).contains(&g.first))
                .count() as i32
        };
        if row >= self.top {
            (row - self.top) as i32 * ROW_H + heads(self.top, row) * GROUP_H
        } else {
            -((self.top - row) as i32 * ROW_H + heads(row + 1, self.top) * GROUP_H)
        }
    }

    /// The largest `top` that still fills the body.
    fn max_top(&self, lay: &Layout) -> usize {
        if self.groups.is_empty() {
            return self.rows.len().saturating_sub(lay.rows);
        }
        (0..self.rows.len())
            .rev()
            .take_while(|&r| self.slots_from(r).map(slot_h).sum::<i32>() <= lay.body.height)
            .last()
            .unwrap_or(self.rows.len().saturating_sub(1))
    }

    pub fn follow(&mut self, lay: &Layout) {
        let Some(i) = self.selection.borrow().focus else {
            return;
        };
        if i < self.top {
            self.top = i;
            return;
        }
        // Walk up from the focused row adding each row and its header until one no longer fits; the row
        // below it is the new top. The focused row always shows.
        let mut g = self.groups.partition_point(|g| g.first <= i);
        let (mut t, mut used) = (i, 0);
        loop {
            let head = g > 0 && self.groups[g - 1].first == t;
            while g > 0 && self.groups[g - 1].first >= t {
                g -= 1;
            }
            let need = ROW_H + if head { GROUP_H } else { 0 };
            if t < i && used + need > lay.body.height {
                t += 1;
                break;
            }
            used += need;
            if t == self.top {
                break;
            }
            t -= 1;
        }
        self.top = t;
    }

    /// The pointer moved to (`x`, `y`): the row under it turns hot. True when a boxing look's drawing changed.
    pub fn hover(&mut self, lay: &Layout, x: i32, y: i32) -> bool {
        let hot = match self.hit(lay, x, y) {
            Hit::Row(i) => Some(i),
            _ => None,
        };
        std::mem::replace(&mut self.hot, hot) != hot
    }

    /// What (`x`, `y`) lands on. A divider wins over the header buttons it borders, inside the header strip only.
    pub fn hit(&self, lay: &Layout, x: i32, y: i32) -> Hit {
        if lay.header.contains(x, y) {
            let mut cx = lay.header.x - self.left_in(lay);
            for (i, col) in self.columns.as_ref().iter().enumerate() {
                let edge = cx + col.width;
                if (x - edge).abs() <= DIVIDER_GRAB {
                    return Hit::Divider(i);
                }
                if x < edge {
                    return Hit::Header(i);
                }
                cx = edge;
            }
            return Hit::Nothing;
        }
        if let Some(sb) = lay.scrollbar.filter(|sb| sb.contains(x, y)) {
            let p = scrollbar_parts(
                sb,
                1,
                self.top_slot(),
                lay.rows,
                self.slots(),
                false,
                lay.thumb_min,
            );
            return Hit::VBar(bar_hit(&p, x, y, false));
        }
        if let Some(hb) = lay.hscroll.filter(|hb| hb.contains(x, y)) {
            return Hit::HBar(bar_hit(&self.hparts(lay, hb), x, y, true));
        }
        if lay.body.contains(x, y) {
            let dy = y - lay.body.y;
            for (slot, at) in self.slots_at(self.top).take(lay.rows) {
                if dy >= at + slot_h(slot) {
                    continue;
                }
                let Slot::Row(i) = slot else {
                    return Hit::Background;
                };
                if self.checks.is_some() && check_box(lay, at).contains(x, y) {
                    return Hit::Check(i);
                }
                return Hit::Row(i);
            }
            return Hit::Background;
        }
        Hit::Nothing
    }

    /// Row `row`'s name as `draw` shows it, at 1x: the ink cut by the Name column, 2 px of highlight
    /// either side, never past the column's right edge.
    pub fn name_rect(&self, lay: &Layout, row: usize, text: &TextRenderer, px: u32) -> Rect {
        let y = lay.body.y + self.slot_y(row);
        let cx = lay.body.x - self.left_in(lay);
        let cw = self.columns.as_ref().first().map_or(0, |c| c.width);
        let tx = match &self.checks {
            Some(_) => lay.frame.x + CHECK_X + lay.check + CHECK_TEXT_GAP,
            None => cx + NAME_X,
        };
        let name = self.rows.cell(row, 0);
        let shown = text.ellipsize(&name, (cx + cw - tx - 2).max(0), false, px);
        let right = (tx + text.measure(&shown, false, px) + 2).min(cx + cw);
        Rect::new(tx - 2, y, (right - tx + 2).max(0), ROW_H)
    }

    /// The rename box over `row`'s name cell, border included, sized to its text.
    pub fn edit_rect(&self, lay: &Layout, row: usize, text: &TextRenderer, px: u32) -> Rect {
        let shown = match &self.edit {
            Some((i, ed)) if *i == row => ed.text.as_str(),
            _ => &self.rows.cell(row, 0),
        };
        let y = lay.body.y + self.slot_y(row);
        let tx = lay.body.x - self.left_in(lay) + NAME_X;
        let w = text.measure(shown, false, px);
        Rect::new(tx - EDIT_BACK, y, w + EDIT_PAD_L + EDIT_PAD_R, EDIT_H)
    }

    /// The top row a thumb grabbed `grab` px below its top scrolls to when the pointer is at `y`, at
    /// most `max_top`. The bar counts slots, so a header's slot stands for its group's first row.
    pub fn top_at(&self, lay: &Layout, y: i32, grab: i32) -> usize {
        let Some(sb) = lay.scrollbar else {
            return self.top;
        };
        let p = scrollbar_parts(
            sb,
            1,
            self.top_slot(),
            lay.rows,
            self.slots(),
            false,
            lay.thumb_min,
        );
        let slot = scrollbar_pos_at(&p, y - grab, lay.rows, self.slots(), false);
        let row = match self.slots_from(0).nth(slot) {
            Some(Slot::Header(g)) => self.groups[g].first,
            Some(Slot::Row(i)) => i,
            None => self.rows.len(),
        };
        row.min(self.max_top(lay))
    }

    pub fn scroll(&mut self, by: i64, lay: &Layout) -> bool {
        let max = self.max_top(lay) as i64;
        let top = (self.top as i64 + by).clamp(0, max) as usize;
        let changed = top != self.top;
        self.top = top;
        changed
    }

    pub fn hscroll(&mut self, by: i32, lay: &Layout) -> bool {
        let from = self.left_in(lay);
        self.left = scrollbar::held(from + by, lay.body.width, lay.total_w);
        self.left != from
    }

    /// `left` held to what overflows the body, so after a narrowing the last column's end stays at the body's.
    pub fn left_in(&self, lay: &Layout) -> i32 {
        scrollbar::held(self.left, lay.body.width, lay.total_w)
    }

    fn hparts(&self, lay: &Layout, hb: Rect) -> ScrollParts {
        scrollbar_parts(
            hb,
            1,
            self.left_in(lay) as usize,
            lay.body.width.max(0) as usize,
            lay.total_w.max(0) as usize,
            true,
            lay.thumb_min,
        )
    }

    /// The offset a horizontal thumb grabbed `grab` px right of its left edge scrolls to at pointer `x`.
    pub fn left_at(&self, lay: &Layout, x: i32, grab: i32) -> i32 {
        match lay.hscroll {
            Some(hb) => {
                let p = self.hparts(lay, hb);
                let visible = lay.body.width.max(0) as usize;
                scrollbar_pos_at(&p, x - grab, visible, lay.total_w.max(0) as usize, true) as i32
            }
            None => self.left_in(lay),
        }
    }

    /// Row `row`'s check box in control space: [`CHECK_X`] from the frame edge, [`CHECK_Y`] below the row top.
    pub fn check_rect(&self, lay: &Layout, row: usize) -> Rect {
        check_box(lay, self.slot_y(row))
    }

    /// Flip row `row`'s check state; `false` with no check column or `row` out of range.
    pub fn toggle(&mut self, row: usize) -> bool {
        let Some(checks) = self.checks.as_mut() else {
            return false;
        };
        let Some(c) = checks.get_mut(row) else {
            return false;
        };
        *c = !*c;
        true
    }

    /// A press on `hit`: a row's check box is held, drawn pressed, until the release ([`ListView::release_check`]).
    /// True when what draws changed.
    pub fn press_check(&mut self, hit: Hit) -> bool {
        self.held_check.press(Self::check_of(hit), false)
    }

    /// The pointer moved over `hit` with a box held: pressed only while over it. True when what draws changed.
    pub fn motion_check(&mut self, hit: Hit) -> bool {
        self.held_check.holding() && self.held_check.motion(Self::check_of(hit))
    }

    /// The pointer let go over `hit`: the held box toggles if the release lands on it. Returns the toggled row.
    pub fn release_check(&mut self, hit: Hit) -> Option<usize> {
        self.motion_check(hit);
        let row = self.held_check.release();
        self.held_check.clear();
        row.filter(|&i| self.toggle(i))
    }

    fn check_of(hit: Hit) -> Option<usize> {
        match hit {
            Hit::Check(i) => Some(i),
            _ => None,
        }
    }
}

impl<R: Rows> ListView<R> {
    pub fn select(&mut self, i: Option<usize>, lay: &Layout) {
        self.selection.set_only(i.filter(|&i| i < self.rows.len()));
        self.follow(lay);
    }

    pub fn resize_column(&mut self, i: usize, width: i32) {
        if let Some(col) = self.columns.get_mut(i) {
            col.width = width.max(MIN_COL_W);
        }
    }

    /// Move the focus by `step` rows (Home/End with a big step) and let `mods` decide the selection; shows the focus ring.
    pub fn step(&mut self, step: i64, lay: &Layout, mods: Mods) -> bool {
        let Some(to) = step_index(self.selection.focus, step, self.rows.len()) else {
            return false;
        };
        let changed = self.selection.focus != Some(to) || !self.focus_ring;
        self.selection.move_focus(to, mods);
        self.focus_ring = true;
        self.follow(lay);
        changed
    }
}

/// The header strip, clipped to itself so a scrolled column does not spill over the frame.
fn draw_header<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>>(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lv: &ListView<R, C, S>,
    header: Rect,
    left: i32,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    header_widgets::draw_strip(c, p, header);
    let arrow_w = header_widgets::sort_arrow_width(p);
    let arrow_end = p.look.controls().header_arrow_end;
    let arrow_over = p.look.controls().header_arrow_over;
    let mut cx = header.x - left * s;
    for (ci, col) in lv.columns.as_ref().iter().enumerate() {
        let r = Rect::new(cx, header.y, col.width * s, header.height);
        cx += col.width * s;
        if r.right() <= header.x || r.x >= header.right() {
            continue;
        }
        let sorted = lv.sort.filter(|&(i, _)| i == ci);
        let t = header_widgets::draw_item(c, p, r, lv.header.of(ci), sorted.is_some());
        // An arrow over the column centres there and leaves the title where it is unsorted.
        if let Some((_, asc)) = sorted.filter(|_| arrow_over) {
            let ax = r.x + (r.width / s - arrow_w + 1) / 2 * s;
            header_widgets::draw_sort_arrow(c, p, ax, r, asc);
        }
        let sorted = sorted.filter(|_| !arrow_over);
        // A right-aligned sorted column's arrow stops `header_arrow_end` short of the end and its title
        // ends the arrow gap before it; a left-aligned title keeps its place and the arrow follows.
        let right_arrow =
            (sorted.is_some() && col.right).then(|| t.right() - (arrow_end + arrow_w) * s);
        let tr_right = match right_arrow {
            Some(ax) => ax - ARROW_GAP * s,
            None if sorted.is_some() => t.right() - (HEADER_TEXT_X + ARROW_GAP + arrow_w) * s,
            None => t.right() - HEADER_TEXT_X * s,
        };
        let tx = t.x + HEADER_TEXT_X * s;
        let tr = Rect::new(tx, t.y, (tr_right - tx).max(0), t.height);
        let align = if col.right { Align::Right } else { Align::Left };
        let shown = text.ellipsize(&col.title, tr.width, false, px);
        text.draw(c, tr, &shown, false, cs.button_text, align, px);
        if let Some((_, asc)) = sorted {
            let ax = right_arrow
                .unwrap_or_else(|| tr.x + text.measure(&shown, false, px) + ARROW_GAP * s);
            header_widgets::draw_sort_arrow(c, p, ax, r, asc);
        }
    }
}

/// The rows, clipped to their box.
fn draw_rows<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>>(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lv: &ListView<R, C, S>,
    lay: &Layout,
    body: Rect,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(body, cs.window);
    if let Some((sorted, _)) = lv.sort.filter(|_| lv.sort_tint) {
        let mut cx = body.x - lv.left_in(lay) * s;
        for (ci, col) in lv.columns.as_ref().iter().enumerate() {
            if ci == sorted {
                let tint = cs.window.toward(cs.window_text, SORT_TINT);
                c.fill_rect(Rect::new(cx, body.y, col.width * s, body.height), tint);
                break;
            }
            cx += col.width * s;
        }
    }
    let row_h = ROW_H * s;
    for (slot, at) in lv.slots_at(lv.top).take(lay.rows) {
        let y = body.y + at * s;
        let i = match slot {
            Slot::Header(g) => {
                let r = Rect::new(body.x, y, body.width, GROUP_H * s);
                let title_box = (GROUP_TEXT_DY * s, GROUP_TITLE_H * s);
                let title = &lv.groups[g].title;
                draw_group_header(c, text, cs, r, title, title_box, &list_rule(cs), px, s);
                continue;
            }
            Slot::Row(i) => i,
        };
        let editing = matches!(&lv.edit, Some((e, _)) if *e == i);
        let sel = lv.selection.borrow();
        let selected = sel.is(i) || lv.drop_row == Some(i);
        let ring = lv.focus_ring && sel.focus == Some(i) && lv.drop_row != Some(i);
        let mut cx = body.x - lv.left_in(lay) * s;
        let boxed = item::draw_item(
            c,
            p,
            Rect::new(cx, y, body.right() - cx, row_h + s),
            Mark {
                selected,
                hot: lv.hot == Some(i) || lv.drop_row == Some(i),
                lead: sel.focus == Some(i),
                dim: lv.inactive,
                ring,
            },
        );
        for (ci, col) in lv.columns.as_ref().iter().enumerate() {
            let cell = lv.rows.cell(i, ci);
            let cw = col.width * s;
            let (tx, tw) = if ci == 0 {
                if let Some(checks) = &lv.checks {
                    let cr = check_box(lay, at).scaled(s);
                    let checked = checks.get(i).copied().unwrap_or(false);
                    draw_checkbox(c, p, cr, checked, lv.held_check.of(i));
                    let tx = cr.right() + CHECK_TEXT_GAP * s;
                    (tx, cx + cw - tx - 2 * s)
                } else {
                    if let Some(ic) = lv.rows.icon(i) {
                        c.blit_over(cx + ICON_X * s, y + (row_h - ic.height as i32) / 2, ic);
                    }
                    (cx + NAME_X * s, cw - NAME_X * s - 2 * s)
                }
            } else {
                (cx + CELL_X * s, cw - 2 * CELL_X * s)
            };
            cx += cw;
            if ci == 0 && editing {
                continue;
            }
            let tr = Rect::new(tx, y, tw.max(0), row_h);
            let shown = text.ellipsize(&cell, tr.width, false, px);
            let align = if col.right { Align::Right } else { Align::Left };
            if ci == 0 && (selected || ring) && !boxed {
                let w = text.measure(&shown, false, px);
                let hl = Rect::new(tx - 2 * s, y, w + 4 * s, row_h).intersect(&body);
                let ink = if selected {
                    c.fill_rect(hl, cs.hilight);
                    cs.hilight_text
                } else {
                    cs.button_text
                };
                text.draw(c, tr, &shown, false, ink, align, px);
                if ring {
                    draw_focus_ring(c, hl, s, ink);
                }
            } else {
                text.draw(c, tr, &shown, false, cs.button_text, align, px);
            }
        }
    }
    if let Some((i, ed)) = &lv.edit {
        let b = lv.edit_rect(lay, *i, text, px).scaled(s);
        draw_rename_box(c, text, p, ed, b, px);
    }
}

/// A group header's rule: `from` at the left edge fading linearly to `to` over `width` px.
pub(crate) struct GroupRule {
    pub from: Color,
    pub to: Color,
    pub width: i32,
}

pub(crate) fn list_rule(cs: &ColorScheme) -> GroupRule {
    GroupRule {
        from: GROUP_LINE,
        to: cs.window,
        width: GROUP_LINE_W,
    }
}

/// A group header in the already-scaled `r`: bold title [`GROUP_TEXT_X`] in from the left edge in
/// `title_box` (top offset, height), then `rule` [`GROUP_RULE_Y`] down. Shared with the shell view's icon modes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_group_header(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    r: Rect,
    title: &str,
    title_box: (i32, i32),
    rule: &GroupRule,
    px: u32,
    s: i32,
) {
    let tr = Rect::new(
        r.x + GROUP_TEXT_X * s,
        r.y + title_box.0,
        (r.width - GROUP_TEXT_X * s).max(0),
        title_box.1,
    );
    let shown = text.ellipsize(title, tr.width, true, px);
    text.draw(c, tr, &shown, true, cs.window_text, Align::Left, px);
    let ly = r.y + GROUP_RULE_Y * s;
    let span = rule.width.max(1);
    // Not `Canvas::hgradient`: that spreads the ramp over the rect and rounds; this runs at a fixed rate and truncates.
    for d in (0..rule.width).take_while(|d| d * s < r.width) {
        let mix = |a: u8, b: u8| (a as i32 + (b as i32 - a as i32) * d / span) as u8;
        let (f, t) = (rule.from, rule.to);
        let ink = Color::rgb(mix(f.r, t.r), mix(f.g, t.g), mix(f.b, t.b));
        c.fill_rect(Rect::new(r.x + d * s, ly, s, s), ink);
    }
}

/// The in-place rename box in the already-scaled `b`: white with a 1 px `window_frame` border, text
/// `EDIT_PAD_L` in and `EDIT_INSET` down, scrolled so the caret shows. `shellview` uses it for every mode but Icons.
pub(crate) fn draw_rename_box(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    ed: &TextEdit,
    b: Rect,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(b, cs.window);
    c.ring_t(b, cs.window_frame, cs.window_frame, s);
    let inner = Rect::new(
        b.x + EDIT_PAD_L * s,
        b.y + EDIT_INSET * s,
        (b.width - (EDIT_PAD_L + EDIT_INSET) * s).max(0),
        b.height - 2 * EDIT_INSET * s,
    );
    let first = edit_first_visible(text, ed, inner.width, px, 0);
    draw_edit(c, text, ed, inner, px, p, Caret::Shown, first);
}

pub fn draw<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>>(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lv: &ListView<R, C, S>,
    lay: &Layout,
    bars: Lit<(Axis, ScrollHit)>,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let frame = lay.frame.scaled(s);
    c.fill_rect(frame, cs.button_face);
    if lv.frame {
        draw_field_frame(c, p, frame, cs.window);
    }
    let (header, body) = (lay.header.scaled(s), lay.body.scaled(s));
    let left = lv.left_in(lay);
    c.with_clip(header, |c| draw_header(c, text, p, lv, header, left, px));
    c.with_clip(body, |c| draw_rows(c, text, p, lv, lay, body, px));
    if let Some(sb) = lay.scrollbar {
        draw_scrollbar(
            c,
            p,
            sb.scaled(s),
            lv.top_slot(),
            lay.rows,
            lv.slots(),
            axis_lit(bars, Axis::Vertical),
        );
    }
    if let Some(hb) = lay.hscroll {
        draw_hscrollbar(
            c,
            p,
            hb.scaled(s),
            left as usize,
            lay.body.width.max(0) as usize,
            lay.total_w.max(0) as usize,
            axis_lit(bars, Axis::Horizontal),
        );
    }
}

/// A Details list with two rows selected, Name sorted ascending, a rename box, more rows than fit and
/// wide columns: drawn by `examples/gallery.rs` and the preview test.
pub fn sample() -> ListView {
    let rows = [
        ("My Music", "", "File Folder"),
        ("My Pictures", "", "File Folder"),
        ("notes.txt", "1 KB", "Text Document"),
        ("report.odt", "28 KB", "OpenDocument Text"),
        ("setup.exe", "412 KB", "Application"),
        ("todo.txt", "1 KB", "Text Document"),
        ("reprise.log", "94 KB", "Text Document"),
    ];
    let mut lv = ListView {
        columns: vec![
            Column::new("Name", 160),
            Column {
                title: "Size".into(),
                width: 70,
                right: true,
            },
            Column::new("Type", 150),
            Column::new("Date Modified", 120),
        ],
        rows: rows
            .iter()
            .map(|(n, s, k)| Row {
                icon: None,
                cells: vec![
                    (*n).into(),
                    (*s).into(),
                    (*k).into(),
                    "14/09/2026 21:40".into(),
                ],
            })
            .collect(),
        sort: Some((0, true)),
        focus_ring: true,
        ..Default::default()
    };
    lv.selection.set_only(Some(0));
    lv.selection.click(
        1,
        Mods {
            ctrl: true,
            shift: false,
        },
    );
    lv.edit = Some((3, TextEdit::new("report.odt", true)));
    lv
}

impl<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>> ListView<R, C, S> {
    /// The list view's `access` part: one option per row, named by its first column and described by
    /// the others. Selected rows are marked, check rows carry their check, scrolled-out rows have no
    /// bounds. Returns the focused row's node when the list has the focus, else the list.
    pub fn access(
        &self,
        lay: &Layout,
        tree: &mut Tree,
        parent: NodeId,
        key: impl std::hash::Hash,
    ) -> NodeId {
        let mut n = access::node(Role::ListBox, lay.frame);
        n.set_multiselectable();
        access::focusable(&mut n, true);
        let id = tree.push(parent, key, n);
        let sel = self.selection.borrow();
        let cols = self.columns.as_ref();
        let mut focus = id;
        for i in 0..self.rows.len() {
            let mut o = Node::new(Role::ListBoxOption);
            o.set_label(self.rows.cell(i, 0).into_owned());
            let rest: Vec<String> = cols
                .iter()
                .enumerate()
                .skip(1)
                .filter_map(|(k, c)| {
                    let v = self.rows.cell(i, k);
                    (!v.is_empty()).then(|| format!("{}: {v}", c.title))
                })
                .collect();
            if !rest.is_empty() {
                o.set_description(rest.join(", "));
            }
            o.set_selected(sel.is(i));
            if let Some(on) = self.checks.as_ref().and_then(|c| c.get(i)) {
                o.set_toggled((*on).into());
            }
            let y = lay.body.y + self.slot_y(i);
            if y >= lay.body.y && y + ROW_H <= lay.body.bottom() {
                o.set_bounds(access::bounds(Rect::new(
                    lay.body.x,
                    y,
                    lay.body.width,
                    ROW_H,
                )));
            }
            let oid = tree.push(id, i, o);
            if sel.focus == Some(i) {
                focus = oid;
            }
        }
        focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    /// The header strip's top row, Name column's left edge, title ink start and sort arrow box.
    const HEADER_Y: i32 = 83;
    const COL0_X: i32 = 204;
    const TITLE_X: i32 = 210;
    const ARROW_BOX: (i32, i32, i32, i32) = (246, 89, 7, 4);
    const HSCROLL: (i32, i32) = (558, 573);
    /// The rename box, text origin inside it and the label it replaced.
    const EDIT_BOX: (i32, i32, i32, i32) = (265, 217, 146, 17);
    const EDIT_TEXT_X: i32 = 270;
    const EDIT_LABEL_X: i32 = 268;
    /// The group rule: left end at the body's edge, and 150 px along it.
    const LINE_LEFT: Color = Color::rgb(166, 202, 240);
    const LINE_MID: Color = Color::rgb(210, 228, 247);

    fn view(n: usize) -> ListView {
        ListView {
            columns: vec![Column::new("Name", 120), Column::new("Size", 60)],
            rows: (0..n)
                .map(|i| Row {
                    icon: None,
                    cells: vec![format!("file{i}.txt"), format!("{i} KB")],
                })
                .collect(),
            ..Default::default()
        }
    }

    /// The vertical bar answers page and thumb hits; `top_at` maps a dragged thumb back to a row.
    #[test]
    fn the_track_pages_and_the_thumb_drags() {
        let mut lv = view(40);
        let r = Rect::new(0, 0, 200, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let sb = lay.scrollbar.expect("a vertical bar");
        assert_eq!(
            lv.hit(&lay, sb.x + 2, sb.bottom() - 3),
            Hit::VBar(BarHit::Forward)
        );
        assert_eq!(
            lv.hit(
                &lay,
                sb.x + 2,
                sb.bottom() - ControlMetrics::CLASSIC.scroll - 2
            ),
            Hit::VBar(BarHit::PageForward)
        );
        let p = scrollbar_parts(sb, 1, lv.top, lay.rows, lv.slots(), false, lay.thumb_min);
        let thumb = p.thumb.expect("a thumb");
        let Hit::VBar(BarHit::Thumb(grab)) = lv.hit(&lay, thumb.x + 2, thumb.y + 3) else {
            panic!("the thumb");
        };
        assert_eq!(grab, 3);
        assert_eq!(lv.top_at(&lay, thumb.y + 3, grab), 0, "not moved yet");
        let bottom = lv.top_at(&lay, sb.bottom(), grab);
        assert_eq!(bottom, lv.max_top(&lay), "dragged past the end clamps");
        lv.top = bottom;
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(
            lv.hit(&lay, sb.x + 2, sb.y + ControlMetrics::CLASSIC.scroll + 2),
            Hit::VBar(BarHit::PageBack)
        );
    }

    /// With groups the bar counts slots, headers included; `top_at` stays within `max_top`.
    #[test]
    fn a_grouped_thumb_drags_to_rows_within_max_top() {
        let mut lv = view(20);
        lv.groups = vec![
            Group {
                title: "A".into(),
                first: 0,
            },
            Group {
                title: "B".into(),
                first: 10,
            },
        ];
        let r = Rect::new(0, 0, 200, 4 + HEADER_H + 6 * ROW_H);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let sb = lay.scrollbar.expect("a vertical bar");
        let bottom = lv.top_at(&lay, sb.bottom(), 0);
        assert_eq!(bottom, lv.max_top(&lay), "dragged past the end clamps");
        lv.top = 12;
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let p = scrollbar_parts(
            sb,
            1,
            lv.top_slot(),
            lay.rows,
            lv.slots(),
            false,
            lay.thumb_min,
        );
        let thumb = p.thumb.expect("a thumb");
        assert_eq!(lv.top_at(&lay, thumb.y, 0), 12, "the thumb stays put");
    }

    /// `follow` scrolls a grouped list just far enough that the focused row and its header fit.
    #[test]
    fn follow_brings_a_grouped_row_just_into_view() {
        let mut lv = view(20);
        lv.groups = vec![
            Group {
                title: "A".into(),
                first: 0,
            },
            Group {
                title: "B".into(),
                first: 10,
            },
        ];
        let r = Rect::new(0, 0, 200, 4 + HEADER_H + 6 * ROW_H);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        lv.select(Some(12), &lay);
        assert_eq!(lv.top, 9);
        lv.select(Some(19), &lay);
        assert_eq!(lv.top, 14, "the last six rows");
        lv.select(Some(11), &lay);
        assert_eq!(lv.top, 11, "up to the row itself");
        let short = layout(
            &lv,
            Rect::new(0, 0, 200, 4 + HEADER_H + ROW_H - 1),
            &ControlMetrics::CLASSIC,
        );
        lv.top = 0;
        lv.select(Some(10), &short);
        assert_eq!(lv.top, 10);
    }

    fn render(lv: &ListView, r: Rect) -> (Canvas, Layout, Theme) {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let lay = layout(lv, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            lv,
            &lay,
            None,
            theme.fonts.size_px,
        );
        (c, lay, theme)
    }

    /// The sorted column's band is the window colour 8/255 toward the window text.
    #[test]
    fn the_sort_band_tints_the_schemes_window() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(2);
        lv.sort = Some((1, true));
        lv.sort_tint = true;
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let mut dark = Theme::default().colors;
        dark.window = Color::rgb(32, 32, 32);
        dark.window_text = Color::rgb(255, 255, 255);
        for (cs, band) in [
            (Theme::default().colors, Color::rgb(247, 247, 247)),
            (dark, Color::rgb(39, 39, 39)),
        ] {
            let mut c = Canvas::new(300, 100);
            draw(&mut c, &text, &classic_paint(&cs, 1), &lv, &lay, None, 11);
            let y = lay.body.bottom() - 2;
            assert_eq!(c.get(150, y), band.argb(), "the Size column's band");
            assert_eq!(c.get(60, y), cs.window.argb(), "the Name column");
        }
    }

    /// A group's rule fades into the window colour, so on a dark window its far end is dark.
    #[test]
    fn the_group_rule_fades_into_a_dark_window() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(2);
        lv.groups = vec![Group {
            title: "A".into(),
            first: 0,
        }];
        let r = Rect::new(0, 0, 400, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let mut cs = Theme::default().colors;
        cs.window = Color::rgb(32, 32, 32);
        cs.window_text = Color::rgb(255, 255, 255);
        let mut c = Canvas::new(400, 100);
        draw(&mut c, &text, &classic_paint(&cs, 1), &lv, &lay, None, 11);
        let (x, y) = (lay.body.x + GROUP_LINE_W - 1, lay.body.y + GROUP_RULE_Y);
        assert_eq!(
            c.get(x, y),
            Color::rgb(33, 33, 33).argb(),
            "the rule's last px"
        );
    }

    #[test]
    fn layout_hits_and_scrolls() {
        let lv = view(3);
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.header, Rect::new(2, 2, 296, HEADER_H));
        assert_eq!(lay.body.y, 2 + HEADER_H);
        assert!(lay.scrollbar.is_none());
        assert!(lay.hscroll.is_none());
        assert_eq!(lay.total_w, 180);
        assert_eq!(lv.hit(&lay, 10, 5), Hit::Header(0));
        assert_eq!(lv.hit(&lay, 130, 5), Hit::Header(1));
        assert_eq!(lv.hit(&lay, 10, lay.body.y + ROW_H + 3), Hit::Row(1));
        assert_eq!(
            lv.hit(&lay, 10, lay.body.y + 3 * ROW_H + 3),
            Hit::Background
        );
        assert_eq!(lv.hit(&lay, 1, 1), Hit::Nothing);
        let mut many = view(20);
        let lay = layout(&many, r, &ControlMetrics::CLASSIC);
        assert!(lay.scrollbar.is_some());
        assert_eq!(lay.rows, (96 - HEADER_H) as usize / ROW_H as usize);
        assert!(many.step(1, &lay, Mods::default()));
        assert_eq!(many.selection.focus, Some(0));
        many.step(100, &lay, Mods::default());
        assert_eq!(many.selection.focus, Some(19));
        assert_eq!(many.top, 20 - lay.rows);
        many.step(-100, &lay, Mods::default());
        assert_eq!((many.selection.focus, many.top), (Some(0), 0));
        assert!(many.scroll(2, &lay));
        assert_eq!(
            lv.hit(&lay, lay.scrollbar.unwrap().x + 3, lay.body.y + 3),
            Hit::VBar(BarHit::Back)
        );
    }

    /// Scrolled columns, highlight, rename box and check box stay inside the rows' box and header strip.
    #[test]
    fn scrolled_contents_stay_inside_the_body_and_header() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut lv = view(20);
        lv.resize_column(0, 400);
        lv.checks = Some(vec![true; 20]);
        lv.select(
            Some(3),
            &layout(&lv, Rect::new(0, 0, 1, 1), &ControlMetrics::CLASSIC),
        );
        lv.edit = Some((4, TextEdit::new("a long name that runs on", true)));
        let r = Rect::new(20, 30, 200, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        lv.hscroll(40, &lay);
        let sentinel = 0xFF12_3456;
        let mut c = Canvas::new(260, 160);
        c.pixels.fill(sentinel);
        let mut bare = c.clone();
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &lv,
            &lay,
            None,
            theme.fonts.size_px,
        );
        let empty = ListView {
            columns: Vec::new(),
            ..view(0)
        };
        let bare_lay = Layout {
            total_w: lay.total_w,
            ..layout(&empty, r, &ControlMetrics::CLASSIC)
        };
        draw(
            &mut bare,
            &text,
            &classic_paint(&theme.colors, 1),
            &empty,
            &bare_lay,
            None,
            theme.fonts.size_px,
        );
        let inside = |x, y| lay.header.contains(x, y) || lay.body.contains(x, y);
        for y in 0..160 {
            for x in 0..260 {
                if !inside(x, y) && !r.contains(x, y) {
                    assert_eq!(c.get(x, y), sentinel, "outside the rect at ({x}, {y})");
                }
            }
        }
        for y in r.y..r.y + 2 {
            for x in r.x..r.right() {
                assert_eq!(c.get(x, y), bare.get(x, y), "frame at ({x}, {y})");
            }
        }
        for y in r.y..r.bottom() {
            for x in [r.x, r.x + 1, r.right() - 2, r.right() - 1] {
                assert_eq!(c.get(x, y), bare.get(x, y), "frame at ({x}, {y})");
            }
        }
    }

    #[test]
    fn shift_arrow_extends_the_selection() {
        let mut lv = view(6);
        let lay = layout(&lv, Rect::new(0, 0, 300, 200), &ControlMetrics::CLASSIC);
        lv.select(Some(1), &lay);
        let shift = Mods {
            ctrl: false,
            shift: true,
        };
        lv.step(2, &lay, shift);
        assert_eq!(
            lv.selection.set.iter().copied().collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(lv.selection.focus, Some(3));
    }

    #[test]
    fn the_divider_wins_either_side_of_a_column_edge() {
        let lv = view(3);
        let lay = layout(&lv, Rect::new(0, 0, 300, 100), &ControlMetrics::CLASSIC);
        let edge = lay.header.x + lv.columns[0].width;
        for dx in -DIVIDER_GRAB..=DIVIDER_GRAB {
            assert_eq!(lv.hit(&lay, edge + dx, 5), Hit::Divider(0), "dx {dx}");
        }
        assert_eq!(lv.hit(&lay, edge - DIVIDER_GRAB - 1, 5), Hit::Header(0));
        assert_eq!(lv.hit(&lay, edge + DIVIDER_GRAB + 1, 5), Hit::Header(1));
    }

    /// A look with a 17 px scrollbar lays both bars 17 px thick; the body gives up 17 px to each.
    #[test]
    fn a_17_px_scrollbar_takes_17_px_off_the_body_each_way() {
        let mut lv = view(20);
        lv.resize_column(0, 300);
        let r = Rect::new(0, 0, 300, 100);
        let cm = ControlMetrics {
            scroll: 17,
            ..ControlMetrics::CLASSIC
        };
        let lay = layout(&lv, r, &cm);
        let vb = lay.scrollbar.expect("20 rows overflow the body");
        let hb = lay.hscroll.expect("the columns overflow the body");
        assert_eq!(vb, Rect::new(281, 2 + HEADER_H, 17, lay.body.height));
        assert_eq!(hb, Rect::new(2, lay.body.bottom(), 279, 17));
        assert_eq!(vb.right(), r.right() - 2);
        assert_eq!(hb.bottom(), r.bottom() - 2);
    }

    /// A Blue list lays its bar 17 px thick with a 10 px thumb at least, draws the thumb on the rows
    /// `hit` answers Thumb on, and pages either side. Classic's thumb stays 8 px.
    #[test]
    fn a_blue_bar_is_drawn_where_it_is_hit() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = theme.look.controls();
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(500);
        lv.top = 250;
        let r = Rect::new(0, 0, 200, 150);
        let lay = layout(&lv, r, &cm);
        let sb = lay.scrollbar.expect("500 rows overflow the body");
        assert_eq!(sb.width, 17);
        let p = scrollbar_parts(sb, 1, lv.top_slot(), lay.rows, lv.slots(), false, 10);
        let thumb = p.thumb.expect("a thumb");
        assert_eq!(thumb.height, 10, "never shorter than 10 px");
        let x = sb.x + sb.width / 2;
        assert_eq!(lv.hit(&lay, x, thumb.y), Hit::VBar(BarHit::Thumb(0)));
        assert_eq!(
            lv.hit(&lay, x, thumb.bottom() - 1),
            Hit::VBar(BarHit::Thumb(9))
        );
        assert_eq!(lv.hit(&lay, x, thumb.y - 1), Hit::VBar(BarHit::PageBack));
        assert_eq!(
            lv.hit(&lay, x, thumb.bottom()),
            Hit::VBar(BarHit::PageForward)
        );
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(&mut c, &text, &theme.paint(), &lv, &lay, None, 11);
        let track = c.get(x, p.track.y);
        let drawn: Vec<i32> = (p.track.y..p.track.bottom())
            .filter(|&y| c.get(x, y) != track)
            .collect();
        assert_eq!(drawn.first(), Some(&thumb.y));
        assert_eq!(drawn.last(), Some(&(thumb.bottom() - 1)));

        let classic = layout(&lv, r, &ControlMetrics::CLASSIC);
        let sb = classic.scrollbar.expect("500 rows overflow the body");
        let p = scrollbar_parts(sb, 1, lv.top_slot(), classic.rows, lv.slots(), false, 8);
        assert_eq!(p.thumb.map(|t| t.height), Some(8));
        assert_eq!(classic.thumb_min, 8);
    }

    /// After scrolling to the far end, a widening list or narrowing column draws and hits as if
    /// scrolled to its new far end (0 once the columns fit): no blank past the last column.
    #[test]
    fn a_wider_list_holds_its_sideways_scroll_to_the_columns() {
        let text = TextRenderer::load(&FontConfig::default());
        let cs = Theme::default().colors;
        let mut lv = view(3);
        lv.resize_column(0, 300);
        let lay = |r| layout(&lv, r, &ControlMetrics::CLASSIC);
        let narrow = lay(Rect::new(0, 0, 200, 100));
        let mut scrolled = lv.clone();
        scrolled.hscroll(1000, &narrow);
        let far = scrolled.left;
        assert_eq!(far, narrow.total_w - narrow.body.width);
        let shot = |lv: &ListView, l: &Layout| {
            let mut c = Canvas::new(500, 100);
            draw(&mut c, &text, &classic_paint(&cs, 1), lv, l, None, 11);
            c
        };

        let wider = lay(Rect::new(0, 0, 300, 100));
        let end = wider.total_w - wider.body.width;
        assert!((1..far).contains(&end), "still overflowing, by less");
        assert_eq!(scrolled.left_in(&wider), end);
        let held = ListView {
            left: end,
            ..scrolled.clone()
        };
        assert!(
            shot(&scrolled, &wider) == shot(&held, &wider),
            "drawn at the new far end"
        );
        let (x, y) = (wider.header.x + 5, wider.header.y + 5);
        assert_eq!(scrolled.hit(&wider, x, y), held.hit(&wider, x, y));
        assert!(!scrolled.hscroll(1, &wider), "already at the far end");

        scrolled.left = far;
        let roomy = lay(Rect::new(0, 0, 500, 100));
        assert!(roomy.hscroll.is_none());
        assert_eq!(scrolled.left_in(&roomy), 0);
        assert_eq!(
            scrolled.hit(&roomy, roomy.header.x + 5, roomy.header.y + 5),
            Hit::Header(0),
            "the Name column is back in reach"
        );
        assert!(shot(&scrolled, &roomy) == shot(&lv, &roomy));
    }

    #[test]
    fn resizing_a_column_grows_the_total_and_brings_the_hscroll() {
        let mut lv = view(3);
        let r = Rect::new(0, 0, 300, 100);
        assert!(layout(&lv, r, &ControlMetrics::CLASSIC).hscroll.is_none());
        lv.resize_column(0, 300);
        assert_eq!(lv.columns[0].width, 300);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.total_w, 360);
        let hb = lay.hscroll.expect("the columns overflow the body");
        assert_eq!(hb.height, HSCROLL.1 - HSCROLL.0 + 1);
        assert_eq!(
            hb,
            Rect::new(
                lay.body.x,
                lay.body.bottom(),
                lay.body.width,
                ControlMetrics::CLASSIC.scroll
            )
        );
        assert_eq!(hb.bottom(), lay.frame.bottom() - 2);
        assert_eq!(lv.hit(&lay, hb.x + 3, hb.y + 3), Hit::HBar(BarHit::Back));
        assert_eq!(
            lv.hit(&lay, hb.right() - 3, hb.y + 3),
            Hit::HBar(BarHit::Forward)
        );
        // The track pages; the thumb reports its grab offset; dragging it to the end scrolls to the last page.
        let p = scrollbar_parts(
            hb,
            1,
            0,
            lay.body.width as usize,
            lay.total_w as usize,
            true,
            lay.thumb_min,
        );
        let thumb = p.thumb.expect("a thumb");
        assert_eq!(
            lv.hit(&lay, thumb.x + 2, hb.y + 3),
            Hit::HBar(BarHit::Thumb(2))
        );
        assert_eq!(
            lv.hit(&lay, thumb.right() + 2, hb.y + 3),
            Hit::HBar(BarHit::PageForward)
        );
        assert_eq!(
            lv.left_at(&lay, p.track.right() + 2, 2),
            lay.total_w - lay.body.width
        );
        assert_eq!(lv.left_at(&lay, p.track.x, 2), 0);
        assert!(lv.hscroll(1000, &lay));
        assert_eq!(lv.left, lay.total_w - lay.body.width);
        assert!(!lv.hscroll(1, &lay));
        assert_eq!(
            lv.hit(&lay, hb.x + ControlMetrics::CLASSIC.scroll + 2, hb.y + 3),
            Hit::HBar(BarHit::PageBack)
        );
        lv.resize_column(0, 1);
        assert_eq!(lv.columns[0].width, MIN_COL_W);
    }

    #[test]
    fn both_bars_leave_each_other_room() {
        let mut lv = view(40);
        lv.resize_column(0, 300);
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let (sb, hb) = (lay.scrollbar.unwrap(), lay.hscroll.unwrap());
        assert_eq!(sb.x, lay.body.right());
        assert_eq!(sb.bottom(), hb.y);
        assert_eq!(hb.right(), sb.x);
        let (c, _, theme) = render(&lv, r);
        assert_eq!(c.get(sb.x + 3, hb.y + 3), theme.colors.button_face.argb());
    }

    /// Under Blue the header is 20 rows and the rows start right under it; the pointer's column draws
    /// lit, the held one sunk.
    #[test]
    fn a_blue_header_lights_the_column_the_pointer_is_on() {
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let cm = theme.look.controls();
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(3);
        let r = Rect::new(0, 0, 200, 4 + 20 + 3 * ROW_H);
        let lay = layout(&lv, r, &cm);
        assert_eq!(lay.header, Rect::new(2, 2, 196, 20));
        assert_eq!(lay.body.y, 22, "the rows start under the header");
        let (x0, x1) = (lay.header.x + 10, lay.header.x + 130);
        assert_eq!(lv.hit(&lay, x1, 10).column(), Some(1));
        assert_eq!(lv.hit(&lay, x1, 30).column(), None, "a row is no column");
        let band = |lv: &ListView| {
            let mut c = Canvas::new(r.width as u32, r.height as u32);
            draw(&mut c, &text, &theme.paint(), lv, &lay, None, 11);
            (c.get(x0, 2 + 17), c.get(x1, 2 + 17), c.get(x1, 2))
        };
        let faces = &crate::blue::Variant::Blue.scheme().header_item;
        let (rest, _, _) = band(&lv);
        lv.header.motion(Some(1));
        let (left, lit, _) = band(&lv);
        assert_eq!(left, rest, "the other column stays at rest");
        assert_ne!(lit, rest, "the lit column's band");
        assert!(
            (lit >> 16 & 0xFF) > (lit & 0xFF) + 0x40,
            "the lit column's band is orange"
        );
        lv.header.press(Some(1), false);
        let (_, _, top) = band(&lv);
        assert_eq!(
            top,
            faces.pressed.down[0].1.argb(),
            "the held column's shadow along its top"
        );
    }

    #[test]
    fn the_sort_arrow_sits_after_the_title() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(3);
        lv.sort = Some((0, true));
        let r = Rect::new(0, 0, 300, 100);
        let (c, lay, theme) = render(&lv, r);
        let cs = &theme.colors;
        let dx = lay.header.x - COL0_X;
        let dy = lay.header.y - HEADER_Y;
        assert_eq!(
            lay.header.x + HEADER_TEXT_X,
            TITLE_X + dx,
            "the title starts at the expected x"
        );
        let title_w = text.measure("Name", false, theme.fonts.size_px);
        let (ax, ay) = (
            lay.header.x + HEADER_TEXT_X + title_w + ARROW_GAP,
            lay.header.y + ARROW_BOX.1 - HEADER_Y,
        );
        assert_eq!((ax, ay), (ARROW_BOX.0 + dx, ARROW_BOX.1 + dy));
        let (arrow_w, arrow_h) = (ARROW_BOX.2, ARROW_BOX.3);
        for row in 0..arrow_h {
            let (w, inset) = (1 + 2 * row, arrow_h - 1 - row);
            assert_eq!(c.get(ax + inset - 1, ay + row), cs.button_face.argb());
            for k in 0..w {
                assert_eq!(
                    c.get(ax + inset + k, ay + row),
                    cs.button_shadow.argb(),
                    "row {row} px {k}"
                );
            }
            assert_eq!(c.get(ax + inset + w, ay + row), cs.button_face.argb());
        }
        assert_eq!(arrow_w, 7, "the arrow's widest row");
        assert_eq!(c.get(ax + 3, ay - 1), cs.button_face.argb());
        assert_eq!(c.get(ax + 3, ay + arrow_h), cs.button_face.argb());
        lv.sort = Some((0, false));
        let (c, ..) = render(&lv, r);
        assert_eq!(c.get(ax + 3, ay), cs.button_shadow.argb());
        assert_eq!(c.get(ax, ay), cs.button_shadow.argb());
        assert_eq!(c.get(ax, ay + arrow_h - 1), cs.button_face.argb());
    }

    /// A right-aligned 96 px column, sorted: the arrow ends where the unsorted title's ink did, 7 px
    /// short of the column's end, and the title ends its ink 10 px before the arrow.
    #[test]
    fn a_right_aligned_sorted_column_puts_its_arrow_after_the_title() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let mut lv = view(0);
        lv.columns = vec![
            Column::new("Name", 180),
            Column {
                right: true,
                ..Column::new("Size", 96)
            },
        ];
        let r = Rect::new(0, 0, 400, 100);
        let inks = |lv: &ListView| {
            let (c, lay, theme) = render(lv, r);
            let x0 = lay.header.x + 180;
            let y = lay.header.y;
            let cols = |ink: Color| -> Vec<i32> {
                (x0..x0 + 96)
                    .filter(|&x| (y + 2..y + 15).any(|y| c.get(x, y) == ink.argb()))
                    .map(|x| x - x0)
                    .collect()
            };
            let cs = &theme.colors;
            (cols(cs.button_text), cols(cs.button_shadow))
        };
        let (title, _) = inks(&lv);
        assert_eq!(
            title.last(),
            Some(&88),
            "the unsorted title's last ink column"
        );
        lv.sort = Some((1, true));
        let (title, arrow) = inks(&lv);
        assert_eq!(
            title.last(),
            Some(&71),
            "the sorted title's last ink column"
        );
        let arrow: Vec<i32> = arrow.into_iter().filter(|&x| x < 90).collect();
        assert_eq!(arrow, (82..89).collect::<Vec<_>>(), "the arrow's columns");
    }

    /// Blue's Details header with Name 180, Size 96 (right-aligned) and Type 120, sorted by `sort`:
    /// per column, the title and arrow ink columns from the left edge.
    fn blue_header_inks(sort: Option<(usize, bool)>) -> Vec<(Vec<i32>, Vec<i32>)> {
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(0);
        lv.columns = vec![
            Column::new("Name", 180),
            Column {
                right: true,
                ..Column::new("Size", 96)
            },
            Column::new("Type", 120),
        ];
        lv.sort = sort;
        let r = Rect::new(0, 0, 500, 100);
        let lay = layout(&lv, r, &theme.look.controls());
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(
            &mut c,
            &text,
            &theme.paint(),
            &lv,
            &lay,
            None,
            theme.fonts.size_px,
        );
        let y = lay.header.y;
        let ink = |x0: i32, w: i32, ink: u32| -> Vec<i32> {
            (x0..x0 + w)
                .filter(|&x| (y + 3..y + 16).any(|y| c.get(x, y) == ink))
                .map(|x| x - x0)
                .collect()
        };
        let text_ink = theme.colors.button_text.argb();
        let arrow_ink = crate::blue::Variant::Blue.scheme().system.gray_text.argb();
        [(0, 180), (180, 96), (276, 120)]
            .into_iter()
            .map(|(x, w)| {
                let x0 = lay.header.x + x;
                (ink(x0, w, text_ink), ink(x0, w, arrow_ink))
            })
            .collect()
    }

    /// Under Blue a sorted left-aligned title sits 3 px further in (ink from 9 to 12 px) with the arrow
    /// 10 px after its ink; a sorted right-aligned one stops its 9 px arrow against the column's 2 px
    /// divider (85..93 of 96) and ends its ink 10 px before it, at 74 (unsorted: 88).
    #[test]
    fn blue_sorted_titles_move_in_and_a_right_aligned_arrow_meets_the_divider() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let rest = blue_header_inks(None);
        assert_eq!(rest[0].0.first(), Some(&9), "Name at rest");
        assert_eq!(rest[1].0.last(), Some(&88), "Size at rest");
        assert_eq!(rest[2].0.first(), Some(&9), "Type at rest");
        assert!(rest.iter().all(|(_, a)| a.is_empty()), "no arrow at rest");
        for (ci, name) in [(0, "Name"), (2, "Type")] {
            for asc in [true, false] {
                let inks = blue_header_inks(Some((ci, asc)));
                let (title, arrow) = &inks[ci];
                assert_eq!(title.first(), Some(&12), "sorted {name} moves in");
                let end = *title.last().unwrap();
                assert_eq!(
                    *arrow,
                    (end + 11..end + 20).collect::<Vec<_>>(),
                    "{name}'s arrow 10 px after its ink"
                );
            }
        }
        for asc in [true, false] {
            let inks = blue_header_inks(Some((1, asc)));
            let (title, arrow) = &inks[1];
            assert_eq!(*arrow, (85..94).collect::<Vec<_>>(), "Size's arrow");
            assert_eq!(title.last(), Some(&74), "Size's title");
        }
    }

    #[test]
    fn every_selected_row_gets_the_highlight() {
        let mut lv = view(4);
        let r = Rect::new(0, 0, 300, 200);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        lv.select(Some(1), &lay);
        lv.selection.click(
            3,
            Mods {
                ctrl: true,
                shift: false,
            },
        );
        let (c, lay, theme) = render(&lv, r);
        let cs = &theme.colors;
        let at = |c: &Canvas, k: i32| c.get(lay.body.x + NAME_X - 1, lay.body.y + k * ROW_H + 8);
        assert_eq!(at(&c, 0), cs.window.argb());
        assert_eq!(at(&c, 1), cs.hilight.argb());
        assert_eq!(at(&c, 2), cs.window.argb());
        assert_eq!(at(&c, 3), cs.hilight.argb());
        let mut lv = view(4);
        lv.drop_row = Some(2);
        lv.focus_ring = true;
        let (c, ..) = render(&lv, r);
        assert_eq!(at(&c, 2), cs.hilight.argb());
    }

    #[test]
    fn scrolling_left_shifts_the_header_and_the_cells() {
        let mut lv = view(3);
        let r = Rect::new(0, 0, 300, 100);
        lv.resize_column(0, 300);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let (before, ..) = render(&lv, r);
        assert!(lv.hscroll(4, &lay));
        let (after, ..) = render(&lv, r);
        let theme = Theme::default();
        let black = theme.colors.button_text.argb();
        let ink = |c: &Canvas, y: i32| {
            (lay.header.x..lay.header.right())
                .find(|&x| c.get(x, y) == black)
                .expect("the title's ink")
        };
        let hy = lay.header.y + 8;
        assert_eq!(ink(&after, hy), ink(&before, hy) - 4);
        let ry = lay.body.y + 8;
        assert_eq!(ink(&after, ry), ink(&before, ry) - 4);
        for y in lay.header.y..lay.body.bottom() {
            assert_eq!(
                after.get(0, y),
                theme.colors.button_shadow.argb(),
                "frame at y {y}"
            );
        }
    }

    #[test]
    fn the_rename_box_is_a_black_border_on_white_over_the_name() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let cs = &theme.colors;
        let mut lv = view(3);
        lv.edit = Some((1, TextEdit::new("supermium_144_32_setup", true)));
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let b = lv.edit_rect(&lay, 1, &text, px);
        let name_x = lay.body.x + NAME_X;
        assert_eq!(name_x - b.x, EDIT_LABEL_X - EDIT_BOX.0);
        assert_eq!(b.x + EDIT_PAD_L - name_x, EDIT_TEXT_X - EDIT_LABEL_X);
        assert_eq!((b.width, b.height), (EDIT_BOX.2, EDIT_BOX.3));
        assert_eq!(b.y, lay.body.y + ROW_H);
        let (c, ..) = render(&lv, r);
        let mid = b.y + b.height / 2;
        assert_eq!(c.get(b.x, mid), cs.window_frame.argb(), "left border");
        assert_eq!(c.get(b.right() - 1, mid), cs.window_frame.argb());
        assert_eq!(c.get(b.x + 2, b.y), cs.window_frame.argb(), "top border");
        assert_eq!(c.get(b.x + 2, b.bottom() - 1), cs.window_frame.argb());
        assert_eq!(c.get(b.x + 1, mid), cs.window.argb(), "white inside");
        // The whole name is selected: text sits on the highlight from EDIT_PAD_L, EDIT_PAD_R of white after.
        let sel_y = b.y + EDIT_INSET;
        assert_eq!(c.get(b.x + EDIT_PAD_L, sel_y), cs.hilight.argb());
        assert_eq!(c.get(b.x + EDIT_PAD_L - 1, sel_y), cs.window.argb());
        assert_eq!(c.get(b.right() - 3, mid), cs.window.argb());
        assert_eq!(c.get(b.x - 1, mid), cs.window.argb());
    }

    #[test]
    fn check_rect_sits_8_px_in_and_3_px_down() {
        let mut lv = view(3);
        lv.checks = Some(vec![false; 3]);
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lv.check_rect(&lay, 0), Rect::new(8, lay.body.y + 3, 13, 13));
        assert_eq!(
            lv.check_rect(&lay, 1),
            Rect::new(8, lay.body.y + ROW_H + 3, 13, 13)
        );
    }

    #[test]
    fn a_check_box_takes_the_looks_side() {
        let mut lv = view(3);
        lv.checks = Some(vec![false; 3]);
        let cm = ControlMetrics {
            check: 11,
            ..ControlMetrics::CLASSIC
        };
        let lay = layout(&lv, Rect::new(0, 0, 300, 100), &cm);
        assert_eq!(lv.check_rect(&lay, 0), Rect::new(8, lay.body.y + 3, 11, 11));
        let y = lay.body.y + 3 + 10;
        assert_eq!(
            lv.hit(&lay, 8 + 10, y),
            Hit::Check(0),
            "the box's last column"
        );
        assert_eq!(lv.hit(&lay, 8 + 11, y), Hit::Row(0), "the row past it");
    }

    #[test]
    fn hit_on_the_box_gives_check() {
        let mut lv = view(3);
        lv.checks = Some(vec![false; 3]);
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        let cr = lv.check_rect(&lay, 1);
        assert_eq!(lv.hit(&lay, cr.x + 1, cr.y + 1), Hit::Check(1));
    }

    #[test]
    fn a_layout_taken_at_another_top_still_draws_every_check_box() {
        let mut lv = view(10);
        lv.checks = Some(vec![false; 10]);
        let r = Rect::new(0, 0, 300, 4 + HEADER_H + 8 * ROW_H);
        lv.top = 5;
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.rows, 8);
        lv.top = 0;
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &lv,
            &lay,
            None,
            theme.fonts.size_px,
        );
        let cr = lv.check_rect(&lay, 7);
        assert_eq!(cr, Rect::new(8, lay.body.y + 7 * ROW_H + 3, 13, 13));
        assert_eq!(c.get(cr.x + 6, cr.y + 6), theme.colors.window.argb());
        assert_ne!(c.get(cr.x, cr.y), theme.colors.window.argb());
    }

    #[test]
    fn toggle_flips_a_row_and_reports_whether_it_could() {
        let mut lv = view(3);
        assert!(!lv.toggle(0), "no checks column yet");
        lv.checks = Some(vec![false, false, false]);
        assert!(lv.toggle(0));
        assert!(lv.checks.as_ref().unwrap()[0]);
        assert!(lv.toggle(0));
        assert!(!lv.checks.as_ref().unwrap()[0]);
        assert!(!lv.toggle(3), "row out of range");
    }

    /// A row's check box acts on release: the press holds it, a release over it toggles, a release
    /// after dragging off changes nothing, and dragging back on re-arms it.
    #[test]
    fn a_row_check_box_toggles_on_the_release_over_it() {
        use crate::pointer::Interaction;
        let mut lv = view(3);
        lv.checks = Some(vec![false; 3]);
        let on = |lv: &ListView, i: usize| lv.checks.as_ref().unwrap()[i];

        assert!(lv.press_check(Hit::Check(1)), "the press draws it held");
        assert_eq!(lv.held_check.of(1), Interaction::Pressed);
        assert!(!on(&lv, 1), "the press waits for the release");
        assert_eq!(lv.release_check(Hit::Check(1)), Some(1));
        assert!(on(&lv, 1), "the release over it toggles it");
        assert_eq!(lv.held_check.of(1), Interaction::Idle, "at rest after");

        for off in [Hit::Row(1), Hit::Check(2), Hit::Background] {
            lv.press_check(Hit::Check(1));
            assert!(lv.motion_check(off), "{off:?}: dragged off");
            assert_eq!(lv.held_check.of(1), Interaction::Idle, "{off:?}");
            assert_eq!(lv.release_check(off), None, "{off:?}");
            assert_eq!(lv.checks, Some(vec![false, true, false]), "{off:?}");
        }

        lv.press_check(Hit::Check(1));
        lv.motion_check(Hit::Row(1));
        assert!(lv.motion_check(Hit::Check(1)), "back on");
        assert_eq!(lv.held_check.of(1), Interaction::Pressed);
        assert_eq!(lv.release_check(Hit::Check(1)), Some(1));
        assert!(!on(&lv, 1), "back on, the release toggles it");

        assert!(!lv.press_check(Hit::Row(0)), "the row's text holds no box");
        assert!(!lv.motion_check(Hit::Check(0)), "nothing held lights");
        assert_eq!(lv.release_check(Hit::Check(0)), None);
        assert_eq!(lv.checks, Some(vec![false; 3]));
    }

    /// A held row box draws the Pressed frame with its old state; the release draws the new one at rest.
    #[test]
    fn a_held_blue_row_box_draws_pressed_and_ticks_on_the_release() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = theme.look.controls();
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(3);
        lv.checks = Some(vec![false; 3]);
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &cm);
        let cr = lv.check_rect(&lay, 0);
        let px = |lv: &ListView, dx: i32, dy: i32| {
            let mut c = Canvas::new(r.width as u32, r.height as u32);
            draw(&mut c, &text, &theme.paint(), lv, &lay, None, 11);
            c.get(cr.x + dx, cr.y + dy)
        };
        let mut ticked = lv.clone();
        ticked.checks = Some(vec![true; 3]);
        let faces = &crate::blue::Variant::Blue.scheme().check_box;
        let rest = px(&lv, 6, 6);
        lv.press_check(Hit::Check(0));
        assert_ne!(px(&lv, 1, 1), px(&ticked, 1, 1), "the pressed well");
        assert_ne!(px(&lv, 6, 6), rest, "the pressed fill");
        assert_ne!(
            px(&lv, 6, 7),
            0xFF00_0000 | faces.pressed_checked.mark.unwrap().argb(),
            "no tick while held"
        );
        lv.release_check(Hit::Check(0));
        assert_eq!(px(&lv, 1, 1), px(&ticked, 1, 1), "a ticked box at rest");
        assert_eq!(
            px(&lv, 6, 7),
            0xFF00_0000 | faces.normal_checked.mark.unwrap().argb(),
            "the tick"
        );
    }

    #[test]
    fn without_checks_the_box_spot_hits_the_row() {
        let lv = view(3);
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lv.hit(&lay, 10, lay.body.y + 3), Hit::Row(0));
    }

    #[test]
    fn draws_the_selected_name_on_the_highlight() {
        let mut lv = view(3);
        let r = Rect::new(0, 0, 300, 100);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        lv.select(Some(1), &lay);
        let (c, lay, theme) = render(&lv, r);
        let cs = &theme.colors;
        let y = lay.body.y + ROW_H + 8;
        assert_eq!(c.get(lay.body.x + NAME_X - 1, y), cs.hilight.argb());
        assert_eq!(c.get(lay.body.x + NAME_X - 3, y), cs.window.argb());
        assert_eq!(c.get(lay.body.x + 1, lay.body.y + 8), cs.window.argb());
        assert_eq!(c.get(lay.header.x, lay.header.y), cs.button_hilight.argb());
    }

    /// A grouped list: two groups over four rows.
    fn grouped() -> ListView {
        let mut lv = view(4);
        lv.groups = vec![
            Group {
                title: "LAN or High-Speed Internet".into(),
                first: 0,
            },
            Group {
                title: "Wizard".into(),
                first: 2,
            },
        ];
        lv
    }

    #[test]
    fn group_headers_take_a_taller_row() {
        let lv = grouped();
        let lay = layout(&lv, Rect::new(0, 0, 300, 200), &ControlMetrics::CLASSIC);
        assert_eq!(lv.slots(), 6, "four rows and a header over each group");
        let at = |y: i32| lv.hit(&lay, 10, lay.body.y + y + 3);
        // Slots down the body: a 31 px header, two rows, the second header, two more rows.
        assert_eq!(at(0), Hit::Background, "the first group's header");
        assert_eq!(at(GROUP_H), Hit::Row(0));
        assert_eq!(at(GROUP_H + ROW_H), Hit::Row(1));
        assert_eq!(
            at(GROUP_H + 2 * ROW_H),
            Hit::Background,
            "the second header"
        );
        assert_eq!(at(2 * GROUP_H + 2 * ROW_H), Hit::Row(2));
        assert_eq!(at(2 * GROUP_H + 3 * ROW_H), Hit::Row(3));
        assert_eq!(
            at(2 * GROUP_H + 4 * ROW_H),
            Hit::Background,
            "past the last"
        );
        assert_eq!(
            lv.hit(&lay, 10, lay.body.y + GROUP_H + 2 * ROW_H - 1),
            Hit::Row(1)
        );
        assert_eq!(view(4).slots(), 4);
        let plain = layout(
            &view(4),
            Rect::new(0, 0, 300, 200),
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(
            plain.rows,
            layout(
                &view(99),
                Rect::new(0, 0, 300, 200),
                &ControlMetrics::CLASSIC
            )
            .rows
        );
    }

    #[test]
    fn a_borderless_list_lays_out_on_the_rect() {
        // A borderless list puts the header and body on the rect's own top and left.
        let mut lv = view(4);
        lv.frame = false;
        let r = Rect::new(20, 30, 300, 200);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(lay.header, Rect::new(20, 30, 300, HEADER_H));
        assert_eq!(lay.body.x, 20);
        assert_eq!(lay.body.y, 30 + HEADER_H);
        assert_eq!(lay.body.width, 300);
        lv.frame = true;
        let framed = layout(&lv, r, &ControlMetrics::CLASSIC);
        assert_eq!(framed.header, Rect::new(22, 32, 296, HEADER_H));
        assert_eq!(framed.body.x, 22);
        if !crate::text::tahoma_installed() {
            return;
        }
        lv.frame = false;
        let (c, _, theme) = render(&lv, Rect::new(0, 0, 300, 200));
        assert_ne!(
            c.get(0, 0),
            theme.colors.button_shadow.argb(),
            "no sunken edge"
        );
    }

    #[test]
    fn a_group_header_is_bold_over_the_gradient_rule() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let lv = grouped();
        let r = Rect::new(0, 0, 400, 200);
        let (c, lay, theme) = render(&lv, r);
        let cs = &theme.colors;
        let mid = lay.body.y + ROW_H / 2;
        let ink = (lay.body.x..lay.body.right())
            .find(|&x| c.get(x, mid) == cs.window_text.argb())
            .expect("the title's ink");
        assert_eq!(ink, lay.body.x + GROUP_TEXT_X, "inks 14 px in");
        // The rule from that edge: three pixels at 0, 150 and 300 px in.
        let ly = lay.body.y + GROUP_RULE_Y;
        assert_eq!(c.get(lay.body.x, ly), LINE_LEFT.argb(), "the left end");
        assert_eq!(c.get(lay.body.x + 150, ly), LINE_MID.argb(), "half way");
        assert_eq!(
            c.get(lay.body.x + GROUP_LINE_W, ly),
            cs.window.argb(),
            "white once the fade is spent"
        );
        assert_eq!(c.get(lay.body.x, ly - 1), cs.window.argb());
        assert_eq!(c.get(lay.body.x, ly + 1), cs.window.argb());
        assert_eq!(
            c.get(lay.body.x + 1, lay.body.y + ROW_H + 8),
            cs.window.argb()
        );
    }

    #[test]
    fn a_header_at_the_top_pushes_its_own_rows_down() {
        let mut lv = view(10);
        lv.groups = vec![Group {
            title: "LAN or High-Speed Internet".into(),
            first: 0,
        }];
        let r = Rect::new(0, 0, 300, 4 + HEADER_H + 5 * ROW_H);
        let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
        // The 31 px header slot against 17 px rows: four slots fill what held five rows.
        assert_eq!(lay.rows, 4);
        assert_eq!(
            lv.hit(&lay, 10, lay.body.y + GROUP_H + 2 * ROW_H + 3),
            Hit::Row(2)
        );
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        assert_eq!(lv.edit_rect(&lay, 0, &text, px).y, lay.body.y + GROUP_H);
        assert_eq!(
            lv.edit_rect(&lay, 1, &text, px).y,
            lay.body.y + GROUP_H + ROW_H
        );
        // Focus scrolls one row sooner than without the header.
        lv.select(Some(4), &lay);
        assert_eq!(lv.top, 1, "row 4 sits below the header at top 0");
        // Scrolled past the group's start the header is gone: row 4 is three slots down, row 0 one above.
        assert_eq!(lv.edit_rect(&lay, 4, &text, px).y, lay.body.y + 3 * ROW_H);
        assert_eq!(lv.edit_rect(&lay, 0, &text, px).y, lay.body.y - ROW_H);
    }

    #[test]
    #[ignore]
    fn render_listview() {
        let (c, ..) = render(&sample(), Rect::new(0, 0, 320, 130));
        crate::raster::write_preview(&c, "listview-preview.png");
    }

    /// The bar part the host lights draws its Blue Hot frame, a held one Pressed; `Hit::bar` names it.
    #[test]
    fn a_blue_lists_lit_bar_part_draws_hot_or_pressed() {
        use crate::classic::widgets::{Axis, ScrollHit};
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = theme.look.controls();
        let text = TextRenderer::load(&FontConfig::default());
        let mut lv = view(500);
        let r = Rect::new(0, 0, 200, 150);
        let lay = layout(&lv, r, &cm);
        let sb = lay.scrollbar.expect("500 rows overflow the body");
        let up = Some((Axis::Vertical, ScrollHit::Back));
        assert_eq!(lv.hit(&lay, sb.x + 8, sb.y + 2).bar(), up);
        let drawn = |lv: &ListView| {
            let mut c = Canvas::new(r.width as u32, r.height as u32);
            draw(&mut c, &text, &theme.paint(), lv, &lay, lv.bars.lit(), 11);
            c.get(sb.x + 8, sb.y + 2)
        };
        assert!(lv.bars.motion(up));
        assert_eq!(
            drawn(&lv),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Hot,
                8,
                2
            ),
            "the hot up arrow"
        );
        assert!(lv.bars.press_bar(up));
        assert_eq!(
            drawn(&lv),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Pressed,
                8,
                2
            ),
            "the held up arrow"
        );
        assert!(lv.bars.release_bar(), "back to hot");
        assert!(lv.bars.leave());
        assert_eq!(
            drawn(&lv),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Normal,
                8,
                2
            ),
            "the up arrow at rest"
        );
    }
}
