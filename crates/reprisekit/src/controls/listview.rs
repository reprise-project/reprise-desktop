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
    Axis, BarHit, ScrollHit, ScrollParts, axis_lit, bar_hit, draw_focus_ring, edit_first_visible,
    scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar;
use crate::look::Paint;
use crate::look::widgets::header as header_widgets;
use crate::look::widgets::item::{self, Mark};
use crate::look::widgets::{
    draw_checkbox, draw_edit_text, draw_field_frame, draw_hscrollbar, draw_scrollbar,
};
use crate::pointer::{Click, DoubleClick, Lit, Pointer};
use crate::raster::{Canvas, Rect};
use crate::selection::{Mods, Selection, step_index};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, TextEdit, TextSpot};
use crate::theme::{Color, ColorScheme, TextUnits};
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
pub(crate) const ICON_X: i32 = 2;
const NAME_X: i32 = ICON_X + ICON_PX + 2;

/// The name's x in its row past an icon column `slot` px wide.
fn name_x(slot: i32) -> i32 {
    NAME_X - ICON_PX + slot
}
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
    /// 100% px: the layout grows it by the text size.
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
    /// The icon column's width at text size `u`; rows that hand grown icons ([`Rows::icon_in`])
    /// grow it, the rest keep [`ICON_PX`].
    fn icon_slot(&self, u: TextUnits) -> i32 {
        let _ = u;
        ICON_PX
    }
    /// Row `row`'s icon for an icon column `slot` px wide, centred in it when the slot is wider.
    fn icon_in(&self, row: usize, slot: i32) -> Option<&Canvas> {
        let _ = slot;
        self.icon(row)
    }
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
    /// The last left press on a row, for [`ListView::activated`].
    pub clicks: DoubleClick<usize>,
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
            clicks: DoubleClick::default(),
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
    /// How the rows and group headers grow with the text ([`ControlMetrics::text`]).
    pub text: TextUnits,
}

impl Layout {
    /// A row's height at the text size.
    pub fn row_h(&self) -> i32 {
        self.text.len(ROW_H)
    }

    /// A group header's slot at the text size.
    pub fn group_h(&self) -> i32 {
        self.text.len(GROUP_H)
    }

    /// `col`'s width at the text size.
    pub fn col_w(&self, col: &Column) -> i32 {
        self.text.len(col.width)
    }
}

pub fn layout<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>>(
    lv: &ListView<R, C, S>,
    r: Rect,
    cm: &ControlMetrics,
) -> Layout {
    let inner = if lv.frame { r.inset(2) } else { r };
    let header = Rect::new(inner.x, inner.y, inner.width, cm.list_header);
    let (full_w, full_h) = (inner.width, inner.height - cm.list_header);
    let u = cm.text;
    let total_w: i32 = lv.columns.as_ref().iter().map(|c| u.len(c.width)).sum();
    let content_h: i32 = if lv.groups.is_empty() {
        lv.rows.len() as i32 * u.len(ROW_H)
    } else {
        lv.slots_from(0).map(|sl| slot_h(sl, u)).sum()
    };
    // Reserving one bar can make the other necessary and neither is given back: two rounds converge.
    let (mut vbar, mut hbar) = (false, false);
    for _ in 0..2 {
        let w = full_w - if vbar { cm.scroll } else { 0 };
        let h = full_h - if hbar { cm.scroll } else { 0 };
        hbar |= total_w > w;
        vbar |= content_h > h.max(u.len(ROW_H));
    }
    let body = Rect::new(
        inner.x,
        header.bottom(),
        full_w - if vbar { cm.scroll } else { 0 },
        full_h - if hbar { cm.scroll } else { 0 },
    );
    let rows = lv.visible(body.height, u);
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
        text: u,
    }
}

/// A row's check box, centred down a row taller than 17 px.
fn check_box(lay: &Layout, at: i32) -> Rect {
    Rect::new(
        lay.frame.x + CHECK_X,
        lay.body.y + at + CHECK_Y + (lay.row_h() - ROW_H) / 2,
        lay.check,
        lay.check,
    )
}

/// How tall a slot is at the text size `u`: [`GROUP_H`] for a header, [`ROW_H`] for a row.
fn slot_h(slot: Slot, u: TextUnits) -> i32 {
    u.len(match slot {
        Slot::Header(_) => GROUP_H,
        Slot::Row(_) => ROW_H,
    })
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
    fn slots_at(&self, from: usize, u: TextUnits) -> impl Iterator<Item = (Slot, i32)> + '_ {
        let mut y = 0;
        self.slots_from(from).map(move |s| {
            let at = y;
            y += slot_h(s, u);
            (s, at)
        })
    }

    /// How many slot positions `height` px hold from `top`: the slots, then empty [`ROW_H`] positions
    /// once they run out.
    fn visible(&self, height: i32, u: TextUnits) -> usize {
        let mut slots = self.slots_from(self.top);
        let (mut y, mut n) = (0, 0);
        loop {
            let h = slot_h(slots.next().unwrap_or(Slot::Row(0)), u);
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
    fn slot_y(&self, row: usize, lay: &Layout) -> i32 {
        let (row_h, group_h) = (lay.row_h(), lay.group_h());
        let heads = |lo: usize, hi: usize| {
            self.groups
                .iter()
                .filter(|g| (lo..=hi).contains(&g.first))
                .count() as i32
        };
        if row >= self.top {
            (row - self.top) as i32 * row_h + heads(self.top, row) * group_h
        } else {
            -((self.top - row) as i32 * row_h + heads(row + 1, self.top) * group_h)
        }
    }

    /// The largest `top` that still fills the body.
    fn max_top(&self, lay: &Layout) -> usize {
        if self.groups.is_empty() {
            return self.rows.len().saturating_sub(lay.rows);
        }
        (0..self.rows.len())
            .rev()
            .take_while(|&r| {
                let h: i32 = self.slots_from(r).map(|sl| slot_h(sl, lay.text)).sum();
                h <= lay.body.height
            })
            .last()
            .unwrap_or(self.rows.len().saturating_sub(1))
    }

    pub fn follow(&mut self, lay: &Layout) {
        if let Some(i) = self.selection.borrow().focus {
            self.show(i, lay);
        }
    }

    /// Scroll row `i` into view without picking it, for a reader's ScrollIntoView. True when it moved.
    pub fn reveal(&mut self, i: usize, lay: &Layout) -> bool {
        let top = self.top;
        if i < self.rows.len() {
            self.show(i, lay);
        }
        self.top != top
    }

    fn show(&mut self, i: usize, lay: &Layout) {
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
            let need = lay.row_h() + if head { lay.group_h() } else { 0 };
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
                let edge = cx + lay.col_w(col);
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
            for (slot, at) in self.slots_at(self.top, lay.text).take(lay.rows) {
                if dy >= at + slot_h(slot, lay.text) {
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
        let y = lay.body.y + self.slot_y(row, lay);
        let cx = lay.body.x - self.left_in(lay);
        let cw = self.columns.as_ref().first().map_or(0, |c| lay.col_w(c));
        let tx = match &self.checks {
            Some(_) => lay.frame.x + CHECK_X + lay.check + CHECK_TEXT_GAP,
            None => cx + name_x(self.rows.icon_slot(lay.text)),
        };
        let name = self.rows.cell(row, 0);
        let shown = text.ellipsize(&name, (cx + cw - tx - 2).max(0), false, px);
        let right = (tx + text.measure(&shown, false, px) + 2).min(cx + cw);
        Rect::new(tx - 2, y, (right - tx + 2).max(0), lay.row_h())
    }

    /// The rename box over `row`'s name cell, border included, sized to its text.
    pub fn edit_rect(&self, lay: &Layout, row: usize, text: &TextRenderer, px: u32) -> Rect {
        let shown = match &self.edit {
            Some((i, ed)) if *i == row => ed.text.as_str(),
            _ => &self.rows.cell(row, 0),
        };
        let y = lay.body.y + self.slot_y(row, lay);
        let tx = lay.body.x - self.left_in(lay) + name_x(self.rows.icon_slot(lay.text));
        let w = text.measure(shown, false, px);
        Rect::new(tx - EDIT_BACK, y, w + EDIT_PAD_L + EDIT_PAD_R, lay.row_h())
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
        scrollbar::hparts(
            hb,
            self.left_in(lay),
            lay.body.width,
            lay.total_w,
            lay.thumb_min,
        )
    }

    /// The offset a horizontal thumb grabbed `grab` px right of its left edge scrolls to at pointer `x`.
    pub fn left_at(&self, lay: &Layout, x: i32, grab: i32) -> i32 {
        match lay.hscroll {
            Some(hb) => {
                let p = self.hparts(lay, hb);
                scrollbar::left_at(&p, x, grab, lay.body.width, lay.total_w)
            }
            None => self.left_in(lay),
        }
    }

    /// Row `row`'s check box in control space: [`CHECK_X`] from the frame edge, [`CHECK_Y`] below the row top.
    pub fn check_rect(&self, lay: &Layout, row: usize) -> Rect {
        check_box(lay, self.slot_y(row, lay))
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

    /// A left press `at` on `hit`: the row it activates (Enter's twin) when it is the second press on that row within
    /// the double-click time and distance. Every left press goes through it, so one elsewhere breaks the pair.
    pub fn activated(&mut self, hit: Hit, at: Click) -> Option<usize> {
        match hit {
            Hit::Row(i) => self.clicks.press(i, at).then_some(i),
            _ => {
                self.clicks.reset();
                None
            }
        }
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

    /// Set column `i` to `width` 100% px, held to [`MIN_COL_W`].
    pub fn resize_column(&mut self, i: usize, width: i32) {
        if let Some(col) = self.columns.get_mut(i) {
            col.width = width.max(MIN_COL_W);
        }
    }

    /// Give column `i` what the other columns, grown by `u`, leave of `width` px: the longest
    /// 100% width that still fits, so the columns fill `width` without overflowing it.
    pub fn fill_column(&mut self, i: usize, width: i32, u: TextUnits) {
        let others: i32 = (self.columns.iter().enumerate())
            .filter(|&(k, _)| k != i)
            .map(|(_, c)| u.len(c.width))
            .sum();
        self.resize_column(i, u.within(width - others));
    }

    /// [`Self::fill_column`] to what a list laid out in `rect` keeps beside its vertical bar,
    /// so a list that scrolls grows no horizontal bar.
    pub fn fill_beside_bar(&mut self, i: usize, rect: Rect, cm: &ControlMetrics) {
        let inner = if self.frame { rect.inset(2) } else { rect };
        self.fill_column(i, inner.width - cm.scroll, cm.text);
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
    lay: &Layout,
    header: Rect,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let left = lv.left_in(lay);
    header_widgets::draw_strip(c, p, header);
    let arrow_w = header_widgets::sort_arrow_width(p);
    let arrow_over = p.look.controls().header_arrow_over;
    let mut cx = header.x - left * s;
    for (ci, col) in lv.columns.as_ref().iter().enumerate() {
        let r = Rect::new(cx, header.y, lay.col_w(col) * s, header.height);
        cx += lay.col_w(col) * s;
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
        let (tr, right_arrow) = title_room(p, col, t, sorted.is_some());
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

/// Where `col`'s title goes in its item's title rect `t`, and the x of a right-aligned sorted
/// column's arrow. `sorted` is set when the arrow sits beside the title rather than over it.
fn title_room(p: &Paint, col: &Column, t: Rect, sorted: bool) -> (Rect, Option<i32>) {
    let s = p.s;
    let arrow_w = header_widgets::sort_arrow_width(p);
    let arrow_end = p.look.controls().header_arrow_end;
    // A right-aligned sorted column's arrow stops `header_arrow_end` short of the end and its title
    // ends the arrow gap before it; a left-aligned title keeps its place and the arrow follows.
    let right_arrow = (sorted && col.right).then(|| t.right() - (arrow_end + arrow_w) * s);
    let tr_right = match right_arrow {
        Some(ax) => ax - ARROW_GAP * s,
        None if sorted => t.right() - (HEADER_TEXT_X + ARROW_GAP + arrow_w) * s,
        None => t.right() - HEADER_TEXT_X * s,
    };
    let tx = t.x + HEADER_TEXT_X * s;
    (
        Rect::new(tx, t.y, (tr_right - tx).max(0), t.height),
        right_arrow,
    )
}

/// The columns whose header title runs past its room, by index and px over, so a host's audit
/// test catches a title the header would ellipsize.
pub fn header_overflows<R: Rows, C: AsRef<[Column]>, S: Borrow<Selection>>(
    lv: &ListView<R, C, S>,
    lay: &Layout,
    text: &TextRenderer,
    p: &Paint,
    px: u32,
) -> Vec<(usize, i32)> {
    let arrow_over = p.look.controls().header_arrow_over;
    lv.columns
        .as_ref()
        .iter()
        .enumerate()
        .filter_map(|(ci, col)| {
            let r = Rect::new(0, 0, lay.col_w(col) * p.s, lay.header.height * p.s);
            let sorted = lv.sort.is_some_and(|(i, _)| i == ci);
            let t = header_widgets::item_title_rect(p, r, lv.header.of(ci), sorted);
            let (tr, _) = title_room(p, col, t, sorted && !arrow_over);
            let by = text.measure(&col.title, false, px) - tr.width;
            (by > 0).then_some((ci, by))
        })
        .collect()
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
                c.fill_rect(Rect::new(cx, body.y, lay.col_w(col) * s, body.height), tint);
                break;
            }
            cx += lay.col_w(col) * s;
        }
    }
    let row_h = lay.row_h() * s;
    let icon_w = lv.rows.icon_slot(lay.text);
    let title_h = lay.text.len(GROUP_TITLE_H);
    let rule = GroupRule {
        y: title_h - 1,
        ..list_rule(cs)
    };
    for (slot, at) in lv.slots_at(lv.top, lay.text).take(lay.rows) {
        let y = body.y + at * s;
        let i = match slot {
            Slot::Header(g) => {
                let r = Rect::new(body.x, y, body.width, lay.group_h() * s);
                let title_box = (GROUP_TEXT_DY * s, title_h * s);
                let title = &lv.groups[g].title;
                draw_group_header(c, text, cs, r, title, title_box, &rule, px, s);
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
            let cw = lay.col_w(col) * s;
            let (tx, tw) = if ci == 0 {
                if let Some(checks) = &lv.checks {
                    let cr = check_box(lay, at).scaled(s);
                    let checked = checks.get(i).copied().unwrap_or(false);
                    draw_checkbox(c, p, cr, checked, lv.held_check.of(i));
                    let tx = cr.right() + CHECK_TEXT_GAP * s;
                    (tx, cx + cw - tx - 2 * s)
                } else {
                    if let Some(ic) = lv.rows.icon_in(i, icon_w) {
                        let dx = match icon_w > ICON_PX {
                            true => (icon_w * s - ic.width as i32).max(0) / 2,
                            false => 0,
                        };
                        c.blit_over(cx + ICON_X * s + dx, y + (row_h - ic.height as i32) / 2, ic);
                    }
                    let name_x = name_x(icon_w);
                    (cx + name_x * s, cw - name_x * s - 2 * s)
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

/// A group header's rule `y` px down the header: `from` at the left edge fading linearly to `to`
/// over `width` px.
pub(crate) struct GroupRule {
    pub from: Color,
    pub to: Color,
    pub width: i32,
    pub y: i32,
}

pub(crate) fn list_rule(cs: &ColorScheme) -> GroupRule {
    GroupRule {
        from: GROUP_LINE,
        to: cs.window,
        width: GROUP_LINE_W,
        y: GROUP_RULE_Y,
    }
}

/// A group header in the already-scaled `r`: bold title [`GROUP_TEXT_X`] in from the left edge in
/// `title_box` (top offset, height), then `rule` its `y` down. Shared with the shell view's icon modes.
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
    let ly = r.y + rule.y * s;
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
    draw_edit_text(c, text, ed, inner, px, p, Caret::Shown, first);
    // The field is the whole box: a press on its frame is a press in the field.
    if let Some(spot) = c.take_spot() {
        c.report_spot(TextSpot { field: b, ..spot });
    }
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
    c.with_clip(header, |c| draw_header(c, text, p, lv, lay, header, px));
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
    /// The list view's `access` part: one option per row in view and for the focused row, named by
    /// its first column, described by the others and placed in the set by its index. Selected rows
    /// are marked, check rows carry their check. The list's bounds are the body, so a row scrolled
    /// out lies outside them, where a reader can ask to scroll it in. Returns the focused row's
    /// node when the list has the focus, else the list.
    pub fn access(
        &self,
        lay: &Layout,
        tree: &mut Tree,
        parent: NodeId,
        key: impl std::hash::Hash,
    ) -> NodeId {
        let mut n = access::node(Role::ListBox, lay.body);
        n.set_multiselectable();
        access::focusable(&mut n, true);
        let row_h = f64::from(lay.row_h());
        n.set_scroll_y(self.top as f64 * row_h);
        n.set_scroll_y_min(0.0);
        n.set_scroll_y_max(self.max_top(lay) as f64 * row_h);
        let id = tree.push(parent, key, n);
        let sel = self.selection.borrow();
        let cols = self.columns.as_ref();
        let mut focus = id;
        let count = self.rows.len();
        let shown = self.top.min(count)..(self.top + lay.rows + 1).min(count);
        let off = sel.focus.filter(|f| *f < count && !shown.contains(f));
        let before = off.filter(|f| *f < shown.start);
        let after = off.filter(|f| *f >= shown.end);
        for i in before.into_iter().chain(shown).chain(after) {
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
            o.set_position_in_set(i + 1);
            o.set_size_of_set(count);
            if let Some(on) = self.checks.as_ref().and_then(|c| c.get(i)) {
                o.set_toggled((*on).into());
            }
            let y = lay.body.y + self.slot_y(i, lay);
            let row = Rect::new(lay.body.x, y, lay.body.width, lay.row_h());
            o.set_bounds(access::row_bounds(lay.body, row));
            let oid = tree.push(id, i, o);
            if sel.focus == Some(i) {
                focus = oid;
            }
        }
        focus
    }
}

#[cfg(test)]
mod tests;
