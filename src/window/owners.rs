use crate::compositor::State;

use super::{Window, WindowId};

/// Visit windows below `root` owner-first, siblings in `children`'s order; iterative with a visited
/// set so parent cycles (X11 transients) are safe.
pub(super) fn walk_owned<T: Copy + Eq + std::hash::Hash, S: Copy>(
    root: T,
    state: S,
    mut children: impl FnMut(T) -> Vec<T>,
    mut visit: impl FnMut(T, S) -> S,
) {
    let mut seen = std::collections::HashSet::from([root]);
    let mut stack: Vec<(T, S)> = children(root)
        .into_iter()
        .rev()
        .map(|k| (k, state))
        .collect();
    while let Some((w, owner_state)) = stack.pop() {
        if !seen.insert(w) {
            continue;
        }
        let s = visit(w, owner_state);
        stack.extend(children(w).into_iter().rev().map(|k| (k, s)));
    }
}

/// Follow `modal` from `root` to the end of the chain: the window that takes focus in `root`'s
/// place. Stops at a revisited window.
pub(super) fn modal_end<T: Copy + Eq + std::hash::Hash>(
    root: T,
    mut modal: impl FnMut(T) -> Option<T>,
) -> T {
    let mut seen = std::collections::HashSet::from([root]);
    let mut w = root;
    while let Some(k) = modal(w) {
        if !seen.insert(k) {
            break;
        }
        w = k;
    }
    w
}

pub(super) fn owner_chain_has<T: Copy + Eq + std::hash::Hash>(
    w: T,
    owner: T,
    mut parent: impl FnMut(T) -> Option<T>,
) -> bool {
    let mut seen = std::collections::HashSet::from([w]);
    let mut at = w;
    while let Some(p) = parent(at) {
        if p == owner {
            return true;
        }
        if !seen.insert(p) {
            return false;
        }
        at = p;
    }
    false
}

impl State {
    pub fn parent_window(&self, w: WindowId) -> Option<WindowId> {
        let p = self.win(w)?.parent_handle()?;
        self.windows.iter().find(|x| x.handle() == p).map(|x| x.id)
    }

    pub fn is_owner_of(&self, owner: WindowId, w: WindowId) -> bool {
        owner_chain_has(w, owner, |x| self.parent_window(x))
    }

    pub fn stays_on_top(&self, w: WindowId) -> bool {
        let mut at = w;
        for _ in 0..=self.windows.len() {
            if self.win(at).is_some_and(|x| x.on_top) {
                return true;
            }
            match self.parent_window(at) {
                Some(p) => at = p,
                None => return false,
            }
        }
        false
    }

    pub fn set_on_top(&mut self, w: WindowId, on: bool) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if on == win.on_top {
            return;
        }
        win.on_top = on;
        let tree = win.tree;
        self.restack();
        self.scene.raise_to_top(tree);
        self.raise_children(w);
        self.redraw_all();
    }

    fn owned_by(&self, w: WindowId) -> Vec<WindowId> {
        let Some(own) = self.win(w).map(Window::handle) else {
            return Vec::new();
        };
        self.windows
            .iter()
            .filter(|x| x.mapped && x.parent_handle().as_ref() == Some(&own))
            .map(|x| x.id)
            .collect()
    }

    pub fn modal_target(&self, w: WindowId) -> WindowId {
        modal_end(w, |x| {
            self.owned_by(x)
                .into_iter()
                .rev()
                .find(|&k| self.win(k).is_some_and(Window::is_modal))
        })
    }

    pub fn on_parent_changed(&mut self, w: WindowId) {
        if !self.win(w).is_some_and(|x| x.mapped) {
            return;
        }
        let Some((cx, cy)) = self.win(w).map(Window::client_origin) else {
            return;
        };
        self.relayout(w);
        if let Some(c) = self.win(w).map(|x| x.geom.client) {
            self.move_to(w, cx - c.x, cy - c.y);
        }
    }

    /// Raise windows owned by `w` above it, recursively.
    pub fn raise_children(&mut self, w: WindowId) {
        let mut order = Vec::new();
        walk_owned(w, (), |x| self.owned_by(x), |k, ()| order.push(k));
        for k in order {
            if let Some(tree) = self.win(k).map(|x| x.tree) {
                self.scene.raise_to_top(tree);
            }
        }
    }

    pub(crate) fn show_children(&mut self, w: WindowId, on: bool) {
        let mut shown = Vec::new();
        walk_owned(
            w,
            on,
            |x| self.owned_by(x),
            |k, owner_shown| {
                let show = owner_shown && self.win(k).is_some_and(|k| !k.hidden());
                shown.push((k, show));
                show
            },
        );
        for (k, show) in shown {
            if let Some(tree) = self.win(k).map(|x| x.tree) {
                self.scene.set_enabled(tree, show);
            }
        }
    }
}
