//! Virtual desktops. Every output shows the same one. A window is on one desktop unless sticky, and a dialog goes with its owner. Windows of other desktops keep their place and state; only their scene trees are disabled.

use smithay::backend::input::InputTime;

use crate::compositor::State;
use crate::window::WindowId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Prev,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desktops {
    pub count: usize,
    pub current: usize,
}

impl Desktops {
    pub fn new(count: usize) -> Desktops {
        Desktops {
            count: count.max(1),
            current: 0,
        }
    }

    /// More than one desktop: the pager, the window menu's desktop rows and the ext-workspace global exist.
    pub fn enabled(&self) -> bool {
        self.count > 1
    }

    /// `count` desktops (at least one), keeping the current one or the last when it is gone.
    pub fn resized(self, count: usize) -> Desktops {
        let count = count.max(1);
        Desktops {
            count,
            current: self.current.min(count - 1),
        }
    }

    /// The desktop one step from the current one; None past either end, which do not wrap.
    pub fn step(&self, s: Step) -> Option<usize> {
        match s {
            Step::Prev => self.current.checked_sub(1),
            Step::Next => Some(self.current + 1).filter(|&n| n < self.count),
        }
    }
}

/// Which of `count` desktops hold a window asking for attention, other than `current`, whose own taskbar shows it. `windows` gives each window's desktop (None: all) and attention flag.
pub fn attention(
    count: usize,
    current: usize,
    windows: impl IntoIterator<Item = (Option<usize>, bool)>,
) -> Vec<bool> {
    let mut v = vec![false; count];
    for (d, asks) in windows {
        if let Some(d) = d
            && asks
            && d != current
            && let Some(slot) = v.get_mut(d)
        {
            *slot = true;
        }
    }
    v
}

impl State {
    /// The top of `w`'s owner chain (`w` itself without an owner): the window whose desktop a dialog follows.
    pub fn home(&self, w: WindowId) -> WindowId {
        let mut at = w;
        // A parent cycle ends after one lap of the window list.
        for _ in 0..=self.windows.len() {
            match self.parent_window(at) {
                Some(p) => at = p,
                None => break,
            }
        }
        at
    }

    pub fn desktop_of(&self, w: WindowId) -> Option<usize> {
        let home = self.win(self.home(w))?;
        (!home.sticky).then_some(home.desktop)
    }

    pub fn on_current_desktop(&self, w: WindowId) -> bool {
        self.desktop_of(w)
            .is_none_or(|d| d == self.desktops.current)
    }

    /// Whether `w` or one up its owner chain is minimized, or a tab its group hides: an owned window hides with its owner.
    pub fn minimized_along_chain(&self, w: WindowId) -> bool {
        let mut at = w;
        for _ in 0..=self.windows.len() {
            if self.win(at).is_some_and(|x| x.hidden()) {
                return true;
            }
            match self.parent_window(at) {
                Some(p) => at = p,
                None => return false,
            }
        }
        false
    }

    /// Bring every window in line with the current desktop: a snapped window moved off its group's desktop leaves the group and a group on every desktop moves to the one shown, a mapped window's tree is enabled when it is on it and nothing up its owner chain is minimized. A window leaving the screen loses any move, held task button or window menu, and the focused one loses focus (the caller hands it on); one arriving joins an open 3D switcher.
    pub fn desktops_changed(&mut self) {
        self.wm.snap.desktop = self.desktops.current;
        let desktops: Vec<(WindowId, Option<usize>)> = (self.windows.iter())
            .map(|w| (w.id, self.desktop_of(w.id)))
            .collect();
        (self.wm.snap).settle_desktops(|w| desktops.iter().find(|d| d.0 == w)?.1);
        let all: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.mapped)
            .map(|w| w.id)
            .collect();
        for w in all {
            let here = self.on_current_desktop(w);
            if !here {
                self.cancel_interaction_for(w);
                self.end_task_press_for(w);
                self.close_window_menu_for(w);
                crate::taskpreview::forget_window(self, w);
                crate::snappickerview::forget_window(self, w);
                self.switcher_forget(crate::switcherview::Entry::Window(w));
            }
            let show = here && !self.minimized_along_chain(w);
            if let Some(tree) = self.win(w).map(|x| x.tree) {
                self.scene.set_enabled(tree, show);
            }
        }
        if let Some(f) = self.focused
            && !self.on_current_desktop(f)
        {
            self.drop_window_focus();
        }
        crate::idle::inhibition_changed(self);
        self.flip_join();
        self.overview_refresh();
        self.redraw_taskbars();
        self.redraw_all();
    }

    pub fn show_desktop(&mut self, n: usize) -> bool {
        self.show_desktop_as(n, true)
    }

    /// `show_desktop`, starting the switch animation when `animate` (a swipe's slide is already on screen).
    fn show_desktop_as(&mut self, n: usize, animate: bool) -> bool {
        let d = &mut self.desktops;
        if n >= d.count || n == d.current {
            return false;
        }
        log::info!("desktop {} of {}", n + 1, d.count);
        // The switch animation takes its stills before `current` changes.
        if animate {
            self.switch_leaving();
        }
        self.desktops.current = n;
        self.desktops_changed();
        true
    }

    /// Switch to desktop `n`: the top window of its focus order takes focus. Refused under the lock.
    pub fn switch_desktop(&mut self, n: usize) {
        self.switch_desktop_as(n, true);
    }

    pub fn switch_desktop_as(&mut self, n: usize, animate: bool) {
        if self.is_locked() || !self.show_desktop_as(n, animate) {
            return;
        }
        self.focus_top();
        self.repoint(self.pointer_location(), InputTime::now());
    }

    /// Switch one desktop along, taking the focused window's family with it when `carry` is set (a window on all desktops stays put).
    pub fn step_desktop(&mut self, step: Step, carry: bool) {
        let Some(n) = self.desktops.step(step) else {
            return;
        };
        if carry && let Some(f) = self.focused {
            let home = self.home(f);
            if let Some(h) = self.win_mut(home)
                && !h.sticky
            {
                h.desktop = n;
            }
        }
        self.switch_desktop(n);
    }

    /// Put `windows` (with owners and dialogs) on desktop `n`, staying on the current one; one on every desktop stays put. The next window here takes focus if it was theirs.
    pub fn move_to_desktop(&mut self, windows: &[WindowId], n: usize) {
        if n >= self.desktops.count {
            return;
        }
        let mut moved = false;
        for w in self.with_tab_groups(windows) {
            let home = self.home(w);
            if let Some(h) = self.win_mut(home)
                && !h.sticky
                && h.desktop != n
            {
                h.desktop = n;
                moved = true;
            }
        }
        if !moved {
            return;
        }
        self.desktops_changed();
        if self.focused.is_none() {
            self.focus_top();
        }
        self.repoint(self.pointer_location(), InputTime::now());
    }

    pub fn set_all_desktops(&mut self, w: WindowId, on: bool) {
        let current = self.desktops.current;
        if self.win(self.home(w)).is_none_or(|h| h.sticky == on) {
            return;
        }
        for m in self.with_tab_groups(&[w]) {
            let home = self.home(m);
            if let Some(h) = self.win_mut(home) {
                h.sticky = on;
                h.desktop = current;
            }
        }
        self.desktops_changed();
    }

    /// Change to `count` desktops (`[workspaces] count` on reload). Windows on a removed desktop move to the last one left, and so does the current desktop.
    pub fn set_desktop_count(&mut self, count: usize) {
        let d = self.desktops.resized(count);
        if d == self.desktops {
            return;
        }
        log::info!("desktop count {}", d.count);
        for w in &mut self.windows {
            w.desktop = w.desktop.min(d.count - 1);
        }
        self.desktops = d;
        self.workspaces_resized();
        self.desktops_changed();
        if self.focused.is_none() {
            self.focus_top();
        }
        self.repoint(self.pointer_location(), InputTime::now());
    }

    pub fn pager_buttons(&self) -> Vec<reprisekit::taskbar::PagerButton> {
        let d = self.desktops;
        if !d.enabled() {
            return Vec::new();
        }
        let asks = self
            .windows
            .iter()
            .filter(|w| w.mapped && w.attention.is_some())
            .map(|w| (self.desktop_of(w.id), true));
        attention(d.count, d.current, asks)
            .into_iter()
            .enumerate()
            .map(|(i, attention)| reprisekit::taskbar::PagerButton {
                current: i == d.current,
                attention,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resize_keeps_the_current_desktop_or_takes_the_last() {
        let d = Desktops {
            count: 4,
            current: 2,
        };
        assert_eq!(
            d.resized(6),
            Desktops {
                count: 6,
                current: 2
            }
        );
        assert_eq!(
            d.resized(2),
            Desktops {
                count: 2,
                current: 1
            }
        );
        assert_eq!(
            d.resized(0),
            Desktops {
                count: 1,
                current: 0
            }
        );
        assert!(!d.resized(1).enabled());
    }

    #[test]
    fn steps_stop_at_the_first_and_the_last_desktop() {
        let mut d = Desktops::new(4);
        assert_eq!(d.step(Step::Prev), None);
        assert_eq!(d.step(Step::Next), Some(1));
        d.current = 3;
        assert_eq!(d.step(Step::Next), None);
        assert_eq!(d.step(Step::Prev), Some(2));
    }

    #[test]
    fn one_desktop_turns_the_feature_off() {
        let d = Desktops::new(1);
        assert!(!d.enabled());
        assert_eq!(d.step(Step::Next), None);
        assert_eq!(d.step(Step::Prev), None);
        assert!(Desktops::new(2).enabled());
        assert_eq!(Desktops::new(0).count, 1);
    }

    #[test]
    fn attention_marks_other_desktops_only() {
        let flags = attention(
            4,
            1,
            [
                (Some(0), false),
                (Some(1), true), // the current desktop: its taskbar shows it
                (Some(2), true),
                (None, true), // on every desktop
                (Some(9), true),
            ],
        );
        assert_eq!(flags, vec![false, false, true, false]);
    }
}
