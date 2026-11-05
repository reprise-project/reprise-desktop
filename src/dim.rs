//! `[windows] dim_inactive`: a black shade over the client area of every window outside the
//! focused window's family (its parent chain and their dialogs). Screen only: pictures of one
//! window leave it out.

use std::time::{Duration, Instant};

use crate::compositor::State;
use crate::window::WindowId;

pub const FADE: Duration = Duration::from_millis(150);

/// One window's shade: its alpha fading from `from` to `to` since `since`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shade {
    from: f32,
    to: f32,
    since: Option<Instant>,
}

impl Shade {
    pub fn alpha(&self, now: Instant, fade: Duration) -> f32 {
        let Some(t) = self.since.filter(|_| !fade.is_zero()) else {
            return self.to;
        };
        let k = (now.saturating_duration_since(t).as_secs_f32() / fade.as_secs_f32()).min(1.0);
        self.from + (self.to - self.from) * k
    }

    pub fn fading(&self, now: Instant, fade: Duration) -> bool {
        self.since
            .is_some_and(|t| !fade.is_zero() && now.saturating_duration_since(t) < fade)
    }

    /// Head for `to` from wherever the shade is now; whether `to` is new.
    pub fn aim(&mut self, to: f32, now: Instant, fade: Duration) -> bool {
        if to == self.to {
            return false;
        }
        self.from = self.alpha(now, fade);
        self.to = to;
        self.since = Some(now);
        true
    }
}

fn root(state: &State, w: WindowId) -> WindowId {
    let mut at = w;
    // A parent loop a client made stops at the window count.
    for _ in 0..state.windows.len() {
        match state.parent_window(at) {
            Some(p) => at = p,
            None => break,
        }
    }
    at
}

/// The alpha `w`'s shade heads for, the focused window's family having root `bright`.
fn target(state: &State, w: WindowId, bright: Option<WindowId>) -> f32 {
    let level = state.config.dim_inactive;
    let Some(win) = state.win(w) else {
        return 0.0;
    };
    if level == 0 || !win.mapped || win.override_redirect || win.fullscreen {
        return 0.0;
    }
    if bright == Some(root(state, w)) {
        return 0.0;
    }
    f32::from(level) / 100.0
}

/// Before a frame and on a focus change: every shade heads for its target, and the windows whose
/// shade changed redraw, again each frame while one fades. Fades with `[effects]
/// window_animation`, else snaps.
pub fn frame(state: &mut State) {
    let now = Instant::now();
    let fade = if state.config.effects.window_animation {
        FADE
    } else {
        Duration::ZERO
    };
    let off = |w: &crate::window::Window| w.dim_shade.to == 0.0 && !state.scene.enabled(w.dim);
    if state.config.dim_inactive == 0 && state.windows.iter().all(off) {
        return;
    }
    let bright = state.focused.map(|f| root(state, f));
    let targets: Vec<(WindowId, f32)> = (state.windows.iter())
        .map(|w| (w.id, target(state, w.id, bright)))
        .collect();
    let mut redraw = Vec::new();
    for (w, to) in targets {
        let Some(win) = state.win_mut(w) else {
            continue;
        };
        win.dim_shade.aim(to, now, fade);
        let a = win.dim_shade.alpha(now, fade);
        let fading = win.dim_shade.fading(now, fade);
        let (rect, tree) = (win.dim, win.tree);
        if state.scene.rect_color(rect).map(|c| c[3]) != Some(a) {
            state.scene.rect_set_color(rect, [0.0, 0.0, 0.0, a]);
            state.scene.set_enabled(rect, a > 0.0);
            redraw.push(tree.into());
        } else if fading {
            redraw.push(tree.into());
        }
    }
    if !redraw.is_empty() {
        state.redraw_nodes(Vec::new(), &redraw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shade_fades_linearly_and_turns_back_from_where_it_is() {
        let t0 = Instant::now();
        let mut s = Shade::default();
        assert!(s.aim(0.2, t0, FADE));
        assert_eq!(s.alpha(t0, FADE), 0.0);
        let half = t0 + FADE / 2;
        assert!((s.alpha(half, FADE) - 0.1).abs() < 1e-4);
        assert!(s.fading(half, FADE));
        assert!(!s.aim(0.2, half, FADE), "the same target goes on");
        assert!(s.aim(0.0, half, FADE));
        assert!((s.alpha(half, FADE) - 0.1).abs() < 1e-4, "no jump");
        assert_eq!(s.alpha(half + FADE, FADE), 0.0);
        assert!(!s.fading(half + FADE, FADE));
    }

    fn shade(h: &crate::harness::Harness, w: WindowId) -> Option<f32> {
        let win = h.state.win(w).expect("the window");
        (h.state.scene.enabled(win.dim)).then(|| h.state.scene.rect_color(win.dim).unwrap()[3])
    }

    #[test]
    fn every_window_but_the_focused_family_takes_the_shade() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "a", 200, 100);
        let b = h.map_toplevel(&mut c, "b", 300, 150);
        let d = h.map_dialog(&mut c, "b", Some(b), "", (120, 80));
        h.state.focus_window(a.id);
        frame(&mut h.state);
        for w in [a.id, b.id, d.id] {
            assert_eq!(shade(&h, w), None, "0 draws no shade");
        }
        h.state.config.dim_inactive = 20;
        frame(&mut h.state);
        assert_eq!(shade(&h, a.id), None, "the focused window is untouched");
        assert_eq!(shade(&h, b.id), Some(0.2));
        assert_eq!(shade(&h, d.id), Some(0.2));
        let size = h
            .state
            .scene
            .rect_size(h.state.win(b.id).unwrap().dim)
            .unwrap();
        assert_eq!((size.w, size.h), (300, 150), "the client area");
        h.state.focus_window(d.id);
        frame(&mut h.state);
        assert_eq!(shade(&h, a.id), Some(0.2));
        assert_eq!(
            shade(&h, b.id),
            None,
            "a focused dialog keeps its parent bright"
        );
        assert_eq!(shade(&h, d.id), None);
        h.state.set_fullscreen(a.id, true);
        frame(&mut h.state);
        assert_eq!(shade(&h, a.id), None, "full screen stays bright");
        h.state.config.dim_inactive = 0;
        frame(&mut h.state);
        assert_eq!(shade(&h, a.id), None);
    }

    #[test]
    fn without_a_fade_a_shade_snaps() {
        let t0 = Instant::now();
        let mut s = Shade::default();
        s.aim(0.3, t0, Duration::ZERO);
        assert_eq!(s.alpha(t0, Duration::ZERO), 0.3);
        assert!(!s.fading(t0, Duration::ZERO));
    }
}
