//! Sheets arriving, sliding between slots, opening and leaving, and their frame tick.

use reprisekit::anim::{Ease, Tween};
use reprisekit::controls::notifysheet::{
    RISE, drip_bead_at, drip_bead_share, drip_corner, drip_ms, drip_pose, pose,
};
use reprisekit::style::SheetShape;
use reprisekit::taskbar::Edge;
use reprisescene::{Input, NodeId};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::{Logical, Rectangle};
use std::rc::Rc;
use std::time::Instant;

use crate::compositor::{DeskScene, State};
use crate::config::EffectsConfig;
use crate::glass::GlassCtx;
use crate::raster::Rect;

use super::clock::{hold_clocks, rearm};
use super::layout::Drawn;
use super::paint::{lay_stack, redraw};
use super::pointer::{hold_at, rehover};
use super::warm::warm_openings;
use super::{Balloon, NOTIFY_TICK, NotifyView, sheets};

/// A sheet's way in, out and between slots, and a Droplet's opening.
#[derive(Clone, Copy, Debug)]
pub(super) struct Motion {
    /// 0 hidden, 1 at rest.
    pub(super) k: Tween,
    /// Logical px in y from the rest position.
    offset: Tween,
    /// 0 collapsed, 1 expanded.
    pub(super) open: Tween,
    /// `k` runs a Droplet's arrival, posed by `drip_pose`.
    pub(super) drip: bool,
}

/// A Droplet opens while the pointer is on it or its reply row is open.
pub(super) fn open_target(hovered: bool, replying: bool) -> f64 {
    if hovered || replying { 1.0 } else { 0.0 }
}

impl Motion {
    pub(super) fn arrive(e: &EffectsConfig, now: Instant) -> Motion {
        let ms = if e.opens() { e.window_open_ms } else { 0 };
        Motion {
            k: Tween::new(0.0, 1.0, now, ms, Ease::OutCubic),
            ..Motion::settled_at(now)
        }
    }

    /// A Droplet's arrival; the overshoot lives in `drip_pose`, so `k` runs linear.
    pub(super) fn drip(e: &EffectsConfig, now: Instant) -> Motion {
        let ms = if e.opens() {
            drip_ms(e.window_open_ms)
        } else {
            0
        };
        Motion {
            k: Tween::new(0.0, 1.0, now, ms, Ease::Linear),
            drip: true,
            ..Motion::settled_at(now)
        }
    }

    pub(super) fn settled_at(now: Instant) -> Motion {
        Motion {
            k: Tween::new(1.0, 1.0, now, 0, Ease::OutCubic),
            offset: Tween::new(0.0, 0.0, now, 0, Ease::OutCubic),
            open: Tween::new(0.0, 0.0, now, 0, Ease::OutCubic),
            drip: false,
        }
    }

    pub(super) fn leave(&mut self, e: &EffectsConfig, now: Instant) {
        let ms = if e.closes() { e.window_close_ms } else { 0 };
        self.k.retarget(0.0, now, ms);
        self.drip = false;
        let open = self.open.value(now);
        self.open = Tween::new(open, open, now, 0, Ease::OutCubic);
    }

    /// Opens toward `to` over the window open or close time; false when already headed there.
    pub(super) fn aim_open(&mut self, to: f64, e: &EffectsConfig, now: Instant) -> bool {
        if self.open.target() == to {
            return false;
        }
        let ms = match (to > self.open.value(now), e.opens(), e.closes()) {
            (true, true, _) => e.window_open_ms,
            (false, _, true) => e.window_close_ms,
            _ => 0,
        };
        self.open.retarget(to, now, ms);
        true
    }

    /// The rest moved by `-dy`: the sheet starts where it was and eases to its new rest.
    pub(super) fn slide_by(&mut self, dy: f64, now: Instant, ms: u32) {
        self.offset = Tween::new(self.offset(now) + dy, 0.0, now, ms, Ease::OutCubic);
    }

    pub(super) fn offset(&self, now: Instant) -> f64 {
        self.offset.value(now)
    }

    /// Arrived and at its slot; a Droplet may still be opening.
    pub(super) fn placed(&self, now: Instant) -> bool {
        self.k.settled(now) && self.offset.settled(now)
    }

    pub(super) fn settled(&self, now: Instant) -> bool {
        self.placed(now) && self.open.settled(now)
    }
}

/// A sheet's slot moved from `was` to `now_at`, each (top, height) logical: it slides by the move
/// of its anchored edge, the top in a stack growing down, else the bottom.
pub(super) fn follow_slot(
    m: &mut Motion,
    was: (i32, i32),
    now_at: (i32, i32),
    grows_down: bool,
    now: Instant,
    ms: u32,
) {
    let edge = |(y, h): (i32, i32)| if grows_down { y } else { y + h };
    let dy = edge(was) - edge(now_at);
    if dy != 0 {
        m.slide_by(f64::from(dy), now, ms);
    }
}

/// Points each Droplet's opening at its [`open_target`]; true when one turned.
pub(super) fn aim_open(state: &mut State) -> bool {
    if !sheets(state) || state.config.glass.notification.shape() != SheetShape::Droplet {
        return false;
    }
    aim_stack(
        &mut state.notify.stack,
        &state.config.effects,
        Instant::now(),
    )
}

pub(super) fn aim_stack(stack: &mut [Balloon], e: &EffectsConfig, now: Instant) -> bool {
    let mut turned = false;
    for b in stack {
        let to = open_target(b.pointer_in, b.reply.is_some());
        turned |= b.motion.aim_open(to, e, now);
    }
    turned
}

/// Places `b` as its motion stands at `now` by the bar on `edge`: posed about its rest, faded, its
/// glass following. A Droplet arriving swells from a bead at the tray-side cap.
pub(super) fn pose_balloon(
    scene: &mut DeskScene,
    glass: &Rc<GlassCtx>,
    b: &mut Balloon,
    now: Instant,
    edge: Option<Edge>,
    scale_from: f64,
    bounds: Option<Rectangle<i32, Logical>>,
) {
    let Some(drawn) = &b.drawn else {
        return;
    };
    let s = b.scale;
    let ((ox, oy), (w, h), (fw, fh)) = (drawn.origin(), drawn.size(), drawn.footprint());
    let (ox, oy) = (ox / s, oy / s);
    let y = b.at.1 + b.motion.offset(now).round() as i32;
    let rest = Rect::new(b.at.0 - ox, y - oy, w / s, h / s);
    let k = b.motion.k.value(now);
    let ratio = |to: i32, from: i32| f64::from(to) / f64::from(from.max(1));
    let times = |v: i32, k: f64| (f64::from(v) * k).round() as i32;
    let tray_right = edge != Some(Edge::Left);
    let (r, inset, alpha, pane, bead) = if b.motion.drip && k < 1.0 {
        let fp = Rect::new(b.at.0, y, fw / s, fh / s);
        let (p, alpha) = drip_pose(k, fp, tray_right);
        let (kx, ky) = (ratio(p.width, fp.width), ratio(p.height, fp.height));
        let inset = (times(ox, kx), times(oy, ky));
        let r = Rect::new(
            p.x - inset.0,
            p.y - inset.1,
            times(rest.width, kx),
            times(rest.height, ky),
        );
        let pane = b
            .pane
            .as_ref()
            .map(|_| crate::glass::sheet_pane((p.width, p.height), drip_corner(p.height)));
        let bead = (drip_bead_at(p, tray_right), drip_bead_share(k), p);
        (r, inset, alpha, pane, Some(bead))
    } else {
        let toward_tray = if edge == Some(Edge::Top) { -1 } else { 1 };
        let (r, alpha) = pose(k, rest, RISE, toward_tray, scale_from);
        let inset = (
            times(ox, ratio(r.width, rest.width)),
            times(oy, ratio(r.height, rest.height)),
        );
        let pane = b
            .pane
            .as_ref()
            .map(|pane| crate::windowanim::scale_pane(pane, rest, r));
        (r, inset, alpha, pane, None)
    };
    let at = (r.x + inset.0, r.y + inset.1);
    scene.set_position(b.tree, at.0, at.1);
    scene.set_clip(b.tree, clip_to(bounds, at));
    scene.set_position(b.buffer, -inset.0, -inset.1);
    scene.buffer_set_dest_size(b.buffer, Some((r.width, r.height).into()));
    scene.set_opacity(b.tree, alpha);
    let share = bead.map_or(0.0, |(_, share, _)| share);
    scene.set_opacity(b.buffer, 1.0 - share);
    match (&b.bead, bead) {
        (Some(bd), Some((q, share, p))) if share > 0.0 => {
            let k = f64::from(q.height) / f64::from(bd.body.height.max(1));
            scene.set_position(
                bd.buffer,
                q.x - p.x - times(bd.body.x, k),
                q.y - p.y - times(bd.body.y, k),
            );
            let size = (times(bd.size.0, k), times(bd.size.1, k));
            scene.buffer_set_dest_size(bd.buffer, Some(size.into()));
            scene.set_opacity(bd.buffer, share);
        }
        (Some(_), _) => {
            if let Some(gone) = b.bead.take() {
                scene.destroy(gone.buffer);
            }
        }
        (None, _) => {}
    }
    if let (Some(backdrop), Some(pane)) = (b.backdrop.as_mut(), pane) {
        backdrop.set(scene, glass, b.glass_tree, Some(pane));
    }
}

/// `bounds` (layout) as a clip for a tree at `at`: a sheet's shadow stops at its output's edge
/// rather than spilling onto the output beside it.
pub(super) fn clip_to(
    bounds: Option<Rectangle<i32, Logical>>,
    at: (i32, i32),
) -> Option<Rectangle<i32, Logical>> {
    bounds.map(|o| Rectangle::new((o.loc.x - at.0, o.loc.y - at.1).into(), o.size))
}

pub(super) fn still(v: &NotifyView, now: Instant) -> bool {
    v.stack
        .iter()
        .chain(&v.leaving)
        .all(|b| b.motion.settled(now))
}

/// A tick not `settled` at its start runs once more, so the next one sees the rest and re-checks
/// the pointer; one settled runs on while a Droplet it opened at `later` moves.
pub(super) fn keeps_ticking(v: &NotifyView, settled: bool, later: Instant) -> bool {
    !settled || !still(v, later)
}

pub(super) fn arm_tick(state: &mut State) {
    if state.notify.tick_timer.is_some() || still(&state.notify, Instant::now()) {
        return;
    }
    match state
        .handle
        .insert_source(Timer::from_duration(NOTIFY_TICK), |_, _, state| {
            if tick(state) {
                TimeoutAction::ToDuration(NOTIFY_TICK)
            } else {
                state.notify.tick_timer = None;
                TimeoutAction::Drop
            }
        }) {
        Ok(t) => state.notify.tick_timer = Some(t),
        Err(e) => log::warn!("notification motion timer: {}", e.error),
    }
}

/// Redraws the Droplets opening, with the stack following their height and the hold the still
/// pointer, poses every sheet and takes down those done leaving; true while any still moves.
/// Once all rest, the still pointer is re-checked as on a move.
pub(super) fn tick(state: &mut State) -> bool {
    let now = Instant::now();
    let v = &state.notify;
    if v.stack
        .iter()
        .any(|b| b.motion.open.value(now) != b.open_drawn)
    {
        if lay_stack(state, true) {
            let p = state.pointer_location();
            if hold_at(&mut state.notify, p.x, p.y, now) && hold_clocks(state) {
                rearm(state);
            }
        }
        if state
            .notify
            .stack
            .iter()
            .all(|b| b.motion.open.settled(now))
        {
            crate::access::touch(state);
        }
    }
    let (edge, scale_from) = (state.notify.edge, state.config.effects.window_scale);
    let State {
        notify,
        scene,
        glass,
        ..
    } = state;
    let bounds = notify.output.as_ref().and_then(reprisescene::output_rect);
    for b in notify.stack.iter_mut().chain(notify.leaving.iter_mut()) {
        pose_balloon(scene, glass, b, now, edge, scale_from, bounds);
    }
    let (gone, kept): (Vec<Balloon>, Vec<Balloon>) = std::mem::take(&mut notify.leaving)
        .into_iter()
        .partition(|b| b.motion.settled(now));
    notify.leaving = kept;
    restack(scene, &notify.stack, &notify.leaving, now);
    for b in gone {
        destroy(state, b);
    }
    redraw(state);
    let settled = still(&state.notify, now);
    if settled {
        let p = state.pointer_location();
        if hold_at(&mut state.notify, p.x, p.y, now) && hold_clocks(state) {
            rearm(state);
        }
        // A Droplet this opens moves on, and `paint` leaves the running tick armed.
        rehover(state);
    }
    let moving = keeps_ticking(&state.notify, settled, Instant::now());
    if !moving {
        warm_openings(state);
    }
    moving
}

/// Sheet `b` leaves its slot: it eases out without the pointer, then goes. A balloon goes at once.
pub(super) fn leave(state: &mut State, mut b: Balloon) {
    let now = Instant::now();
    b.motion.leave(&state.config.effects, now);
    if !matches!(b.drawn, Some(Drawn::Sheet(_))) || b.motion.settled(now) {
        destroy(state, b);
        return;
    }
    state.scene.set_input(b.buffer, Input::Passthrough);
    state.notify.leaving.push(b);
    let v = &state.notify;
    restack(&mut state.scene, &v.stack, &v.leaving, now);
    crate::autohideview::poke(state);
    crate::access::touch(state);
    arm_tick(state);
}

/// A sheet's band in the overlay, lowest first: leaving ones draw under arriving ones, and both
/// under those resting or sliding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Band {
    Leaving,
    Arriving,
    Resting,
}

pub(super) fn band(m: &Motion, leaving: bool, now: Instant) -> Band {
    if leaving {
        Band::Leaving
    } else if !m.k.settled(now) {
        Band::Arriving
    } else {
        Band::Resting
    }
}

/// Orders the trees of `stack` and `leaving` by `band` within the overlay slots they already
/// hold, keeping their scene order within a band. A sheet's blur pane lives in its tree.
pub(super) fn restack(scene: &mut DeskScene, stack: &[Balloon], leaving: &[Balloon], now: Instant) {
    let mut trees: Vec<(Band, usize, NodeId)> = stack
        .iter()
        .map(|b| (band(&b.motion, false, now), b.tree))
        .chain(leaving.iter().map(|b| (Band::Leaving, b.tree)))
        .filter_map(|(band, tree)| {
            let at = scene
                .children(scene.parent(tree)?)
                .iter()
                .position(|c| *c == tree.into())?;
            Some((band, at, tree.into()))
        })
        .collect();
    trees.sort_by_key(|&(band, at, _)| (band, at));
    let order: Vec<NodeId> = trees.into_iter().map(|t| t.2).collect();
    scene.reorder_in_place(&order);
}

pub(super) fn drop_leaving(state: &mut State) {
    for b in std::mem::take(&mut state.notify.leaving) {
        destroy(state, b);
    }
}

pub(super) fn destroy(state: &mut State, mut b: Balloon) {
    if let Some(mut gone) = b.backdrop.take() {
        gone.set(&mut state.scene, &state.glass, b.glass_tree, None);
    }
    state.scene.destroy(b.tree);
    redraw(state);
    crate::autohideview::poke(state);
    crate::access::touch(state);
}
