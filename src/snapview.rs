//! Window snapping on the scene: the drag outline, snap groups and their shared edges, and
//! `snap_to_zone`.
//!
//! The outline is `snapframe`'s four strips (or a selection box under Glass) on the overlay
//! layer; it never takes the pointer and stays out of what glass blurs.

mod groups;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use reprisekit::classic::frame::{Edges, FrameGeometry};
use reprisekit::style::SelectionBox;
use reprisescene::{Input, RectId, TreeId};
use smithay::output::Output;

use crate::compositor::State;
use crate::input::Interaction;
use crate::output::upload_scaled;
use crate::raster::Rect;
use crate::snap::{ActiveLayout, Dir, KeyStep, Recents, Splits, Suggestion, Target, ZoneLayout};
use crate::theme::Metrics;
use crate::window::{Window, WindowId};

use groups::followed;
use groups::partner_sides;

/// Windows snapped together into one layout on one output. A drag sizing one along a shared
/// edge sizes the others too.
#[derive(Clone, Debug)]
struct Group {
    id: u64,
    output: String,
    desktop: usize,
    layout: ZoneLayout,
    /// Zones as this group's own resizes left them.
    splits: Splits,
    /// (zone, window)
    members: Vec<(usize, WindowId)>,
}

/// `Wm::snap`: the outline, snap groups, per-desktop active layouts and recent arrangements.
#[derive(Default)]
pub struct Snapping {
    tree: Option<TreeId>,
    /// What the outline shows, so an unchanged target is not redrawn on every motion.
    shown: Option<Rect>,
    groups: Vec<Group>,
    next_group: u64,
    /// Active layout per output name and desktop; dropped when no group there is left.
    active: HashMap<(String, usize), ActiveLayout>,
    /// The desktop on screen: whose active layouts count and where a new group forms.
    pub desktop: usize,
    pub recents: Recents,
    partners: Vec<Partner>,
    /// The grouped window a drag sizes: its start frame, dragged sides and last frame given.
    resize: Option<(WindowId, Rect, Edges)>,
    last: Option<Rect>,
    /// A move's window, group, zone and active layout at its start, for Escape.
    move_undo: Option<MoveUndo>,
}

/// Where a suggestion's pick puts its windows.
#[derive(Clone, Debug, PartialEq)]
pub struct SuggestionPick {
    pub zones: Vec<Rect>,
    pub members: Vec<(WindowId, Rect)>,
}

/// Where a Super and arrow step puts its window.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyLanding {
    pub zones: Vec<Rect>,
    pub swapped: Option<(WindowId, usize)>,
}

#[derive(Clone, Debug)]
struct MoveUndo {
    window: WindowId,
    group: Group,
    zone: usize,
    active: Option<ActiveLayout>,
}

/// A window sized along with the one a drag sizes.
#[derive(Clone, Copy, Debug)]
struct Partner {
    window: WindowId,
    /// The dragged window's side whose line it rides, and its own side on that line.
    side: Edges,
    edge: Edges,
    start: Rect,
}

/// A size that moved its window: the splits (vertical, from, to) its followed sides moved, and
/// whether a side no member followed moved too, which took the window off its zone.
#[derive(Debug, PartialEq)]
struct SizeEnd {
    window: WindowId,
    start: Rect,
    moves: Vec<(bool, i32, i32)>,
    off_zone: bool,
}

/// Whether a window is tiled: in a snap group, neither maximized nor fullscreen.
fn tiled(maximized: bool, fullscreen: bool, group: Option<u64>) -> bool {
    group.is_some() && !maximized && !fullscreen
}

/// The client size that makes `win`'s frame the size of `r`, within the client's limits.
fn client_for(m: &Metrics, win: &Window, r: Rect) -> (i32, i32) {
    let client = FrameGeometry::client_size_for_total(m, r.width, r.height, false, win.chrome());
    crate::snap::commit_size(client, win.size_limits())
}

/// Five rects under `tree` for [`place_selection_box`], disabled until placed.
pub(crate) fn selection_box_nodes(
    scene: &mut crate::compositor::DeskScene,
    tree: TreeId,
) -> [RectId; 5] {
    std::array::from_fn(|_| {
        let n = scene.rect_create(tree, (0, 0), [0.0; 4]);
        scene.set_input(n, Input::Passthrough);
        scene.set_enabled(n, false);
        n
    })
}

/// Show `b` over `r` in `nodes`: the translucent fill and four 1 px lines; unneeded nodes are
/// disabled.
pub(crate) fn place_selection_box(
    scene: &mut crate::compositor::DeskScene,
    nodes: &[RectId; 5],
    b: SelectionBox,
    r: Rect,
) {
    let rects = b.rects(r, 1);
    for (i, &n) in nodes.iter().enumerate() {
        let Some(&(q, color, alpha)) = rects.get(i) else {
            scene.set_enabled(n, false);
            continue;
        };
        let a = f32::from(alpha) / 255.0;
        let [cr, cg, cb, _] = color.as_f32();
        scene.rect_set_color(n, [cr * a, cg * a, cb * a, a]);
        scene.rect_set_size(n, (q.width, q.height));
        scene.set_position(n, q.x, q.y);
        scene.raise_to_top(n);
        scene.set_enabled(n, true);
    }
}

impl State {
    /// Outline `target`, or clear for None. Runs on every settled motion; an unchanged target
    /// costs nothing.
    fn show_snap_outline(&mut self, target: Option<Rect>) {
        let Some(r) = target else {
            self.hide_snap_outline();
            return;
        };
        if self.wm.snap.shown == Some(r) && self.wm.snap.tree.is_some() {
            return;
        }
        self.hide_snap_outline();
        self.wm.snap.tree = self.outline_tree(r, reprisekit::classic::snapframe::BAND);
        self.wm.snap.shown = Some(r);
        self.redraw_all();
    }

    /// A tree on the overlay layer at `r` holding the outline `band` px wide, or the selection
    /// box. None for an empty `r`; the caller destroys it.
    pub(crate) fn outline_tree(&mut self, r: Rect, band: i32) -> Option<TreeId> {
        if r.width <= 0 || r.height <= 0 {
            return None;
        }
        let tree = self.scene.tree_create(self.layers.overlay);
        crate::glass::leave_out(&mut self.scene, tree);
        self.scene.set_position(tree, r.x, r.y);
        if let Some(b) = reprisekit::look::widgets::selection_box(&self.config.theme.look) {
            let nodes = selection_box_nodes(&mut self.scene, tree);
            place_selection_box(
                &mut self.scene,
                &nodes,
                b,
                Rect::new(0, 0, r.width, r.height),
            );
            return Some(tree);
        }
        let out = self.output_at(r.x as f64 + 0.5, r.y as f64 + 0.5);
        let scale = self.draw_scale_at(out.as_ref());
        // Four small buffers, not one the size of the rect.
        for ((x, y), strip) in
            reprisekit::classic::snapframe::strips(r.width, r.height, band, scale)
        {
            let sb = self.scene.buffer_create(tree);
            self.scene.set_input(sb, Input::Passthrough);
            self.scene.set_position(sb, x, y);
            upload_scaled(&mut self.scene, sb, &strip, scale);
        }
        Some(tree)
    }

    pub fn hide_snap_outline(&mut self) {
        if let Some(tree) = self.wm.snap.tree.take() {
            self.scene.destroy(tree);
            self.redraw_all();
        }
        self.wm.snap.shown = None;
    }

    /// Fill `target` with `w`: the client size that makes the frame the zone's size, within the
    /// client's limits.
    fn snap_commit(&mut self, w: WindowId, target: Rect) {
        let Some(win) = self.win(w) else {
            return;
        };
        let (cw, ch) = client_for(&self.config.theme.metrics, win, target);
        let frame = win.frame_rect();
        if let Some(win) = self.win_mut(w)
            && win.snapped.is_none()
        {
            win.snapped = Some(frame);
        }
        self.move_to(w, target.x, target.y);
        // An anchor left from a drag-resize would move the window off the zone on commit.
        if let Some(win) = self.win_mut(w) {
            win.resize_anchor = None;
        }
        // Same configure as the size, so a client that rounds to a grid takes the zone's size
        // exactly.
        self.set_tiled(w, true);
        self.request_size(w, cw, ch);
    }

    /// Set or clear the four tiled states in `w`'s pending xdg configure; whether they changed.
    fn set_tiled(&self, w: WindowId, on: bool) -> bool {
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as S;
        let Some(t) = self.win(w).and_then(crate::window::Window::xdg) else {
            return false;
        };
        t.with_pending_state(|s| {
            let mut changed = false;
            for state in [S::TiledLeft, S::TiledRight, S::TiledTop, S::TiledBottom] {
                changed |= if on {
                    s.states.set(state)
                } else {
                    s.states.unset(state)
                };
            }
            changed
        })
    }

    /// Windows tiled while they hold a snap zone; ones that left one hear so. Runs with
    /// `settle_moves`.
    pub(crate) fn sync_tiled(&mut self) {
        let want: Vec<(WindowId, bool)> = (self.windows.iter())
            .filter(|x| x.xdg().is_some())
            .map(|x| {
                (
                    x.id,
                    tiled(x.maximized, x.fullscreen, self.wm.snap.group_of(x.id)),
                )
            })
            .collect();
        for (w, on) in want {
            if self.set_tiled(w, on) {
                self.send_configure(w);
            }
        }
    }

    /// A drag starts sizing `w` along `edges` from `start`: windows of its group with a side on
    /// a dragged side's line come along.
    pub(crate) fn begin_group_resize(&mut self, w: WindowId, edges: Edges, start: Rect) {
        self.end_group_resize();
        self.widen_group_resize(w, edges, start);
    }

    /// A keyboard size of `w` that began at `start` now sizes `edges`: the sides it picks one
    /// by one bring their followers along, as a drag's do.
    pub(crate) fn widen_group_resize(&mut self, w: WindowId, edges: Edges, start: Rect) {
        let had = (self.wm.snap.resize)
            .filter(|r| r.0 == w)
            .map_or(Edges::NONE, |r| r.2);
        // Tracked for an ungrouped window too: `end_group_resize` sees whether it left its zone.
        self.wm.snap.resize = Some((w, start, edges));
        let Some(g) = self.wm.snap.group_of(w) else {
            return;
        };
        let mut others: Vec<(WindowId, Rect)> = Vec::new();
        for win in &self.windows {
            let free = win.mapped && !win.minimized && !win.maximized && !win.fullscreen;
            if win.id == w || !free || self.wm.snap.group_of(win.id) != Some(g) {
                continue;
            }
            if !self.on_current_desktop(win.id) {
                continue;
            }
            others.push((win.id, win.frame_rect()));
        }
        let frames: Vec<Rect> = others.iter().map(|o| o.1).collect();
        let mut partners = Vec::new();
        for side in crate::snap::sides(edges).filter(|&s| !had.contains(s)) {
            for (i, edge) in crate::snap::line_followers(start, side, &frames) {
                partners.push(Partner {
                    window: others[i].0,
                    side,
                    edge,
                    start: others[i].1,
                });
            }
        }
        for p in &partners {
            self.set_resizing(p.window, true);
        }
        self.wm.snap.partners.extend(partners);
    }

    /// `r`, the drag's frame for its window, held back so followers keep their smallest size.
    pub(crate) fn hold_for_partners(&self, mut r: Rect) -> Rect {
        for p in &self.wm.snap.partners {
            let Some(win) = self.win(p.window) else {
                continue;
            };
            let limits = win.size_limits();
            let pad = (
                win.geom.total.0 - win.geom.client.width,
                win.geom.total.1 - win.geom.client.height,
            );
            let min = (limits.min.0 + pad.0, limits.min.1 + pad.1);
            r = crate::snap::hold_for(r, p.side, p.start, p.edge, min);
        }
        r
    }

    /// Move each follower's sides onto `r`; left and top sides are placed once the client's size
    /// lands.
    pub(crate) fn drag_partners(&mut self, r: Rect) {
        self.wm.snap.last = Some(r);
        for (window, start, sides) in partner_sides(&self.wm.snap.partners) {
            let (next, edges) = followed(start, &sides, r);
            let Some(win) = self.win(window) else {
                continue;
            };
            let (cw, ch) = client_for(&self.config.theme.metrics, win, next);
            let anchored = edges.intersects(Edges::LEFT | Edges::TOP);
            if let Some(win) = self.win_mut(window) {
                win.resize_anchor = anchored.then_some(crate::window::ResizeAnchor {
                    edges,
                    right: start.right(),
                    bottom: start.bottom(),
                });
            }
            self.request_size(window, cw, ch);
        }
    }

    /// Escape on a keyboard size: the followers go back to where they started, and the group's
    /// zones stay.
    pub(crate) fn cancel_group_resize(&mut self) {
        let partners = std::mem::take(&mut self.wm.snap.partners);
        self.wm.snap.resize = None;
        self.wm.snap.last = None;
        for (window, start, _) in partner_sides(&partners) {
            self.set_resizing(window, false);
            let Some(win) = self.win(window) else {
                continue;
            };
            let (cw, ch) = client_for(&self.config.theme.metrics, win, start);
            self.move_to(window, start.x, start.y);
            self.request_size(window, cw, ch);
        }
    }

    /// The size ended: followers stop, the group's zones take the moved splits, and a side no
    /// member followed takes the sized window off its zone: out of the group, with no size from
    /// before the snap. A window still in its zone keeps that size, followers included.
    pub(crate) fn end_group_resize(&mut self) {
        let (followers, end) = self.wm.snap.end_resize();
        for w in followers {
            self.set_resizing(w, false);
        }
        let Some(end) = end else {
            return;
        };
        let start = end.start;
        let centre = (start.x + start.width / 2, start.y + start.height / 2);
        if let Some(out) = self.output_at(f64::from(centre.0), f64::from(centre.1)) {
            let work = self.usable_area(Some(&out));
            self.wm.snap.move_splits(end.window, work, &end.moves);
        }
        if end.off_zone {
            self.wm.snap.leave_group(end.window);
            if let Some(win) = self.win_mut(end.window) {
                win.snapped = None;
            }
        }
    }

    /// After a settled motion: the snap bar follows the pointer, and the bar tile or drop under
    /// it becomes the release's.
    pub fn track_snap(&mut self, w: WindowId) {
        let pos = self.pointer_location();
        let Interaction::Move { mut armed, .. } = self.interaction else {
            return;
        };
        // Over a bar tile the tile's zone is outlined, and nothing drops.
        let hot = crate::snapbarview::motion(self, w, pos.x, pos.y);
        let out = self.output_exactly_at(pos.x, pos.y);
        let bar = (out.as_ref()).and_then(|o| crate::snapbarview::bar_rect(self, o));
        let target = match hot {
            Some(_) => None,
            None => self.move_target(w, pos.x, pos.y, bar, &mut armed),
        };
        if let Interaction::Move { snap, armed: a, .. } = &mut self.interaction {
            *snap = target.map(|(d, _)| d);
            *a = armed;
        }
        self.show_snap_outline(hot.or(target.map(|(_, r)| r)));
    }

    /// The drop and outline rect a release of `w` at (`cx`, `cy`) makes; None unless drag
    /// snapping is on and `w` is snappable.
    fn move_target(
        &mut self,
        w: WindowId,
        cx: f64,
        cy: f64,
        bar: Option<Rect>,
        armed: &mut bool,
    ) -> Option<(Target, Rect)> {
        if !self.config.window_snapping.enabled || !self.win(w).is_some_and(|x| x.snappable()) {
            return None;
        }
        let out = self.output_exactly_at(cx, cy)?;
        let work = self.usable_area(Some(&out));
        let (x, y) = (cx.floor() as i32, cy.floor() as i32);
        let live = self.snap_live();
        let live = |m| live.contains(&m);
        let shift = (self.seat.get_keyboard()).is_some_and(|k| k.modifier_state().shift);
        let outputs: Vec<Rect> = (self.scene.outputs().iter())
            .map(|o| self.layout_box(Some(o)))
            .collect();
        let outer = crate::snap::outer_sides(self.layout_box(Some(&out)), &outputs, x, y);
        let band = self.config.window_snapping.distance;
        (self.wm.snap).outlined_target(
            &out.name(),
            work,
            band,
            shift,
            bar,
            outer,
            armed,
            x,
            y,
            &live,
        )
    }

    /// The release of a move: the snap bar's lit tile, else the drop. A restore afterwards goes
    /// to `restore`.
    pub fn end_move(&mut self, w: WindowId, target: Option<Target>, restore: Rect) {
        self.wm.snap.end_move();
        self.hide_snap_outline();
        if let Some((out, cell, zone)) = crate::snapbarview::take_pick(self) {
            crate::snappickerview::pick(self, w, &out, &cell, zone);
        } else if let Some(d) = target {
            self.release_on(w, d);
        } else {
            return;
        }
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if win.maximized {
            win.saved = Some(restore);
        } else if win.snapped.is_some() {
            win.snapped = Some(restore);
        }
    }

    /// Let go of `w` on `target`: maximize, a quarter (becomes the active layout) or a zone of
    /// the active layout.
    fn release_on(&mut self, w: WindowId, target: Target) {
        let p = self.pointer_location();
        let Some(out) = self.output_exactly_at(p.x, p.y) else {
            return;
        };
        if target == Target::Maximize {
            self.set_maximized(w, true);
            return;
        }
        let work = self.usable_area(Some(&out));
        let live = self.snap_live();
        let live = |m| live.contains(&m);
        if let Some((layout, explicit, zone)) =
            (self.wm.snap).target_layout(&out.name(), work, target, &live)
        {
            self.apply_layout(&out, layout, explicit, w, zone);
        }
    }

    pub(crate) fn output_exactly_at(&self, x: f64, y: f64) -> Option<Output> {
        self.scene
            .outputs()
            .iter()
            .find(|o| reprisescene::output_rect(o).is_some_and(|r| r.to_f64().contains((x, y))))
            .cloned()
    }

    /// `out`'s active zones over its usable area and whether a pick set them.
    fn active_zones(&self, out: &Output) -> (Vec<Rect>, bool) {
        let work = self.usable_area(Some(out));
        let live = self.snap_live();
        let live = |m| live.contains(&m);
        let (_, explicit, zones) = self.wm.snap.active_target(&out.name(), work, &live);
        (zones, explicit)
    }

    /// Snap `w` into zone `zone` of `layout` on `out`, join or start its group, and offer the
    /// group's free zones to other windows (snap assist).
    pub fn apply_layout(
        &mut self,
        out: &Output,
        layout: ZoneLayout,
        explicit: bool,
        w: WindowId,
        zone: usize,
    ) {
        let work = self.usable_area(Some(out));
        let name = out.name();
        let live = self.snap_live();
        let Some(zones) = (self.wm.snap).apply(w, &name, layout, explicit, true, work, zone, |m| {
            live.contains(&m)
        }) else {
            return;
        };
        self.snap_to_zone(w, zones[zone]);
        let free = self.free_zones(w, &zones);
        self.open_snap_assist(w, free);
    }

    /// Snap `w` into zone `zone` of suggestion `s` on `out` and bring its other snappable
    /// members back into their zones in one group.
    pub fn apply_suggestion(&mut self, out: &Output, s: &Suggestion, w: WindowId, zone: usize) {
        let work = self.usable_area(Some(out));
        let live = self.snap_live();
        // One that cannot fill the zone neither joins nor claims it.
        let fills =
            |m: WindowId| self.win(m).is_some_and(|x| x.snappable()) && self.on_current_desktop(m);
        let fills: Vec<WindowId> = (s.filled.iter())
            .map(|f| f.1)
            .filter(|&m| fills(m))
            .collect();
        let Some(SuggestionPick { zones, members }) = (self.wm.snap).apply_suggestion(
            w,
            &out.name(),
            s,
            work,
            zone,
            |m| live.contains(&m),
            |m| fills.contains(&m),
        ) else {
            return;
        };
        for (m, r) in members {
            self.snap_to_zone(m, r);
        }
        // Last, so it has the focus.
        self.snap_to_zone(w, zones[zone]);
        let free = self.free_zones(w, &zones);
        self.open_snap_assist(w, free);
    }

    fn free_zones(&self, w: WindowId, zones: &[Rect]) -> Vec<(usize, Rect)> {
        let live = self.snap_live();
        let free = self
            .wm
            .snap
            .free_zones(w, zones.len(), |m| live.contains(&m));
        free.into_iter().map(|i| (i, zones[i])).collect()
    }

    pub(crate) fn snap_live(&self) -> Vec<WindowId> {
        (self.windows.iter())
            .filter(|x| x.mapped && !x.minimized && self.on_current_desktop(x.id))
            .map(|x| x.id)
            .collect()
    }

    /// After a usable area may have moved: grouped windows on outputs whose area changed refill
    /// their zones.
    pub fn refit_snapped(&mut self) {
        let outputs: Vec<Output> = self.scene.outputs().to_vec();
        let mut moves = Vec::new();
        for out in outputs {
            let work = self.usable_area(Some(&out));
            for (w, r) in self.wm.snap.refit(&out.name(), work) {
                if (self.win(w)).is_some_and(|x| x.mapped && !x.maximized && !x.fullscreen) {
                    moves.push((w, r));
                }
            }
        }
        for (w, r) in moves {
            self.snap_commit(w, r);
        }
    }

    pub fn snap_outputs_changed(&mut self) {
        self.hide_snap_outline();
        crate::snapbarview::end(self);
        if let Interaction::Move { snap, .. } = &mut self.interaction {
            *snap = None;
        }
        let live: Vec<String> = self.scene.outputs().iter().map(|o| o.name()).collect();
        self.wm.snap.outputs_gone(&live);
    }

    /// Super and an arrow: the bind window (`bind_window`) steps across the active layout, on to
    /// the next output, then maximized or back to its pre-snap size.
    pub fn key_snap(&mut self, dir: Dir) {
        if self.snap_assist_open() {
            self.close_overview();
        }
        let Some(w) = self.bind_window() else {
            return;
        };
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.snappable() {
            return;
        }
        self.cancel_interaction_for(w);
        let Some(win) = self.win(w) else {
            return;
        };
        let maximized = win.maximized;
        // A maximized window's frame keeps the maximized size until the client commits.
        let r = if maximized {
            win.saved.unwrap_or(win.frame_rect())
        } else {
            win.frame_rect()
        };
        let grouped = win.snapped.is_some() || self.wm.snap.group_of(w).is_some();
        let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
        let Some(out) = self.output_at(f64::from(cx), f64::from(cy)) else {
            return;
        };
        let work = self.usable_area(Some(&out));
        if work.width <= 0 || work.height <= 0 {
            return;
        }
        let (zones, _) = self.active_zones(&out);
        // The group says which zone it is in; the frame only for a window in no group there.
        let live = self.snap_live();
        let member = (self.wm.snap).member_zone(w, &out.name(), &|m| live.contains(&m));
        let at = member.or_else(|| grouped.then(|| crate::snap::zone_of(&zones, r)).flatten());
        let snapped = self.win(w).is_some_and(|x| x.snapped.is_some());
        match crate::snap::key_step(work, &zones, at, maximized, snapped, cy, dir) {
            KeyStep::Zone(i) => self.key_into_zone(&out, w, i, maximized),
            KeyStep::NextOutput => {
                let b = self.layout_box(Some(&out));
                let outs = self.scene.outputs().to_vec();
                let boxes: Vec<Rect> = outs.iter().map(|o| self.layout_box(Some(o))).collect();
                let Some(next) = crate::snap::beside(b, &boxes, dir == Dir::Right, r, cy)
                    .map(|i| outs[i].clone())
                else {
                    return;
                };
                let nwork = self.usable_area(Some(&next));
                let (nzones, _) = self.active_zones(&next);
                if let Some(i) = crate::snap::edge_zone(nwork, &nzones, dir == Dir::Right, cy) {
                    self.key_into_zone(&next, w, i, maximized);
                }
            }
            KeyStep::Maximize => {
                // The pre-snap rect survives the maximize.
                let kept = self.win(w).and_then(|x| x.snapped);
                self.set_maximized(w, true);
                if let Some(win) = self.win_mut(w)
                    && win.maximized
                {
                    win.snapped = kept;
                }
            }
            KeyStep::Unmaximize => {
                let kept = self.win(w).and_then(|x| x.snapped);
                self.set_maximized(w, false);
                if let Some(win) = self.win_mut(w) {
                    win.snapped = kept;
                }
            }
            KeyStep::Unsnap => self.unsnap(w),
            KeyStep::Nothing => {}
        }
    }

    /// `w` into zone `zone` of `out`'s active layout, in the group a drop there joins; a member
    /// it swaps with fills the zone `w` left. Zones the group leaves free get snap assist.
    fn key_into_zone(&mut self, out: &Output, w: WindowId, zone: usize, maximized: bool) {
        let work = self.usable_area(Some(out));
        let live = self.snap_live();
        let landing =
            (self.wm.snap).key_zone(w, &out.name(), work, zone, maximized, |m| live.contains(&m));
        let Some(l) = landing else {
            return;
        };
        if let Some((o, from)) = l.swapped {
            self.snap_to_zone(o, l.zones[from]);
        }
        self.snap_to_zone(w, l.zones[zone]);
        let free = self.free_zones(w, &l.zones);
        self.open_snap_assist(w, free);
    }

    fn unsnap(&mut self, w: WindowId) {
        self.wm.snap.leave_group(w);
        let Some(prev) = self.win_mut(w).and_then(|x| x.snapped.take()) else {
            return;
        };
        let prev = self.caption_clear(prev);
        let m = &self.config.theme.metrics;
        let Some(chrome) = self.win(w).map(|x| x.chrome()) else {
            return;
        };
        let (cw, ch) =
            FrameGeometry::client_size_for_total(m, prev.width, prev.height, false, chrome);
        self.move_to(w, prev.x, prev.y);
        if let Some(win) = self.win_mut(w) {
            win.resize_anchor = None;
        }
        self.request_size(w, cw, ch);
    }

    /// Fill `zone` with `w` from the snap picker or assist, focused and raised; minimized or
    /// maximized windows are restored first.
    pub fn snap_to_zone(&mut self, w: WindowId, zone: Rect) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.snappable() {
            return;
        }
        self.cancel_interaction_for(w);
        if self.win(w).is_some_and(|x| x.minimized) {
            self.set_minimized(w, false);
        }
        let Some(win) = self.win(w) else {
            return;
        };
        // A maximized window's frame keeps the maximized size until the client commits.
        let before = win.snapped.or(if win.maximized { win.saved } else { None });
        if win.maximized {
            self.set_maximized(w, false);
        }
        self.snap_commit(w, zone);
        if let Some(b) = before
            && let Some(win) = self.win_mut(w)
        {
            win.snapped = Some(b);
        }
        self.focus_window(w);
    }
}
