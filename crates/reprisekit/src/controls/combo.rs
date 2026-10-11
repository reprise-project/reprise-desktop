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

fn items_row_h(items: &[ListItem], cm: &ControlMetrics) -> i32 {
    if shell(items) {
        SHELL_ROW_H
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
        }
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
        match self.open {
            Some(_) => {
                self.open = Some(next);
                self.keep_in_view(next);
                false
            }
            None => {
                let changed = next != self.selected;
                self.selected = next;
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
    let n = combo.visible() as i32;
    Rect::new(
        field.x,
        field.bottom(),
        field.width,
        n * combo.row_h(cm) + 2,
    )
}

pub fn layout(combo: &Combo, field: Rect, cm: &ControlMetrics) -> Layout {
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
    let list = list_rect(combo, field, cm);
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
        row_h: combo.row_h(cm),
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
    pub fn drop_text(&mut self, lay: &Layout, line: &str, text: &TextRenderer, px: u32) -> bool {
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
                combo.text(),
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
        text.draw(c, tr, combo.value(), false, cs.button_text, Align::Left, px);
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
                let w = text.measure(combo.value(), false, px);
                let h = LINE_H * s;
                Rect::new(tr.x, tr.y + (tr.height - h) / 2, w.min(tr.width), h)
            }
            None => lay.highlight.scaled(s),
        };
        c.fill_rect(hl, cs.hilight);
        text.draw(
            c,
            tr,
            combo.value(),
            false,
            cs.hilight_text,
            Align::Left,
            px,
        );
        if cue {
            // The field's ring alternates the inverted highlight with
            // ink; the second pass, its origin off by one, takes the
            // other parity.
            draw_focus_ring_screen(c, hl, s, cs.hilight.inverted(), (0, 0));
            draw_focus_ring_screen(c, hl, s, cs.button_text, (1, 0));
        }
    } else {
        text.draw(c, tr, combo.value(), false, cs.button_text, Align::Left, px);
    }
    draw_combo_drop(c, p, lay.drop.scaled(s), combo.enabled, lit.drop, editable);
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
    let rh = items_row_h(items, &p.look.controls()) * s;
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
            // The text is set on a 13 px line centred in the row: the
            // whole of a 13 px row, a row down a 15 px one.
            let line = LINE_H * s;
            let ty = r.y + (r.height - line) / 2;
            let tr = Rect::new(r.x + 2 * s, ty, r.width - 2 * s, line);
            text.draw(c, tr, &items[i].text, false, fg, Align::Left, px);
            continue;
        }
        let ix = r.x + (LIST_ICON_X + items[i].indent * INDENT_STEP) * s;
        if let Some(icon) = &items[i].icon {
            c.blit_over(ix, r.y, icon);
        }
        let tx = ix + (16 + ICON_GAP) * s;
        let tw = text.measure(&items[i].text, false, px);
        if highlighted == Some(i) {
            let hl = Rect::new(tx - s, r.y + s, tw + 2 * s, r.height - s).intersect(&inner);
            c.fill_rect(hl, bg);
            // On a shell row the focus rectangle alternates the inverted highlight with the highlight itself, not with ink.
            draw_focus_ring_screen(c, hl, s, cs.hilight.inverted(), (0, 0));
        }
        let tr = Rect::new(tx, r.y + s, (inner.right() - tx).max(0), r.height);
        text.draw(c, tr, &items[i].text, false, fg, Align::Left, px);
    }
}

/// The combo's `access` part: a combo box over its rows (the picked one selected, the highlighted one while open), plus a text input for an editable combo. Returns the focused node: the highlighted row while open, else the field of an editable combo, else the combo.
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
    let list = tree.push(id, "list", access::node(Role::ListBox, lay.list));
    let (rh, shown) = (lay.row_h, combo.top..combo.top + combo.visible());
    let picked = combo.open.unwrap_or(combo.selected);
    for (i, item) in combo.items.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(item.text.as_str());
        if combo.open.is_some() && shown.contains(&i) {
            let y = lay.list.y + 1 + (i - combo.top) as i32 * rh;
            o.set_bounds(access::bounds(Rect::new(
                lay.list.x + 1,
                y,
                lay.list.width - 2,
                rh,
            )));
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
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{Color, FontConfig, Theme};

    fn combo() -> Combo {
        Combo::new(&["Reprise Classic", "Slate", "Harbor"], 0, 8)
    }

    #[test]
    fn layout_puts_the_button_inside_the_edge_and_the_list_under_the_field() {
        let c = combo();
        let l = layout(
            &c,
            Rect::new(24, 119, 192, COMBO_H),
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(l.drop, Rect::new(198, 121, 16, 17));
        assert_eq!(l.highlight, Rect::new(27, 122, 171, 15));
        assert_eq!(l.text.x, 28);
        assert_eq!(l.list, Rect::new(24, 140, 192, 3 * 13 + 2));
        assert!(l.scrollbar.is_none());
        let mut many = Combo::new(&["a"; 30], 0, 8);
        let l = layout(
            &many,
            Rect::new(0, 0, 100, COMBO_H),
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(l.list.height, 8 * 13 + 2);
        assert_eq!(l.scrollbar, Some(Rect::new(83, 22, 16, 104)));
        // Hits: the list only counts while open.
        assert_eq!(hit(&many, &l, 90, 30), None);
        assert_eq!(hit(&many, &l, 90, 10), Some(Hit::Drop));
        assert_eq!(hit(&many, &l, 10, 10), Some(Hit::Field));
        assert!(many.toggle());
        assert_eq!(hit(&many, &l, 10, 22 + 13 + 3), Some(Hit::Row(1)));
        assert_eq!(hit(&many, &l, 90, 25), Some(Hit::ScrollUp));
        assert_eq!(hit(&many, &l, 90, 122), Some(Hit::ScrollDown));
        // 30 items, 8 in view: a 19 px thumb at the top of a 72 px
        // track, and the rest of the track pages.
        assert_eq!(hit(&many, &l, 90, 45), Some(Hit::Thumb(7)));
        assert_eq!(hit(&many, &l, 90, 70), Some(Hit::PageDown));
        // Dragging the thumb by that grab point walks the whole range.
        assert_eq!(top_at(&many, &l, 45, 7), 0);
        assert_eq!(top_at(&many, &l, 98, 7), 22);
        many.top = 22;
        assert_eq!(hit(&many, &l, 90, 45), Some(Hit::PageUp));
    }

    /// Blue's combo at the Themes tab's 192x21: a 15 px drop button with
    /// a white column on each side, and the picked item's highlight
    /// ending 3 px short of the button.
    #[test]
    fn a_blue_combo_drop_is_15_px_and_3_px_clear_of_the_highlight() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = theme.look.controls();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let c = combo();
        let field = Rect::new(3, 3, 192, COMBO_H);
        let l = layout(&c, field, &cm);
        assert_eq!(l.drop, Rect::new(178, 5, 15, 17));
        assert_eq!(l.highlight, Rect::new(6, 6, 169, 15));
        assert_eq!(hit(&c, &l, 178, 12), Some(Hit::Drop));
        assert_eq!(hit(&c, &l, 177, 12), Some(Hit::Field));
        let mut canvas = Canvas::new(198, 27);
        canvas.fill_rect(canvas.bounds(), cs.button_face);
        let p = theme.paint();
        draw(
            &mut canvas,
            &text,
            &p,
            &c,
            &l,
            true,
            true,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
        let line = crate::blue::Variant::Blue
            .scheme()
            .edit_border
            .normal
            .rim
            .argb();
        let white = cs.window.argb();
        let dots = [cs.hilight.inverted().argb(), cs.button_text.argb()];
        for y in 6..21 {
            assert!(
                dots.contains(&canvas.get(174, y)),
                "(174, {y}) is a focus dot on the highlight's last column"
            );
            if (7..20).contains(&y) {
                assert_eq!(canvas.get(173, y), cs.hilight.argb(), "(173, {y})");
            }
            for x in 175..178 {
                assert_eq!(canvas.get(x, y), white, "({x}, {y}) is white");
            }
            assert_eq!(canvas.get(193, y), white, "(193, {y}) is white");
            assert_eq!(canvas.get(194, y), line, "(194, {y}) is the field line");
        }
        let mut drop = Canvas::new(198, 27);
        // The button's round corners let the white well through.
        drop.fill_rect(drop.bounds(), cs.window);
        crate::look::widgets::draw_combo_drop(
            &mut drop,
            &p,
            l.drop,
            true,
            Interaction::Idle,
            false,
        );
        for y in 5..22 {
            for x in 178..193 {
                assert_eq!(canvas.get(x, y), drop.get(x, y), "drop art at ({x}, {y})");
            }
        }
    }

    /// Blue's greyed combo at the Effects dialog's 120x21: a #C9C7BA line,
    /// a #F5F4EA ring inside it around the text part, face inside that,
    /// and the greyed drop button in a white well.
    #[test]
    fn a_greyed_blue_combo_has_a_c9c7ba_line_and_its_button_a_white_well() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut c = combo();
        c.enabled = false;
        let l = layout(&c, Rect::new(3, 3, 120, COMBO_H), &theme.look.controls());
        assert_eq!(l.drop, Rect::new(106, 5, 15, 17));
        let mut canvas = Canvas::new(126, 27);
        canvas.fill_rect(canvas.bounds(), cs.button_face);
        let p = theme.paint();
        draw(
            &mut canvas,
            &text,
            &p,
            &c,
            &l,
            false,
            false,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
        let greyed = crate::blue::Variant::Blue.scheme().edit_border.disabled;
        let (line, ring) = (greyed.rim.argb(), greyed.inner.argb());
        let (face, white) = (cs.button_face.argb(), cs.window.argb());
        for x in 3..123 {
            assert_eq!(canvas.get(x, 3), line, "top line at x {x}");
            assert_eq!(canvas.get(x, 23), line, "bottom line at x {x}");
            let inside = if x == 3 || x == 122 {
                line
            } else if x < 105 {
                ring
            } else {
                white
            };
            assert_eq!(canvas.get(x, 4), inside, "row 4 at x {x}");
            assert_eq!(canvas.get(x, 22), inside, "row 22 at x {x}");
        }
        for y in 4..23 {
            assert_eq!(canvas.get(3, y), line, "left line at y {y}");
            assert_eq!(canvas.get(122, y), line, "right line at y {y}");
            assert_eq!(canvas.get(4, y), ring, "left ring at y {y}");
            assert_eq!(canvas.get(104, y), ring, "the ring's right side at y {y}");
            assert_eq!(canvas.get(105, y), white, "the well's left side at y {y}");
            assert_eq!(canvas.get(121, y), white, "the well's right side at y {y}");
        }
        assert_eq!(canvas.get(5, 5), face, "face inside the ring");
        assert_eq!(canvas.get(103, 21), face, "face inside the ring");
        let mut drop = Canvas::new(126, 27);
        crate::look::widgets::draw_combo_drop(
            &mut drop,
            &p,
            l.drop,
            false,
            Interaction::Idle,
            false,
        );
        for y in 5..22 {
            for x in 106..121 {
                // The button's round corners let what lies under it through.
                if (x - 106).min(120 - x) < 2 && (y - 5).min(21 - y) < 2 {
                    continue;
                }
                assert_eq!(
                    canvas.get(x, y),
                    drop.get(x, y),
                    "greyed drop at ({x}, {y})"
                );
            }
        }
    }

    /// A greyed 75 px Position combo on a 386x495
    /// page: the C9C7BA line, the F5F4EA ring, then one ring of the page
    /// itself around a face interior, on the left and right of the text
    /// part and above and below it. On the dialog that page ring is face.
    #[test]
    fn a_greyed_blue_combo_on_a_page_rings_its_face_with_the_page() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let face = theme.colors.button_face;
        let p = theme.paint();
        let page = Rect::new(9, 49, 386, 495);
        let mut c = Combo::new(&["Center", "Tile", "Stretch"], 2, 8);
        c.enabled = false;
        let l = layout(&c, Rect::new(306, 308, 75, COMBO_H), &theme.look.controls());
        let mut bare = Canvas::new(400, 560);
        bare.fill_rect(bare.bounds(), face);
        crate::look::widgets::draw_page(&mut bare, &p, page);
        let mut on_page = bare.clone();
        let mut on_dialog = Canvas::new(400, 560);
        on_dialog.fill_rect(on_dialog.bounds(), face);
        for (canvas, paint) in [(&mut on_page, p.on_page(page)), (&mut on_dialog, p)] {
            draw(
                canvas,
                &text,
                &paint,
                &c,
                &l,
                false,
                false,
                Caret::Unfocused,
                ComboLit::IDLE,
                11,
            );
        }
        let greyed = crate::blue::Variant::Blue.scheme().edit_border.disabled;
        let (line, ring, white, facev) = (
            greyed.rim.argb(),
            greyed.inner.argb(),
            0xFFFF_FFFF,
            face.argb(),
        );
        let well = l.drop.x - 1;
        // Across the text part, row by row down its left edge: line,
        // ring, page, face.
        assert_eq!(l.field.bottom(), 329);
        for y in 311..326 {
            let row: Vec<u32> = (306..310).map(|x| on_page.get(x, y)).collect();
            assert_eq!(
                row,
                [line, ring, bare.get(308, y), facev],
                "left edge at y {y}"
            );
            let row: Vec<u32> = (well - 3..=well).map(|x| on_page.get(x, y)).collect();
            assert_eq!(
                row,
                [facev, bare.get(well - 2, y), ring, white],
                "right of the text at y {y}"
            );
            assert_eq!(
                on_dialog.get(308, y),
                facev,
                "on the dialog the ring is face at y {y}"
            );
        }
        for x in 308..well - 1 {
            let col: Vec<u32> = (308..311).map(|y| on_page.get(x, y)).collect();
            assert_eq!(col, [line, ring, bare.get(x, 310)], "top edge at x {x}");
            let col: Vec<u32> = (326..329).map(|y| on_page.get(x, y)).collect();
            assert_eq!(col, [bare.get(x, 326), ring, line], "bottom edge at x {x}");
        }
        // A value longer than the field leaves the ring's right column to
        // the page: "Reprise Classic" in combos 50 to 80 px wide, so some
        // glyph's ink reaches the text rect's last column.
        let mut long = Combo::new(&["Reprise Classic"], 0, 8);
        long.enabled = false;
        for w in 50..80 {
            let l = layout(
                &long,
                Rect::new(306, 308, w, COMBO_H),
                &theme.look.controls(),
            );
            let mut c = bare.clone();
            draw(
                &mut c,
                &text,
                &p.on_page(page),
                &long,
                &l,
                false,
                false,
                Caret::Unfocused,
                ComboLit::IDLE,
                11,
            );
            let ring_x = l.drop.x - 3;
            for y in 311..326 {
                assert_eq!(
                    c.get(ring_x, y),
                    bare.get(ring_x, y),
                    "{w} px: the ring at y {y} is the page"
                );
            }
        }
        assert_eq!(
            on_page.get(308, 320),
            0xFFF6_F4EE,
            "the page's own pixel at row 320"
        );
    }

    #[test]
    fn a_classic_combo_frame_is_the_field_frame_in_both_states() {
        let theme = Theme::default();
        let p = classic_paint(&theme.colors, 1);
        let r = Rect::new(2, 2, 60, 21);
        for (enabled, fill) in [(true, p.cs.window), (false, p.cs.button_face)] {
            let (mut a, mut b) = (Canvas::new(64, 25), Canvas::new(64, 25));
            let look = ComboLook {
                enabled,
                focused: false,
                ptr: Interaction::Idle,
                editable: false,
            };
            draw_combo_frame(&mut a, &p, r, Rect::new(44, 4, 16, 17), fill, look);
            crate::classic::widgets::draw_field_frame(&mut b, &p, r, fill);
            assert!(
                a == b,
                "Classic's combo frame (enabled {enabled}) is its sunken edge"
            );
        }
    }

    #[test]
    fn an_editable_combo_starts_its_text_6_px_into_the_field() {
        let mut c = Combo::new(&["notepad"], 0, 3);
        c.edit = Some(TextEdit::new("notepad", false));
        let field = Rect::new(24, 119, 192, COMBO_H);
        let l = layout(&c, field, &ControlMetrics::CLASSIC);
        assert_eq!(l.text, Rect::new(30, 122, 166, 15));
    }

    /// An editable combo over a history: Down from typed text starts at
    /// the newest item, a step or a pick puts the item's text in the
    /// field selected, the list opens on the item the field holds, and
    /// the frame's bottom pixel under the last row in view is no row.
    #[test]
    fn an_editable_combo_steps_its_field_through_the_items() {
        let mut c = Combo::new(&["notepad", "mspaint", "calc", "cmd"], 0, 3);
        c.edit = Some(TextEdit::new("typed", false));
        let text = |c: &Combo| c.edit.as_ref().unwrap().text.clone();
        assert!(!c.step(false), "Up from typed text does nothing");
        assert!(c.step(true));
        assert_eq!(text(&c), "notepad");
        assert!(c.edit.as_ref().unwrap().all_selected());
        assert!(c.step(true));
        assert_eq!(text(&c), "mspaint");
        assert!(c.toggle());
        assert_eq!((c.open, c.top), (Some(1), 0));
        c.step(true);
        c.step(true);
        assert_eq!((c.open, c.top, text(&c)), (Some(3), 1, "cmd".to_string()));
        let lay = layout(&c, Rect::new(0, 0, 100, COMBO_H), &ControlMetrics::CLASSIC);
        assert_eq!(hit(&c, &lay, 5, lay.list.bottom() - 2), Some(Hit::Row(3)));
        assert_eq!(hit(&c, &lay, 5, lay.list.bottom() - 1), None);
        assert!(c.pick(0));
        assert_eq!((c.open, text(&c)), (None, "notepad".to_string()));
    }

    #[test]
    fn keys_pick_directly_when_closed_and_move_the_highlight_when_open() {
        let mut c = combo();
        assert!(c.step(true));
        assert_eq!(c.selected, 1);
        assert!(!c.step(false) || c.selected == 0);
        assert!(c.jump(true));
        assert_eq!(c.selected, 2);
        assert!(c.toggle());
        assert_eq!(c.open, Some(2));
        assert!(!c.step(false));
        assert_eq!((c.open, c.selected), (Some(1), 2));
        assert!(c.take());
        assert_eq!((c.open, c.selected), (None, 1));
        c.enabled = false;
        assert!(!c.toggle());
        assert!(!c.step(true));
    }

    #[test]
    fn a_greyed_row_is_skipped_refused_and_drawn_grey() {
        let mut c = combo();
        c.items[1].greyed = true;
        assert!(c.step(true));
        assert_eq!(c.selected, 2, "Down steps over the greyed row");
        assert!(c.step(false));
        assert_eq!(c.selected, 0, "and Up back over it");
        c.items[0].greyed = true;
        c.selected = 2;
        assert!(!c.jump(false), "Home stops at the first row not greyed");
        assert_eq!(c.selected, 2);
        assert!(c.toggle());
        assert!(!c.pick(1));
        assert_eq!((c.open, c.selected), (Some(2), 2), "the list stays open");

        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let rows = c.items.len();
        let list = Rect::new(
            0,
            0,
            120,
            rows as i32 * ControlMetrics::CLASSIC.combo_row + 2,
        );
        let mut canvas = Canvas::new(120, list.height as u32);
        draw_list(
            &mut canvas,
            &text,
            &classic_paint(cs, 1),
            &c.items,
            None,
            0,
            rows,
            list,
            None,
            None,
            11,
        );
        let inks = |row: i32| {
            let r = Rect::new(
                1,
                1 + row * ControlMetrics::CLASSIC.combo_row,
                118,
                ControlMetrics::CLASSIC.combo_row,
            );
            (r.x..r.right())
                .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
                .map(|(x, y)| canvas.get(x, y))
                .filter(|&p| p != cs.window.argb())
                .collect::<std::collections::HashSet<_>>()
        };
        assert!(inks(1).contains(&cs.gray_text.argb()));
        assert!(!inks(1).contains(&cs.button_text.argb()));
        assert!(inks(2).contains(&cs.button_text.argb()));
    }

    #[test]
    fn draws_the_field_highlight_and_button_pixels() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let c = combo();
        let field = Rect::new(24, 119, 192, COMBO_H);
        let l = layout(&c, field, &ControlMetrics::CLASSIC);
        let mut canvas = Canvas::new(240, 160);
        canvas.fill_rect(canvas.bounds(), cs.button_face);
        draw(
            &mut canvas,
            &text,
            &classic_paint(cs, 1),
            &c,
            &l,
            true,
            true,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
        // The sunken edge, the highlight with its dots, the raised button.
        assert_eq!(canvas.get(24, 119), cs.button_shadow.argb());
        assert_eq!(canvas.get(25, 120), cs.button_dk_shadow.argb());
        assert_eq!(canvas.get(26, 121), cs.window.argb());
        assert_eq!(canvas.get(27, 122), Color::rgb(245, 219, 149).argb());
        assert_eq!(canvas.get(28, 122), cs.button_text.argb());
        assert_eq!(canvas.get(100, 130), cs.hilight.argb());
        assert_eq!(canvas.get(198, 121), cs.button_face.argb());
        assert_eq!(canvas.get(199, 122), cs.button_hilight.argb());
        assert_eq!(canvas.get(213, 121), cs.button_dk_shadow.argb());
        assert_eq!(canvas.get(212, 136), cs.button_shadow.argb());
        assert_eq!(canvas.get(205, 127), cs.button_text.argb(), "arrow tip row");
        assert_eq!(canvas.get(215, 130), cs.button_hilight.argb());
        // Without the focus the item is plain; disabled it is face and etched.
        let mut plain = Canvas::new(240, 160);
        draw(
            &mut plain,
            &text,
            &classic_paint(cs, 1),
            &c,
            &l,
            false,
            false,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
        assert_eq!(plain.get(100, 130), cs.window.argb());
        let mut off = c.clone();
        off.enabled = false;
        let mut dis = Canvas::new(240, 160);
        draw(
            &mut dis,
            &text,
            &classic_paint(cs, 1),
            &off,
            &l,
            false,
            false,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
        assert_eq!(dis.get(180, 130), cs.button_face.argb());
        assert_eq!(dis.get(205, 127), cs.button_shadow.argb());
        assert_eq!(
            dis.get(209, 128),
            cs.button_hilight.argb(),
            "greyed arrow copy"
        );
        // Focused by a click: the highlight stays, the dots do not. They
        // wait until the keyboard is used, so the highlight's top edge
        // runs solid.
        let mut nocue = Canvas::new(240, 160);
        nocue.fill_rect(nocue.bounds(), cs.button_face);
        draw(
            &mut nocue,
            &text,
            &classic_paint(cs, 1),
            &c,
            &l,
            true,
            false,
            Caret::Unfocused,
            ComboLit::IDLE,
            11,
        );
        assert_eq!(nocue.get(100, 130), cs.hilight.argb(), "still highlighted");
        let edge = l.highlight;
        for x in edge.x..edge.right() {
            assert_eq!(
                nocue.get(x, edge.y),
                cs.hilight.argb(),
                "no dot at {x} on the highlight's top edge"
            );
        }
        // With the cue the same edge is dotted.
        assert!((edge.x..edge.right()).any(|x| canvas.get(x, edge.y) == cs.button_text.argb()));
    }

    #[test]
    fn a_click_at_scale_2_lands_on_the_character_drawn_there() {
        let text = TextRenderer::load(&FontConfig::default());
        let (s, px) = (2, 22);
        let mut c = combo();
        c.edit = Some(TextEdit::new(
            "C:\\Documents and Settings\\All Users",
            false,
        ));
        let lay = layout(&c, Rect::new(0, 0, 120, COMBO_H), &ControlMetrics::CLASSIC);
        let edit = c.edit.as_ref().unwrap();
        // The caret at the end scrolls the start out of view.
        let tr = lay.text.scaled(s);
        let first = edit_first_visible(&text, edit, tr.width, px, 0);
        assert!(first > 0);
        let k = first + 3;
        let dx = tr.x + text.measure(&edit.text[first..k], false, px);
        assert_eq!(index_at(&c, &lay, &text, px, s, dx / s), k);
    }

    #[test]
    fn a_shell_row_highlight_ends_one_px_past_its_label_at_scale_2() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let (s, px) = (2, 22);
        let items = [ListItem::shell("Harbor", Some(Canvas::new(32, 32)), 0)];
        let mut c = Canvas::new(400, 40);
        let list = Rect::new(0, 0, 200, SHELL_ROW_H + 2);
        draw_list(
            &mut c,
            &text,
            &classic_paint(cs, s),
            &items,
            Some(0),
            0,
            1,
            list,
            None,
            None,
            px,
        );
        // The label starts at the frame, the icon inset and the icon with
        // its gap: (1 + 3 + 16 + 4) px, scaled.
        let tx = (1 + LIST_ICON_X + 16 + ICON_GAP) * s;
        let right = tx + text.measure("Harbor", false, px) + s;
        let mid = SHELL_ROW_H * s / 2;
        assert_ne!(
            c.get(right - 1, mid),
            cs.window.argb(),
            "the ring on the highlight's last column"
        );
        assert_eq!(
            c.get(right, mid),
            cs.window.argb(),
            "white past the highlight"
        );
    }

    fn run_box(value: &str) -> (Combo, Layout) {
        let mut c = combo();
        c.edit = Some(TextEdit::new(value, false));
        let lay = layout(&c, Rect::new(0, 0, 120, COMBO_H), &ControlMetrics::CLASSIC);
        (c, lay)
    }

    #[test]
    fn text_dropped_on_an_editable_field_lands_at_the_drop_caret_selected() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let (mut c, lay) = run_box("foot");
        assert!(
            !c.drag_over(&lay, lay.drop.x + 2, 10),
            "the drop button takes nothing"
        );
        assert!(c.drop_x.is_none());
        let x = lay.text.x + text.measure("fo", false, px);
        assert!(c.drag_over(&lay, x, 10));
        assert!(c.drop_text(&lay, "X", &text, px));
        let edit = c.edit.as_ref().unwrap();
        assert_eq!(edit.text, "foXot");
        assert_eq!(edit.selected_text(), "X");
        assert!(c.drop_x.is_none(), "the drop ends the drag");
    }

    #[test]
    fn a_plain_open_or_greyed_combo_takes_no_drop() {
        let lay = layout(
            &combo(),
            Rect::new(0, 0, 120, COMBO_H),
            &ControlMetrics::CLASSIC,
        );
        assert!(!combo().drag_over(&lay, 20, 10), "a pick list has no caret");
        let (mut c, lay) = run_box("x");
        c.open = Some(0);
        assert!(!c.drag_over(&lay, 20, 10));
        let (mut c, lay) = run_box("x");
        c.enabled = false;
        assert!(!c.drag_over(&lay, 20, 10));
    }

    #[test]
    fn a_drag_resting_at_the_fields_edge_scrolls_it_and_the_drop_caret_shows() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let (mut c, lay) = run_box("/usr/share/applications/org.example.Long.desktop");
        let shot = |c: &Combo| {
            let mut canvas = Canvas::new(120, COMBO_H as u32);
            let caret = Caret::new(false, true);
            draw(
                &mut canvas,
                &text,
                &classic_paint(cs, 1),
                c,
                &lay,
                false,
                false,
                caret,
                ComboLit::IDLE,
                px,
            );
            canvas
        };
        shot(&c);
        let end = c.first.get();
        assert!(end > 0, "the caret at the end scrolled the start away");
        assert!(c.drag_over(&lay, lay.text.x + 1, 10));
        assert_eq!(c.drag_scroll(&lay, &text, px), Some(end - 1));
        assert!(c.drag_tick(&lay, &text, px));
        let before = shot(&c);
        assert_eq!(c.first.get(), end - 1, "the draw keeps the scrolled view");
        let x = lay.text.x + 30;
        c.drag_over(&lay, x, 10);
        let after = shot(&c);
        c.drag_leave();
        let plain = shot(&c);
        assert_eq!(
            c.first.get(),
            end,
            "without a drag the caret pins the view again"
        );
        let changed: Vec<(i32, i32)> = (0..COMBO_H)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&(x, y)| before.get(x, y) != after.get(x, y))
            .collect();
        let mut cols: Vec<i32> = changed.iter().map(|&(x, _)| x).collect();
        cols.dedup();
        cols.sort();
        cols.dedup();
        assert_eq!(
            cols.len(),
            2,
            "the drop caret moved from one column to another: {cols:?}"
        );
        assert_eq!(
            cols[0], lay.text.x,
            "the field's left edge, where it rested"
        );
        assert!(
            (x - 4..=x + 4).contains(&cols[1]),
            "near the pointer: {cols:?}"
        );
        let bar = changed.iter().filter(|&&(cx, _)| cx == cols[1]);
        assert!(
            bar.clone()
                .all(|&(cx, cy)| after.get(cx, cy) == cs.button_text.argb())
        );
        assert!(bar.count() >= 8, "a line tall");
        assert_ne!(plain.pixels, after.pixels);
    }
}
