//! Edit fields: the focused edit, paste, and text dragged onto a field.

use std::borrow::Cow;
use std::cell::Cell;
use zeroize::Zeroize;

use crate::classic::widgets::{self, LINE_H};
use crate::controls::combo;
use crate::controls::multiedit;
use crate::text::TextRenderer;
use crate::textedit::TextEdit;

use super::{Control, Event, Form, Hit, Part};

/// A form's focused edit control ([`Form::focused_edit`]): a one-line
/// field or a memo's lines.
#[derive(Clone, Copy, Debug)]
pub enum FocusedEdit<'a> {
    Line(&'a TextEdit),
    Lines(&'a multiedit::MultiEdit),
}

impl FocusedEdit<'_> {
    /// The selection as Copy and Cut hand it to the clipboard: `None`
    /// from a secret field, which never hands its text out.
    pub fn copy_text(&self) -> Option<String> {
        match self {
            FocusedEdit::Line(e) if e.is_secret() => None,
            FocusedEdit::Line(e) => Some(e.selected_text().to_string()),
            FocusedEdit::Lines(e) => Some(e.selected_text()),
        }
    }
}

/// The edit field a drag rests on and where, in form px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FieldDrop {
    pub(super) item: usize,
    pub(super) x: i32,
    pub(super) y: i32,
}

impl Form {
    /// The focused edit control, a field or a memo.
    pub fn focused_edit(&self) -> Option<FocusedEdit<'_>> {
        match &self.items.get(self.focus?)?.control {
            Control::Edit { edit, .. } => Some(FocusedEdit::Line(edit)),
            Control::Memo(m) => Some(FocusedEdit::Lines(&m.edit)),
            Control::UpDown(u) => u.edit().map(FocusedEdit::Line),
            _ => None,
        }
    }

    /// A host's clipboard text for the focused edit field, taken as
    /// [`TextEdit::paste`] takes it. The text is zeroized once used, since
    /// the field may be a password.
    pub fn paste(&mut self, mut text: String) -> Event {
        let field = self.focus.filter(|_| self.open_combo().is_none());
        let ev = match field.map(|i| (self.items[i].id, &mut self.items[i].control)) {
            Some((id, Control::Memo(m))) => {
                if m.edit.paste(&text) {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            Some((id, Control::Edit { edit, .. })) => {
                if edit.paste(&text, &[]) {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            Some((id, Control::UpDown(u))) if u.enabled => {
                if u.paste(&text) {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            _ => Event::None,
        };
        text.zeroize();
        ev
    }

    /// A drag at (`x`, `y`): over an enabled, unmasked edit field or editable combo the field
    /// shows a drop caret there and takes the drag (`true`); anywhere else the drop caret goes.
    pub fn drag_over(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool {
        self.drag_leave();
        let Some(Hit { item, part, .. }) = self.hit(x, y, text, px) else {
            return false;
        };
        let cm = self.controls();
        let placed = self.at(&self.items[item]);
        let it = &mut self.items[item];
        let takes = match (part, &mut it.control) {
            (Part::Edit(_), Control::Edit { edit, .. }) => !edit.is_secret(),
            (Part::Combo(combo::Hit::Field), Control::Combo(c)) => {
                c.drag_over(&combo::layout(c, placed, &cm), x, y)
            }
            _ => false,
        };
        if takes {
            self.field_drop = Some(FieldDrop { item, x, y });
        }
        takes
    }

    /// The drag left or ended: `true` when a drop caret was showing.
    pub fn drag_leave(&mut self) -> bool {
        let Some(d) = self.field_drop.take() else {
            return false;
        };
        if let Some(Control::Combo(c)) = self.items.get_mut(d.item).map(|it| &mut it.control) {
            c.drag_leave();
        }
        true
    }

    /// The scroll offset one step on for the field under a resting drag
    /// ([`crate::textedit::drag_scroll`], a [`LINE_H`] band at each side), None when it stays.
    pub fn drag_scroll(&self, text: &TextRenderer, px: u32) -> Option<usize> {
        let d = self.field_drop?;
        let it = self.items.get(d.item)?;
        let text = text.dialog(self.font);
        let cm = self.controls();
        match &it.control {
            Control::Edit { edit, first, .. } => {
                let inner = self.at(it).inset(widgets::FIELD_PAD);
                crate::textedit::drag_scroll(
                    text,
                    &edit.text,
                    visible_from(edit, first),
                    inner.width,
                    self.units().len(LINE_H),
                    d.x - inner.x,
                    px,
                )
            }
            Control::Combo(c) => c.drag_scroll(&combo::layout(c, self.at(it), &cm), text, px),
            _ => None,
        }
    }

    /// Take [`Form::drag_scroll`]'s step, for a host's tick; `true` when the field moved.
    pub fn drag_tick(&mut self, text: &TextRenderer, px: u32) -> bool {
        let Some(to) = self.drag_scroll(text, px) else {
            return false;
        };
        match self
            .field_drop
            .and_then(|d| self.items.get(d.item))
            .map(|it| &it.control)
        {
            Some(Control::Edit { first, .. }) => first.set(to),
            Some(Control::Combo(c)) => c.first.set(to),
            _ => return false,
        }
        true
    }

    /// A drag dropped `line` on the field under it: it goes in at the drop caret as
    /// [`TextEdit::drop_text`] puts it, and the field takes the focus. `Changed` with the field's
    /// id, or `None` when no field was taking the drag or nothing went in.
    pub fn drop_text(
        &mut self,
        line: &crate::textedit::DropLine,
        text: &TextRenderer,
        px: u32,
    ) -> Event {
        let Some(d) = self.field_drop.take() else {
            return Event::None;
        };
        let hit = self.hit(d.x, d.y, text, px).filter(|h| h.item == d.item);
        let cm = self.controls();
        let dialog = text.dialog(self.font);
        let placed = self.at(&self.items[d.item]);
        let it = &mut self.items[d.item];
        let took = match (hit.map(|h| h.part), &mut it.control) {
            (Some(Part::Edit(at)), Control::Edit { edit, .. }) => edit.drop_text(at, line),
            (_, Control::Combo(c)) => c.drop_text(&combo::layout(c, placed, &cm), line, dialog, px),
            _ => false,
        };
        if !took {
            return Event::None;
        }
        let id = it.id;
        self.set_focus(Some(d.item));
        Event::Changed(id)
    }
}

/// A field's scroll offset, guarded: `first` is a draw-time cache, and
/// the value under it may have been replaced since the last frame set
/// it. An offset past the end, or inside a character, starts again at 0.
pub(crate) fn visible_from(edit: &TextEdit, first: &Cell<usize>) -> usize {
    let n = first.get();
    if n <= edit.text.len() && edit.text.is_char_boundary(n) {
        n
    } else {
        0
    }
}

const BULLET: &str = "\u{25cf}";

/// The bullets a masked field shows, with caret and anchor mapped by char index; `first`
/// indexes these bullets. Secret itself, so the field shows no preedit and
/// reports no text to an input method.
pub(super) fn masked(edit: &TextEdit) -> TextEdit {
    let at = |i: usize| edit.text[..i].chars().count() * BULLET.len();
    let mut m = TextEdit::secret();
    m.set_cue(edit.cue());
    m.text.push_str(&BULLET.repeat(edit.text.chars().count()));
    m.set_selection(at(edit.anchor()), at(edit.caret()));
    m
}

/// What a field shows of `edit`: its bullets when masked ([`masked`]), else the value.
pub(super) fn as_shown(edit: &TextEdit) -> Cow<'_, TextEdit> {
    if edit.is_secret() {
        Cow::Owned(masked(edit))
    } else {
        Cow::Borrowed(edit)
    }
}

pub(super) fn unmasked_index(edit: &TextEdit, at: usize) -> usize {
    edit.text
        .char_indices()
        .nth(at / BULLET.len())
        .map_or(edit.text.len(), |(i, _)| i)
}
