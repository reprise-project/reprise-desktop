//! A plain combo a host fills with names and acts on: building and syncing it, the presses and
//! keys on its field and open list, and its list laid out above the field.

use super::{Combo, Hit, Layout, widen_list};
use crate::controls::form::Key;
use crate::raster::Rect;
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

    /// A key with the list open: arrows, Home / End and the Page keys move the highlight, Enter
    /// takes it, Esc, Tab or the drop key close the list.
    pub fn list_key(&mut self, key: Key) -> Act {
        let page = self.rows as i32;
        let _ = match key {
            Key::Up | Key::Down => self.step(key == Key::Down),
            Key::Home | Key::End => self.jump(key == Key::End),
            Key::PageUp => self.scroll(-page),
            Key::PageDown => self.scroll(page),
            Key::Enter if self.take() => return Act::Picked(self.selected),
            Key::Enter | Key::Escape | Key::Tab { .. } | Key::Drop => {
                self.open = None;
                false
            }
            _ => return Act::None,
        };
        Act::Redraw
    }

    /// A key on the focused combo with its list closed: arrows, Home / End and the Page keys pick at
    /// once; F4 or Alt+Down opens the list.
    pub fn closed_key(&mut self, key: Key) -> Act {
        let changed = match key {
            Key::Up | Key::Left => self.step(false),
            Key::Down | Key::Right => self.step(true),
            Key::Home | Key::PageUp => self.jump(false),
            Key::End | Key::PageDown => self.jump(true),
            Key::Drop => {
                return if self.toggle() {
                    Act::Redraw
                } else {
                    Act::None
                };
            }
            _ => false,
        };
        if changed {
            Act::Picked(self.selected)
        } else {
            Act::None
        }
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
