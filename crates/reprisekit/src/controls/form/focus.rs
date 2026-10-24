use crate::controls::updown::UpDown;

use super::{Control, Event, Form};

impl Form {
    pub fn default_button(&self) -> Option<usize> {
        self.items.iter().position(|i| {
            matches!(
                &i.control,
                Control::Button {
                    default: true,
                    enabled: true,
                    ..
                }
            )
        })
    }

    pub fn focus(&self) -> Option<usize> {
        self.focus
    }

    /// Move the focus to item `to`, or off the form with `None`, as the keyboard does: a spinner
    /// being left shows its clamped value, one arriving selects its number. `click` goes through here.
    pub fn set_focus(&mut self, to: Option<usize>) {
        if to == self.focus {
            return;
        }
        if let Some(u) = self.updown_at(self.focus) {
            u.leave();
        }
        if let Some(Control::HotKey(h)) = self
            .focus
            .and_then(|i| self.items.get_mut(i))
            .map(|i| &mut i.control)
        {
            h.blur();
        }
        self.focus = to;
        if let Some(u) = self.updown_at(to) {
            u.enter();
        }
    }

    fn updown_at(&mut self, i: Option<usize>) -> Option<&mut UpDown> {
        match &mut self.items.get_mut(i?)?.control {
            Control::UpDown(u) => Some(u),
            _ => None,
        }
    }

    pub fn focus_id(&mut self, id: &str) {
        let to = self.index_of(id).filter(|&i| self.items[i].focusable());
        self.set_focus(to);
    }

    pub(super) fn open_combo(&self) -> Option<usize> {
        self.items.iter().position(|i| {
            matches!(&i.control, Control::Combo(c) if c.open.is_some())
                || matches!(&i.control, Control::Color(b) if b.open)
        })
    }

    pub fn popup_open(&self) -> bool {
        self.open_combo().is_some()
    }

    pub(super) fn close_popups(&mut self) -> bool {
        let mut closed = false;
        for i in &mut self.items {
            match &mut i.control {
                Control::Combo(c) if c.open.is_some() => {
                    c.open = None;
                    closed = true;
                }
                Control::Color(b) if b.open => {
                    b.open = false;
                    closed = true;
                }
                _ => {}
            }
        }
        closed
    }

    pub(super) fn move_focus(&mut self, back: bool) -> Event {
        let n = self.items.len();
        if n == 0 {
            return Event::None;
        }
        let start = self.focus.unwrap_or(if back { 0 } else { n - 1 });
        for k in 1..=n {
            let i = if back {
                (start + n - k) % n
            } else {
                (start + k) % n
            };
            if self.tab_stop(i) {
                self.set_focus(Some(i));
                return Event::Redraw;
            }
        }
        Event::None
    }

    /// Tab without `move_focus`'s wrap: `false` when the focus is already at that end, where a
    /// property sheet takes it onto its own OK / Cancel / Apply.
    pub fn tab_no_wrap(&mut self, back: bool) -> bool {
        let Some(start) = self.focus else {
            return false;
        };
        let next = if back {
            (0..start).rev().find(|&i| self.tab_stop(i))
        } else {
            (start + 1..self.items.len()).find(|&i| self.tab_stop(i))
        };
        match next {
            Some(i) => {
                self.set_focus(Some(i));
                true
            }
            None => false,
        }
    }

    /// Focus the form's first or last tab stop, for a host handing focus back. `false` if there is none.
    pub fn focus_edge(&mut self, last: bool) -> bool {
        let n = self.items.len();
        let found = if last {
            (0..n).rev().find(|&i| self.tab_stop(i))
        } else {
            (0..n).find(|&i| self.tab_stop(i))
        };
        self.set_focus(found);
        found.is_some()
    }

    pub(super) fn focus_after(&mut self, i: usize) -> Event {
        for j in i + 1..self.items.len() {
            if self.tab_stop(j) {
                self.set_focus(Some(j));
                return Event::Redraw;
            }
        }
        Event::None
    }

    pub(super) fn changed(&self, i: usize, changed: bool) -> Event {
        if changed {
            Event::Changed(self.items[i].id)
        } else {
            Event::Redraw
        }
    }

    pub(super) fn pick_radio(&mut self, i: usize) -> bool {
        let Control::Radio { group, .. } = &self.items[i].control else {
            return false;
        };
        let group = *group;
        let mut changed = false;
        for (k, it) in self.items.iter_mut().enumerate() {
            if let Control::Radio {
                group: g, checked, ..
            } = &mut it.control
            {
                if *g != group {
                    continue;
                }
                let want = k == i;
                if *checked != want {
                    *checked = want;
                    changed = true;
                }
            }
        }
        changed
    }

    /// Whether Tab stops here. A radio group is one stop: the checked
    /// radio, or the first enabled one when none is checked.
    pub(super) fn tab_stop(&self, i: usize) -> bool {
        if !self.items[i].focusable() {
            return false;
        }
        let Control::Radio { group, .. } = &self.items[i].control else {
            return true;
        };
        let mut first = None;
        for (k, it) in self.items.iter().enumerate() {
            if let Control::Radio {
                group: g,
                checked,
                enabled,
                ..
            } = &it.control
            {
                if g != group || !*enabled {
                    continue;
                }
                if *checked {
                    return k == i;
                }
                first.get_or_insert(k);
            }
        }
        first == Some(i)
    }

    /// The next enabled radio in `from`'s group, in item order, without wrapping and excluding
    /// `from` itself; `None` at the group's ends.
    pub(super) fn next_radio(&self, from: usize, back: bool) -> Option<usize> {
        let Control::Radio { group, .. } = &self.items[from].control else {
            return None;
        };
        let same = |i: &usize| {
            matches!(
                &self.items[*i].control,
                Control::Radio {
                    group: g,
                    enabled: true,
                    ..
                } if g == group
            )
        };
        if back {
            (0..from).rev().find(same)
        } else {
            (from + 1..self.items.len()).find(same)
        }
    }

    /// An arrow on the push button at `from`: focus goes to the previous or next enabled push
    /// button, stopping at the row's ends.
    pub(super) fn next_button(&mut self, from: usize, back: bool) -> Event {
        let button = |i: &usize| {
            matches!(
                self.items[*i].control,
                Control::Button { enabled: true, .. }
            )
        };
        let to = if back {
            (0..from).rev().find(button)
        } else {
            (from + 1..self.items.len()).find(button)
        };
        match to {
            Some(j) => {
                self.set_focus(Some(j));
                Event::Redraw
            }
            None => Event::None,
        }
    }
}
