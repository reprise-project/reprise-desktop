//! Outputs: which mode, scale and transform a connector runs, whether it
//! is in the layout, where it sits, and what the shell redoes when that
//! changes (`State::update_layout`). `backend` drives the connectors; this
//! module decides what they run.
//!
//! Outputs line up left to right in the order they came, at y 0, unless
//! `[[outputs]] position` or output management placed them: placed outputs
//! keep their position and the others follow right of the rightmost placed
//! one (`place`). An output that leaves and comes back without a configured
//! position returns to where it was placed (`Places`).

use std::cell::Cell;

use reprisekit::style::Part;
use reprisescene::BufferId;
use smithay::{
    backend::input::InputTime,
    output::{Output, Scale},
    utils::{Logical, Point, Rectangle, Transform},
};

use crate::{
    compositor::{DeskScene, State},
    config::OutputMode,
    raster::{Canvas, Rect},
    window::WindowId,
};

/// The integer scale chrome is drawn at on an output of `scale`: rounded
/// up (a hair above a whole number still draws at it) and clamped to
/// 1..=4, so the scene only ever scales a canvas down.
pub(crate) fn draw_scale(scale: f32) -> i32 {
    if scale.is_finite() {
        ((scale - 1e-3).ceil() as i32).clamp(1, 4)
    } else {
        1
    }
}

/// Show `canvas` in `sb` at `scale`; whether any of its pixels differ from those shown before.
pub fn upload_scaled(scene: &mut DeskScene, sb: BufferId, canvas: &Canvas, scale: i32) -> bool {
    let scale = scale.max(1);
    let (w, h) = (canvas.width as i32, canvas.height as i32);
    let changed = scene.buffer_set_canvas(sb, &canvas.pixels, w, h);
    scene.buffer_set_dest_size(sb, Some((w / scale, h / scale).into()));
    !changed.is_empty()
}

/// Where output management placed an output, in its user data; None when automatic.
struct Placement(Cell<Option<Point<i32, Logical>>>);

fn placement(output: &Output) -> &Placement {
    output
        .user_data()
        .insert_if_missing(|| Placement(Cell::new(None)));
    output
        .user_data()
        .get::<Placement>()
        .expect("inserted above")
}

/// The box an output was last laid out at, in its user data; None before its first layout
/// and once it leaves.
struct LaidOut(Cell<Option<Rect>>);

fn laid_out(output: &Output) -> &LaidOut {
    output
        .user_data()
        .insert_if_missing(|| LaidOut(Cell::new(None)));
    output.user_data().get::<LaidOut>().expect("inserted above")
}

/// How far each of `frames` moves when the outputs went from the boxes `before` to `after`
/// (keyed alike): a frame whose centre lay on an output that is still laid out moves with
/// that output's top-left. None for a frame that stays.
pub fn carried<K: PartialEq>(
    frames: &[Rect],
    before: &[(K, Rect)],
    after: &[(K, Rect)],
) -> Vec<Option<(i32, i32)>> {
    frames
        .iter()
        .map(|f| {
            let (cx, cy) = (f.x + f.width / 2, f.y + f.height / 2);
            let (k, old) = before.iter().find(|(_, r)| r.contains(cx, cy))?;
            let (_, new) = after.iter().find(|(n, _)| n == k)?;
            let d = (new.x - old.x, new.y - old.y);
            (d != (0, 0)).then_some(d)
        })
        .collect()
}

/// Put `output` at layout `pos` for good, or back to automatic placement with None.
pub fn set_placement(output: &Output, pos: Option<Point<i32, Logical>>) {
    placement(output).0.set(pos);
}

/// Where each connector was placed when it left the layout, for the session: an output turned
/// off and on, or unplugged and plugged in, comes back there unless `[[outputs]] position` says,
/// or another output now covers part of that spot.
#[derive(Default)]
pub struct Places(std::collections::HashMap<String, Rectangle<i32, Logical>>);

impl Places {
    /// `name` left the layout placed at `at`, its box then (None: automatic).
    pub fn left(&mut self, name: &str, at: Option<Rectangle<i32, Logical>>) {
        match at {
            Some(r) => self.0.insert(name.to_string(), r),
            None => self.0.remove(name),
        };
    }

    /// Where `name` goes as it comes back: `configured` if set, else where it left unless one of
    /// `others` (the outputs in the layout) overlaps that box, else automatic.
    pub fn back(
        &self,
        name: &str,
        configured: Option<(i32, i32)>,
        others: &[Rectangle<i32, Logical>],
    ) -> Option<(i32, i32)> {
        configured.or_else(|| {
            let r = self
                .0
                .get(name)
                .filter(|r| !others.iter().any(|o| o.overlaps(**r)))?;
            Some((r.loc.x, r.loc.y))
        })
    }
}

/// One output as `place` sees it: its fixed position (None for automatic) and logical size.
pub type Slot = (Option<(i32, i32)>, (i32, i32));

/// The layout position of each output, in the order they came: fixed ones
/// where they asked, the automatic ones left to right from the right edge
/// of the rightmost fixed one at its y ((0, 0) when none is fixed).
pub fn place(outputs: &[Slot]) -> Vec<(i32, i32)> {
    let mut right: Option<(i32, i32)> = None;
    for &(fixed, (w, _)) in outputs {
        if let Some((x, y)) = fixed
            && right.is_none_or(|(r, _)| x + w > r)
        {
            right = Some((x + w, y));
        }
    }
    let (mut x, y) = right.unwrap_or((0, 0));
    outputs
        .iter()
        .map(|&(fixed, (w, _))| {
            fixed.unwrap_or_else(|| {
                let at = (x, y);
                x += w;
                at
            })
        })
        .collect()
}

/// The point of `rects` nearest to `pos`: `pos` itself inside one, else
/// the closest point on an edge (right and bottom excluded). None without a rect.
pub fn closest_point(rects: &[Rect], pos: (f64, f64)) -> Option<(f64, f64)> {
    rects
        .iter()
        .filter(|r| r.width > 0 && r.height > 0)
        .map(|r| {
            let max_x = f64::from(r.x + r.width - 1);
            let max_y = f64::from(r.y + r.height - 1);
            let p = (
                pos.0.clamp(f64::from(r.x), max_x),
                pos.1.clamp(f64::from(r.y), max_y),
            );
            let d = (p.0 - pos.0).powi(2) + (p.1 - pos.1).powi(2);
            (d, p)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p)| p)
}

pub fn transform(t: crate::config::Transform) -> Transform {
    use crate::config::Transform as T;
    match t {
        T::Normal => Transform::Normal,
        T::Rotate90 => Transform::_90,
        T::Rotate180 => Transform::_180,
        T::Rotate270 => Transform::_270,
        T::Flipped => Transform::Flipped,
        T::Flipped90 => Transform::Flipped90,
        T::Flipped180 => Transform::Flipped180,
        T::Flipped270 => Transform::Flipped270,
    }
}

/// What one head of an output-management configuration asks for (`outputmgmt`).
#[derive(Clone, Debug, PartialEq)]
pub struct HeadRequest {
    pub enabled: bool,
    /// The connector's mode to run; None keeps the current one (or picks
    /// the configured one for an output coming back).
    pub mode: Option<usize>,
    /// None places it automatically.
    pub position: Option<(i32, i32)>,
    pub transform: Transform,
    pub scale: f64,
    pub adaptive_sync: bool,
}

impl State {
    pub fn output_added(&mut self, _output: &Output) {
        self.cancel_switch_anim();
        self.close_overview();
        self.finish_window_anims();
        self.update_layout();
    }

    pub fn output_removed(&mut self, output: &Output) {
        self.cancel_switch_anim();
        self.close_overview();
        self.finish_window_anims();
        self.magnifier.retain(self.scene.outputs());
        laid_out(output).0.set(None);
        let bar = self
            .bars
            .iter()
            .find(|b| &b.output == output)
            .map(|b| b.tree);
        let on_it = match self.interaction {
            crate::input::Interaction::TaskPress { bar: b, .. }
            | crate::input::Interaction::QuickPress { bar: b, .. }
            | crate::input::Interaction::ArrowPress { bar: b }
            | crate::input::Interaction::BarMove { bar: b, .. }
            | crate::input::Interaction::BarWidth { bar: b, .. } => Some(b) == bar,
            _ => false,
        };
        if on_it {
            if matches!(
                self.interaction,
                crate::input::Interaction::BarMove { .. }
                    | crate::input::Interaction::BarWidth { .. }
            ) {
                self.cancel_bar_drag();
            }
            self.interaction = crate::input::Interaction::Idle;
            self.set_cursor("default");
        }
        self.close_menus_off_scene();
        self.close_start_panel_off_scene();
        self.update_layout();
        crate::notifyview::output_removed(self);
    }

    /// Put `output` in the layout with its `[[outputs]]` settings, or take
    /// it out. Each way ends in `output_added` or `output_removed`.
    pub fn set_in_layout(&mut self, output: &Output, on: bool) {
        let name = output.name();
        if !on {
            log::info!("output {name}: leaving the layout");
            self.leave_layout(output);
            return;
        }
        if self.scene.outputs().contains(output) {
            return;
        }
        let at = self.back_place(output);
        set_placement(output, at.map(Point::from));
        if !self.enable_configured(output) {
            log::error!("output {name}: no output state accepted, leaving it out");
            return;
        }
        log::info!("output {name}: in the layout");
        self.output_added(output);
    }

    /// Where `output` goes as it comes back into the layout (`Places::back`).
    fn back_place(&self, output: &Output) -> Option<(i32, i32)> {
        let name = output.name();
        let others: Vec<_> = (self.scene.outputs().iter())
            .filter(|o| *o != output)
            .filter_map(reprisescene::output_rect)
            .collect();
        (self.output_places).back(&name, self.config.position_for_output_name(&name), &others)
    }

    pub fn leave_layout(&mut self, output: &Output) {
        if self.scene.outputs().contains(output) {
            let at = placement(output)
                .0
                .get()
                .and_then(|p| reprisescene::output_rect(output).map(|r| Rectangle::new(p, r.size)));
            self.output_places.left(&output.name(), at);
        }
        if crate::backend::disable(self, output) {
            self.output_removed(output);
        }
    }

    /// Drive the parked `output` with the config's mode, scale, transform
    /// and adaptive sync; falls back to the preferred mode at scale 1
    /// untransformed when refused. Returns whether either went through.
    fn enable_configured(&mut self, output: &Output) -> bool {
        let name = output.name();
        let Some(head) = crate::backend::heads(self)
            .into_iter()
            .find(|h| &h.output == output)
        else {
            return false;
        };
        let want = self.config.mode_for_output_name(&name);
        let Some(index) = pick_mode(&head.modes, want.as_ref()) else {
            return false;
        };
        if let Some(want) = want
            && !want_matches(&head.modes[index], &want)
        {
            log::warn!(
                "{name}: no {}x{} mode, using the preferred one",
                want.width,
                want.height
            );
        }
        let scale = quarter_scale(self.config.scale_for_output_name(&name));
        let t = transform(self.config.transform_for_output_name(&name));
        output.change_current_state(
            None,
            Some(t),
            Some(Scale::Fractional(f64::from(scale))),
            None,
        );
        if !crate::backend::enable(self, output, index) {
            log::warn!(
                "output {name}: the configured output state was refused, trying the preferred mode"
            );
            let Some(preferred) = pick_mode(&head.modes, None) else {
                return false;
            };
            output.change_current_state(
                None,
                Some(Transform::Normal),
                Some(Scale::Fractional(1.0)),
                None,
            );
            if !crate::backend::enable(self, output, preferred) {
                return false;
            }
        }
        if self.config.adaptive_sync_for_output_name(&name) {
            crate::backend::set_vrr(self, output, true);
        }
        true
    }

    /// Run one head of an output-management configuration. Returns whether
    /// it took; the caller lays out once at the end.
    pub fn apply_head(&mut self, output: &Output, req: &HeadRequest) -> bool {
        let name = output.name();
        let driven = self.scene.outputs().contains(output);
        if !req.enabled {
            if driven {
                log::info!("output {name}: leaving the layout");
                self.leave_layout(output);
            }
            return true;
        }
        let scale = Scale::Fractional(f64::from(quarter_scale(req.scale as f32)));
        if output.current_transform() != req.transform
            || output.current_scale().fractional_scale() != scale.fractional_scale()
        {
            output.change_current_state(None, Some(req.transform), Some(scale), None);
        }
        let at = req.position.or_else(|| self.back_place(output));
        set_placement(output, at.map(Point::from));
        if driven {
            if let Some(i) = req.mode
                && !crate::backend::set_mode(self, output, i)
            {
                return false;
            }
            return crate::backend::set_vrr(self, output, req.adaptive_sync) || !req.adaptive_sync;
        }
        let ok = match req.mode {
            Some(i) => crate::backend::enable(self, output, i),
            None => {
                let heads = crate::backend::heads(self);
                let want = self.config.mode_for_output_name(&name);
                heads
                    .iter()
                    .find(|h| &h.output == output)
                    .and_then(|h| pick_mode(&h.modes, want.as_ref()))
                    .is_some_and(|i| crate::backend::enable(self, output, i))
            }
        };
        if !ok {
            return false;
        }
        if req.adaptive_sync {
            crate::backend::set_vrr(self, output, true);
        }
        log::info!("output {name}: back in the layout");
        self.output_added(output);
        true
    }

    /// Commit, on each driven output, the config keys that differ from
    /// `old`'s and from what it runs; runs when the config file changes. An
    /// unchanged key is left alone, so what output management set stays. A
    /// missing mode keeps the current one, and turning adaptive sync off is
    /// always accepted. While the display is busy it all waits for
    /// `backend::on_quiet`. A scale no output draws at yet waits while a job paints its art.
    /// Returns whether any output's geometry changed.
    pub fn apply_output_config(&mut self, old: &crate::config::Config) -> bool {
        self.apply_outputs(old, true)
    }

    fn apply_outputs(&mut self, old: &crate::config::Config, warm: bool) -> bool {
        if !crate::backend::modes_settle(self) {
            crate::backend::defer_output_config(self, old);
            return false;
        }
        if warm {
            if self.scale_warm.is_none() {
                let scales: Vec<f32> = (crate::backend::heads(self).iter())
                    .filter(|h| h.enabled)
                    .map(|h| h.output.name())
                    .filter(|name| crate::configwatch::output_change(old, &self.config, name).scale)
                    .map(|name| self.config.scale_for_output_name(&name))
                    .collect();
                self.start_scale_warm(scales);
            }
            if let Some(w) = self.scale_warm.as_mut() {
                w.old.get_or_insert_with(|| Box::new(old.clone()));
                return false;
            }
        }
        let mut geometry_changed = false;
        for head in crate::backend::heads(self) {
            if !head.enabled {
                continue;
            }
            let output = head.output;
            let name = output.name();
            let change = crate::configwatch::output_change(old, &self.config, &name);
            if !change.any() {
                continue;
            }
            if change.mode
                && let Some(want) = self.config.mode_for_output_name(&name)
                && let Some(i) = pick_mode(&head.modes, Some(&want))
                && head.current != Some(i)
            {
                if crate::backend::set_mode(self, &output, i) {
                    geometry_changed = true;
                } else {
                    log::warn!("output {name}: the configured mode was refused");
                }
            }
            if change.transform {
                let t = transform(self.config.transform_for_output_name(&name));
                if t != output.current_transform() {
                    output.change_current_state(None, Some(t), None, None);
                    geometry_changed = true;
                }
            }
            if change.scale {
                let want = f64::from(quarter_scale(self.config.scale_for_output_name(&name)));
                if want != output.current_scale().fractional_scale() {
                    output.change_current_state(None, None, Some(Scale::Fractional(want)), None);
                    geometry_changed = true;
                }
            }
            if change.adaptive_sync {
                let want = self.config.adaptive_sync_for_output_name(&name);
                if want != head.vrr_on {
                    crate::backend::set_vrr(self, &output, want);
                }
            }
            if change.position {
                let at = self.config.position_for_output_name(&name);
                set_placement(&output, at.map(Point::from));
                geometry_changed = true;
            }
        }
        geometry_changed
    }

    /// Paints, on a job, the art of the draw scales among output `scales` that no output runs at,
    /// and loads the desktop's theme icons at them; meanwhile `scale_warm` holds the output
    /// changes, which its end applies. Else nothing.
    pub fn start_scale_warm(&mut self, scales: Vec<f32>) {
        let reprisekit::look::Look::Styled(style) = &self.config.theme.look else {
            return;
        };
        let used: Vec<i32> = (self.scene.outputs().iter())
            .map(|o| self.draw_scale_at(Some(o)))
            .collect();
        let scales = new_draw_scales(&used, scales);
        let icons = &mut self.icons;
        let mut icon = |n| crate::taskbarview::start_icon(&self.config, icons, n);
        let sides = reprisekit::frame::caption_button_sides(&self.config.theme.metrics);
        let warm: Vec<_> = scales
            .iter()
            .flat_map(|&n| [style.warm(n, icon(n)), style.warm_button_sides(n, &sides)])
            .flatten()
            .collect();
        if warm.is_empty() {
            return;
        }
        let desk: Vec<(String, i32)> = (scales.iter())
            .flat_map(|&n| self.desktop_theme_icons(n))
            .collect();
        let (theme, icon_scale) = (self.icons.theme(), self.icons.scale());
        log::debug!("output scales {scales:?}: painting their art before the change");
        self.scale_warm = Some(ScaleWarm {
            old: None,
            heads: Vec::new(),
            scales,
        });
        let spawned = self.spawn_job(move || {
            for w in &warm {
                w.paint();
            }
            let desk: Vec<_> = (desk.into_iter())
                .map(|(name, px)| {
                    let icon = reprisekit::icontheme::load_icon(&theme, &name, px, icon_scale);
                    (name, px, icon)
                })
                .collect();
            Box::new(move |state: &mut State| {
                state.icons.put_icons(&theme, desk);
                finish_scale_warm(state);
            })
        });
        if !spawned {
            self.scale_warm = None;
        }
    }

    /// The layout changed: place the outputs, then refit the desktop to
    /// them (windows, taskbars, menus, wallpaper, icons, output management
    /// clients, pointer).
    pub fn update_layout(&mut self) {
        self.place_outputs();
        self.drop_unused_scales();
        self.warm_popup_parts();
        self.settle_moves();
        crate::layer::outputs_changed(self);
        self.refit_maximized();
        self.rescue_offscreen();
        self.snap_outputs_changed();
        self.close_dialogs_off_scene();
        self.keep_dialogs_inside();
        crate::tray::forget_output(self);
        crate::tray::sync_icon_sizes(self);
        crate::trayflyout::output_gone(self);
        crate::calendarview::output_gone(self);
        crate::displaysview::output_gone(self);
        crate::displays::outputs_changed(self);
        crate::taskpreview::output_gone(self);
        crate::snappickerview::output_gone(self);
        crate::tooltip::output_gone(self);
        self.relayout_desktop();
        // An output's scale can change under a window (wlr-randr --scale).
        // X11 menus and tooltips keep the client's own placement.
        let mapped: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.mapped && !w.override_redirect)
            .map(|w| w.id)
            .collect();
        for w in mapped {
            self.refit_scale(w);
        }
        self.sync_bars();
        crate::notifyview::reposition(self);
        for depth in 0..self.menus.len() {
            self.redraw_menu(depth);
        }
        self.refit_wallpaper();
        crate::outputmgmt::publish(self);
        let pos = self.pointer_location();
        let rects: Vec<Rect> = self
            .scene
            .outputs()
            .iter()
            .map(|o| self.layout_box(Some(o)))
            .collect();
        if let Some(p) = closest_point(&rects, (pos.x, pos.y))
            && p != (pos.x, pos.y)
        {
            self.repoint(p.into(), InputTime::now());
        }
        self.redraw_switcher();
        if self.flip_misplaced() {
            self.switcher_cancel();
        }
        self.input_layout_changed();
        self.redraw_all();
        // Last: the lid may take a panel in or out, which runs this again.
        crate::idle::outputs_changed(self);
    }

    fn place_outputs(&mut self) {
        let outputs = self.scene.outputs().to_vec();
        let asked: Vec<Slot> = outputs
            .iter()
            .map(|o| {
                let size = reprisescene::output_rect(o).map_or((0, 0), |r| (r.size.w, r.size.h));
                (placement(o).0.get().map(|p| (p.x, p.y)), size)
            })
            .collect();
        let before: Vec<(Output, Rect)> = outputs
            .iter()
            .filter_map(|o| Some((o.clone(), laid_out(o).0.get()?)))
            .collect();
        for (o, (x, y)) in outputs.iter().zip(place(&asked)) {
            if o.current_location() != Point::from((x, y)) {
                o.change_current_state(None, None, None, Some((x, y).into()));
            }
        }
        let after: Vec<(Output, Rect)> = outputs
            .iter()
            .map(|o| (o.clone(), self.layout_box(Some(o))))
            .collect();
        for (o, r) in &after {
            laid_out(o).0.set(Some(*r));
        }
        self.carry_windows(&before, &after);
    }

    /// Windows keep their place on an output that moved in the layout (an arrangement change, or
    /// an output left of it growing): each moves by its output's move, restore boxes included.
    fn carry_windows(&mut self, before: &[(Output, Rect)], after: &[(Output, Rect)]) {
        let windows: Vec<(WindowId, Rect)> = self
            .windows
            .iter()
            .filter(|w| w.mapped && !w.override_redirect)
            .map(|w| (w.id, w.frame_rect()))
            .collect();
        let frames: Vec<Rect> = windows.iter().map(|w| w.1).collect();
        let moves = carried(&frames, before, after);
        for ((w, r), d) in windows.into_iter().zip(moves) {
            let Some((dx, dy)) = d else {
                continue;
            };
            if let Some(win) = self.win_mut(w) {
                let shift = |b: &mut Option<Rect>| {
                    if let Some(b) = b.as_mut() {
                        b.x += dx;
                        b.y += dy;
                    }
                };
                shift(&mut win.saved);
                shift(&mut win.saved_fullscreen);
                shift(&mut win.snapped);
            }
            self.move_to(w, r.x + dx, r.y + dy);
        }
    }

    /// Centre every ordinary window whose frame centre is on no output on
    /// the output under the pointer. Maximized and fullscreen windows are
    /// `refit_maximized`'s.
    pub fn rescue_offscreen(&mut self) {
        let pos = self.pointer_location();
        let Some(target) = self.output_at(pos.x, pos.y) else {
            return;
        };
        let rects: Vec<Rect> = self
            .scene
            .outputs()
            .iter()
            .map(|o| self.layout_box(Some(o)))
            .collect();
        let lost: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.mapped && !w.maximized && !w.fullscreen && !w.override_redirect)
            .filter(|w| {
                let r = w.frame_rect();
                let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
                !rects.iter().any(|o| o.contains(cx, cy))
            })
            .map(|w| w.id)
            .collect();
        for w in lost {
            self.centre_window(w, Some(&target));
        }
    }

    /// Box of one output in layout coordinates, or of the whole layout for None.
    pub fn layout_box(&self, output: Option<&Output>) -> Rect {
        let rect = match output {
            Some(o) => reprisescene::output_rect(o),
            None => self
                .scene
                .outputs()
                .iter()
                .filter_map(reprisescene::output_rect)
                .reduce(|a, b| a.merge(b)),
        };
        rect.map_or_else(Rect::default, |b| {
            Rect::new(b.loc.x, b.loc.y, b.size.w, b.size.h)
        })
    }

    /// The pointer's position held on the outputs: itself on one, else the
    /// nearest point of any (a 1x1 box at 0,0 with none).
    pub fn clamp_to_layout(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        let rects: Vec<Rect> = self
            .scene
            .outputs()
            .iter()
            .map(|o| self.layout_box(Some(o)))
            .collect();
        closest_point(&rects, (pos.x, pos.y)).map_or_else(
            || crate::input::clamp_to(pos, Rectangle::from_size((1, 1).into())),
            Point::from,
        )
    }

    /// Output under a layout point, falling back to the first output.
    pub fn output_at(&self, lx: f64, ly: f64) -> Option<Output> {
        let outputs = self.scene.outputs();
        outputs
            .iter()
            .find(|o| reprisescene::output_rect(o).is_some_and(|r| r.to_f64().contains((lx, ly))))
            .or_else(|| outputs.iter().find(|o| self.layout_box(Some(o)).width > 0))
            .cloned()
    }

    /// What is left of an output for ordinary windows: what `layer::arrange`
    /// last recorded, else its box without the taskbar's strip. An empty
    /// result is a real answer, so a full-height exclusive zone is honoured.
    pub fn usable_area(&self, output: Option<&Output>) -> Rect {
        let b = self.layout_box(output);
        if b.width <= 0 {
            return b;
        }
        output
            .and_then(|o| self.layers_shell.usable(o, b))
            .unwrap_or_else(|| {
                let (theme, tb) = (&self.config.theme, &self.config.taskbar);
                let t = crate::barplace::thickness(
                    tb.position,
                    &theme.metrics,
                    &theme.taskbar_shape(),
                    tb.side_width,
                    b,
                );
                let p =
                    crate::barplace::place_hiding(b, tb.position, t, tb.auto_hide.then_some(0.0));
                crate::barplace::usable(b, &p)
            })
    }

    /// Lets the look forget the art it drew for scales no output runs at any more.
    fn drop_unused_scales(&self) {
        let used: Vec<i32> = self
            .scene
            .outputs()
            .iter()
            .map(|o| self.draw_scale_at(Some(o)))
            .collect();
        let warming = self.scale_warm.as_ref().map_or(&[][..], |w| &w.scales[..]);
        if let reprisekit::look::Look::Styled(s) = &self.config.theme.look {
            s.keep_scales(|n| used.contains(&n) || warming.contains(&n));
        }
    }

    /// Paints, on a job, the popups' art and a grown caption's buttons at each draw scale the
    /// outputs run at, so the first Alt+Tab, preview, Start menu or frame of a session, look, text
    /// size or scale finds them painted.
    fn warm_popup_parts(&mut self) {
        if self.config.theme.look.style().is_none() {
            return;
        }
        let mut scales: Vec<i32> = (self.scene.outputs().iter())
            .map(|o| self.draw_scale_at(Some(o)))
            .collect();
        scales.sort_unstable();
        scales.dedup();
        self.warm_user_pictures(&scales);
        let reprisekit::look::Look::Styled(style) = &self.config.theme.look else {
            return;
        };
        let sides = reprisekit::frame::caption_button_sides(&self.config.theme.metrics);
        let warm: Vec<_> = (scales.iter())
            .flat_map(|&n| {
                [
                    style.warm_parts(n, &POPUP_PARTS),
                    style.warm_button_sides(n, &sides),
                ]
            })
            .flatten()
            .collect();
        if warm.is_empty() {
            return;
        }
        self.spawn_job(move || {
            for w in &warm {
                w.paint();
            }
            Box::new(|_| ())
        });
    }

    pub fn draw_scale_at(&self, output: Option<&Output>) -> i32 {
        output.map_or(1, |o| {
            draw_scale(o.current_scale().fractional_scale() as f32)
        })
    }
}

/// The parts of the popups a key, click or hover opens, painted ahead by `warm_popup_parts`.
const POPUP_PARTS: [Part; 11] = [
    Part::SwitcherFrame,
    Part::SwitcherIconFrame,
    Part::SwitcherSelect,
    Part::SwitcherHot,
    Part::SwitcherShadow,
    Part::PreviewFrame,
    Part::PreviewThumb,
    Part::PreviewClose,
    Part::SnapFrame,
    Part::FlyoutShadow,
    Part::StartPanel,
];

/// Output changes waiting for a job painting the art of new draw scales.
pub struct ScaleWarm {
    /// The config before the first reload whose changes wait.
    old: Option<Box<crate::config::Config>>,
    /// Output management applies, in arrival order.
    heads: Vec<crate::outputmgmt::Pending>,
    /// Kept by `drop_unused_scales` while the job paints them.
    scales: Vec<i32>,
}

impl ScaleWarm {
    /// Holds an output management apply until the job ends.
    pub fn wait(&mut self, p: crate::outputmgmt::Pending) {
        self.heads.push(p);
    }
}

/// A scale warm's end: applies what waited, with the art of the new scales painted.
fn finish_scale_warm(state: &mut State) {
    let Some(w) = state.scale_warm.take() else {
        return;
    };
    // The reload rethemed at the old scale: lay out at the new one, which draws the bars,
    // frames and desktop, then what the layout leaves at the old scale.
    if let Some(old) = w.old
        && state.apply_outputs(&old, false)
    {
        state.update_layout();
        crate::configwatch::redraw_for_scale(state);
    }
    for p in w.heads {
        crate::outputmgmt::answer(state, p);
    }
}

/// The draw scales of output `scales` that are not in `used`, each once.
fn new_draw_scales(used: &[i32], scales: impl IntoIterator<Item = f32>) -> Vec<i32> {
    let mut out = Vec::new();
    for s in scales {
        let n = draw_scale(quarter_scale(s));
        if !used.contains(&n) && !out.contains(&n) {
            out.push(n);
        }
    }
    out
}

pub(crate) use repriseconf::config::quarter_scale;

/// A connector mode as `pick_mode` weighs it: size, refresh in mHz, preferred.
pub type ModeInfo = crate::config::OfferedMode;

/// Whether `mode` is the size `want` asks for.
fn want_matches(mode: &ModeInfo, want: &OutputMode) -> bool {
    mode.width == want.width && mode.height == want.height
}

/// The index of the mode to run among `modes`: the `[[outputs]]` mode when
/// the connector has it (`OutputMode::pick`), else the preferred mode,
/// else the first. None for an empty list.
pub fn pick_mode(modes: &[ModeInfo], want: Option<&OutputMode>) -> Option<usize> {
    if let Some(i) = want.and_then(|w| w.pick(modes)) {
        return Some(i);
    }
    modes
        .iter()
        .position(|m| m.preferred)
        .or((!modes.is_empty()).then_some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_move_with_the_output_their_centre_lies_on() {
        let before = [
            ("A", Rect::new(0, 0, 1920, 1080)),
            ("B", Rect::new(1920, 0, 4096, 2160)),
        ];
        // B goes left of A and A down beside its middle.
        let after = [
            ("A", Rect::new(4096, 531, 1920, 1080)),
            ("B", Rect::new(0, 0, 4096, 2160)),
        ];
        let frames = [
            Rect::new(100, 100, 500, 400),
            Rect::new(2100, 150, 500, 400),
            Rect::new(1500, 100, 500, 400),
            Rect::new(-900, 100, 500, 400),
        ];
        assert_eq!(
            carried(&frames, &before, &after),
            vec![Some((4096, 531)), Some((-1920, 0)), Some((4096, 531)), None],
            "on A, on B, straddling with its centre on A, on no output"
        );
        assert_eq!(
            carried(&frames, &before, &before),
            vec![None; 4],
            "nothing moved"
        );
        assert_eq!(
            carried(&frames, &before, &after[..1]),
            vec![Some((4096, 531)), None, Some((4096, 531)), None],
            "B left the layout: its windows are rescue_offscreen's"
        );
        assert_eq!(
            carried(&frames, &[], &after),
            vec![None; 4],
            "no layout before"
        );
    }

    #[test]
    fn chrome_draws_at_the_scale_rounded_up() {
        assert_eq!(draw_scale(1.0), 1);
        assert_eq!(draw_scale(1.25), 2);
        assert_eq!(draw_scale(1.5), 2);
        assert_eq!(draw_scale(2.0), 2);
        assert_eq!(
            draw_scale(2.000_000_2),
            2,
            "float noise over a whole number"
        );
        assert_eq!(draw_scale(2.25), 3);
        assert_eq!(draw_scale(0.5), 1);
        assert_eq!(draw_scale(9.0), 4);
        assert_eq!(draw_scale(f32::NAN), 1);
    }

    #[test]
    fn a_scale_change_warms_each_draw_scale_no_output_runs_at_once() {
        assert_eq!(
            new_draw_scales(&[1], [1.25, 1.5, 1.0, 2.2, 1.1]),
            vec![2, 3],
            "1.25 and 1.5 draw at 2, 2.2 runs at 2.25 and draws at 3, 1.1 runs at 1"
        );
        assert_eq!(new_draw_scales(&[1, 2], [2.0, 1.75]), Vec::<i32>::new());
    }

    fn mode(width: i32, height: i32, hz: i32, preferred: bool) -> ModeInfo {
        ModeInfo {
            width,
            height,
            refresh_mhz: hz * 1000,
            preferred,
        }
    }

    fn want(width: i32, height: i32, hz: Option<i32>) -> OutputMode {
        OutputMode {
            width,
            height,
            refresh_mhz: hz.map(|hz| hz * 1000),
        }
    }

    #[test]
    fn the_configured_mode_wins_over_the_preferred_one() {
        let modes = [
            mode(1024, 768, 60, true),
            mode(1920, 1080, 30, false),
            mode(1920, 1080, 60, false),
        ];
        assert_eq!(
            pick_mode(&modes, Some(&want(1920, 1080, Some(60)))),
            Some(2)
        );
        assert_eq!(
            pick_mode(&modes, Some(&want(1920, 1080, Some(29)))),
            Some(1)
        );
        assert_eq!(
            pick_mode(&modes, Some(&want(1920, 1080, None))),
            Some(2),
            "no refresh asked takes the highest"
        );
    }

    #[test]
    fn a_missing_mode_falls_back_to_the_preferred_then_the_first() {
        let modes = [mode(800, 600, 60, false), mode(1024, 768, 60, true)];
        assert_eq!(pick_mode(&modes, Some(&want(1920, 1080, None))), Some(1));
        assert_eq!(pick_mode(&modes, None), Some(1));
        assert_eq!(pick_mode(&modes[..1], None), Some(0));
        assert_eq!(pick_mode(&[], None), None);
    }

    #[test]
    fn scale_rounds_to_a_quarter_within_one_to_four() {
        assert_eq!(quarter_scale(1.0), 1.0);
        assert_eq!(quarter_scale(1.3), 1.25);
        assert_eq!(quarter_scale(1.5), 1.5);
        assert_eq!(quarter_scale(1.875), 2.0);
        assert_eq!(quarter_scale(0.5), 1.0);
        assert_eq!(quarter_scale(9.0), 4.0);
        assert_eq!(quarter_scale(f32::NAN), 1.0);
    }

    #[test]
    fn outputs_line_up_left_to_right_in_the_order_they_came() {
        assert_eq!(
            place(&[
                (None, (1920, 1080)),
                (None, (1280, 1024)),
                (None, (800, 600))
            ]),
            vec![(0, 0), (1920, 0), (3200, 0)]
        );
        assert_eq!(place(&[]), Vec::<(i32, i32)>::new());
    }

    #[test]
    fn an_output_comes_back_where_it_was_placed_unless_configured() {
        let mut places = Places::default();
        assert_eq!(
            places.back("HDMI-A-1", None, &[]),
            None,
            "never placed: automatic"
        );
        places.left(
            "HDMI-A-1",
            Some(Rectangle::new((3840, 0).into(), (1920, 1080).into())),
        );
        assert_eq!(places.back("HDMI-A-1", None, &[]), Some((3840, 0)));
        assert_eq!(places.back("HDMI-A-2", None, &[]), None, "per connector");
        assert_eq!(
            places.back("HDMI-A-1", Some((0, 1080)), &[]),
            Some((0, 1080)),
            "a configured position wins"
        );
        places.left("HDMI-A-1", None);
        assert_eq!(places.back("HDMI-A-1", None, &[]), None, "left automatic");
    }

    #[test]
    fn an_output_whose_spot_another_output_took_comes_back_automatic() {
        let mut places = Places::default();
        places.left(
            "HDMI-A-2",
            Some(Rectangle::new((1920, 0).into(), (1920, 1080).into())),
        );
        let beside = Rectangle::new((0, 0).into(), (1920, 1080).into());
        assert_eq!(
            places.back("HDMI-A-2", None, &[beside]),
            Some((1920, 0)),
            "an output touching the edge leaves the spot free"
        );
        let moved = Rectangle::new((2560, 0).into(), (1920, 1080).into());
        assert_eq!(
            places.back("HDMI-A-2", None, &[beside, moved]),
            None,
            "an output over part of the spot sends it to automatic placement"
        );
        assert_eq!(
            places.back("HDMI-A-2", Some((0, 1080)), &[moved]),
            Some((0, 1080)),
            "a configured position is kept even when taken"
        );
    }

    #[test]
    fn placed_outputs_stay_and_the_rest_follow_the_rightmost() {
        assert_eq!(
            place(&[(None, (1920, 1080)), (Some((-1280, -200)), (1280, 1024))]),
            vec![(0, -200), (-1280, -200)],
            "an automatic output follows the placed one's right edge, at its y"
        );
        assert_eq!(
            place(&[
                (Some((0, 0)), (1920, 1080)),
                (Some((1920, 100)), (1280, 1024)),
                (None, (800, 600)),
            ]),
            vec![(0, 0), (1920, 100), (3200, 100)],
            "after the rightmost placed one"
        );
    }

    #[test]
    fn the_pointer_stays_on_the_union_of_the_outputs() {
        // A tall output left, a short one right, bottoms not aligned: the
        // corner under the short one belongs to neither.
        let rects = [Rect::new(0, 0, 1000, 1000), Rect::new(1000, 0, 800, 600)];
        assert_eq!(
            closest_point(&rects, (1500.0, 300.0)),
            Some((1500.0, 300.0))
        );
        assert_eq!(
            closest_point(&rects, (1500.0, 900.0)),
            Some((1500.0, 599.0)),
            "below the short output: up onto its bottom row"
        );
        assert_eq!(closest_point(&rects, (-5.0, 2000.0)), Some((0.0, 999.0)));
        assert_eq!(closest_point(&rects, (5000.0, 10.0)), Some((1799.0, 10.0)));
        assert_eq!(closest_point(&[], (1.0, 1.0)), None);
    }
}
