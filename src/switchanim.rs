use std::time::Instant;

use reprisescene::{BufferId, Input, TreeId};
use smithay::backend::renderer::Texture;
use smithay::output::Output;
use smithay::reexports::calloop::Idle;
use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::config::SwitchStyle;
use crate::raster::Rect;
use crate::window::WindowId;
use crate::windowanim::ease;

const ZOOM_TO: f64 = 0.9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    /// Layout coordinates.
    pub dest: Rect,
    /// Logical px from the still's origin.
    pub src: Rect,
    pub opacity: f32,
}

fn clipped(moved: Rect, area: Rect, still: Rect, opacity: f32) -> Option<Piece> {
    let dest = moved.intersect(&area);
    if dest.is_empty() {
        return None;
    }
    let src = Rect::new(
        dest.x - moved.x + area.x - still.x,
        dest.y - moved.y + area.y - still.y,
        dest.width,
        dest.height,
    );
    Some(Piece { dest, src, opacity })
}

/// Where the old and the new desktop's stills go at eased progress `e`.
pub fn pieces(
    style: SwitchStyle,
    still: Rect,
    area: Rect,
    forward: bool,
    e: f64,
) -> (Option<Piece>, Option<Piece>) {
    let e = e.clamp(0.0, 1.0);
    let whole = Rect::new(area.x - still.x, area.y - still.y, area.width, area.height);
    let sign = if forward { -1 } else { 1 };
    let fading = (1.0 - e) as f32;
    match style {
        SwitchStyle::None => (None, None),
        SwitchStyle::Slide => {
            let shift = sign * (e * area.width as f64).round() as i32;
            let old = Rect::new(area.x + shift, area.y, area.width, area.height);
            let new = Rect::new(old.x - sign * area.width, area.y, area.width, area.height);
            (
                clipped(old, area, still, 1.0),
                clipped(new, area, still, 1.0),
            )
        }
        SwitchStyle::SlideVertical => {
            let shift = sign * (e * area.height as f64).round() as i32;
            let old = Rect::new(area.x, area.y + shift, area.width, area.height);
            let new = Rect::new(area.x, old.y - sign * area.height, area.width, area.height);
            (
                clipped(old, area, still, 1.0),
                clipped(new, area, still, 1.0),
            )
        }
        SwitchStyle::Fade => (
            Some(Piece {
                dest: area,
                src: whole,
                opacity: fading,
            }),
            None,
        ),
        SwitchStyle::Zoom => {
            let s = 1.0 - (1.0 - ZOOM_TO) * e;
            let w = (area.width as f64 * s).round() as i32;
            let h = (area.height as f64 * s).round() as i32;
            let dest = Rect::new(
                area.x + (area.width - w) / 2,
                area.y + (area.height - h) / 2,
                w,
                h,
            );
            let old = (!dest.is_empty()).then_some(Piece {
                dest,
                src: whole,
                opacity: fading,
            });
            (old, None)
        }
        SwitchStyle::Wipe => {
            let cut = (e * area.width as f64).round() as i32;
            let left = if forward { 0 } else { cut };
            let dest = Rect::new(area.x + left, area.y, area.width - cut, area.height);
            let old = (!dest.is_empty()).then_some(Piece {
                dest,
                src: Rect::new(whole.x + left, whole.y, dest.width, dest.height),
                opacity: 1.0,
            });
            (old, None)
        }
    }
}

fn needs_new_still(style: SwitchStyle) -> bool {
    matches!(style, SwitchStyle::Slide | SwitchStyle::SlideVertical)
}

struct Screen {
    output: Output,
    still: Rect,
    area: Rect,
    fullscreen: bool,
    old: Still,
    new: Option<Still>,
}

struct Still {
    node: BufferId,
    px: (i32, i32),
}

pub struct SwitchAnim {
    tree: Option<TreeId>,
    idle: Option<Idle<'static>>,
    screens: Vec<Screen>,
    style: SwitchStyle,
    ms: u32,
    from: usize,
    forward: bool,
    started: Option<Instant>,
    span: (f64, f64),
    drag: Option<(usize, f64)>,
}

impl Default for SwitchAnim {
    fn default() -> Self {
        SwitchAnim {
            tree: None,
            idle: None,
            screens: Vec::new(),
            style: SwitchStyle::None,
            ms: 0,
            from: 0,
            forward: true,
            started: None,
            span: (0.0, 1.0),
            drag: None,
        }
    }
}

pub fn drag_finish(p: f64, commit: bool) -> ((f64, f64), f64) {
    let p = p.clamp(0.0, 1.0);
    let to = if commit { 1.0 } else { 0.0 };
    ((p, to), (to - p).abs().max(0.2))
}

impl State {
    pub fn cancel_switch_anim(&mut self) {
        let a = &mut self.switch_anim;
        if let Some(idle) = a.idle.take() {
            idle.cancel();
        }
        a.started = None;
        a.drag = None;
        a.span = (0.0, 1.0);
        let screens = std::mem::take(&mut a.screens);
        if let Some(tree) = a.tree.take() {
            self.scene.destroy(tree);
        }
        for s in &screens {
            crate::backend::schedule(self, &s.output);
        }
    }

    /// Called before `desktops.current` changes.
    pub fn switch_leaving(&mut self) {
        self.finish_window_anims();
        self.cancel_switch_anim();
        let cfg = self.config.workspace_animation;
        if !cfg.enabled() || self.is_locked() {
            return;
        }
        let outputs: Vec<Output> = self
            .scene
            .outputs()
            .iter()
            .filter(|o| !self.layout_box(Some(o)).is_empty())
            .cloned()
            .collect();
        let tree = self.scene.tree_create(self.layers.switching);
        let mut screens = Vec::new();
        for output in outputs {
            let b = self.layout_box(Some(&output));
            let Some(old) = self.still_of(&output, b, tree) else {
                continue;
            };
            let whole = Piece {
                dest: b,
                src: Rect::new(0, 0, b.width, b.height),
                opacity: 1.0,
            };
            self.place_still(&old, b, Some(whole));
            screens.push(Screen {
                fullscreen: self.fullscreen_on(b),
                output,
                still: b,
                area: b,
                old,
                new: None,
            });
        }
        if screens.is_empty() {
            self.scene.destroy(tree);
            return;
        }
        let idle = self.handle.insert_idle(|state| state.switch_settled());
        let a = &mut self.switch_anim;
        a.tree = Some(tree);
        a.idle = Some(idle);
        a.screens = screens;
        a.style = cfg.style;
        a.ms = cfg.ms;
        a.from = self.desktops.current;
    }

    pub fn desktop_drag(&mut self, p: f64) -> bool {
        use crate::desktops::Step;
        if self.is_locked() {
            return false;
        }
        let step = if p < 0.0 { Step::Prev } else { Step::Next };
        let target = self.desktops.step(step);
        if self.switch_anim.drag.is_none() {
            let Some(to) = target else {
                return false;
            };
            if !self.drag_begin(to) {
                return false;
            }
        }
        let e = match target {
            Some(to) => {
                if self.switch_anim.drag.is_some_and(|(n, _)| n != to) && !self.drag_retarget(to) {
                    return false;
                }
                p.abs().min(1.0)
            }
            None => 0.0,
        };
        let a = &mut self.switch_anim;
        if let Some(d) = a.drag.as_mut() {
            d.1 = e;
        }
        a.forward = p >= 0.0;
        self.drag_place(e);
        true
    }

    pub fn desktop_drag_end(&mut self, commit: bool) -> bool {
        let Some((to, e)) = self.switch_anim.drag.take() else {
            return false;
        };
        let commit = commit && e > 0.0;
        if commit {
            self.switch_desktop_as(to, false);
        }
        let (span, share) = drag_finish(e, commit);
        let a = &mut self.switch_anim;
        a.span = span;
        a.ms = ((f64::from(a.ms) * share).round() as u32).max(1);
        a.started = Some(Instant::now());
        let outputs: Vec<Output> = a.screens.iter().map(|s| s.output.clone()).collect();
        for o in &outputs {
            crate::backend::schedule(self, o);
        }
        true
    }

    fn drag_begin(&mut self, to: usize) -> bool {
        self.finish_window_anims();
        self.cancel_switch_anim();
        let outputs: Vec<Output> = self
            .scene
            .outputs()
            .iter()
            .filter(|o| !self.layout_box(Some(o)).is_empty())
            .cloned()
            .collect();
        let tree = self.scene.tree_create(self.layers.switching);
        let mut screens = Vec::new();
        for output in outputs {
            let b = self.layout_box(Some(&output));
            let Some(old) = self.still_of(&output, b, tree) else {
                continue;
            };
            let area = if self.fullscreen_on(b) {
                b
            } else {
                self.usable_area(Some(&output)).intersect(&b)
            };
            if area.is_empty() {
                self.scene.destroy(old.node);
                continue;
            }
            screens.push(Screen {
                fullscreen: self.fullscreen_on(b),
                output,
                still: b,
                area,
                old,
                new: None,
            });
        }
        let a = &mut self.switch_anim;
        a.tree = Some(tree);
        a.screens = screens;
        a.style = SwitchStyle::Slide;
        a.ms = self.config.workspace_animation.ms;
        a.from = self.desktops.current;
        a.drag = Some((to, 0.0));
        if !self.drag_retarget(to) {
            self.cancel_switch_anim();
            return false;
        }
        true
    }

    /// Take desktop `to`'s stills for the running swipe, dropping any
    /// other neighbour's. False, the slide given up, when an output
    /// cannot render one.
    fn drag_retarget(&mut self, to: usize) -> bool {
        let Some(tree) = self.switch_anim.tree else {
            return false;
        };
        let mut screens = std::mem::take(&mut self.switch_anim.screens);
        for s in &mut screens {
            if let Some(n) = s.new.take() {
                self.scene.destroy(n.node);
            }
        }
        let shown = self.show_only_desktop(to);
        let mut ok = !screens.is_empty();
        for s in &mut screens {
            s.new = self.still_of(&s.output, s.still, tree);
            ok &= s.new.is_some();
        }
        for (tree, on) in shown {
            self.scene.set_enabled(tree, on);
        }
        self.switch_anim.screens = screens;
        if let Some(d) = self.switch_anim.drag.as_mut() {
            d.0 = to;
        }
        ok
    }

    fn show_only_desktop(&mut self, n: usize) -> Vec<(TreeId, bool)> {
        let all: Vec<(WindowId, TreeId)> = self
            .windows
            .iter()
            .filter(|w| w.mapped)
            .map(|w| (w.id, w.tree))
            .collect();
        let mut was = Vec::with_capacity(all.len());
        for (w, tree) in all {
            was.push((tree, self.scene.enabled(tree)));
            let show = self.desktop_of(w).is_none_or(|d| d == n) && !self.minimized_along_chain(w);
            self.scene.set_enabled(tree, show);
        }
        was
    }

    fn drag_place(&mut self, e: f64) {
        let a = &self.switch_anim;
        let mut steps = Vec::new();
        for s in &a.screens {
            let (old, new) = pieces(SwitchStyle::Slide, s.still, s.area, a.forward, e);
            steps.push((s.old.node, s.old.px, s.still, old));
            if let Some(n) = &s.new {
                steps.push((n.node, n.px, s.still, new));
            }
        }
        let outputs: Vec<Output> = a.screens.iter().map(|s| s.output.clone()).collect();
        for (node, px, b, piece) in steps {
            self.place_still(&Still { node, px }, b, piece);
        }
        for o in &outputs {
            crate::backend::schedule(self, o);
        }
    }

    fn switch_settled(&mut self) {
        self.switch_anim.idle = None;
        let a = &self.switch_anim;
        let Some(tree) = a.tree.filter(|_| self.desktops.current != a.from) else {
            self.cancel_switch_anim();
            return;
        };
        let style = a.style;
        let outputs: Vec<(Output, Rect, bool)> = a
            .screens
            .iter()
            .map(|s| (s.output.clone(), s.still, s.fullscreen))
            .collect();
        let mut fits = Vec::with_capacity(outputs.len());
        for (output, b, was_full) in outputs {
            let area = if was_full || self.fullscreen_on(b) {
                b
            } else {
                self.usable_area(Some(&output)).intersect(&b)
            };
            let new = if needs_new_still(style) {
                self.still_of(&output, b, tree)
            } else {
                None
            };
            fits.push((area, new));
        }
        let mut screens = std::mem::take(&mut self.switch_anim.screens);
        for (s, (area, new)) in screens.iter_mut().zip(fits) {
            s.area = area;
            s.new = new;
        }
        // A slide with a still missing would show the live desktop at the
        // wrong place; give that output up.
        screens.retain(|s| {
            let keep = !s.area.is_empty() && (s.new.is_some() || !needs_new_still(style));
            if !keep {
                self.scene.destroy(s.old.node);
                if let Some(n) = &s.new {
                    self.scene.destroy(n.node);
                }
            }
            keep
        });
        if screens.is_empty() {
            self.cancel_switch_anim();
            return;
        }
        let outputs: Vec<Output> = screens.iter().map(|s| s.output.clone()).collect();
        self.switch_anim.screens = screens;
        self.switch_anim.forward = self.desktops.current > self.switch_anim.from;
        self.switch_anim.started = Some(Instant::now());
        for o in &outputs {
            crate::backend::schedule(self, o);
        }
    }

    /// Called from the output's render, so the steps follow its own refresh.
    pub fn switch_anim_frame(&mut self, output: &Output) {
        let a = &self.switch_anim;
        let Some(started) = a.started else {
            return;
        };
        let t = started.elapsed().as_secs_f64() * 1000.0 / f64::from(a.ms.max(1));
        if t >= 1.0 {
            self.cancel_switch_anim();
            return;
        }
        let forward = a.forward;
        let (from, to) = a.span;
        let e = from + (to - from) * ease(t);
        let mut steps = Vec::new();
        for s in a.screens.iter().filter(|s| s.output == *output) {
            let (old, new) = pieces(a.style, s.still, s.area, forward, e);
            steps.push((s.old.node, s.old.px, s.still, old));
            if let Some(n) = &s.new {
                steps.push((n.node, n.px, s.still, new));
            }
        }
        if steps.is_empty() {
            return;
        }
        for (node, px, b, piece) in steps {
            self.place_still(&Still { node, px }, b, piece);
        }
        // Late in the ease a frame can move nothing and so damage nothing; ask
        // anyway, so the end still comes.
        crate::backend::schedule(self, output);
    }

    fn place_still(&mut self, still: &Still, b: Rect, piece: Option<Piece>) {
        let node = still.node;
        let Some(p) = piece else {
            self.scene.set_enabled(node, false);
            return;
        };
        // From the still's own size, so a rounded fractional scale still lines up.
        let sx = f64::from(still.px.0) / f64::from(b.width.max(1));
        let sy = f64::from(still.px.1) / f64::from(b.height.max(1));
        let src = Rectangle::new(
            (f64::from(p.src.x) * sx, f64::from(p.src.y) * sy).into(),
            (f64::from(p.src.width) * sx, f64::from(p.src.height) * sy).into(),
        );
        self.scene.buffer_set_source_box(node, Some(src));
        self.scene
            .buffer_set_dest_size(node, Some((p.dest.width, p.dest.height).into()));
        self.scene.set_opacity(node, p.opacity);
        self.scene.set_position(node, p.dest.x, p.dest.y);
        self.scene.set_enabled(node, true);
    }

    /// Render `output`'s box `b` of the scene as it would show without
    /// anything from `layers.switching` up, onto a new, hidden buffer on
    /// `tree`. None when it cannot be rendered.
    fn still_of(&mut self, output: &Output, b: Rect, tree: TreeId) -> Option<Still> {
        let l = &self.layers;
        let above = [l.switching, l.menu, l.overlay, l.lock];
        let shown: Vec<bool> = above.iter().map(|&t| self.scene.enabled(t)).collect();
        for &t in &above {
            self.scene.set_enabled(t, false);
        }
        let scale = output.current_scale().fractional_scale();
        let texture = self.render_region((b.x, b.y), b.width, b.height, scale);
        for (&t, on) in above.iter().zip(shown) {
            self.scene.set_enabled(t, on);
        }
        let texture = texture?;
        let size = texture.size();
        let State { scene, backend, .. } = self;
        let renderer = &backend.as_ref()?.renderer;
        let node = scene.buffer_create(tree);
        scene.buffer_set_texture(node, renderer, texture);
        scene.set_input(node, Input::Passthrough);
        scene.set_enabled(node, false);
        Some(Still {
            node,
            px: (size.w, size.h),
        })
    }

    pub(crate) fn fullscreen_on(&self, b: Rect) -> bool {
        let lifted = self.layers.fullscreen;
        self.windows.iter().any(|w| {
            w.mapped
                && self.scene.enabled(w.tree)
                && self.scene.parent(w.tree) == Some(lifted)
                && !w.frame_rect().intersect(&b).is_empty()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Rect = Rect::new(100, 0, 1000, 800);
    const A: Rect = Rect::new(100, 0, 1000, 760);

    fn at(style: SwitchStyle, forward: bool, e: f64) -> (Option<Piece>, Option<Piece>) {
        pieces(style, B, A, forward, e)
    }

    #[test]
    fn a_slide_starts_on_the_old_desktop_and_ends_on_the_new_one() {
        for style in [SwitchStyle::Slide, SwitchStyle::SlideVertical] {
            for forward in [true, false] {
                let (old, new) = at(style, forward, 0.0);
                let old = old.unwrap();
                assert_eq!(old.dest, A, "{style:?}");
                assert_eq!(old.src, Rect::new(0, 0, 1000, 760));
                assert_eq!(new, None, "the new one waits off the area");
                let (old, new) = at(style, forward, 1.0);
                assert_eq!(old, None);
                let new = new.unwrap();
                assert_eq!(new.dest, A);
                assert_eq!(new.src, Rect::new(0, 0, 1000, 760));
            }
        }
    }

    #[test]
    fn a_later_desktop_slides_in_from_the_right() {
        let (old, new) = at(SwitchStyle::Slide, true, 0.25);
        let (old, new) = (old.unwrap(), new.unwrap());
        assert_eq!(old.dest, Rect::new(100, 0, 750, 760));
        assert_eq!(old.src, Rect::new(250, 0, 750, 760));
        assert_eq!(new.dest, Rect::new(850, 0, 250, 760));
        assert_eq!(new.src, Rect::new(0, 0, 250, 760));
        let (old, new) = at(SwitchStyle::Slide, false, 0.25);
        assert_eq!(old.unwrap().dest, Rect::new(350, 0, 750, 760));
        let new = new.unwrap();
        assert_eq!(new.dest, Rect::new(100, 0, 250, 760));
        assert_eq!(new.src, Rect::new(750, 0, 250, 760));
    }

    #[test]
    fn a_later_desktop_slides_up_from_below_and_never_over_the_taskbar() {
        let (old, new) = at(SwitchStyle::SlideVertical, true, 0.5);
        let (old, new) = (old.unwrap(), new.unwrap());
        assert_eq!(old.dest, Rect::new(100, 0, 1000, 380));
        assert_eq!(old.src, Rect::new(0, 380, 1000, 380));
        assert_eq!(new.dest, Rect::new(100, 380, 1000, 380));
        assert_eq!(new.src, Rect::new(0, 0, 1000, 380));
        assert!(new.dest.bottom() <= A.bottom());
    }

    #[test]
    fn the_source_is_offset_into_the_still_by_the_area() {
        let area = Rect::new(100, 40, 1000, 760);
        let (old, _) = pieces(SwitchStyle::Slide, B, area, true, 0.0);
        assert_eq!(old.unwrap().src, Rect::new(0, 40, 1000, 760));
        let (old, _) = pieces(SwitchStyle::Fade, B, area, true, 0.5);
        assert_eq!(old.unwrap().src, Rect::new(0, 40, 1000, 760));
    }

    #[test]
    fn a_fade_thins_the_old_desktop_over_the_live_one() {
        let (old, new) = at(SwitchStyle::Fade, true, 0.25);
        let old = old.unwrap();
        assert_eq!(old.dest, A);
        assert_eq!(old.opacity, 0.75);
        assert_eq!(new, None);
        assert_eq!(at(SwitchStyle::Fade, false, 1.0).0.unwrap().opacity, 0.0);
    }

    #[test]
    fn a_zoom_shrinks_the_old_desktop_about_the_middle() {
        let (old, new) = at(SwitchStyle::Zoom, true, 1.0);
        let old = old.unwrap();
        assert_eq!(old.dest, Rect::new(150, 38, 900, 684));
        assert_eq!(
            old.src,
            Rect::new(0, 0, 1000, 760),
            "the whole still, scaled"
        );
        assert_eq!(old.opacity, 0.0);
        assert_eq!(new, None);
        assert_eq!(at(SwitchStyle::Zoom, true, 0.0).0.unwrap().dest, A);
    }

    #[test]
    fn a_wipe_uncovers_from_the_side_the_desktop_comes_from() {
        let (old, _) = at(SwitchStyle::Wipe, true, 0.25);
        let old = old.unwrap();
        assert_eq!(old.dest, Rect::new(100, 0, 750, 760));
        assert_eq!(old.src, Rect::new(0, 0, 750, 760));
        let (old, _) = at(SwitchStyle::Wipe, false, 0.25);
        let old = old.unwrap();
        assert_eq!(old.dest, Rect::new(350, 0, 750, 760));
        assert_eq!(old.src, Rect::new(250, 0, 750, 760));
        assert_eq!(at(SwitchStyle::Wipe, true, 1.0).0, None);
    }

    #[test]
    fn no_style_shows_nothing_and_only_slides_want_the_new_still() {
        assert_eq!(at(SwitchStyle::None, true, 0.5), (None, None));
        for style in SwitchStyle::ALL {
            let wants = matches!(style, SwitchStyle::Slide | SwitchStyle::SlideVertical);
            assert_eq!(needs_new_still(style), wants, "{style:?}");
            if !wants {
                assert_eq!(at(style, true, 0.5).1, None, "{style:?}");
            }
        }
    }

    #[test]
    fn a_swipe_runs_on_from_where_the_fingers_left_it() {
        assert_eq!(drag_finish(0.4, true), ((0.4, 1.0), 0.6));
        assert_eq!(drag_finish(0.4, false), ((0.4, 0.0), 0.4));
        assert_eq!(drag_finish(0.95, true).1, 0.2);
        assert_eq!(drag_finish(1.3, true), ((1.0, 1.0), 0.2));
    }
}
