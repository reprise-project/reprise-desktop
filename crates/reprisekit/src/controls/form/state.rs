//! Reading and setting items by id, and carrying them over when the host rebuilds the form.

use crate::classic::widgets::CheckState;
use crate::raster::{Canvas, Rect};

use super::item::Item;
use super::{Control, Form};

impl Form {
    /// Take over `old`'s hot or held item when the host rebuilds this page under the pointer.
    /// Items are re-matched by id; the part is dropped when the id is gone or the item no longer offers that light.
    pub fn carry_pointer(&mut self, old: &Form) {
        self.point = old.point;
        self.pointer = old.pointer.carry(|(i, light)| {
            let id = old.items.get(i)?.id;
            let j = self.items.iter().position(|it| it.id == id)?;
            light
                .offered_by(&self.items[j].control)
                .then_some((j, light))
        });
    }

    /// Take over what `old`'s controls hold, its focus and its pointer when the host builds the
    /// same form again for a new text size or font. Items are matched by id; labels and notes
    /// keep their new text, which was measured for the new size.
    pub fn carry(&mut self, old: &Form) {
        for it in &mut self.items {
            if matches!(
                it.control,
                Control::Label { .. } | Control::Lines(_) | Control::GreyedLines(_)
            ) {
                continue;
            }
            if let Some(o) = old.item(it.id)
                && std::mem::discriminant(&o.control) == std::mem::discriminant(&it.control)
            {
                it.control = o.control.clone();
            }
        }
        let focus = old.focus.and_then(|i| self.index_of(old.items.get(i)?.id));
        self.set_focus(focus);
        self.carry_pointer(old);
    }

    /// Move each control to the rect of `template`'s item with the same id, moved by `offset`;
    /// a control without one stays. Lets a resized window rebuild the page and keep what the controls hold.
    pub fn take_layout(&mut self, template: &Form, offset: (i32, i32)) {
        for it in &mut self.items {
            if let Some(t) = template.items.iter().find(|t| t.id == it.id) {
                it.rect = Rect::new(
                    t.rect.x + offset.0,
                    t.rect.y + offset.1,
                    t.rect.width,
                    t.rect.height,
                );
            }
        }
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn combo_selected(&self, id: &str) -> Option<usize> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::Combo(c)) => Some(c.selected),
            _ => None,
        }
    }

    pub fn track_pos(&self, id: &str) -> Option<usize> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::TrackBar(t)) => Some(t.pos),
            _ => None,
        }
    }

    pub fn checked(&self, id: &str) -> Option<bool> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::Check { checked, .. }) | Some(Control::Radio { checked, .. }) => {
                Some(*checked)
            }
            _ => None,
        }
    }

    /// Put `s` in label `id`, or make it note `id`'s one line.
    pub fn set_text(&mut self, id: &str, s: &str) {
        match self.item_mut(id).map(|i| &mut i.control) {
            Some(Control::Label { text, .. }) => *text = s.to_string(),
            Some(Control::Lines(lines) | Control::GreyedLines(lines)) => *lines = vec![s.into()],
            _ => {}
        }
    }

    pub fn set_lines(&mut self, id: &str, s: &[String]) {
        if let Some(Control::Lines(lines) | Control::GreyedLines(lines)) =
            self.item_mut(id).map(|i| &mut i.control)
        {
            *lines = s.to_vec();
        }
    }

    pub fn set_picture(&mut self, id: &str, c: Canvas) {
        if let Some(Control::Picture { canvas, .. }) = self.item_mut(id).map(|i| &mut i.control) {
            *canvas = Some(c);
        }
    }

    /// Check or clear check box or radio `id`; a three-state box leaves
    /// its indeterminate state.
    pub fn set_check(&mut self, id: &str, on: bool) {
        match self.item_mut(id).map(|i| &mut i.control) {
            Some(Control::Check { checked, mixed, .. }) => {
                *checked = on;
                if let Some(m) = mixed {
                    *m = false;
                }
            }
            // A radio's own dot only: the host sets its group's others.
            Some(Control::Radio { checked, .. }) => *checked = on,
            _ => {}
        }
    }

    pub fn check_state(&self, id: &str) -> Option<CheckState> {
        match self.item(id).map(|i| &i.control) {
            Some(Control::Check {
                mixed: Some(true), ..
            }) => Some(CheckState::Mixed),
            Some(Control::Check { checked: true, .. }) => Some(CheckState::On),
            Some(Control::Check { .. }) => Some(CheckState::Off),
            _ => None,
        }
    }

    /// Put check box `id` in `state`. A two-state box reads `Mixed` as
    /// off.
    pub fn set_check_state(&mut self, id: &str, state: CheckState) {
        if let Some(Control::Check { checked, mixed, .. }) =
            self.item_mut(id).map(|i| &mut i.control)
        {
            *checked = state == CheckState::On;
            if let Some(m) = mixed {
                *m = state == CheckState::Mixed;
            }
        }
    }

    pub fn set_combo(&mut self, id: &str, selected: usize) {
        if let Some(Control::Combo(c)) = self.item_mut(id).map(|i| &mut i.control) {
            c.selected = selected.min(c.items.len().saturating_sub(1));
        }
    }

    /// Move trackbar `id` to `pos`, clamped to its last tick. Its thumb
    /// turns hot or cold when it moved to or from under the pointer, or
    /// was greyed or enabled under it.
    pub fn set_track(&mut self, id: &str, pos: usize) {
        let Some(i) = self.index_of(id) else {
            return;
        };
        if let Control::TrackBar(t) = &mut self.items[i].control {
            t.set(pos);
            self.rehit_thumb(i);
        }
    }

    /// Enable or grey control `id`; a control with no greyed state (a
    /// group, a picture, a paragraph) is left alone.
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        match self.item_mut(id).map(|i| &mut i.control) {
            Some(Control::Label { enabled, .. })
            | Some(Control::Check { enabled, .. })
            | Some(Control::Radio { enabled, .. })
            | Some(Control::Button { enabled, .. })
            | Some(Control::Edit { enabled, .. }) => *enabled = on,
            Some(Control::Combo(c)) => c.enabled = on,
            Some(Control::UpDown(u)) => u.enabled = on,
            Some(Control::Color(b)) => b.enabled = on,
            Some(Control::TrackBar(t)) => t.enabled = on,
            Some(Control::Calendar(cal)) => cal.enabled = on,
            Some(Control::Time(tf)) => tf.enabled = on,
            Some(Control::HotKey(h)) => h.enabled = on,
            Some(Control::Memo(m)) => m.enabled = on,
            _ => {}
        }
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.items.iter().position(|i| i.id == id)
    }
}

/// A click, Space or access key on a check box: a two-state box flips, a
/// three-state one takes the next of indeterminate, off, on.
pub(super) fn toggle_check(checked: &mut bool, mixed: &mut Option<bool>) {
    let state = match (*mixed, *checked) {
        (Some(true), _) => CheckState::Mixed,
        (_, true) => CheckState::On,
        (_, false) => CheckState::Off,
    };
    let next = state.cycled(mixed.is_some());
    *checked = next == CheckState::On;
    if let Some(m) = mixed {
        *m = next == CheckState::Mixed;
    }
}
