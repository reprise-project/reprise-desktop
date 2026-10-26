//! The up-down (spinner) edit: a 50x20 sunken field with the number
//! right-aligned in its white part and two 15x8 raised arrow buttons at the
//! right. Up / Down and the buttons step the value inside `min..=max`; typed
//! digits edit the number at the caret and the value follows, clamped.
//!
//! The field edits only while it has the focus. The form opens the edit when
//! the focus arrives ([`UpDown::enter`]): by keyboard the whole number is
//! selected so typing replaces it, and a click puts the caret under the pointer
//! ([`UpDown::place`]). [`UpDown::leave`] drops the edit.
//!
//! A `buddy` spinner (the Date and Time sheet's year) left-aligns the number and
//! uses the look's `buddy_spin` column of two buttons: in Classic 16 px, flush
//! against the field's right edge, outside it and one pixel shorter. A look with
//! `buddy_spin_inside` puts it inside the field's frame at the right.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::Arrow;
use crate::classic::widgets::{FIELD_PAD, LINE_H};
use crate::look::widgets::{draw_disabled_label, draw_spin_buttons, draw_spin_field};
use crate::metrics::ControlMetrics;
use crate::paint::Paint;
use crate::pointer::Lit;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::{EditKey, TextEdit, TextSpot};
use crate::theme::TextUnits;
use accesskit::{NodeId, Role};

pub const WIDTH: i32 = 50;
pub const HEIGHT: i32 = 20;
const BUTTON_W: i32 = 15;
const MIN_DIGITS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpDown {
    value: i32,
    pub min: i32,
    pub max: i32,
    pub enabled: bool,
    /// The digits being edited, open only while the field has the focus.
    edit: Option<TextEdit>,
    pub buddy: bool,
}

impl UpDown {
    pub fn new(value: i32, min: i32, max: i32) -> UpDown {
        debug_assert!(min <= max, "UpDown range {min}..={max} is empty");
        let mut ud = UpDown {
            value,
            min,
            max,
            enabled: true,
            edit: None,
            buddy: false,
        };
        ud.value = ud.bound(value);
        ud
    }

    pub fn with_buddy(mut self) -> UpDown {
        self.buddy = true;
        self
    }

    fn digits(&self) -> usize {
        MIN_DIGITS.max(self.max.unsigned_abs().to_string().len())
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    /// `v` inside `min..=max`; an empty range (min > max) pins it to `max` rather
    /// than panicking like `i32::clamp`.
    fn bound(&self, v: i32) -> i32 {
        v.max(self.min).min(self.max)
    }

    /// Put `v` in the field, clamped. An open edit shows a new value all selected;
    /// the same value leaves the edit alone, so a host refreshing the page does not
    /// disturb it. Returns whether the value changed.
    pub fn set(&mut self, v: i32) -> bool {
        let next = self.bound(v);
        let changed = next != self.value;
        self.value = next;
        if changed && self.edit.is_some() {
            self.enter();
        }
        changed
    }

    /// Up / Down or an arrow button: step the value and show it all selected, even
    /// at a bound.
    pub fn step(&mut self, delta: i32) -> bool {
        let changed = self.set(self.value.saturating_add(delta));
        if self.edit.is_some() {
            self.enter();
        }
        changed
    }

    pub fn enter(&mut self) {
        self.edit = Some(TextEdit::new(self.value.to_string(), true));
    }

    pub fn leave(&mut self) {
        self.edit = None;
    }

    pub fn place(&mut self, at: usize) {
        self.edit_mut().place(at, false);
    }

    pub fn editing(&self) -> bool {
        self.edit.is_some()
    }

    /// The edit, opened on the value first if the focus arrived without `enter`.
    fn edit_mut(&mut self) -> &mut TextEdit {
        let value = self.value;
        self.edit
            .get_or_insert_with(|| TextEdit::new(value.to_string(), true))
    }

    /// An editing key: digits go in at the caret (over the selection), caret keys and
    /// Backspace / Delete work the number, and the value follows, clamped. `None`
    /// for a key the field does not take, which the form reads as a dialog key.
    /// `Some(changed)` says whether the value changed.
    pub fn key(&mut self, key: &EditKey) -> Option<bool> {
        let digits = self.digits();
        let edit = self.edit_mut();
        match key {
            EditKey::Insert(s) => {
                if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let kept = edit.text.len() - edit.selection().len();
                if kept + s.len() > digits {
                    return Some(false);
                }
            }
            EditKey::Backspace { .. }
            | EditKey::Delete { .. }
            | EditKey::Left { .. }
            | EditKey::Right { .. }
            | EditKey::Home { .. }
            | EditKey::End { .. }
            | EditKey::SelectAll
            | EditKey::Cut
            | EditKey::Undo
            | EditKey::Redo => {}
            // The host copies the selection and pastes through `paste`.
            EditKey::Copy | EditKey::Paste => return Some(false),
            EditKey::DeleteSurrounding { .. } => return None,
        }
        edit.key(key.clone());
        let typed = edit.text.parse::<i32>().unwrap_or(self.min);
        let next = self.bound(typed);
        let changed = next != self.value;
        self.value = next;
        Some(changed)
    }

    /// Clipboard text: a number (blanks around it dropped) goes in at the caret as typed digits
    /// would; anything else is refused. Whether the value changed.
    pub fn paste(&mut self, text: &str) -> bool {
        let t = text.trim();
        !t.is_empty()
            && t.bytes().all(|b| b.is_ascii_digit())
            && self.key(&EditKey::Insert(t.to_string())) == Some(true)
    }

    /// The digits being edited, while the field has the focus.
    pub fn edit(&self) -> Option<&TextEdit> {
        self.edit.as_ref()
    }

    /// A right press at byte `at`, ahead of the edit menu: inside the selection it stays,
    /// anywhere else the caret moves there.
    pub fn press_for_menu(&mut self, at: usize) {
        self.edit_mut().press_for_menu(at);
    }

    pub fn digit(&mut self, d: char) -> bool {
        d.is_ascii_digit() && self.key(&EditKey::Insert(d.to_string())) == Some(true)
    }

    pub fn backspace(&mut self) -> bool {
        self.key(&EditKey::Backspace { word: false }) == Some(true)
    }

    pub fn shown(&self) -> String {
        match &self.edit {
            Some(e) => e.text.clone(),
            None => self.value.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub field: Rect,
    pub edit: Rect,
    pub up: Rect,
    pub down: Rect,
}

pub fn layout(x: i32, y: i32) -> Layout {
    layout_in(Rect::new(x, y, WIDTH, HEIGHT), HEIGHT)
}

/// A plain spinner at `r`'s top-left, `field_h` tall, its buttons splitting the column inside
/// the frame.
fn layout_in(r: Rect, field_h: i32) -> Layout {
    let field = Rect::new(r.x, r.y, r.width, field_h);
    let inner = field.inset(2);
    let bx = inner.right() - BUTTON_W;
    let (up, down) = split_buttons(Rect::new(bx, inner.y, BUTTON_W, inner.height));
    Layout {
        field,
        edit: Rect::new(inner.x, inner.y, bx - inner.x, inner.height),
        up,
        down,
    }
}

/// A buddy spinner filling `r`, its button column `cm.buddy_spin` wide.
/// Beside the field, the field is `r` less the column and the buttons
/// split the column's top `r.height - 1` px. Inside it, the field is all
/// of `r` and the column stands inside its 2 px frame at the right, split
/// by [`split_gapped`].
pub fn buddy_layout(r: Rect, cm: &ControlMetrics) -> Layout {
    if cm.buddy_spin_inside {
        let inner = r.inset(2);
        let col = Rect::new(
            inner.right() - cm.buddy_spin,
            inner.y,
            cm.buddy_spin,
            inner.height,
        );
        let (up, down) = split_gapped(col);
        return Layout {
            field: r,
            edit: Rect::new(inner.x, inner.y, col.x - inner.x, inner.height),
            up,
            down,
        };
    }
    let field = Rect::new(r.x, r.y, r.width - cm.buddy_spin, r.height);
    let col = Rect::new(field.right(), r.y, cm.buddy_spin, r.height - 1);
    let (up, down) = split_buttons(col);
    Layout {
        field,
        edit: field.inset(2),
        up,
        down,
    }
}

/// `ud` filling `r`, a plain one as tall as `u` grows its 20 px field.
pub fn layout_for(ud: &UpDown, r: Rect, cm: &ControlMetrics, u: TextUnits) -> Layout {
    if ud.buddy {
        buddy_layout(r, cm)
    } else {
        layout_in(r, u.len(HEIGHT))
    }
}

/// A button column cut into its up and down halves; an odd pixel goes to
/// the down button.
pub fn split_buttons(col: Rect) -> (Rect, Rect) {
    let up_h = col.height / 2;
    (
        Rect::new(col.x, col.y, col.width, up_h),
        Rect::new(col.x, col.y + up_h, col.width, col.height - up_h),
    )
}

/// A button column with one gap row held out between its two halves, so
/// a 17 px column is 8, a gap row, then 8, and a 16 px column (no pixel to
/// spare for the gap) is 7, a gap row, then 8: the same rule
/// [`split_buttons`] uses, an odd pixel goes to the down button.
pub fn split_gapped(col: Rect) -> (Rect, Rect) {
    let up_h = (col.height - 1) / 2;
    let down_h = col.height - 1 - up_h;
    (
        Rect::new(col.x, col.y, col.width, up_h),
        Rect::new(col.x, col.bottom() - down_h, col.width, down_h),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Edit(usize),
    Up,
    Down,
}

pub fn hit(ud: &UpDown, lay: &Layout, x: i32, y: i32, text: &TextRenderer, px: u32) -> Option<Hit> {
    if lay.up.contains(x, y) {
        Some(Hit::Up)
    } else if lay.down.contains(x, y) {
        Some(Hit::Down)
    } else if lay.field.contains(x, y) {
        let (tr, align) = text_rect(ud, lay.edit, 1);
        let shown = ud.shown();
        let x0 = text_x(text, &shown, tr, align, px);
        Some(Hit::Edit(crate::textedit::index_at(
            text,
            &shown,
            x - x0,
            px,
        )))
    } else {
        None
    }
}

fn text_rect(ud: &UpDown, edit: Rect, s: i32) -> (Rect, Align) {
    if ud.buddy {
        // A buddy field's number starts a pixel further in than an edit's.
        let pad = (FIELD_PAD + 1) * s;
        let r = Rect::new(edit.x + pad, edit.y, edit.width - pad, edit.height);
        (r, Align::Left)
    } else {
        let r = Rect::new(edit.x, edit.y, edit.width - 2 * s, edit.height);
        (r, Align::Right)
    }
}

fn text_x(text: &TextRenderer, shown: &str, tr: Rect, align: Align, px: u32) -> i32 {
    match align {
        Align::Left => tr.x,
        _ => tr.right() - text.measure(shown, false, px),
    }
}

/// Draw it; `caret` is whether the field has the focus.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    ud: &UpDown,
    lay: &Layout,
    caret: bool,
    spin: Lit<Arrow>,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let field = lay.field.scaled(s);
    draw_spin_field(
        c,
        p,
        field,
        lay.up.scaled(s),
        lay.down.scaled(s),
        ud.enabled,
    );
    let (tr, align) = text_rect(ud, lay.edit.scaled(s), s);
    let shown = ud.shown();
    if ud.enabled {
        text.draw(c, tr, &shown, false, cs.button_text, align, px);
        if caret {
            let x0 = text_x(text, &shown, tr, align, px);
            let at = |i: usize| x0 + text.measure(&shown[..i], false, px);
            let (sel, cx) = match &ud.edit {
                Some(e) => (e.selection(), at(e.caret())),
                None => (0..0, at(shown.len())),
            };
            if !sel.is_empty() {
                // One text line tall, centred in the field, like the time
                // field's current part.
                let (a, b) = (at(sel.start), at(sel.end));
                let lh = p.text.len(LINE_H) * s;
                let hl = Rect::new(a, tr.y + (tr.height - lh) / 2, b - a, lh);
                c.fill_rect(hl, cs.hilight);
                text.draw(
                    c,
                    Rect::new(a, tr.y, tr.right() - a, tr.height),
                    &shown[sel],
                    false,
                    cs.hilight_text,
                    Align::Left,
                    px,
                );
            }
            let bar = Rect::new(cx, tr.y + 2 * s, s, tr.height - 4 * s);
            c.fill_rect(bar, cs.button_text);
            if let Some(e) = &ud.edit {
                c.report_spot(TextSpot {
                    digits: true,
                    ..e.spot(lay.edit.scaled(s), bar)
                });
            }
        }
    } else {
        draw_disabled_label(
            c,
            text,
            p,
            tr,
            &shown,
            align,
            px,
            Some(crate::style::Part::EditBorder),
        );
    }
    draw_spin_buttons(c, p, lay.up, lay.down, ud.enabled, spin);
}

pub fn access(
    ud: &UpDown,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::range(
        Role::SpinButton,
        lay.field,
        ud.value.into(),
        ud.min.into(),
        ud.max.into(),
        ud.enabled,
    );
    if let Some(e) = &ud.edit
        && e.text != ud.value.to_string()
    {
        n.set_value(e.text.as_str());
    }
    tree.push(parent, key, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    /// The Date and Time sheet's greyed 79x21 year spinner on its page.
    #[test]
    fn a_blue_spin_gap_row_on_a_page_shows_the_page() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let face = theme.colors.button_face;
        let p = theme.paint();
        let sheet = crate::controls::propsheet::PropSheet::new(
            &["Date & Time"],
            vec![crate::controls::form::Form::new(Vec::new())],
            (398, 317),
        );
        let page = sheet.frame().1;
        let mut ud = UpDown::new(2026, 1970, 2099).with_buddy();
        ud.enabled = false;
        let l = layout_for(
            &ud,
            Rect::new(112, 58, 79, 21),
            &theme.look.controls(),
            TextUnits::ONE,
        );
        let gap = Rect::new(
            l.up.x - 1,
            l.up.bottom(),
            l.up.width + 2,
            l.down.y - l.up.bottom(),
        );
        assert_eq!(gap, Rect::new(173, 68, 17, 1));
        let mut bare = Canvas::new(398, 317);
        bare.fill_rect(bare.bounds(), face);
        crate::look::widgets::draw_page(&mut bare, &p, page);
        let mut on_page = bare.clone();
        draw(
            &mut on_page,
            &text,
            &p.on_page(page),
            &ud,
            &l,
            false,
            None,
            11,
        );
        let mut on_dialog = Canvas::new(398, 317);
        on_dialog.fill_rect(on_dialog.bounds(), face);
        draw(&mut on_dialog, &text, &p, &ud, &l, false, None, 11);
        for x in gap.x..gap.right() {
            assert_eq!(
                on_page.get(x, 68),
                0xFFFA_FBFA,
                "the page's row 68 at x {x}"
            );
            assert_eq!(
                on_page.get(x, 68),
                bare.get(x, 68),
                "the gap is the page at x {x}"
            );
            assert_eq!(
                on_dialog.get(x, 68),
                face.argb(),
                "the gap is face at x {x}"
            );
        }
    }

    #[test]
    fn layout_and_value_stepping() {
        let text = TextRenderer::load(&FontConfig::default());
        let l = layout(61, 282);
        assert_eq!(l.field, Rect::new(61, 282, 50, 20));
        assert_eq!(l.edit, Rect::new(63, 284, 31, 16));
        assert_eq!(l.up, Rect::new(94, 284, 15, 8));
        assert_eq!(l.down, Rect::new(94, 292, 15, 8));
        let mut ud = UpDown::new(10, 1, 9999);
        assert_eq!(hit(&ud, &l, 100, 286, &text, 11), Some(Hit::Up));
        assert_eq!(hit(&ud, &l, 100, 295, &text, 11), Some(Hit::Down));
        // "10" is right-aligned, so the field's left part is before it.
        assert_eq!(hit(&ud, &l, 70, 290, &text, 11), Some(Hit::Edit(0)));
        assert_eq!(hit(&ud, &l, 91, 290, &text, 11), Some(Hit::Edit(2)));
        assert!(ud.step(-1));
        assert_eq!(ud.value(), 9);
        assert!(!UpDown::new(1, 1, 9).step(-1));
        assert!(ud.digit('3'));
        assert!(ud.digit('0'));
        assert_eq!((ud.value(), ud.shown().as_str()), (30, "30"));
        assert!(ud.backspace());
        assert_eq!(ud.value(), 3);
        assert!(ud.digit('0') && ud.digit('0') && ud.digit('0'));
        assert_eq!(ud.value(), 3000);
        assert!(!ud.digit('0'), "four digits at most");
        let mut small = UpDown::new(1, 1, 500);
        assert!(small.digit('9') && small.digit('9') && small.digit('9'));
        assert_eq!(small.value(), 500, "clamped to max");
    }

    #[test]
    fn leaving_drops_the_digits_and_shows_the_clamped_value() {
        let mut ud = UpDown::new(30, 1, 300);
        ud.enter();
        for d in ['9', '9', '9'] {
            ud.digit(d);
        }
        assert_eq!((ud.value(), ud.shown().as_str()), (300, "999"));
        ud.leave();
        assert!(!ud.editing());
        assert_eq!(ud.shown(), "300", "the field shows the clamped value");
        // Back by the keyboard: the number is selected, so a digit
        // replaces it.
        ud.enter();
        assert!(ud.digit('1'));
        assert_eq!((ud.value(), ud.shown().as_str()), (1, "1"));
    }

    #[test]
    fn a_placed_caret_takes_digits_where_it_is() {
        let mut ud = UpDown::new(30, 1, 999);
        ud.enter();
        ud.place(1);
        assert!(ud.digit('5'));
        assert_eq!((ud.value(), ud.shown().as_str()), (350, "350"));
        assert!(ud.key(&EditKey::Home { select: false }) == Some(false));
        assert!(
            ud.digit('1') && ud.value() == 999,
            "1350 is four digits, clamped"
        );
        assert_eq!(
            ud.key(&EditKey::Insert("a".into())),
            None,
            "a letter is not taken"
        );
    }

    #[test]
    fn setting_the_same_value_keeps_the_caret() {
        let mut ud = UpDown::new(30, 1, 99);
        ud.enter();
        ud.place(0);
        assert!(!ud.set(30));
        assert!(ud.digit('1'));
        assert_eq!(ud.shown(), "130", "the caret stayed in front");
        assert!(ud.set(5));
        assert!(ud.digit('7'));
        assert_eq!(ud.value(), 7, "a new value is selected");
    }

    #[test]
    fn a_step_selects_the_new_value() {
        let mut ud = UpDown::new(30, 1, 99);
        ud.enter();
        ud.place(2);
        assert!(ud.step(1));
        assert!(ud.digit('7'));
        assert_eq!(ud.value(), 7, "the stepped value was replaced");
    }

    #[test]
    fn an_empty_range_pins_the_value_to_max() {
        let mut ud = UpDown {
            value: 5,
            min: 9,
            max: 1,
            enabled: true,
            edit: None,
            buddy: false,
        };
        assert!(ud.step(1));
        assert_eq!(ud.value(), 1);
        assert!(!ud.digit('7'));
        assert!(!ud.backspace());
        assert_eq!(ud.value(), 1);
    }

    #[test]
    fn a_focused_field_highlights_its_selection() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let l = layout(61, 282);
        let mut ud = UpDown::new(10, 1, 9999);
        ud.enter();
        let render = |ud: &UpDown| {
            let mut c = Canvas::new(130, 320);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &text, &classic_paint(cs, 1), ud, &l, true, None, 11);
            c
        };
        // "10" ends 2 px inside the white part's right edge, with the
        // caret at x 92; the highlight runs from the number's left edge,
        // 13 px tall from row 285, a pixel under the white part's top.
        let w = text.measure("10", false, 11);
        let c = render(&ud);
        let x0 = 92 - w;
        assert_eq!(c.get(x0, 285), cs.hilight.argb());
        assert_eq!(c.get(91, 297), cs.hilight.argb());
        assert_eq!(c.get(x0, 284), cs.window.argb(), "white above it");
        assert_eq!(c.get(x0, 298), cs.window.argb(), "white under it");
        assert_eq!(
            c.get(x0 - 1, 290),
            cs.window.argb(),
            "white before the number"
        );
        assert_eq!(c.get(92, 290), cs.button_text.argb(), "caret after it");
        let ink = (x0..92).any(|x| (284..300).any(|y| c.get(x, y) == cs.hilight_text.argb()));
        assert!(ink, "the digits are drawn in the highlight text colour");
        // A click drops the selection and puts the caret between the
        // digits.
        ud.place(1);
        let c = render(&ud);
        assert_eq!(c.get(x0, 285), cs.window.argb());
        let cx = x0 + text.measure("1", false, 11);
        assert_eq!(c.get(cx, 290), cs.button_text.argb(), "caret after the 1");
        assert_eq!(c.get(92, 290), cs.window.argb(), "no caret at the end");
    }

    #[test]
    fn a_focused_field_reports_its_digits_for_the_edit_menu() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let p = classic_paint(&theme.colors, 1);
        let l = layout(61, 282);
        let mut ud = UpDown::new(10, 1, 9999);
        let render = |ud: &UpDown, focused: bool| {
            let mut c = Canvas::new(130, 320);
            draw(&mut c, &text, &p, ud, &l, focused, None, 11);
            c.take_spot()
        };
        assert_eq!(render(&ud, false), None, "an unfocused field reports none");
        ud.enter();
        let spot = render(&ud, true).expect("a focused field reports its spot");
        assert_eq!(spot.field, l.edit, "the digits' part, not the arrows");
        assert!(spot.selected && !spot.multiline && !spot.secret);
        assert!(spot.digits, "digits only, so no input method turns on");
        assert_eq!(spot.caret.x, 92, "the caret after the number");
    }

    #[test]
    fn a_paste_takes_a_number_that_fits() {
        let mut ud = UpDown::new(10, 1, 9999);
        ud.enter();
        assert!(ud.paste(" 250\n"));
        assert_eq!(ud.value(), 250);
        assert!(!ud.paste("12a"), "not a number");
        assert!(!ud.paste(""));
        ud.key(&EditKey::SelectAll);
        assert!(!ud.paste("12345"), "more digits than the field holds");
        assert_eq!(ud.value(), 250);
    }

    #[test]
    fn draws_its_pixels() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let l = layout(61, 282);
        let ud = UpDown::new(10, 1, 9999);
        let mut c = Canvas::new(130, 320);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &ud,
            &l,
            false,
            None,
            11,
        );
        assert_eq!(c.get(61, 282), cs.button_shadow.argb());
        assert_eq!(c.get(110, 282), cs.button_hilight.argb());
        assert_eq!(c.get(70, 290), cs.window.argb());
        // Up button: face then dark shadow on its top row, the inner
        // hilight under it; the arrow tip 40 px right of the field's left
        // edge and 5 px below its top, the base row under it.
        assert_eq!(c.get(94, 284), cs.button_face.argb());
        assert_eq!(c.get(108, 284), cs.button_dk_shadow.argb());
        assert_eq!(c.get(95, 285), cs.button_hilight.argb());
        assert_eq!(c.get(107, 285), cs.button_shadow.argb());
        assert_eq!(c.get(101, 287), cs.button_text.argb());
        assert_eq!(c.get(100, 288), cs.button_text.argb());
        assert_eq!(c.get(102, 288), cs.button_text.argb());
        assert_eq!(c.get(101, 286), cs.button_face.argb());
        // Down button: base row 13 px below the field's top, tip 14; its
        // dark shadow row 17.
        assert_eq!(c.get(100, 295), cs.button_text.argb());
        assert_eq!(c.get(101, 296), cs.button_text.argb());
        assert_eq!(c.get(100, 299), cs.button_dk_shadow.argb());
        assert_eq!(c.get(109, 290), cs.button_face.argb(), "light column");
    }

    #[test]
    fn a_buddy_spinner_puts_its_buttons_outside_the_field() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let ud = UpDown::new(2026, 1970, 2099).with_buddy();
        let l = layout_for(
            &ud,
            Rect::new(112, 55, 79, 21),
            &ControlMetrics::CLASSIC,
            TextUnits::ONE,
        );
        assert_eq!(l.field, Rect::new(112, 55, 63, 21));
        assert_eq!(l.up, Rect::new(175, 55, 16, 10));
        assert_eq!(l.down, Rect::new(175, 65, 16, 10));
        let mut c = Canvas::new(200, 80);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &ud,
            &l,
            false,
            None,
            11,
        );
        // The field's sunken edge ends at x 174, the buttons start at 175:
        // outer face column, inner hilight, dark outer right edge at 190.
        assert_eq!(c.get(174, 60), cs.button_hilight.argb());
        assert_eq!(c.get(176, 60), cs.button_hilight.argb());
        assert_eq!(c.get(190, 60), cs.button_dk_shadow.argb());
        // Up arrow: tip (182, 59), base 181..=183 on row 60; down arrow:
        // base on row 69, tip (182, 70).
        assert_eq!(c.get(182, 59), cs.button_text.argb());
        assert_eq!(c.get(181, 60), cs.button_text.argb());
        assert_eq!(c.get(183, 60), cs.button_text.argb());
        assert_eq!(c.get(181, 69), cs.button_text.argb());
        assert_eq!(c.get(182, 70), cs.button_text.argb());
        assert_eq!(c.get(181, 70), cs.button_face.argb());
        // The year is left-aligned: its ink starts inside the first 8 px.
        let ink = (114..130).find(|&x| (57..73).any(|y| c.get(x, y) == cs.button_text.argb()));
        assert!(
            ink.is_some_and(|x| x <= 119),
            "the year starts at the left, got {ink:?}"
        );
    }

    /// Blue's greyed Wait spinner on a tab page: the field keeps its
    /// line and shows the page through it, and its buttons sit in
    /// a white well.
    #[test]
    fn a_greyed_blue_spinner_shows_the_page_and_wells_its_buttons() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let pane = crate::theme::Color::rgb(0xF5, 0xF4, 0xF0);
        let l = layout(3, 4);
        assert_eq!(l.up, Rect::new(36, 6, 15, 8));
        assert_eq!(l.down, Rect::new(36, 14, 15, 8));
        let mut ud = UpDown::new(10, 1, 9999);
        ud.enabled = false;
        let mut c = Canvas::new(56, 28);
        c.fill_rect(c.bounds(), pane);
        let p = theme.paint();
        draw(&mut c, &text, &p, &ud, &l, false, None, 11);
        let (line, white) = (
            crate::blue::Variant::Blue
                .scheme()
                .edit_border
                .normal
                .rim
                .argb(),
            cs.window.argb(),
        );
        for x in 3..53 {
            assert_eq!(c.get(x, 4), line, "top line at x {x}");
            assert_eq!(c.get(x, 23), line, "bottom line at x {x}");
        }
        for y in 4..24 {
            assert_eq!(c.get(3, y), line, "left line at y {y}");
            assert_eq!(c.get(52, y), line, "right line at y {y}");
        }
        for (x, y) in [(4, 5), (4, 22), (6, 8), (34, 5), (34, 22)] {
            assert_eq!(c.get(x, y), pane.argb(), "the page shows at ({x}, {y})");
        }
        let digits: Vec<u32> = (6..22)
            .flat_map(|y| (4..35).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .collect();
        assert!(
            digits.contains(&cs.gray_text.argb()),
            "the number inks #ACA899"
        );
        assert!(!digits.contains(&white), "with no white copy under it");
        for y in 5..23 {
            assert_eq!(c.get(35, y), white, "the well's left side at y {y}");
            assert_eq!(c.get(51, y), white, "the well's right side at y {y}");
        }
        for x in 35..52 {
            assert_eq!(c.get(x, 5), white, "the well's top at x {x}");
            assert_eq!(c.get(x, 22), white, "the well's bottom at x {x}");
        }
        let mut b = Canvas::new(56, 28);
        b.fill_rect(b.bounds(), pane);
        crate::look::widgets::draw_spin_buttons(&mut b, &p, l.up, l.down, false, None);
        for y in 6..22 {
            for x in 36..51 {
                assert_eq!(c.get(x, y), b.get(x, y), "greyed button at ({x}, {y})");
            }
        }
    }

    /// Blue's year field at the Date and Time sheet's 79x21: the buttons
    /// stand inside the field's line, 15 px wide, 8 px each with a face
    /// row between them, in a white well.
    #[test]
    fn a_blue_buddy_spinner_puts_its_buttons_inside_the_field() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let ud = UpDown::new(2026, 1970, 2099).with_buddy();
        let r = Rect::new(112, 58, 79, 21);
        let l = layout_for(&ud, r, &theme.look.controls(), TextUnits::ONE);
        assert_eq!(l.field, r);
        assert_eq!(l.edit, Rect::new(114, 60, 60, 17));
        assert_eq!(l.up, Rect::new(174, 60, 15, 8));
        assert_eq!(l.down, Rect::new(174, 69, 15, 8));
        assert_eq!(hit(&ud, &l, 180, 64, &text, 11), Some(Hit::Up));
        assert_eq!(hit(&ud, &l, 180, 72, &text, 11), Some(Hit::Down));
        let mut c = Canvas::new(200, 85);
        c.fill_rect(c.bounds(), cs.button_face);
        let p = theme.paint();
        draw(&mut c, &text, &p, &ud, &l, false, None, 11);
        let line = crate::blue::Variant::Blue
            .scheme()
            .edit_border
            .normal
            .rim
            .argb();
        let (white, face) = (cs.window.argb(), cs.button_face.argb());
        for y in 58..79 {
            assert_eq!(c.get(190, y), line, "the field's right line at y {y}");
            assert_eq!(c.get(191, y), face, "nothing right of the line at y {y}");
        }
        for x in 173..190 {
            assert_eq!(c.get(x, 59), white, "the well's top at x {x}");
            assert_eq!(c.get(x, 68), face, "the gap row at x {x}");
            assert_eq!(c.get(x, 77), white, "the well's bottom at x {x}");
        }
        for y in (60..68).chain(69..77) {
            assert_eq!(c.get(173, y), white, "the well's left side at y {y}");
            assert_eq!(c.get(189, y), white, "the well's right side at y {y}");
        }
        let mut b = Canvas::new(200, 85);
        b.fill_rect(b.bounds(), cs.window);
        crate::look::widgets::draw_spin_buttons(&mut b, &p, l.up, l.down, true, None);
        for y in (60..68).chain(69..77) {
            for x in 174..189 {
                assert_eq!(c.get(x, y), b.get(x, y), "button art at ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_disabled_updown_etches_its_arrows() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let l = layout(61, 282);
        let mut ud = UpDown::new(10, 1, 9999);
        ud.enabled = false;
        let mut c = Canvas::new(130, 320);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &ud,
            &l,
            false,
            None,
            11,
        );
        // Up arrow: shadow tip (101, 287) and base 100..=102 on row 288,
        // over a hilight copy one pixel down and right.
        assert_eq!(c.get(101, 287), cs.button_shadow.argb());
        assert_eq!(c.get(100, 288), cs.button_shadow.argb());
        assert_eq!(c.get(102, 288), cs.button_shadow.argb());
        assert_eq!(c.get(102, 287), cs.button_face.argb());
        assert_eq!(c.get(101, 289), cs.button_hilight.argb());
        assert_eq!(c.get(103, 289), cs.button_hilight.argb());
        // Down arrow: shadow base on row 295 and tip (101, 296), the
        // hilight copy under and right of them.
        assert_eq!(c.get(100, 295), cs.button_shadow.argb());
        assert_eq!(c.get(101, 296), cs.button_shadow.argb());
        assert_eq!(c.get(103, 296), cs.button_hilight.argb());
        assert_eq!(c.get(102, 297), cs.button_hilight.argb());
    }
}
