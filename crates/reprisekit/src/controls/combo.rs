//! The combo box: a sunken field with a drop button and, while open, the list under it in the look's 1 px frame, with a scrollbar once the items outgrow `rows`.
//!
//! Two kinds: the plain one (`Combo::new`) shows the pick on a highlight in a label field, with rows the look's `combo_row` tall; the shell one (`Combo::with_items` with icons) has 16 px rows with an icon at an indent level and the highlight over the label only. With `edit` set the field is an icon and a [`TextEdit`] (the address bar, see [`addressbar`](crate::controls::addressbar)).
//!
//! An editable combo without icons is a command line over its history (Run): picking or stepping to an item puts its text in the field, selected, and the list opens on the item the field holds.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    BarHit, FIELD_PAD, LINE_H, ROW_H, RingInk, ScrollHit, ScrollParts, access_edit, draw_edit,
    draw_edit_drop_point, draw_focus_ring_screen, edit_first_visible, scrollbar_hit,
    scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::hosted::{Anchor, Hosted, Spot};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar::ThumbDrag;
use crate::look::Paint;
use crate::look::widgets::{
    ComboLook, draw_combo_drop, draw_combo_frame, draw_disabled_label, draw_drop_list,
    draw_scrollbar,
};
use crate::pointer::Axis;
use crate::pointer::Interaction;
use crate::pointer::Lit;
use crate::raster::{Canvas, Rect};
use crate::selection::scroll_into_view;
use crate::style::Part;
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, TextEdit};
use accesskit::{Node, NodeId, Role};
use std::cell::Cell;

/// One row of the drop list: label, optional 16 px icon and indent level (levels under the root). A list with no icon anywhere is plain and ignores indents. A greyed row draws in `gray_text` and is never highlighted or picked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListItem {
    pub text: String,
    pub icon: Option<Canvas>,
    pub indent: i32,
    pub greyed: bool,
}

impl ListItem {
    pub fn new(text: impl Into<String>) -> ListItem {
        ListItem {
            text: text.into(),
            icon: None,
            indent: 0,
            greyed: false,
        }
    }

    pub fn shell(text: impl Into<String>, icon: Option<Canvas>, indent: i32) -> ListItem {
        ListItem {
            text: text.into(),
            icon,
            indent,
            greyed: false,
        }
    }
}

/// A row with an icon, at 100% text: it grows with the text and centres its 16 px icon.
pub const SHELL_ROW_H: i32 = 16;

/// One indent level, in px.
pub const INDENT_STEP: i32 = 10;

/// A shell row's icon sits this far inside the list's frame, past the
/// 1 px frame line.
const LIST_ICON_X: i32 = 3;

/// The gap between a 16 px icon and the label after it, in the list and
/// in the field alike.
const ICON_GAP: i32 = 4;

fn shell(items: &[ListItem]) -> bool {
    items.iter().any(|i| i.icon.is_some())
}

/// A shell list's icon column: 16 px, or as wide as its widest icon drawn `device` px per px,
/// up to the `row_h` row it sits in.
fn icon_slot(items: &[ListItem], device: i32, row_h: i32) -> i32 {
    let widest = items
        .iter()
        .filter_map(|i| i.icon.as_ref())
        .map(|c| c.width as i32 / device.max(1))
        .max()
        .unwrap_or(0);
    widest.min(row_h).max(16)
}

fn items_row_h(items: &[ListItem], cm: &ControlMetrics) -> i32 {
    if shell(items) {
        cm.text.len(SHELL_ROW_H)
    } else {
        cm.combo_row
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combo {
    pub items: Vec<ListItem>,
    pub selected: usize,
    pub open: Option<usize>,
    pub top: usize,
    pub rows: usize,
    pub enabled: bool,
    /// Set: the field is an edit the user types into, not a label showing
    /// the pick.
    pub edit: Option<TextEdit>,
    /// The edit's first visible byte, a draw-time cache `draw` keeps in
    /// step with the caret so the text scrolls only as far as it must.
    pub first: Cell<usize>,
    /// Drawn in the field before the text (the address bar's place).
    pub icon: Option<Canvas>,
    /// A drag of text resting on the editable field, at this logical x ([`Combo::drag_over`]).
    pub drop_x: Option<i32>,
    /// The open list's corner where a host showing it on a popup put it ([`list_rect`]).
    pub list_at: Option<(i32, i32)>,
    /// A host shows the open list on a popup: its owner's draw leaves it out.
    pub hosted: bool,
    /// The first row [`Combo::select_text`] added for a value the list lacked.
    pub added_at: Option<usize>,
}

impl Combo {
    pub fn new(items: &[&str], selected: usize, rows: usize) -> Combo {
        Combo::with_items(
            items.iter().map(|s| ListItem::new(*s)).collect(),
            selected,
            rows,
        )
    }

    pub fn with_items(items: Vec<ListItem>, selected: usize, rows: usize) -> Combo {
        Combo {
            items,
            selected,
            open: None,
            top: 0,
            rows: rows.max(1),
            enabled: true,
            edit: None,
            first: Cell::new(0),
            icon: None,
            drop_x: None,
            list_at: None,
            hosted: false,
            added_at: None,
        }
    }

    /// Selects the row showing `want`, adding it after the listed rows when they lack it. A row
    /// added for an earlier value goes first.
    pub fn select_text(&mut self, want: &str) {
        if let Some(n) = self.added_at.take() {
            self.items.truncate(n);
        }
        if !self.items.iter().any(|i| i.text == want) {
            self.added_at = Some(self.items.len());
            self.items.push(ListItem::new(want));
        }
        self.selected = self.items.iter().position(|i| i.text == want).unwrap_or(0);
    }

    fn matching(&self) -> Option<usize> {
        let text = &self.edit.as_ref()?.text;
        self.items.iter().position(|i| i.text == *text)
    }

    fn fill(&mut self, i: usize) {
        if let (Some(edit), Some(item)) = (self.edit.as_mut(), self.items.get(i)) {
            edit.set_selected(item.text.clone());
        }
    }

    pub fn value(&self) -> &str {
        self.items
            .get(self.selected)
            .map_or("", |i| i.text.as_str())
    }

    pub fn text(&self) -> &str {
        match &self.edit {
            Some(e) => &e.text,
            None => self.value(),
        }
    }

    /// Replace an editable combo's text, caret at the end; does nothing to a plain combo.
    pub fn set_text(&mut self, text: &str) {
        if self.edit.is_some() {
            self.edit = Some(TextEdit::new(text, false));
        }
    }

    /// The byte index nearest `x` px from the start of an editable
    /// combo's text as it was last drawn, scrolled by `first`.
    pub fn edit_index(&self, text: &TextRenderer, px: u32, x: i32) -> usize {
        let Some(edit) = &self.edit else {
            return 0;
        };
        let from = crate::controls::form::visible_from(edit, &self.first);
        from + crate::textedit::index_at(text, &edit.text[from..], x, px)
    }

    pub fn row_h(&self, cm: &ControlMetrics) -> i32 {
        items_row_h(&self.items, cm)
    }

    fn visible(&self) -> usize {
        self.rows.min(self.items.len()).max(1)
    }

    fn keep_in_view(&mut self, i: usize) {
        let visible = self.visible();
        scroll_into_view(&mut self.top, i, visible);
    }

    pub fn toggle(&mut self) -> bool {
        if !self.enabled || self.items.is_empty() {
            return false;
        }
        if self.open.is_some() {
            self.open = None;
        } else {
            let at = self.matching().unwrap_or(self.selected);
            self.open = Some(at);
            self.list_at = None;
            self.top = at.saturating_sub(self.visible() - 1).min(at);
        }
        true
    }

    pub fn pick(&mut self, i: usize) -> bool {
        if self.items.get(i).is_some_and(|it| it.greyed) {
            return false;
        }
        self.open = None;
        if i >= self.items.len() {
            return false;
        }
        let was = self.edit.as_ref().map(|e| e.text.clone());
        self.fill(i);
        let changed = i != self.selected || was != self.edit.as_ref().map(|e| e.text.clone());
        self.selected = i;
        changed
    }

    /// Up / Down: with the list open move its highlight, else pick the neighbouring item, skipping greyed rows. An editable combo's field takes the item's text and steps from the item it holds (Down from other text goes to the first, Up does nothing). Returns whether the pick or field changed.
    pub fn step(&mut self, down: bool) -> bool {
        let n = self.items.len();
        if !self.enabled || n == 0 {
            return false;
        }
        if self.edit.is_some() {
            let next = match (self.open.or_else(|| self.matching()), down) {
                (None, true) => 0,
                (None, false) => return false,
                (Some(i), true) => (i + 1).min(n - 1),
                (Some(i), false) => i.saturating_sub(1),
            };
            if self.matching() == Some(next) && self.open.is_none_or(|o| o == next) {
                return false;
            }
            self.fill(next);
            if self.open.is_some() {
                self.open = Some(next);
                self.keep_in_view(next);
            }
            return true;
        }
        let cur = self.open.unwrap_or(self.selected);
        let next = if down {
            (cur + 1..n).find(|&i| !self.items[i].greyed)
        } else {
            (0..cur).rev().find(|&i| !self.items[i].greyed)
        }
        .unwrap_or(cur);
        self.move_to(next)
    }

    /// Move the open list's highlight to `i`, scrolled into view, or with the list closed pick `i`.
    /// True when the pick changed.
    fn move_to(&mut self, i: usize) -> bool {
        match self.open {
            Some(_) => {
                self.open = Some(i);
                self.keep_in_view(i);
                false
            }
            None => {
                let changed = i != self.selected;
                self.selected = i;
                changed
            }
        }
    }

    pub fn jump(&mut self, end: bool) -> bool {
        let n = self.items.len();
        if !self.enabled || n == 0 {
            return false;
        }
        let open = |&i: &usize| !self.items[i].greyed;
        let i = if end {
            (0..n).rev().find(open)
        } else {
            (0..n).find(open)
        };
        let Some(i) = i else {
            return false;
        };
        self.move_to(i)
    }

    pub fn hover(&mut self, i: usize) -> bool {
        let follows =
            self.open.is_some_and(|o| o != i) && self.items.get(i).is_some_and(|it| !it.greyed);
        if follows {
            self.open = Some(i);
        }
        follows
    }

    pub fn take(&mut self) -> bool {
        match self.open {
            Some(i) => self.pick(i),
            None => false,
        }
    }

    /// A reader asked to see row `i` of the open list: it scrolls in and the highlight stays.
    /// True when it moved.
    pub fn reveal(&mut self, i: usize) -> bool {
        let top = self.top;
        if self.open.is_some() && i < self.items.len() {
            self.keep_in_view(i);
        }
        self.top != top
    }

    pub fn scroll(&mut self, rows: i32) -> bool {
        let max_top = self.items.len().saturating_sub(self.visible());
        let top = (self.top as i32 + rows).clamp(0, max_top as i32) as usize;
        if top == self.top {
            return false;
        }
        self.top = top;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub field: Rect,
    pub drop: Rect,
    pub icon: Option<Rect>,
    pub highlight: Rect,
    pub text: Rect,
    /// The open list under the field, and its scrollbar when the items
    /// outgrow the rows.
    pub list: Rect,
    pub scrollbar: Option<Rect>,
    /// The look's shortest scrollbar thumb, which hit and drag lay the
    /// bar out with as `draw` does.
    pub thumb_min: i32,
    /// The open list's row height, which hit and access lay the rows out
    /// with as `draw_list` does.
    pub row_h: i32,
}

pub const COMBO_H: i32 = ROW_H;

/// How far into the field an editable combo without icons starts its
/// text: 3 px further in than a plain edit's `FIELD_PAD`.
pub const EDIT_TEXT_X: i32 = 6;

/// Where the open list goes under `field`: its rows plus a 1 px border above and below (a shadow falls outside).
pub fn list_rect(combo: &Combo, field: Rect, cm: &ControlMetrics) -> Rect {
    list_rect_rows(combo, field, combo.row_h(cm))
}

fn list_rect_rows(combo: &Combo, field: Rect, row_h: i32) -> Rect {
    let n = combo.visible() as i32;
    let (x, y) = combo.list_at.unwrap_or((field.x, field.bottom()));
    Rect::new(x, y, field.width, n * row_h + 2)
}

/// A combo's open list for a host that shows it on a popup: `field` is the combo's rect and
/// `cm` its sizes as its owner lays it out, `px` its text px. The owner draws at `s: 1`.
pub struct Popped<'a> {
    pub combo: &'a mut Combo,
    pub field: Rect,
    pub cm: ControlMetrics,
    pub px: u32,
}

/// Dismissing it closes the list.
impl Hosted for Popped<'_> {
    fn spots(&self) -> Vec<Spot> {
        if self.combo.open.is_none() {
            return Vec::new();
        }
        vec![Spot {
            rect: layout(self.combo, self.field, &self.cm).list,
            anchor: Anchor::Below(self.field),
        }]
    }

    fn place(&mut self, level: usize, origin: (i32, i32)) {
        if level == 0 {
            self.combo.list_at = Some(origin);
        }
    }

    fn shadow(&self, theme: &crate::theme::Theme) -> i32 {
        list_shadow(theme)
    }

    fn draw_level(
        &self,
        level: usize,
        c: &mut Canvas,
        by: (i32, i32),
        theme: &crate::theme::Theme,
        text: &TextRenderer,
    ) {
        if level != 0 || self.combo.open.is_none() {
            return;
        }
        let lay = layout(self.combo, self.field, &self.cm);
        let moved = |r: Rect| Rect::new(r.x + by.0, r.y + by.1, r.width, r.height);
        draw_list(
            c,
            text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &self.combo.items,
            self.combo.open,
            self.combo.top,
            self.combo.rows,
            moved(lay.list),
            lay.scrollbar.map(moved),
            None,
            self.px,
        );
    }

    fn host(&mut self) {
        self.combo.hosted = true;
    }

    fn dismiss(&mut self) {
        self.combo.open = None;
    }
}

/// How far right and down of the open list its shadow reaches under `theme`'s look, logical px.
pub fn list_shadow(theme: &crate::theme::Theme) -> i32 {
    match theme
        .look
        .style()
        .and_then(|st| st.drop_list)
        .and_then(|d| d.shadow)
    {
        Some(_) => crate::style::DropShadow::N as i32,
        None => 0,
    }
}

pub fn layout(combo: &Combo, field: Rect, cm: &ControlMetrics) -> Layout {
    layout_rows(combo, field, cm, combo.row_h(cm))
}

/// [`layout`] with the open list's rows `row_h` tall; draw it with [`draw_list_rows`].
pub fn layout_rows(combo: &Combo, field: Rect, cm: &ControlMetrics, row_h: i32) -> Layout {
    let inner = field.inset(2);
    let drop = Rect::new(
        inner.right() - cm.combo_drop,
        inner.y,
        cm.combo_drop,
        inner.height,
    );
    // The place icon sits 3 px inside the frame, centred in the field's
    // 18 px inner box.
    let icon = combo.icon.as_ref().map(|_| {
        Rect::new(
            field.x + FIELD_PAD,
            inner.y + (inner.height - 16) / 2,
            16,
            16,
        )
    });
    let text_x = match (icon, combo.edit.is_some()) {
        (Some(i), _) => i.right() + ICON_GAP,
        (None, true) => field.x + EDIT_TEXT_X,
        (None, false) => inner.x + 2,
    };
    let highlight = Rect::new(
        inner.x + 1,
        inner.y + 1,
        drop.x - cm.combo_gap - inner.x - 1,
        inner.height - 2,
    );
    // The address bar's edit sits 1 px below the centre of the inner
    // box; a plain editable one starts `EDIT_TEXT_X` in and fills the
    // field's height less its 3 px pad.
    let text = match (icon, combo.edit.is_some()) {
        (None, true) => Rect::new(
            text_x,
            field.y + FIELD_PAD,
            (drop.x - 2 - text_x).max(0),
            (field.height - 2 * FIELD_PAD).max(0),
        ),
        _ => Rect::new(
            text_x,
            inner.y + combo.edit.is_some() as i32,
            (drop.x - 2 - text_x).max(0),
            inner.height,
        ),
    };
    let list = list_rect_rows(combo, field, row_h);
    let scrollbar = (combo.items.len() > combo.rows).then(|| {
        Rect::new(
            list.right() - 1 - cm.scroll,
            list.y + 1,
            cm.scroll,
            list.height - 2,
        )
    });
    Layout {
        field,
        drop,
        icon,
        highlight,
        text,
        list,
        scrollbar,
        thumb_min: cm.thumb_min,
        row_h,
    }
}

/// Widens `lay`'s open list past its field to the width its widest row's text takes at `px`,
/// to `right` at most; a row still wider ends in an ellipsis when drawn.
pub fn widen_list(combo: &Combo, lay: &mut Layout, text: &TextRenderer, px: u32, right: i32) {
    let shell = shell(&combo.items);
    let slot = icon_slot(&combo.items, 1, lay.row_h);
    let need = combo
        .items
        .iter()
        .map(|it| {
            let start = match shell {
                true => LIST_ICON_X + it.indent * INDENT_STEP + slot + ICON_GAP,
                false => 2,
            };
            1 + start + text.measure(&it.text, false, px) + 3
        })
        .max()
        .unwrap_or(0)
        + lay.scrollbar.map_or(0, |sb| sb.width);
    let width = need.min(right - lay.list.x).max(lay.list.width);
    let grow = width - lay.list.width;
    lay.list.width = width;
    if let Some(sb) = &mut lay.scrollbar {
        sb.x += grow;
    }
}

/// What a point lands on. The open list covers whatever is under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Field,
    Drop,
    Row(usize),
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Thumb(i32),
}

pub fn hit(combo: &Combo, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    if combo.open.is_some() && lay.list.contains(x, y) {
        if let Some(sb) = lay.scrollbar.filter(|sb| sb.contains(x, y)) {
            let p = scroll_parts(combo, lay, sb);
            return match scrollbar_hit(&p, x, y, false)? {
                ScrollHit::Back => Some(Hit::ScrollUp),
                ScrollHit::Forward => Some(Hit::ScrollDown),
                ScrollHit::PageBack => Some(Hit::PageUp),
                ScrollHit::PageForward => Some(Hit::PageDown),
                ScrollHit::Thumb => Some(Hit::Thumb(y - p.thumb.map_or(0, |t| t.y))),
            };
        }
        // The frame's bottom pixel sits under the last row in view: no
        // row, rather than the hidden one after it.
        let k = ((y - lay.list.y - 1) / lay.row_h).max(0) as usize;
        let row = combo.top + k;
        return (k < combo.visible() && row < combo.items.len()).then_some(Hit::Row(row));
    }
    if lay.drop.contains(x, y) {
        return Some(Hit::Drop);
    }
    lay.field.contains(x, y).then_some(Hit::Field)
}

fn scroll_parts(combo: &Combo, lay: &Layout, sb: Rect) -> ScrollParts {
    let (top, visible, total) = (combo.top, combo.visible(), combo.items.len());
    scrollbar_parts(sb, 1, top, visible, total, false, lay.thumb_min)
}

pub fn top_at(combo: &Combo, lay: &Layout, y: i32, grab: i32) -> usize {
    match lay.scrollbar {
        Some(sb) => {
            let p = scroll_parts(combo, lay, sb);
            scrollbar_pos_at(&p, y - grab, combo.visible(), combo.items.len(), false)
        }
        None => combo.top,
    }
}

impl Hit {
    pub fn bar(self) -> Option<BarHit> {
        match self {
            Hit::ScrollUp => Some(BarHit::Back),
            Hit::ScrollDown => Some(BarHit::Forward),
            Hit::PageUp => Some(BarHit::PageBack),
            Hit::PageDown => Some(BarHit::PageForward),
            Hit::Thumb(grab) => Some(BarHit::Thumb(grab)),
            Hit::Field | Hit::Drop | Hit::Row(_) => None,
        }
    }
}

impl Combo {
    /// A drag at (`x`, `y`): over an enabled editable field with its list closed the field shows
    /// a drop caret there and takes the drag (`true`); anywhere else the drop caret goes.
    pub fn drag_over(&mut self, lay: &Layout, x: i32, y: i32) -> bool {
        let takes = self.enabled
            && self.open.is_none()
            && self.edit.as_ref().is_some_and(|e| !e.is_secret())
            && hit(self, lay, x, y) == Some(Hit::Field);
        self.drop_x = takes.then_some(x);
        takes
    }

    /// The drag left or ended: `true` when a drop caret was showing.
    pub fn drag_leave(&mut self) -> bool {
        self.drop_x.take().is_some()
    }

    /// The scroll offset one step on for a drag resting on the field
    /// ([`crate::textedit::drag_scroll`], a [`LINE_H`] band at each side), None when it stays.
    pub fn drag_scroll(&self, lay: &Layout, text: &TextRenderer, px: u32) -> Option<usize> {
        let (x, edit) = (self.drop_x?, self.edit.as_ref()?);
        let from = crate::controls::form::visible_from(edit, &self.first);
        let w = lay.text.width;
        crate::textedit::drag_scroll(text, &edit.text, from, w, LINE_H, x - lay.text.x, px)
    }

    /// Take [`Combo::drag_scroll`]'s step, for a host's tick; `true` when the field moved.
    pub fn drag_tick(&mut self, lay: &Layout, text: &TextRenderer, px: u32) -> bool {
        let Some(to) = self.drag_scroll(lay, text, px) else {
            return false;
        };
        self.first.set(to);
        true
    }

    /// A drag dropped `line` on the field: it goes in at the drop caret as
    /// [`TextEdit::drop_text`] puts it. `false` when the field was not taking the drag or
    /// nothing went in.
    pub fn drop_text(
        &mut self,
        lay: &Layout,
        line: &crate::textedit::DropLine,
        text: &TextRenderer,
        px: u32,
    ) -> bool {
        let Some(x) = self.drop_x.take() else {
            return false;
        };
        let at = self.edit_index(text, px, x - lay.text.x);
        self.edit.as_mut().is_some_and(|e| e.drop_text(at, line))
    }

    /// A press on part `bar` of the open list's scrollbar: an arrow scrolls a row, the track a page; the thumb is grabbed into `thumb` for [`Combo::drag_bar`]. True when the list scrolled.
    pub fn press_bar(&mut self, bar: BarHit, thumb: &mut ThumbDrag) -> bool {
        match thumb.press(Axis::Vertical, bar).by(1, self.rows as i64) {
            0 => false,
            n => self.scroll(n as i32),
        }
    }

    pub fn drag_bar(&mut self, lay: &Layout, thumb: &ThumbDrag, y: i32) -> bool {
        let Some(grab) = thumb.grab(Axis::Vertical) else {
            return false;
        };
        let top = top_at(self, lay, y, grab);
        let moved = top != self.top;
        self.top = top;
        moved
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComboLit {
    pub frame: Interaction,
    pub drop: Interaction,
}

impl ComboLit {
    pub const IDLE: ComboLit = ComboLit {
        frame: Interaction::Idle,
        drop: Interaction::Idle,
    };
}

/// The closed part: field, picked item and drop button. `focus` draws the picked item on the highlight (its place icon blended half); `cue` adds focus dots on the highlight's edge, hidden until the keyboard is used. Under a style with a [`Part::ComboList`] the list is one button: the picked item stays plain on it, `cue` rings it 3 px inside the button, and it draws held while the list is open. An editable combo draws its icon and edit instead. `lit` is what the pointer does to its frame and drop button. The open list is `draw_list`, drawn last by the caller.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    combo: &Combo,
    lay: &Layout,
    focus: bool,
    cue: bool,
    caret: Caret,
    lit: ComboLit,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let field = lay.field.scaled(s);
    let editable = combo.edit.is_some();
    let fill = if combo.enabled {
        cs.window
    } else {
        cs.button_face
    };
    c.fill_rect(field, fill);
    let list_button = p
        .look
        .style()
        .is_some_and(|st| st.part(Part::ComboList).is_some());
    // A list that is one button draws it held while its list is open,
    // wherever the pointer is.
    let ptr = if list_button && !editable && combo.open.is_some() {
        Interaction::Pressed
    } else {
        lit.frame
    };
    let look = ComboLook {
        enabled: combo.enabled,
        focused: focus && editable,
        ptr,
        editable,
    };
    draw_combo_frame(c, p, field, lay.drop.scaled(s), fill, look);
    if let (Some(icon), Some(r)) = (&combo.icon, lay.icon) {
        let r = r.scaled(s);
        if focus && combo.enabled && !editable && !list_button {
            let tinted = crate::controls::iconview::tint_selected(icon, cs.hilight);
            c.blit_over(r.x, r.y, &tinted);
        } else {
            c.blit_over(r.x, r.y, icon);
        }
    }
    let tr = lay.text.scaled(s);
    let value = fitted(text, combo.text(), tr.width, px);
    if !combo.enabled {
        // Under a style's edit border the ring of what sits behind the
        // combo runs 3 px left of the drop button, inside the text rect's
        // last column, so the greyed value stops short of it.
        let ringed = p
            .look
            .style()
            .is_some_and(|st| st.part(crate::style::Part::EditBorder).is_some());
        let clip = if ringed {
            Rect::new(
                field.x,
                field.y,
                lay.drop.x * s - 3 * s - field.x,
                field.height,
            )
        } else {
            field
        };
        c.with_clip(clip, |c| {
            draw_disabled_label(
                c,
                text,
                p,
                tr,
                &value,
                Align::Left,
                px,
                Some(crate::style::Part::EditBorder),
            )
        });
    } else if let Some(edit) = &combo.edit {
        // A drag resting on the field scrolls it (`drag_tick`), so the caret no longer pins the
        // view.
        let from = crate::controls::form::visible_from(edit, &combo.first);
        let first = match combo.drop_x {
            Some(_) => from,
            None => edit_first_visible(text, edit, tr.width, px, from),
        };
        combo.first.set(first);
        let caret = match combo.drop_x {
            Some(_) => Caret::new(caret.focused(), false),
            None => caret,
        };
        draw_edit(c, text, edit, tr, px, p, caret, first);
        if let Some(x) = combo.drop_x {
            let tail = &edit.text[first..];
            let at = first + crate::textedit::index_at(text, tail, x * s - tr.x, px);
            draw_edit_drop_point(c, text, edit, tr, px, p, first, at);
        }
    } else if list_button {
        text.draw(c, tr, &value, false, cs.button_text, Align::Left, px);
        if focus && cue && combo.open.is_none() {
            // It starts where the highlight would (3 px in, or past an
            // icon a host draws over the field and moves the highlight
            // for), or a pixel right of the combo's own place icon.
            let left = lay
                .icon
                .map_or(lay.highlight.scaled(s).x, |r| r.scaled(s).right() + s);
            let drop = lay.drop.scaled(s);
            let ring = Rect::new(
                left,
                field.y + 3 * s,
                drop.x - 2 * s - left,
                field.height - 6 * s,
            );
            // Dots where x + y is odd counting from the list's top-left.
            let origin = (-field.x.div_euclid(s), -field.y.div_euclid(s));
            let white = crate::theme::Color::rgb(0xFF, 0xFF, 0xFF);
            draw_focus_ring_screen(c, ring, s, RingInk::Xor(white), origin);
        }
    } else if focus {
        // With a place icon the highlight covers the label alone: the
        // text's advance from its left edge, 13 px tall on the text's
        // row.
        let hl = match lay.icon {
            Some(_) => {
                let w = text.measure(&value, false, px);
                let h = p.text.len(LINE_H) * s;
                Rect::new(tr.x, tr.y + (tr.height - h) / 2, w.min(tr.width), h)
            }
            None => lay.highlight.scaled(s),
        };
        c.fill_rect(hl, cs.hilight);
        text.draw(c, tr, &value, false, cs.hilight_text, Align::Left, px);
        if cue {
            // The field's ring alternates the inverted highlight with
            // ink; the second pass, its origin off by one, takes the
            // other parity.
            draw_focus_ring_screen(c, hl, s, cs.hilight.inverted(), (0, 0));
            draw_focus_ring_screen(c, hl, s, cs.button_text, (1, 0));
        }
    } else {
        text.draw(c, tr, &value, false, cs.button_text, Align::Left, px);
    }
    draw_combo_drop(c, p, lay.drop.scaled(s), combo.enabled, lit.drop, editable);
}

/// `s` whole while its ink fits `width`, else cut to an ellipsis.
fn fitted<'t>(text: &TextRenderer, s: &'t str, width: i32, px: u32) -> std::borrow::Cow<'t, str> {
    if text.ink_width(s, false, px) <= width {
        s.into()
    } else {
        text.ellipsize(s, width, false, px)
    }
}

/// Where a click at logical `x` puts an editable combo's caret (a byte index); `px` and `s` are what `draw` got.
pub fn index_at(
    combo: &Combo,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    s: i32,
    x: i32,
) -> usize {
    let Some(edit) = &combo.edit else {
        return 0;
    };
    let tr = lay.text.scaled(s);
    let from = crate::controls::form::visible_from(edit, &combo.first);
    let first = edit_first_visible(text, edit, tr.width, px, from);
    first + crate::textedit::index_at(text, &edit.text[first..], x * s - tr.x, px)
}

/// The open list: the look's frame and shadow ([`crate::look::widgets::draw_drop_list`]), white rows from `top`, and `scrollbar` when given. A plain list highlights the whole row inside the frame, text 3 px in on a 13 px line; a shell list (any item with an icon) draws 16 px rows with the icon at its indent, the label 4 px after it, and the highlight behind the label only.
#[allow(clippy::too_many_arguments)]
pub fn draw_list(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    items: &[ListItem],
    highlighted: Option<usize>,
    top: usize,
    rows: usize,
    list: Rect,
    scrollbar: Option<Rect>,
    bar: Lit<ScrollHit>,
    px: u32,
) {
    let row_h = items_row_h(items, &p.look.controls().grown(p.text));
    draw_list_rows(
        c,
        text,
        p,
        items,
        highlighted,
        top,
        rows,
        list,
        scrollbar,
        bar,
        px,
        row_h,
    );
}

/// [`draw_list`] with rows `row_h` tall, for a list laid out by [`layout_rows`].
#[allow(clippy::too_many_arguments)]
pub fn draw_list_rows(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    items: &[ListItem],
    highlighted: Option<usize>,
    top: usize,
    rows: usize,
    list: Rect,
    scrollbar: Option<Rect>,
    bar: Lit<ScrollHit>,
    px: u32,
    row_h: i32,
) {
    let Paint { cs, s, .. } = *p;
    let l = list.scaled(s);
    draw_drop_list(c, p, l);
    let mut inner = l.inset(s);
    if let Some(sb) = scrollbar {
        let sb = sb.scaled(s);
        inner.width -= sb.width;
        draw_scrollbar(c, p, sb, top, rows, items.len(), bar);
    }
    let shell = shell(items);
    let slot = icon_slot(items, s, row_h);
    let rh = row_h * s;
    for (row, i) in (top..items.len().min(top + rows)).enumerate() {
        let r = Rect::new(inner.x, inner.y + row as i32 * rh, inner.width, rh);
        let (bg, fg) = if highlighted == Some(i) {
            (cs.hilight, cs.hilight_text)
        } else if items[i].greyed {
            (cs.window, cs.gray_text)
        } else {
            (cs.window, cs.button_text)
        };
        if !shell {
            c.fill_rect(r, bg);
            // The text is set on a 13 px line (grown with the text) centred
            // in the row: the whole of a 13 px row, a row down a 15 px one.
            let line = p.text.len(LINE_H) * s;
            let ty = r.y + (r.height - line) / 2;
            let tr = Rect::new(r.x + 2 * s, ty, r.width - 2 * s, line);
            let label = fitted(text, &items[i].text, tr.width, px);
            text.draw(c, tr, &label, false, fg, Align::Left, px);
            continue;
        }
        let ix = r.x + (LIST_ICON_X + items[i].indent * INDENT_STEP) * s;
        if let Some(icon) = &items[i].icon {
            let (mut dx, mut dy) = (0, (row_h - SHELL_ROW_H) / 2 * s);
            if slot > 16 {
                dx = (slot * s - icon.width as i32).max(0) / 2;
                dy = (rh - icon.height as i32).max(0) / 2;
            }
            c.blit_over(ix + dx, r.y + dy, icon);
        }
        let tx = ix + (slot + ICON_GAP) * s;
        let label = fitted(text, &items[i].text, inner.right() - s - tx, px);
        let tw = text.measure(&label, false, px);
        if highlighted == Some(i) {
            let hl = Rect::new(tx - s, r.y + s, tw + 2 * s, r.height - s).intersect(&inner);
            c.fill_rect(hl, bg);
            // On a shell row the focus rectangle alternates the inverted highlight with the highlight itself, not with ink.
            draw_focus_ring_screen(c, hl, s, cs.hilight.inverted(), (0, 0));
        }
        let tr = Rect::new(tx, r.y + s, (inner.right() - tx).max(0), r.height);
        text.draw(c, tr, &label, false, fg, Align::Left, px);
    }
}

/// The combo's `access` part: a combo box over its rows (the picked one selected, the highlighted one while open), plus a text input for an editable combo. While open, the list's bounds are its rows in view and a row scrolled out lies outside them ([`Combo::reveal`]). Returns the focused node: the highlighted row while open, else the field of an editable combo, else the combo.
#[allow(clippy::too_many_arguments)]
pub fn access(
    combo: &Combo,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let role = if combo.edit.is_some() {
        Role::EditableComboBox
    } else {
        Role::ComboBox
    };
    let mut n = access::node(role, lay.field);
    n.set_expanded(combo.open.is_some());
    n.set_has_popup(accesskit::HasPopup::Listbox);
    access::focusable(&mut n, combo.enabled);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    if let Some(edit) = &combo.edit {
        let first = crate::controls::form::visible_from(edit, &combo.first);
        let field = Rect::new(
            lay.text.x - FIELD_PAD,
            lay.field.y,
            lay.text.width + 2 * FIELD_PAD,
            lay.field.height,
        );
        focus = access_edit(
            edit,
            first,
            false,
            field,
            combo.enabled,
            text,
            px,
            tree,
            id,
            "edit",
        );
    }
    let rh = lay.row_h;
    let rows = Rect::new(
        lay.list.x + 1,
        lay.list.y + 1,
        lay.list.width - 2,
        combo.visible() as i32 * rh,
    );
    let list = tree.push(id, "list", access::node(Role::ListBox, rows));
    let picked = combo.open.unwrap_or(combo.selected);
    for (i, item) in combo.items.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(item.text.as_str());
        if combo.open.is_some() {
            let y = rows.y + (i as i32 - combo.top as i32) * rh;
            let row = Rect::new(rows.x, y, rows.width, rh);
            o.set_bounds(access::row_bounds(rows, row));
        }
        o.set_selected(i == picked);
        if item.greyed {
            o.set_disabled();
        }
        let oid = tree.push(list, i, o);
        if combo.open == Some(i) {
            focus = oid;
        }
    }
    focus
}

#[cfg(test)]
mod tests;
