//! A plain combo a host fills with names and acts on: building and syncing it, the presses and
//! keys on its field and open list, and its list laid out above the field.

use super::{Combo, Hit, Layout, widen_list};
use crate::controls::form::Key;
use crate::raster::Rect;
use crate::selection::type_ahead;
use crate::text::TextRenderer;

/// What a press or key on a combo asks of its host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    None,
    Redraw,
    /// Row `.0` is the new pick.
    Picked(usize),
}

impl Combo {
    /// A plain combo over `items`, `picked` (`None`: the first) selected, its list `rows` tall at
    /// most; greyed when there are no items.
    pub fn names(items: &[String], picked: Option<usize>, rows: usize) -> Combo {
        let names: Vec<&str> = items.iter().map(String::as_str).collect();
        let mut c = Combo::new(&names, picked.unwrap_or(0), items.len().min(rows));
        c.enabled = !items.is_empty();
        c
    }

    /// Take `items` and `picked` from the host. The same rows keep the open list and its scroll;
    /// other rows rebuild the combo closed, and the result is true.
    pub fn sync_names(&mut self, items: &[String], picked: Option<usize>, rows: usize) -> bool {
        let same = self.items.len() == items.len()
            && self.items.iter().zip(items).all(|(a, b)| a.text == *b);
        if !same {
            *self = Combo::names(items, picked, rows);
            return true;
        }
        self.selected = picked.unwrap_or(0);
        self.enabled = !items.is_empty();
        false
    }

    /// A press on the open list: a row is picked (`Picked` when the pick changed), an arrow or the
    /// track scrolls, the field or drop button closes it.
    pub fn press_list(&mut self, h: Hit) -> Act {
        let page = self.rows as i32;
        let rows = match h {
            Hit::Row(i) => {
                return if self.pick(i) {
                    Act::Picked(i)
                } else {
                    Act::Redraw
                };
            }
            Hit::Field | Hit::Drop => {
                self.open = None;
                return Act::Redraw;
            }
            Hit::Thumb(_) => return Act::None,
            Hit::ScrollUp => -1,
            Hit::ScrollDown => 1,
            Hit::PageUp => -page,
            Hit::PageDown => page,
        };
        if self.scroll(rows) {
            Act::Redraw
        } else {
            Act::None
        }
    }

    /// A key on the focused combo, its list open or closed; `None` leaves it to the host. An
    /// editable combo leaves characters, Left / Right and Home / End to its field.
    pub fn key(&mut self, key: Key) -> Act {
        let edit = self.edit.is_some();
        if self.open.is_some() {
            let page = self.rows as i32;
            match key {
                Key::Up | Key::Down => self.step(key == Key::Down),
                Key::Home | Key::End if !edit => self.jump(key == Key::End),
                Key::PageUp => self.scroll(-page),
                Key::PageDown => self.scroll(page),
                Key::Enter if !edit && self.take() => return Act::Picked(self.selected),
                Key::Enter | Key::Escape | Key::Tab { .. } | Key::Drop => {
                    self.open = None;
                    false
                }
                Key::Char(ch) if !edit => {
                    if let Some(i) = type_ahead(self.live_names(), self.open, ch) {
                        self.open = Some(i);
                        self.keep_in_view(i);
                    }
                    false
                }
                _ => return Act::None,
            };
            return Act::Redraw;
        }
        let changed = match key {
            Key::Drop => {
                return if self.toggle() {
                    Act::Redraw
                } else {
                    Act::None
                };
            }
            Key::Up => self.step(false),
            Key::Down => self.step(true),
            _ if edit => return Act::None,
            Key::Left => self.step(false),
            Key::Right => self.step(true),
            Key::Home | Key::PageUp => self.jump(false),
            Key::End | Key::PageDown => self.jump(true),
            Key::Char(ch) if ch.is_alphanumeric() && self.enabled => {
                let Some(i) = type_ahead(self.live_names(), Some(self.selected), ch) else {
                    return Act::None;
                };
                if !self.pick(i) {
                    return Act::Redraw;
                }
                true
            }
            _ => false,
        };
        if changed {
            Act::Picked(self.selected)
        } else {
            Act::None
        }
    }

    /// The rows' names for type-ahead, a greyed row's empty so no letter lands on it.
    fn live_names(&self) -> impl Iterator<Item = &str> {
        self.items
            .iter()
            .map(|it| if it.greyed { "" } else { it.text.as_str() })
    }
}

/// Move `lay`'s list from under its field to above it, widened to its longest row and kept inside
/// `inner`.
pub fn list_above(combo: &Combo, lay: &mut Layout, inner: Rect, text: &TextRenderer, px: u32) {
    widen_list(combo, lay, text, px, inner.right() + inner.width);
    let list = &mut lay.list;
    list.width = list.width.min(inner.width);
    list.x = list.x.min(inner.right() - list.width);
    list.y = lay.field.y - list.height;
    if let Some(sb) = &mut lay.scrollbar {
        sb.x = list.right() - 1 - sb.width;
        sb.y = list.y + 1;
    }
}
