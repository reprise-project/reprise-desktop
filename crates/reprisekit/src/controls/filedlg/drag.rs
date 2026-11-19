//! Drags, drops and pastes onto the File name field.

use super::{FileDlg, Focus, Hit, NamePaste};
use crate::classic::widgets::{LINE_H, edit_first_visible};
use crate::raster::Rect;
use crate::text::TextRenderer;

impl FileDlg {
    /// A drag at (`x`, `y`) in `r`: over File name the field shows a drop caret there and takes
    /// the drag (`true`); anywhere else the drop caret goes.
    pub fn drag_over(&mut self, text: &TextRenderer, px: u32, r: Rect, x: i32, y: i32) -> bool {
        if self.views_open || self.hit(text, px, r, x, y) != Some(Hit::FileName) {
            self.name_drop = None;
            return false;
        }
        let first = match self.name_drop {
            Some((_, first)) => first,
            None => {
                let w = self.name_inner(text, px, r).width;
                edit_first_visible(text, &self.filename, w, px, 0)
            }
        };
        self.name_drop = Some((x, first));
        true
    }

    /// The drag left or ended: `true` when a drop caret was showing.
    pub fn drag_leave(&mut self) -> bool {
        self.name_drop.take().is_some()
    }

    /// The first byte shown one step on for a drag resting at File name's edge
    /// ([`crate::textedit::drag_scroll`]), None when it stays.
    pub fn drag_scroll(&self, text: &TextRenderer, px: u32, r: Rect) -> Option<usize> {
        let (x, first) = self.name_drop?;
        let inner = self.name_inner(text, px, r);
        let s = &self.filename.text;
        crate::textedit::drag_scroll(text, s, first, inner.width, LINE_H, x - inner.x, px)
    }

    /// Take [`FileDlg::drag_scroll`]'s step, for a host's tick; `true` when the field moved.
    pub fn drag_tick(&mut self, text: &TextRenderer, px: u32, r: Rect) -> bool {
        let Some(to) = self.drag_scroll(text, px, r) else {
            return false;
        };
        if let Some((_, first)) = &mut self.name_drop {
            *first = to;
        }
        true
    }

    /// Notes the field the paste key was pressed in, [`FileDlg::focused_edit`], as where the
    /// clipboard text it asks for goes.
    pub fn ask_paste(&mut self) {
        self.paste_to = if self.shell.edit.is_some() {
            Some(NamePaste::Rename(self.renames))
        } else {
            (self.focus == Focus::FileName).then_some(NamePaste::FileName)
        };
    }

    /// The clipboard's text into the field its paste key was pressed in ([`FileDlg::ask_paste`]):
    /// that rename box while it is still open (no `/`), or File name, its first line as
    /// [`TextEdit::paste`] takes it. `true` when the field changed.
    pub fn paste(&mut self, text: &str) -> bool {
        match self.paste_to {
            Some(NamePaste::Rename(n)) if n == self.renames => {
                (self.shell.edit.as_mut()).is_some_and(|(_, ed)| ed.paste(text, &['/']))
            }
            Some(NamePaste::FileName) if self.filename.paste(text, &[]) => {
                self.typed = self.filename.text.clone();
                true
            }
            _ => false,
        }
    }

    /// A drag dropped `line` on File name: it goes in at the drop caret as
    /// [`TextEdit::drop_text`] puts it, and the field takes the focus. `false` when the field
    /// was not taking the drag or nothing went in.
    pub fn drop_text(
        &mut self,
        text: &TextRenderer,
        px: u32,
        r: Rect,
        line: &crate::textedit::DropLine,
    ) -> bool {
        let Some((x, first)) = self.name_drop.take() else {
            return false;
        };
        let inner = self.name_inner(text, px, r);
        let first = first.min(self.filename.text.len());
        let tail = &self.filename.text[first..];
        let at = first + crate::textedit::index_at(text, tail, x - inner.x, px);
        if !self.filename.drop_text(at, line) {
            return false;
        }
        self.typed = self.filename.text.clone();
        self.focus = Focus::FileName;
        true
    }

    /// Files dropped on File name replace its text, selected: `names` as the field takes a
    /// selection of several (quoted) where the dialog picks several, else the first.
    pub fn drop_names(&mut self, names: &[String]) -> bool {
        if self.name_drop.take().is_none() {
            return false;
        }
        let text = match names {
            [] => return false,
            [_, _, ..] if self.multi() => names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(" "),
            [first, ..] => first.clone(),
        };
        self.set_file_name(&text);
        self.focus = Focus::FileName;
        true
    }
}
