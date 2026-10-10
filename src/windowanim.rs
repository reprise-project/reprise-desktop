//! Windows fading and growing as they open and shrinking as they close.
//! The live window takes input from the moment it maps; only its picture
//! waits.

use std::time::Instant;

use reprisescene::{BufferId, Input, TreeId};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{BufferAssignment, SurfaceAttributes, with_states};

use crate::compositor::State;
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::raster::Rect;
use crate::window::WindowId;
use crate::windowstill::Still;

pub fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub fn place(rect: Rect, scale: f64, opening: bool, e: f64) -> (Rect, f32) {
    let k = if opening { e } else { 1.0 - e };
    let s = scale + (1.0 - scale) * k;
    let (w, h) = (
        (rect.width as f64 * s).round() as i32,
        (rect.height as f64 * s).round() as i32,
    );
    let r = Rect::new(
        rect.x + (rect.width - w) / 2,
        rect.y + (rect.height - h) / 2,
        w,
        h,
    );
    (r, k as f32)
}

fn scale_pane(pane: &Pane, from: Rect, to: Rect) -> Pane {
    let kx = f64::from(to.width) / f64::from(from.width.max(1));
    let ky = f64::from(to.height) / f64::from(from.height.max(1));
    let r = |r: Rect| {
        let x = |v: i32| (f64::from(v) * kx).round() as i32;
        let y = |v: i32| (f64::from(v) * ky).round() as i32;
        Rect::new(
            x(r.x),
            y(r.y),
            x(r.right()) - x(r.x),
            y(r.bottom()) - y(r.y),
        )
    };
    Pane {
        shapes: pane
            .shapes
            .iter()
            .map(|s| PaneShape {
                rect: r(s.rect),
                corner: s.corner,
                square_bottom: s.square_bottom,
                square_top: s.square_top,
            })
            .collect(),
        holes: pane.holes.iter().map(|&h| r(h)).collect(),
    }
}

/// Output frames an opening may be passed over in (one still per frame)
/// before it gives up and shows whole.
const WAIT_FRAMES: u32 = 2;

struct Shown {
    tree: TreeId,
    buffer: BufferId,
    backdrop_tree: TreeId,
    glass: Backdrop,
    pane: Option<Pane>,
    rect: Rect,
}

struct Anim {
    window: WindowId,
    still: Option<Shown>,
    opening: bool,
    started: Instant,
    ms: u32,
    scale: f64,
    /// An opening's frame when its still was taken: a move or resize ends it.
    frame: Rect,
    /// A closing window was its layer's top: its still stays there, over the
    /// window the focus raises next.
    top: bool,
    /// Where a closing starts: one closing mid-opening goes on from there.
    from: f64,
    waited: u32,
}

#[derive(Default)]
pub struct WindowAnims {
    list: Vec<Anim>,
}

impl State {
    pub fn window_opening(&mut self, w: WindowId) {
        if let Some(r) = self.ripple_frame(w) {
            self.water_ring(r);
        }
        let fx = self.config.effects;
        if !fx.opens() || !self.anim_shows(w) {
            return;
        }
        let Some((tree, frame)) = self.win(w).map(|x| (x.tree, x.frame_rect())) else {
            return;
        };
        if !self.on_some_output(frame) {
            return;
        }
        self.scene.set_opacity(tree, 0.0);
        self.window_anims.list.push(Anim {
            window: w,
            still: None,
            opening: true,
            started: Instant::now(),
            ms: fx.window_open_ms,
            scale: fx.window_scale,
            frame,
            top: false,
            from: 1.0,
            waited: 0,
        });
        self.schedule_rect(frame);
    }

    /// Called while `w` still shows, with its client's last buffers in place.
    pub fn window_closing(&mut self, w: WindowId) {
        if self
            .window_anims
            .list
            .iter()
            .any(|a| !a.opening && a.window == w)
        {
            return;
        }
        if let Some(r) = self.ripple_frame(w) {
            self.water_ring(r);
        }
        let from = self.anim_shown(w);
        self.finish_window_anim(w);
        let fx = self.config.effects;
        if !fx.closes() || !self.anim_shows(w) || from <= 0.0 {
            return;
        }
        let Some((tree, frame)) = self.win(w).map(|x| (x.tree, x.frame_rect())) else {
            return;
        };
        if !self.on_some_output(frame) {
            return;
        }
        let top = self
            .scene
            .parent(tree)
            .is_some_and(|layer| self.scene.children(layer).last() == Some(&tree.node()));
        let Some(still) = self.window_still(w) else {
            return;
        };
        let Some(shown) = self.show_still(w, still) else {
            return;
        };
        let rect = shown.rect;
        self.window_anims.list.push(Anim {
            window: w,
            still: Some(shown),
            opening: false,
            started: Instant::now(),
            ms: fx.window_close_ms,
            scale: fx.window_scale,
            frame: rect,
            top,
            from,
            waited: 0,
        });
        let i = self.window_anims.list.len() - 1;
        self.step_window_anim(i, 0.0);
        self.schedule_rect(rect);
    }

    /// A commit that takes a mapped window's buffer away unmaps it: its closing
    /// still is laid now, while the last buffer is still there.
    pub fn closing_commit(&mut self, surface: &WlSurface) {
        let Some(w) = self.window_for_surface(surface) else {
            return;
        };
        let removed = with_states(surface, |s| {
            matches!(
                s.cached_state.get::<SurfaceAttributes>().current().buffer,
                Some(BufferAssignment::Removed)
            )
        });
        if removed && self.win(w).is_some_and(|x| x.mapped) {
            self.window_closing(w);
        }
    }

    fn anim_shown(&self, w: WindowId) -> f64 {
        self.window_anims
            .list
            .iter()
            .find(|a| a.opening && a.window == w)
            .map_or(1.0, |a| {
                if a.still.is_none() {
                    return 0.0;
                }
                let ms = a.ms.max(1) as f64;
                ease(a.started.elapsed().as_secs_f64() * 1000.0 / ms)
            })
    }

    pub fn finish_window_anim(&mut self, w: WindowId) {
        while let Some(i) = self
            .window_anims
            .list
            .iter()
            .position(|a| a.opening && a.window == w)
        {
            self.complete_window_anim(i);
        }
    }

    pub fn finish_window_anims(&mut self) {
        while !self.window_anims.list.is_empty() {
            self.complete_window_anim(0);
        }
    }

    pub fn window_anims_frame(&mut self, output: &Output) {
        let now = Instant::now();
        let mut took = false;
        let mut i = 0;
        while i < self.window_anims.list.len() {
            let a = &self.window_anims.list[i];
            let (w, opening, pending) = (a.window, a.opening, a.still.is_none());
            let ms = a.ms.max(1) as f64;
            let at =
                |a: &Anim| now.saturating_duration_since(a.started).as_secs_f64() * 1000.0 / ms;
            let cut = if opening {
                self.anim_cut_short(i)
            } else {
                self.peek.active()
            };
            if cut {
                self.complete_window_anim(i);
                continue;
            }
            if pending {
                let frame = self.win(w).map_or(Rect::default(), |x| x.frame_rect());
                if !self.touches(output, frame) {
                    if at(&self.window_anims.list[i]) >= 1.0 {
                        self.complete_window_anim(i);
                        continue;
                    }
                    i += 1;
                    continue;
                }
                if took {
                    let a = &mut self.window_anims.list[i];
                    if a.waited >= WAIT_FRAMES {
                        self.complete_window_anim(i);
                        continue;
                    }
                    a.waited += 1;
                    self.schedule_rect(frame);
                    i += 1;
                    continue;
                }
                took = true;
                // The one try: an opening that cannot have its still just
                // shows.
                if !self.take_opening(i, frame) {
                    self.complete_window_anim(i);
                    continue;
                }
            }
            let a = &self.window_anims.list[i];
            let t = at(a);
            if t >= 1.0 {
                self.complete_window_anim(i);
                continue;
            }
            let rect = a.still.as_ref().map_or(a.frame, |s| s.rect);
            // A step late in the ease may damage nothing; ask anyway, so the end comes.
            self.step_window_anim(i, ease(t));
            self.schedule_rect(rect);
            i += 1;
        }
    }

    fn take_opening(&mut self, i: usize, frame: Rect) -> bool {
        let w = self.window_anims.list[i].window;
        let Some(still) = self.window_still(w) else {
            return false;
        };
        let Some(shown) = self.show_still(w, still) else {
            return false;
        };
        let a = &mut self.window_anims.list[i];
        a.still = Some(shown);
        a.started = Instant::now();
        a.frame = frame;
        true
    }

    fn show_still(&mut self, w: WindowId, still: Still) -> Option<Shown> {
        let win = self.win(w)?;
        let (wtree, rect) = (win.tree, still.rect);
        let pane = match self.config.theme.glass() {
            Some(g) if crate::glass::wanted(&self.config) && win.has_frame() => {
                let mut p = crate::glass::frame_pane(&win.geom, g.corner, win.maximized);
                let (dx, dy) = (win.x - rect.x, win.y - rect.y);
                for s in &mut p.shapes {
                    s.rect = Rect::new(s.rect.x + dx, s.rect.y + dy, s.rect.width, s.rect.height);
                }
                for h in &mut p.holes {
                    *h = Rect::new(h.x + dx, h.y + dy, h.width, h.height);
                }
                Some(p)
            }
            _ => None,
        };
        let layer = self.scene.parent(wtree)?;
        let renderer = &self.backend.as_ref()?.renderer;
        let scene = &mut self.scene;
        let tree = scene.tree_create(layer);
        scene.place_above(tree, wtree);
        scene.set_enabled(tree, false);
        // The still holds the frame's art: out of what glass blurs, its own
        // included.
        crate::glass::leave_out(scene, tree);
        let backdrop_tree = scene.tree_create(tree);
        let buffer = scene.buffer_create(tree);
        scene.set_input(buffer, Input::Passthrough);
        scene.buffer_set_texture(buffer, renderer, still.texture);
        Some(Shown {
            tree,
            buffer,
            backdrop_tree,
            glass: Backdrop::new(&self.glass),
            pane,
            rect,
        })
    }

    fn anim_cut_short(&self, i: usize) -> bool {
        let a = &self.window_anims.list[i];
        if !self.anim_shows(a.window) {
            return true;
        }
        let Some(win) = self.win(a.window) else {
            return true;
        };
        a.still.as_ref().is_some_and(|s| {
            win.frame_rect() != a.frame || self.scene.parent(win.tree) != self.scene.parent(s.tree)
        })
    }

    pub(crate) fn ripple_frame(&self, w: WindowId) -> Option<Rect> {
        if !self.water_on() {
            return None;
        }
        let x = self.win(w)?;
        let frame = x.frame_rect();
        (x.mapped
            && self.scene.enabled(x.tree)
            && !x.hidden()
            && !x.override_redirect
            && self.on_current_desktop(w)
            && self.on_some_output(frame))
        .then_some(frame)
    }

    /// Whether `w` shows as a window that animates: mapped and enabled on
    /// the desktop shown, not minimized or fullscreen, not an X11 menu or
    /// tooltip (override-redirect), and no peek running.
    fn anim_shows(&self, w: WindowId) -> bool {
        self.win(w).is_some_and(|x| {
            x.mapped
                && self.scene.enabled(x.tree)
                && !x.hidden()
                && !x.fullscreen
                && !x.override_redirect
                && self.on_current_desktop(w)
        }) && !self.peek.active()
    }

    fn step_window_anim(&mut self, i: usize, e: f64) {
        let a = &self.window_anims.list[i];
        let (scale, opening, w, top, from) = (a.scale, a.opening, a.window, a.top, a.from);
        let wtree = self.win(w).map(|x| x.tree);
        let State {
            scene,
            glass,
            window_anims,
            ..
        } = self;
        let Some(s) = window_anims.list[i].still.as_mut() else {
            return;
        };
        if top {
            scene.raise_to_top(s.tree);
        }
        if opening && let Some(wtree) = wtree {
            scene.place_above(s.tree, wtree);
        }
        let e = if opening { e } else { 1.0 - from * (1.0 - e) };
        let (r, opacity) = place(s.rect, scale, opening, e);
        scene.buffer_set_dest_size(s.buffer, Some((r.width, r.height).into()));
        scene.set_opacity(s.tree, opacity);
        scene.set_position(s.tree, r.x, r.y);
        scene.set_enabled(s.tree, true);
        let pane = s.pane.as_ref().map(|p| scale_pane(p, s.rect, r));
        s.glass.set(scene, glass, s.backdrop_tree, pane);
    }

    fn complete_window_anim(&mut self, i: usize) {
        let a = self.window_anims.list.remove(i);
        if a.opening
            && let Some(tree) = self.win(a.window).map(|x| x.tree)
        {
            self.scene.set_opacity(tree, 1.0);
        }
        if let Some(s) = a.still {
            self.scene.destroy(s.tree);
            self.schedule_rect(s.rect);
        }
    }

    fn on_some_output(&self, rect: Rect) -> bool {
        self.scene.outputs().iter().any(|o| self.touches(o, rect))
    }

    fn touches(&self, output: &Output, rect: Rect) -> bool {
        !self.layout_box(Some(output)).intersect(&rect).is_empty()
    }

    fn schedule_rect(&mut self, rect: Rect) {
        let outputs: Vec<Output> = self
            .scene
            .outputs()
            .iter()
            .filter(|o| self.touches(o, rect))
            .cloned()
            .collect();
        for o in &outputs {
            crate::backend::schedule(self, o);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opening_grows_from_the_scale_about_the_centre_and_fades_in() {
        let r = Rect::new(100, 50, 400, 200);
        assert_eq!(
            place(r, 0.9, true, 0.0),
            (Rect::new(120, 60, 360, 180), 0.0)
        );
        assert_eq!(place(r, 0.9, true, 1.0), (r, 1.0));
        let (m, o) = place(r, 0.9, true, 0.5);
        assert_eq!(m, Rect::new(110, 55, 380, 190));
        assert_eq!(o, 0.5);
    }

    #[test]
    fn a_closing_shrinks_toward_the_scale_and_fades_out() {
        let r = Rect::new(0, 0, 200, 100);
        assert_eq!(place(r, 0.8, false, 0.0), (r, 1.0));
        assert_eq!(place(r, 0.8, false, 1.0), (Rect::new(20, 10, 160, 80), 0.0));
    }

    #[test]
    fn ease_runs_from_0_to_1_ahead_of_linear() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
        assert!(ease(0.5) > 0.5, "ahead of linear halfway");
        assert!(ease(0.9) < 1.0);
    }

    #[test]
    fn the_glass_under_a_still_stretches_with_it() {
        let pane = Pane {
            shapes: vec![PaneShape {
                rect: Rect::new(10, 10, 200, 100),
                corner: &[3, 1, 1],
                square_bottom: false,
                square_top: false,
            }],
            holes: vec![Rect::new(14, 40, 192, 66)],
        };
        let from = Rect::new(0, 0, 220, 120);
        let half = scale_pane(&pane, from, Rect::new(55, 30, 110, 60));
        assert_eq!(half.shapes[0].rect, Rect::new(5, 5, 100, 50));
        assert_eq!(
            half.shapes[0].corner,
            &[3, 1, 1],
            "the corners keep their cut"
        );
        assert_eq!(half.holes[0], Rect::new(7, 20, 96, 33));
        assert_eq!(scale_pane(&pane, from, from), pane, "whole size: unchanged");
    }
}
