//! The pointer and keys of a check list that sits beside a form: its host hands the list what
//! lands on it and the form the rest.

use super::{Hit, Layout, ListView, Rows};
use crate::controls::form;
use crate::controls::scrollbar::ThumbDrag;
use crate::pointer::Axis;
use crate::raster::Rect;
use crate::selection::Mods;

/// A pointer event as the list reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Press(i32, i32),
    Motion(i32, i32),
    Release(i32, i32),
    /// The wheel at (`x`, `y`), turned `rows` rows (positive is down).
    Scroll(i32, i32, i32),
}

/// What the list did with an event it took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    /// The selection, a held box or the scroll moved.
    Moved,
    /// Row `i`'s check flipped.
    Toggled(usize),
    /// Nothing changed.
    Same,
}

/// How far one arrow press scrolls a list sideways, in every look.
pub const NUDGE: i32 = 16;

fn moved(changed: bool) -> Took {
    if changed { Took::Moved } else { Took::Same }
}

impl<R: Rows> ListView<R> {
    /// A pointer event on the list laid out as `lay` in `rect`, if it takes it: a press inside
    /// selects a row, holds its box (when `checks` may flip), steps or pages a bar or grabs its
    /// thumb; a held box or thumb follows the motion until the release; the wheel inside scrolls.
    pub fn pointer(&mut self, ev: Input, lay: &Layout, rect: Rect, checks: bool) -> Option<Took> {
        match ev {
            Input::Press(x, y) if rect.contains(x, y) => {
                let hit = self.hit(lay, x, y);
                if checks {
                    self.press_check(hit);
                }
                self.thumb = ThumbDrag::default();
                match hit {
                    Hit::Check(i) | Hit::Row(i) => self.select(Some(i), lay),
                    Hit::VBar(b) => {
                        let by = self.thumb.press(Axis::Vertical, b);
                        self.scroll(by.by(1, lay.rows as i64), lay);
                    }
                    Hit::HBar(b) => {
                        let by = self.thumb.press(Axis::Horizontal, b);
                        self.hscroll(by.by(NUDGE.into(), lay.body.width.into()) as i32, lay);
                    }
                    _ => {}
                }
                Some(Took::Moved)
            }
            Input::Motion(x, y) if self.held_check.holding() => {
                let hit = self.hit(lay, x, y);
                Some(moved(self.motion_check(hit)))
            }
            Input::Motion(x, y) => {
                if let Some(grab) = self.thumb.grab(Axis::Horizontal) {
                    let left = self.left_at(lay, x, grab);
                    let changed = left != self.left_in(lay);
                    self.left = left;
                    return Some(moved(changed));
                }
                let grab = self.thumb.grab(Axis::Vertical)?;
                let top = self.top_at(lay, y, grab);
                let changed = top != self.top;
                self.top = top;
                Some(moved(changed))
            }
            Input::Release(x, y) if self.held_check.holding() => {
                let hit = self.hit(lay, x, y);
                Some(match self.release_check(hit) {
                    Some(i) => Took::Toggled(i),
                    None => Took::Moved,
                })
            }
            Input::Release(..) if self.thumb.release() => Some(Took::Same),
            Input::Scroll(x, y, rows) if rect.contains(x, y) => {
                Some(moved(self.scroll(rows as i64, lay)))
            }
            _ => None,
        }
    }

    /// A key on the focused list laid out as `lay`, if it takes it: the arrows, Home / End and the
    /// Page keys move the selection, and Space flips the focused row's check when `checks` may.
    pub fn key(&mut self, key: &form::Key, lay: &Layout, checks: bool) -> Option<Took> {
        let flat = match key {
            form::Key::Edit(e) => form::flat_key(e)?,
            other => other.clone(),
        };
        let n = self.rows.len() as i64;
        let page = lay.rows as i64;
        let mods = Mods::default();
        let changed = match flat {
            form::Key::Up => self.step(-1, lay, mods),
            form::Key::Down => self.step(1, lay, mods),
            form::Key::Home => self.step(-n, lay, mods),
            form::Key::End => self.step(n, lay, mods),
            form::Key::PageUp => self.step(-page, lay, mods),
            form::Key::PageDown => self.step(page, lay, mods),
            form::Key::Space if checks => {
                let i = self.selection.focus?;
                return Some(if self.toggle(i) {
                    Took::Toggled(i)
                } else {
                    Took::Same
                });
            }
            _ => return None,
        };
        Some(moved(changed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::widgets::scrollbar_parts;
    use crate::controls::listview::{Column, Row, layout};
    use crate::controls::metrics::ControlMetrics;

    const R: Rect = Rect::new(0, 0, 200, 100);

    fn checked(n: usize) -> ListView {
        ListView {
            columns: vec![Column::new("Name", 120), Column::new("Size", 60)],
            rows: (0..n)
                .map(|i| Row {
                    icon: None,
                    cells: vec![format!("file{i}.txt"), format!("{i} KB")],
                })
                .collect(),
            checks: Some(vec![true; n]),
            ..Default::default()
        }
    }

    #[test]
    fn the_thumb_drags_the_rows_until_the_release_and_the_track_pages() {
        let mut lv = checked(40);
        let lay = layout(&lv, R, &ControlMetrics::CLASSIC);
        let sb = lay.scrollbar.expect("a vertical bar");
        let p = scrollbar_parts(sb, 1, lv.top, lay.rows, lv.slots(), false, lay.thumb_min);
        let thumb = p.thumb.expect("a thumb");
        let (x, y) = (thumb.x + 2, thumb.y + 3);
        assert_eq!(
            lv.pointer(Input::Press(x, y), &lay, R, true),
            Some(Took::Moved)
        );
        assert_eq!(
            lv.pointer(Input::Motion(x + 40, sb.bottom()), &lay, R, true),
            Some(Took::Moved),
            "off the bar's side it still steers"
        );
        let end = lv.max_top(&lay);
        assert_eq!(lv.top, end);
        assert_eq!(
            lv.pointer(Input::Release(x, sb.bottom()), &lay, R, true),
            Some(Took::Same)
        );
        assert_eq!(
            lv.pointer(Input::Motion(x, sb.y), &lay, R, true),
            None,
            "let go"
        );
        assert_eq!(lv.top, end);
        let back = sb.y + ControlMetrics::CLASSIC.scroll + 2;
        lv.pointer(Input::Press(x, back), &lay, R, true);
        assert_eq!(lv.top, end - lay.rows, "the track pages back");
        assert_eq!(
            lv.pointer(Input::Motion(x, sb.bottom()), &lay, R, true),
            None,
            "a page press holds no thumb"
        );
    }

    #[test]
    fn the_horizontal_arrows_nudge_the_columns_and_its_thumb_drags_them() {
        let mut lv = checked(3);
        lv.columns[0].width = 400;
        let lay = layout(&lv, R, &ControlMetrics::CLASSIC);
        let hb = lay.hscroll.expect("the columns overflow the body");
        let forward = (hb.right() - 3, hb.y + 3);
        lv.pointer(Input::Press(forward.0, forward.1), &lay, R, true);
        assert_eq!(lv.left, NUDGE);
        lv.pointer(Input::Release(forward.0, forward.1), &lay, R, true);
        let (visible, total) = (lay.body.width as usize, lay.total_w as usize);
        let p = scrollbar_parts(hb, 1, NUDGE as usize, visible, total, true, lay.thumb_min);
        let thumb = p.thumb.expect("a thumb");
        let (x, y) = (thumb.x + 2, hb.y + 3);
        lv.pointer(Input::Press(x, y), &lay, R, true);
        assert_eq!(
            lv.pointer(Input::Motion(hb.right(), y - 30), &lay, R, true),
            Some(Took::Moved),
            "off the bar's side it still steers"
        );
        assert_eq!(lv.left, lay.total_w - lay.body.width);
        assert_eq!(lv.top, 0, "a horizontal drag leaves the rows");
        assert_eq!(
            lv.pointer(Input::Release(x, y), &lay, R, true),
            Some(Took::Same)
        );
        let back = hb.x + ControlMetrics::CLASSIC.scroll + 2;
        lv.pointer(Input::Press(back, y), &lay, R, true);
        assert_eq!(
            lv.left,
            lay.total_w - 2 * lay.body.width,
            "the track pages back"
        );
    }

    #[test]
    fn a_box_flips_on_a_release_over_it_and_space_flips_the_focused_row() {
        let mut lv = checked(3);
        let lay = layout(&lv, R, &ControlMetrics::CLASSIC);
        let b = lv.check_rect(&lay, 1);
        let (x, y) = (b.x + 2, b.y + 2);
        assert_eq!(
            lv.pointer(Input::Press(x, y), &lay, R, true),
            Some(Took::Moved)
        );
        assert_eq!(lv.selection.focus, Some(1));
        assert_eq!(
            lv.pointer(Input::Release(x, y), &lay, R, true),
            Some(Took::Toggled(1))
        );
        assert_eq!(lv.checks.as_deref(), Some(&[true, false, true][..]));
        assert_eq!(
            lv.key(&form::Key::Space, &lay, true),
            Some(Took::Toggled(1))
        );
        assert_eq!(lv.key(&form::Key::Space, &lay, false), None);
        assert_eq!(lv.key(&form::Key::Down, &lay, false), Some(Took::Moved));
        lv.pointer(Input::Press(x, y), &lay, R, false);
        assert_eq!(
            lv.pointer(Input::Release(x, y), &lay, R, false),
            None,
            "no box is held while checks may not flip"
        );
        assert_eq!(
            lv.pointer(Input::Press(R.right() + 5, 5), &lay, R, true),
            None
        );
    }
}
