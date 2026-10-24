use crate::classic::widgets::{BarHit, LINE_H, scrollbar_parts, scrollbar_pos_at};
use crate::controls::{combo, form};
use crate::pointer::{Axis, WHEEL_ROWS};

use super::layout::Layout;
use super::{Event, Focus, Hit, TAB_ORDER, TrayCustomize, redraw_if};

impl TrayCustomize {
    /// Press part `f`; nothing when it is greyed.
    fn press(&mut self, f: Focus) -> Event {
        if !self.focusable(f) {
            return Event::None;
        }
        match f {
            Focus::List | Focus::Combo => Event::None,
            Focus::AllIcons => {
                self.all_icons = self.all_icons.map(|on| !on);
                self.combo.open = None;
                self.focus = Focus::AllIcons;
                Event::Redraw
            }
            Focus::Restore => {
                self.restore();
                Event::Redraw
            }
            Focus::Ok => {
                self.take_pick(self.combo.selected);
                Event::Accept(self.choices())
            }
            Focus::Cancel => Event::Cancel,
        }
    }

    pub fn key(&mut self, key: form::Key) -> Event {
        let ev = self.key_inner(key);
        match ev {
            Event::None | Event::Redraw if self.reveal_focus() => Event::Redraw,
            ev => ev,
        }
    }

    fn key_inner(&mut self, key: form::Key) -> Event {
        let height = self.body_height();
        use form::Key;
        if self.combo.open.is_some() {
            return match key {
                Key::Up | Key::Down => {
                    self.combo.step(key == Key::Down);
                    Event::Redraw
                }
                Key::Enter | Key::Drop => {
                    self.combo.take();
                    self.take_pick(self.combo.selected);
                    Event::Redraw
                }
                Key::Escape => {
                    self.combo.open = None;
                    Event::Redraw
                }
                _ => Event::None,
            };
        }
        match key {
            Key::Tab { back } => {
                if self.is_page() && self.focus == Focus::Combo {
                    let next = match (self.selected, back) {
                        (Some(i), false) if i + 1 < self.entries.len() => Some(i + 1),
                        (Some(i), true) if i > 0 => Some(i - 1),
                        _ => None,
                    };
                    if let Some(i) = next {
                        self.select(i);
                        return Event::Redraw;
                    }
                }
                let n = TAB_ORDER.len();
                let mut at = TAB_ORDER.iter().position(|&f| f == self.focus).unwrap_or(0);
                loop {
                    at = (if back { at + n - 1 } else { at + 1 }) % n;
                    if self.focusable(TAB_ORDER[at]) {
                        break;
                    }
                }
                self.focus = TAB_ORDER[at];
                if self.focus == Focus::List && self.selected.is_none() {
                    self.select(0);
                }
                if self.is_page() && self.focus == Focus::Combo {
                    self.select(if back { self.entries.len() - 1 } else { 0 });
                }
                Event::Redraw
            }
            Key::Escape => Event::Cancel,
            Key::Enter => match self.focus {
                Focus::Cancel | Focus::Restore => self.press(self.focus),
                _ => self.press(Focus::Ok),
            },
            Key::Space => match self.focus {
                Focus::AllIcons | Focus::Restore | Focus::Ok | Focus::Cancel => {
                    self.press(self.focus)
                }
                _ => Event::None,
            },
            Key::Access(c) => match c.to_ascii_lowercase() {
                'r' => self.press(Focus::Restore),
                'a' => self.press(Focus::AllIcons),
                _ => Event::None,
            },
            Key::Drop if self.focus == Focus::Combo => redraw_if(self.combo.toggle()),
            Key::Up | Key::Down if self.focus == Focus::Combo => {
                let changed = self.combo.step(key == Key::Down);
                self.take_pick(self.combo.selected);
                redraw_if(changed)
            }
            Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown
                if self.focus == Focus::List && !self.entries.is_empty() =>
            {
                let last = self.entries.len() - 1;
                let page = (height / self.geometry().row_h).max(1) as usize;
                let at = self.selected;
                let next = match key {
                    Key::Up => at.map_or(0, |i| i.saturating_sub(1)),
                    Key::Down => at.map_or(0, |i| (i + 1).min(last)),
                    Key::Home => 0,
                    Key::End => last,
                    Key::PageUp => at.map_or(0, |i| i.saturating_sub(page)),
                    _ => at.map_or(0, |i| (i + page).min(last)),
                };
                let moved = Some(next) != at;
                self.select(next);
                redraw_if(moved)
            }
            _ => Event::None,
        }
    }

    /// The wheel turned `rows` rows over `hit`: the list scrolls, and while the page scrolls,
    /// the page does off the list or where the list stops. Nothing while a combo is open. True
    /// when anything moved.
    pub fn wheel(&mut self, hit: Option<Hit>, rows: i32) -> bool {
        if self.combo.open.is_some() {
            return false;
        }
        let on_list = matches!(hit, Some(Hit::Row(_) | Hit::List | Hit::Bar(_)));
        if (on_list || self.page_scroll.short == 0) && self.scroll(rows) {
            return true;
        }
        let to = self.page_scroll.top + rows * WHEEL_ROWS * LINE_H;
        self.scroll_page_to(to)
    }

    /// Scroll the list by `rows` slots.
    pub fn scroll(&mut self, rows: i32) -> bool {
        let slots = self.slots();
        let max = self.max_top(&slots, self.body_height()) as i32;
        let top = (self.top as i32 + rows).clamp(0, max) as usize;
        let moved = top != self.top;
        self.top = top;
        moved
    }

    /// A pointer press on `hit`.
    pub fn click(&mut self, hit: Option<Hit>) -> Event {
        if self.combo.open.is_some() {
            match hit {
                Some(Hit::Combo(combo::Hit::Row(i))) => {
                    self.combo.pick(i);
                    self.take_pick(i);
                }
                Some(Hit::Combo(h)) if h.bar().is_some() => {
                    let bar = h.bar().unwrap_or(BarHit::Track);
                    self.combo.press_bar(bar, &mut self.combo_thumb);
                    return Event::Redraw;
                }
                _ => self.combo.open = None,
            }
            return Event::Redraw;
        }
        let Some(hit) = hit else {
            return Event::None;
        };
        match hit {
            Hit::Row(i) if self.is_page() && self.list_enabled() => {
                self.focus = Focus::Combo;
                self.select(i);
                self.combo.toggle();
                Event::Redraw
            }
            Hit::Row(i) if self.list_enabled() => {
                self.focus = Focus::List;
                if self.selected != Some(i) {
                    self.select(i);
                }
                Event::Redraw
            }
            Hit::Combo(_) if self.list_enabled() => {
                self.focus = Focus::Combo;
                self.combo.toggle();
                Event::Redraw
            }
            Hit::Bar(bar) if self.list_enabled() => {
                let page = self.page() as i64;
                let by = self.thumb.press(Axis::Vertical, bar).by(1, page);
                redraw_if(self.scroll(by as i32) || matches!(bar, BarHit::Thumb(_)))
            }
            Hit::PageBar(bar) => {
                let to = self.page_scroll.press(bar, self.page_geo().view_h());
                self.scroll_page_to(to);
                Event::Redraw
            }
            Hit::AllIcons => self.arm(Focus::AllIcons),
            Hit::Restore => self.arm(Focus::Restore),
            Hit::Ok => self.arm(Focus::Ok),
            Hit::Cancel => self.arm(Focus::Cancel),
            _ => Event::None,
        }
    }

    /// A press on push button or box `f`: a live one draws pressed until
    /// the release.
    fn arm(&mut self, f: Focus) -> Event {
        if !self.focusable(f) {
            return Event::None;
        }
        if f != Focus::Ok {
            self.focus = f;
        }
        self.pointer.press(Some(f), false);
        Event::Redraw
    }

    /// Whether the pointer holds a push button, the box, a thumb or the page's arrow.
    pub fn holding(&self) -> bool {
        self.pointer.holding()
            || self.thumb.holding()
            || self.combo_thumb.holding()
            || self.page_scroll.holding()
            || self.page_scroll.ticking()
    }

    /// Whether the page's arrow is held: the host sends [`TrayCustomize::tick`] until the
    /// release.
    pub fn ticking(&self) -> bool {
        self.page_scroll.ticking()
    }

    /// A host tick at `now`: a held page arrow steps once it repeats.
    pub fn tick(&mut self, now: std::time::Instant) -> Event {
        let to = self.page_scroll.tick(now);
        redraw_if(to.is_some_and(|to| self.scroll_page_to(to)))
    }

    /// The pointer at `y` with a thumb held: its list or the page follows. `None`
    /// with none held.
    pub fn drag(&mut self, lay: &Layout, y: i32) -> Option<Event> {
        if let (Some(bar), true) = (lay.page_bar, self.page_scroll.holding()) {
            let g = self.page_geo();
            let (visible, total) = (g.view_h(), g.content_h());
            let parts = self.page_scroll.parts(bar, visible, total, lay.thumb_min);
            let to = self.page_scroll.drag(&parts, (bar.x, y), visible, total)?;
            return Some(redraw_if(self.scroll_page_to(to)));
        }
        if let (Some((_, cl)), true) = (&lay.combo, self.combo_thumb.holding()) {
            return Some(redraw_if(self.combo.drag_bar(cl, &self.combo_thumb, y)));
        }
        let grab = self.thumb.grab(Axis::Vertical)?;
        let sb = lay.vbar?;
        let slots = self.slots();
        let parts = scrollbar_parts(sb, 1, self.top, lay.page, slots.len(), false, lay.thumb_min);
        let top = scrollbar_pos_at(&parts, y - grab, lay.page, slots.len(), false)
            .min(self.max_top(&slots, lay.body.height));
        let moved = top != self.top;
        self.top = top;
        Some(redraw_if(moved))
    }

    fn button(hit: Option<Hit>) -> Option<Focus> {
        match hit {
            Some(Hit::AllIcons) => Some(Focus::AllIcons),
            Some(Hit::Restore) => Some(Focus::Restore),
            Some(Hit::Ok) => Some(Focus::Ok),
            Some(Hit::Cancel) => Some(Focus::Cancel),
            _ => None,
        }
    }

    /// The pointer moved over `hit`: a held push button draws pressed
    /// only while the pointer is on it. With nothing held, the button,
    /// box, link or page combo under it draws hot.
    pub fn motion(&mut self, hit: Option<Hit>) -> Event {
        let held = self.pointer.holding();
        let mut changed = self.pointer.motion(Self::button(hit));
        changed |= self.page_scroll.motion(match hit {
            Some(Hit::PageBar(b)) => Some(b),
            _ => None,
        });
        if let (true, Some(open), Some(Hit::Combo(combo::Hit::Row(i)))) =
            (self.is_page(), self.combo.open, hit)
            && open != i
        {
            self.combo.open = Some(i);
            changed = true;
        }
        if !held {
            let row = match hit {
                Some(Hit::Row(i)) if self.is_page() => Some(i),
                Some(Hit::Combo(combo::Hit::Field | combo::Hit::Drop)) if self.is_page() => {
                    self.selected
                }
                _ => None,
            };
            changed |= std::mem::replace(&mut self.hot_row, row) != row;
        }
        redraw_if(changed)
    }

    /// The pointer left the dialog: nothing stays hot.
    pub fn leave(&mut self) -> Event {
        let changed =
            self.pointer.leave() | self.hot_row.take().is_some() | self.page_scroll.leave();
        redraw_if(changed)
    }

    /// The pointer let go over `hit`: the held push button or box acts if
    /// the release lands on it. A held thumb lets go.
    pub fn release(&mut self, hit: Option<Hit>) -> Event {
        if self.thumb.release() | self.combo_thumb.release() | self.page_scroll.release() {
            return Event::Redraw;
        }
        if !self.pointer.holding() {
            return Event::None;
        }
        self.pointer.motion(Self::button(hit));
        let Some(f) = self.pointer.release() else {
            return Event::Redraw;
        };
        match self.press(f) {
            Event::None => Event::Redraw,
            e => {
                self.refocus();
                e
            }
        }
    }
}
