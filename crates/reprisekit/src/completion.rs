//! A completion shown inline in a single-line edit: the typed query, then the completed tail
//! selected so the next typed key replaces it.

use crate::textedit::{EditKey, TextEdit};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inline {
    /// Off after Backspace, Delete or a key that moves off a tail, until the next typed char, so
    /// deleting the tail does not bring it straight back.
    completing: bool,
    /// The byte index in the edit's text where a shown completion starts, `None` when nothing is
    /// completed. The only source of truth for "a completion is shown", never inferred from the
    /// edit's selection.
    tail: Option<usize>,
}

impl Default for Inline {
    fn default() -> Self {
        Inline {
            completing: true,
            tail: None,
        }
    }
}

impl Inline {
    /// `edit`'s text without a shown completion.
    pub fn query<'a>(&self, edit: &'a TextEdit) -> &'a str {
        match self.tail {
            Some(i) => &edit.text[..i],
            None => &edit.text,
        }
    }

    pub fn tail(&self) -> Option<usize> {
        self.tail
    }

    /// Whether the next offer may show: a char was typed since the last Backspace or Delete.
    pub fn completing(&self) -> bool {
        self.completing
    }

    /// `ek` into `edit`. A key that moves (arrows, Home, End, Select All, Copy) keeps the tail as
    /// typed text; any other drops the tail before it lands, so the undo log holds typing alone
    /// and Backspace, Delete, Cut or Undo over a tail take just the tail.
    pub fn key(&mut self, edit: &mut TextEdit, ek: EditKey) {
        let tail = self.tail.take();
        let is_insert = matches!(ek, EditKey::Insert(_));
        let eats = matches!(
            ek,
            EditKey::Backspace { .. } | EditKey::Delete { .. } | EditKey::Cut
        );
        if let Some(i) = tail {
            let moves = matches!(
                ek,
                EditKey::Left { .. }
                    | EditKey::Right { .. }
                    | EditKey::Home { .. }
                    | EditKey::End { .. }
                    | EditKey::SelectAll
                    | EditKey::Copy
            );
            if moves {
                edit.key(ek);
                truncate(edit, i);
            } else {
                truncate(edit, i);
                if !eats && !matches!(ek, EditKey::Undo) {
                    edit.key(ek);
                }
            }
        } else {
            edit.key(ek);
        }
        self.completing = if is_insert {
            true
        } else {
            self.completing && !eats && tail.is_none()
        };
    }

    /// A paste replaces a shown completion as a typed insert would.
    pub fn paste(&mut self, edit: &mut TextEdit, text: &str) {
        self.drop_tail(edit);
        edit.paste(text, &[]);
        self.completing = true;
    }

    /// The tail goes and the field holds the query again.
    pub fn drop_tail(&mut self, edit: &mut TextEdit) {
        if let Some(i) = self.tail.take() {
            truncate(edit, i);
        }
    }

    /// The tail stays as typed text, the caret after it.
    pub fn accept(&mut self, edit: &mut TextEdit) {
        if self.tail.take().is_some() {
            let end = edit.text.len();
            edit.set_selection(end, end);
        }
    }

    /// The edit's text was set from outside: nothing is shown and the next offer may show.
    pub fn reset(&mut self) {
        self.tail = None;
        self.completing = true;
    }

    /// Shows `query` (what the field held without its tail) followed by `rest`, selected.
    pub fn offer(&mut self, edit: &mut TextEdit, query: &str, rest: &str) {
        edit.text = format!("{query}{rest}");
        let end = edit.text.len();
        edit.set_selection(query.len(), end);
        self.tail = Some(query.len());
    }
}

fn truncate(edit: &mut TextEdit, i: usize) {
    edit.text.truncate(i);
    let (anchor, caret) = (edit.anchor(), edit.caret());
    edit.set_selection(anchor, caret);
}
