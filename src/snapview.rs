//! Window snapping on the scene: the drag outline, snap groups and their shared edges, and
//! `snap_to_zone`.
//!
//! The outline is `snapframe`'s four strips (or a selection box under Glass) on the overlay
//! layer; it never takes the pointer and stays out of what glass blurs.

use std::collections::HashMap;

use reprisekit::classic::frame::{Edges, FrameGeometry};
use reprisekit::style::SelectionBox;
use reprisescene::{Input, RectId, TreeId};
use smithay::output::Output;

use crate::compositor::State;
use crate::input::Interaction;
use crate::output::upload_scaled;
use crate::raster::Rect;
use crate::snap::{
    ActiveLayout, Arrangement, Dir, KeyStep, Recents, Splits, Suggestion, Target, ZoneLayout,
};
use crate::window::WindowId;

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

impl Group {
    fn has_live(&self, live: &dyn Fn(WindowId) -> bool) -> bool {
        self.members.iter().any(|&(_, m)| live(m))
    }
}

/// A window sized along with the one a drag sizes.
#[derive(Clone, Copy, Debug)]
struct Partner {
    window: WindowId,
    /// The dragged window's side whose line it rides, and its own side on that line.
    side: Edges,
    edge: Edges,
    start: Rect,
    /// Its `snapped` before the size took it, which a cancel puts back.
    snapped: Option<Rect>,
}

/// A follower, the frame it started from, the (dragged side, own side) pairs it rides, and its
/// `snapped` from before the size.
type Follower = (WindowId, Rect, Vec<(Edges, Edges)>, Option<Rect>);

/// Each follower once; its frame and `snapped` are its first entry's.
fn partner_sides(partners: &[Partner]) -> Vec<Follower> {
    let mut out: Vec<Follower> = Vec::new();
    for p in partners {
        match out.iter_mut().find(|o| o.0 == p.window) {
            Some(o) => o.2.push((p.side, p.edge)),
            None => out.push((p.window, p.start, vec![(p.side, p.edge)], p.snapped)),
        }
    }
    out
}

/// A follower that started at `start`, its sides moved onto the lines of `r` it rides
/// (`sides`, as `partner_sides` gives them), and the edges of its own that moved.
fn followed(start: Rect, sides: &[(Edges, Edges)], r: Rect) -> (Rect, Edges) {
    let mut next = start;
    let mut edges = Edges::NONE;
    for &(side, edge) in sides {
        next = crate::snap::follow(next, edge, crate::snap::side_at(r, side));
        edges |= edge;
    }
    (next, edges)
}

impl Snapping {
    /// `w` starts its own group in zone `zone`, leaving any other.
    pub fn new_group(
        &mut self,
        w: WindowId,
        output: &str,
        layout: &ZoneLayout,
        splits: Splits,
        zone: usize,
    ) -> u64 {
        if let Some(o) = self.remove(w)
            && o != self.key(output)
        {
            self.drop_unheld_layout(&o);
        }
        self.next_group += 1;
        let id = self.next_group;
        self.groups.push(Group {
            id,
            output: output.to_string(),
            desktop: self.desktop,
            layout: layout.clone(),
            splits,
            members: vec![(zone, w)],
        });
        self.record(id);
        id
    }

    /// `w` joins `leader`'s group in zone `zone`, leaving any other.
    pub fn join_group(&mut self, leader: WindowId, w: WindowId, zone: usize) {
        let left = if w != leader { self.remove(w) } else { None };
        // Looked up after the removal, which may drop an emptied group.
        let i = self.group_index(leader);
        if let Some(o) = left
            && i.is_none_or(|i| (self.groups[i].output.clone(), self.groups[i].desktop) != o)
        {
            self.drop_unheld_layout(&o);
        }
        let Some(i) = i else {
            return;
        };
        let g = &mut self.groups[i];
        g.members.retain(|&(_, m)| m != w);
        g.members.push((zone, w));
        let id = g.id;
        self.record(id);
    }

    /// `output`'s active layout while a group there has a member on screen (`live`).
    fn active_on(&self, output: &str, live: &dyn Fn(WindowId) -> bool) -> Option<&ActiveLayout> {
        let held = (self.groups.iter()).any(|g| self.here(g, output) && g.has_live(live));
        self.active.get(&self.key(output)).filter(|_| held)
    }

    /// Where `w` snapped into `layout` on `output` lands: the group it joins (if `join`) and
    /// its zones.
    pub fn landing(
        &self,
        output: &str,
        layout: &ZoneLayout,
        join: bool,
        work: Rect,
        live: &dyn Fn(WindowId) -> bool,
    ) -> (Option<u64>, Vec<Rect>) {
        let Some(a) = self.active_on(output, live).filter(|a| a.layout == *layout) else {
            return (None, layout.zones(work));
        };
        let zones = a.splits.clone().zones(work).to_vec();
        let on: Vec<(u64, Vec<Rect>)> = (self.groups.iter())
            .filter(|g| join && self.here(g, output) && g.layout == *layout && g.has_live(live))
            .map(|g| (g.id, g.splits.clone().zones(work).to_vec()))
            .collect();
        let cut_so = (on.iter()).filter(|g| g.1 == zones).max_by_key(|g| g.0);
        match cut_so.or_else(|| on.iter().max_by_key(|g| g.0)) {
            Some((id, z)) => (Some(*id), z.clone()),
            None => (None, zones),
        }
    }

    /// `output`'s active layout over `work` (halves while none is), whether a pick set it, and
    /// the landing zones.
    pub fn active_target(
        &self,
        output: &str,
        work: Rect,
        live: &dyn Fn(WindowId) -> bool,
    ) -> (ZoneLayout, bool, Vec<Rect>) {
        let (layout, explicit) = match self.active_on(output, live) {
            Some(a) => (a.layout.clone(), a.explicit),
            None => (ActiveLayout::default_for(work).layout, false),
        };
        let zones = self.landing(output, &layout, true, work, live).1;
        (layout, explicit, zones)
    }

    /// The drop a release at (`x`, `y`) makes with edge bands `band` px deep, and the rect its
    /// outline shows (the zone, or `work` to maximize).
    #[allow(clippy::too_many_arguments)]
    pub fn outlined_target(
        &self,
        output: &str,
        work: Rect,
        band: i32,
        shift: bool,
        bar: Option<Rect>,
        outer: Edges,
        armed: &mut bool,
        x: i32,
        y: i32,
        live: &dyn Fn(WindowId) -> bool,
    ) -> Option<(Target, Rect)> {
        let (_, explicit, zones) = self.active_target(output, work, live);
        let target = crate::snap::armed_target(
            armed, work, &zones, band, explicit, shift, bar, outer, x, y,
        )?;
        // As `target_layout` picks the layout, worked out once per motion.
        let rect = match target {
            Target::Maximize => work,
            Target::Zone { zone } => *zones.get(zone)?,
            Target::Corner { quarter } => {
                let quarters = crate::snap::quarters();
                *(self.landing(output, &quarters, true, work, live).1).get(quarter)?
            }
        };
        Some((target, rect))
    }

    /// The layout a release on `target` applies, whether a pick set it, and the zone; None to
    /// maximize.
    pub fn target_layout(
        &self,
        output: &str,
        work: Rect,
        target: Target,
        live: &dyn Fn(WindowId) -> bool,
    ) -> Option<(ZoneLayout, bool, usize)> {
        match target {
            Target::Maximize => None,
            Target::Corner { quarter } => Some((crate::snap::quarters(), true, quarter)),
            Target::Zone { zone } => {
                let (layout, explicit, _) = self.active_target(output, work, live);
                Some((layout, explicit, zone))
            }
        }
    }

    /// `layout` becomes `output`'s active layout and `w` goes into zone `zone`. Returns the
    /// zones `w` lies on, or None for a missing zone.
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        w: WindowId,
        output: &str,
        layout: ZoneLayout,
        explicit: bool,
        join: bool,
        work: Rect,
        zone: usize,
        live: impl Fn(WindowId) -> bool,
    ) -> Option<Vec<Rect>> {
        if zone >= layout.zone_count() {
            return None;
        }
        let (target, zones) = self.landing(output, &layout, join, work, &live);
        let held = self.active_on(output, &live).is_some();
        let a = (self.active.entry(self.key(output)))
            .or_insert_with(|| ActiveLayout::new(layout.clone(), explicit, work));
        if a.layout != layout || !held {
            *a = ActiveLayout::new(layout.clone(), explicit, work);
        }
        a.explicit |= explicit;
        a.zones(work);
        let splits = a.splits.clone();
        match target {
            Some(id) => Some(self.join_zone(id, w, zone, work, live)).filter(|z| zone < z.len()),
            None => {
                self.new_group(w, output, &layout, splits, zone);
                Some(zones)
            }
        }
    }

    /// `w` into zone `zone` of suggestion `s` in a group of its own, and the suggestion's other
    /// members that `fills` into theirs.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_suggestion(
        &mut self,
        w: WindowId,
        output: &str,
        s: &Suggestion,
        work: Rect,
        zone: usize,
        live: impl Fn(WindowId) -> bool,
        fills: impl Fn(WindowId) -> bool,
    ) -> Option<SuggestionPick> {
        let layout = s.layout.clone();
        let zones = self.apply(w, output, layout, true, false, work, zone, live)?;
        let mut filled = vec![zone];
        let mut members = Vec::new();
        for &(z, m) in &s.filled {
            if m == w || !fills(m) || filled.contains(&z) {
                continue;
            }
            let Some(&r) = zones.get(z) else {
                continue;
            };
            self.join_group(w, m, z);
            members.push((m, r));
            filled.push(z);
        }
        Some(SuggestionPick { zones, members })
    }

    /// `w` into zone `zone` of the active layout: swaps with the member holding it if `w` steps
    /// out of a zone in that group, else covers it.
    pub fn key_zone(
        &mut self,
        w: WindowId,
        output: &str,
        work: Rect,
        zone: usize,
        maximized: bool,
        live: impl Fn(WindowId) -> bool,
    ) -> Option<KeyLanding> {
        let (layout, explicit, _) = self.active_target(output, work, &live);
        let from = (!maximized)
            .then(|| self.member_zone(w, output, &live))
            .flatten()
            .filter(|&f| f != zone);
        let group = self.group_of(w);
        let joins = self.landing(output, &layout, true, work, &live).0;
        let occupant = from
            .filter(|_| joins.is_some() && joins == group)
            .and_then(|f| {
                let g = &self.groups[self.group_index(w)?];
                let o = (g.members.iter()).find(|&&(z, m)| z == zone && m != w && live(m))?;
                Some((o.1, f))
            });
        let zones = self.apply(w, output, layout, explicit, true, work, zone, &live)?;
        if let Some((o, f)) = occupant
            && let Some(i) = self.group_index(w)
        {
            let g = &mut self.groups[i];
            g.members.retain(|&(_, m)| m != o);
            g.members.push((f, o));
            let id = g.id;
            self.record(id);
        }
        Some(KeyLanding {
            zones,
            swapped: occupant,
        })
    }

    /// `w` into zone `zone` of group `id`; an on-screen member holding it is covered and leaves.
    fn join_zone(
        &mut self,
        id: u64,
        w: WindowId,
        zone: usize,
        work: Rect,
        live: impl Fn(WindowId) -> bool,
    ) -> Vec<Rect> {
        if self.group_of(w) != Some(id)
            && let Some(o) = self.remove(w)
        {
            self.drop_unheld_layout(&o);
        }
        let Some(g) = self.groups.iter_mut().find(|g| g.id == id) else {
            return Vec::new();
        };
        g.members
            .retain(|&(z, m)| m != w && (z != zone || !live(m)));
        g.members.push((zone, w));
        let zones = g.splits.zones(work).to_vec();
        self.record(id);
        zones
    }

    /// Zones no on-screen member of `w`'s group holds.
    pub fn free_zones(
        &self,
        w: WindowId,
        count: usize,
        live: impl Fn(WindowId) -> bool,
    ) -> Vec<usize> {
        let members = self
            .group_index(w)
            .map_or(&[][..], |i| &self.groups[i].members);
        (0..count)
            .filter(|z| {
                !members
                    .iter()
                    .any(|&(mz, m)| mz == *z && (m == w || live(m)))
            })
            .collect()
    }

    /// A move of `w` begins: what Escape puts back.
    pub fn begin_move(&mut self, w: WindowId) {
        self.move_undo = self.group_index(w).and_then(|i| {
            let group = self.groups[i].clone();
            let zone = group.members.iter().find(|m| m.1 == w)?.0;
            let active = (self.active.get(&(group.output.clone(), group.desktop))).cloned();
            Some(MoveUndo {
                window: w,
                group,
                zone,
                active,
            })
        });
    }

    /// Escape cancelled the move: `w`, its group and the active layout come back.
    pub fn cancel_move(&mut self, w: WindowId) {
        let Some(u) = self.move_undo.take().filter(|u| u.window == w) else {
            return;
        };
        if let Some(o) = self.remove(w) {
            self.drop_unheld_layout(&o);
        }
        let id = u.group.id;
        match self.groups.iter_mut().find(|g| g.id == id) {
            Some(g) => {
                g.members.retain(|&(z, _)| z != u.zone);
                g.members.push((u.zone, w));
            }
            None => self.groups.push(Group {
                members: vec![(u.zone, w)],
                ..u.group.clone()
            }),
        }
        if let Some(a) = u.active {
            let key = (u.group.output.clone(), u.group.desktop);
            self.active.entry(key).or_insert(a);
        }
        self.record(id);
    }

    pub fn end_move(&mut self) {
        self.move_undo = None;
    }

    pub fn shares_group(&self, a: WindowId, b: WindowId) -> bool {
        a == b
            || self
                .group_of(a)
                .is_some_and(|g| self.group_of(b) == Some(g))
    }

    pub fn group_of(&self, w: WindowId) -> Option<u64> {
        self.group_index(w).map(|i| self.groups[i].id)
    }

    /// The zone `w` holds in its group when that group is on `output`'s active layout.
    pub fn member_zone(
        &self,
        w: WindowId,
        output: &str,
        live: &dyn Fn(WindowId) -> bool,
    ) -> Option<usize> {
        let g = &self.groups[self.group_index(w)?];
        let active = self.active_on(output, live)?;
        if !self.here(g, output) || g.layout != active.layout {
            return None;
        }
        g.members.iter().find(|m| m.1 == w).map(|m| m.0)
    }

    pub fn follows_drag(&self, w: WindowId) -> bool {
        self.partners.iter().any(|p| p.window == w)
    }

    /// `w` left its zone: keep the output's active layout while a grouped window remains.
    pub fn leave_group(&mut self, w: WindowId) {
        if let Some(output) = self.remove(w) {
            self.drop_unheld_layout(&output);
        }
    }

    /// Members `desktop_of` puts on another desktop than their group's leave it; None (a window
    /// on every desktop, or gone) stays.
    pub fn leave_other_desktops(&mut self, desktop_of: impl Fn(WindowId) -> Option<usize>) {
        let moved: Vec<WindowId> = (self.groups.iter())
            .flat_map(|g| {
                (g.members.iter())
                    .filter(|&&(_, m)| desktop_of(m).is_some_and(|d| d != g.desktop))
                    .map(|&(_, m)| m)
            })
            .collect();
        for w in moved {
            self.leave_group(w);
        }
    }

    pub fn forget_window(&mut self, w: WindowId) {
        if let Some(output) = self.remove(w) {
            self.drop_unheld_layout(&output);
        }
        self.recents.forget_window(w);
        self.partners.retain(|p| p.window != w);
        self.move_undo = self.move_undo.take().filter(|u| u.window != w);
    }

    pub fn outputs_gone(&mut self, live: &[String]) {
        self.active.retain(|(o, _), _| live.contains(o));
    }

    /// `output`'s usable area is `work` now: the rects each grouped window's zone takes over it.
    pub fn refit(&mut self, output: &str, work: Rect) -> Vec<(WindowId, Rect)> {
        if work.width <= 0 || work.height <= 0 {
            return Vec::new();
        }
        for (_, a) in (self.active.iter_mut()).filter(|((o, _), _)| o == output) {
            a.zones(work);
        }
        let mut out = Vec::new();
        for g in self.groups.iter_mut().filter(|g| g.output == output) {
            if g.splits.work() == work {
                continue;
            }
            let zones = g.splits.zones(work);
            for &(zone, w) in &g.members {
                if let Some(&r) = zones.get(zone) {
                    out.push((w, r));
                }
            }
        }
        out
    }

    /// A drag sizing `w` moved the splits `moves` (vertical, from, to): the group's zones and
    /// the active layout's follow.
    pub fn move_splits(&mut self, w: WindowId, work: Rect, moves: &[(bool, i32, i32)]) {
        let Some(i) = self.group_index(w) else {
            return;
        };
        let g = &mut self.groups[i];
        // Laid on the area now first: `move_split` keeps the area's own edges.
        g.splits.zones(work);
        for &(vertical, from, to) in moves {
            g.splits.move_split(vertical, from, to);
        }
        let key = (g.output.clone(), g.desktop);
        let Some(a) = (self.active.get_mut(&key)).filter(|a| a.layout == g.layout) else {
            return;
        };
        a.zones(work);
        for &(vertical, from, to) in moves {
            a.move_split(vertical, from, to);
        }
    }

    fn group_index(&self, w: WindowId) -> Option<usize> {
        (self.groups.iter()).position(|g| g.members.iter().any(|&(_, m)| m == w))
    }

    /// `w` out of its group; an emptied group goes. The active layout stays: a caller snapping
    /// `w` anew may have just set it.
    fn remove(&mut self, w: WindowId) -> Option<(String, usize)> {
        let i = self.group_index(w)?;
        self.groups[i].members.retain(|&(_, m)| m != w);
        let id = self.groups[i].id;
        self.record(id);
        let g = if self.groups[i].members.is_empty() {
            self.groups.remove(i)
        } else {
            self.groups[i].clone()
        };
        Some((g.output, g.desktop))
    }

    fn drop_unheld_layout(&mut self, key: &(String, usize)) {
        if !(self.groups.iter()).any(|g| g.output == key.0 && g.desktop == key.1) {
            self.active.remove(key);
        }
    }

    fn key(&self, output: &str) -> (String, usize) {
        (output.to_string(), self.desktop)
    }

    fn here(&self, g: &Group, output: &str) -> bool {
        g.output == output && g.desktop == self.desktop
    }

    fn record(&mut self, id: u64) {
        if let Some(g) = self.groups.iter().find(|g| g.id == id) {
            self.recents.record(Arrangement {
                group: g.id,
                output: g.output.clone(),
                layout: g.layout.clone(),
                members: g.members.clone(),
            });
        }
    }
}

/// Whether a window is tiled: in a snap group, neither maximized nor fullscreen.
fn tiled(maximized: bool, fullscreen: bool, group: Option<u64>) -> bool {
    group.is_some() && !maximized && !fullscreen
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
    pub fn show_snap_outline(&mut self, target: Option<Rect>) {
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
    pub fn snap_commit(&mut self, w: WindowId, target: Rect) {
        let m = &self.config.theme.metrics;
        let Some(win) = self.win(w) else {
            return;
        };
        let client = FrameGeometry::client_size_for_total(
            m,
            target.width,
            target.height,
            false,
            win.chrome(),
        );
        let (cw, ch) = crate::snap::commit_size(client, win.size_limits());
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
        let Some(g) = self.wm.snap.group_of(w) else {
            return;
        };
        let had = (self.wm.snap.resize)
            .filter(|r| r.0 == w)
            .map_or(Edges::NONE, |r| r.2);
        let mut others: Vec<(WindowId, Rect, Option<Rect>)> = Vec::new();
        for win in &self.windows {
            let free = win.mapped && !win.minimized && !win.maximized && !win.fullscreen;
            if win.id == w || !free || self.wm.snap.group_of(win.id) != Some(g) {
                continue;
            }
            if !self.on_current_desktop(win.id) {
                continue;
            }
            others.push((win.id, win.frame_rect(), win.snapped));
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
                    snapped: others[i].2,
                });
            }
        }
        for p in &partners {
            if let Some(win) = self.win_mut(p.window) {
                win.snapped = None;
            }
            self.set_resizing(p.window, true);
        }
        self.wm.snap.resize = Some((w, start, edges));
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
        for (window, start, sides, _) in partner_sides(&self.wm.snap.partners) {
            let (next, edges) = followed(start, &sides, r);
            let m = &self.config.theme.metrics;
            let Some(win) = self.win(window) else {
                continue;
            };
            let client = FrameGeometry::client_size_for_total(
                m,
                next.width,
                next.height,
                false,
                win.chrome(),
            );
            let (cw, ch) = crate::snap::commit_size(client, win.size_limits());
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

    /// Escape on a keyboard size: the followers go back to where they started, with the frames
    /// they restore to, and the group's zones stay.
    pub(crate) fn cancel_group_resize(&mut self) {
        let partners = std::mem::take(&mut self.wm.snap.partners);
        self.wm.snap.resize = None;
        self.wm.snap.last = None;
        for (window, start, _, snapped) in partner_sides(&partners) {
            self.set_resizing(window, false);
            if let Some(win) = self.win_mut(window) {
                win.snapped = snapped;
            }
            let m = &self.config.theme.metrics;
            let Some(win) = self.win(window) else {
                continue;
            };
            let client = FrameGeometry::client_size_for_total(
                m,
                start.width,
                start.height,
                false,
                win.chrome(),
            );
            let (cw, ch) = crate::snap::commit_size(client, win.size_limits());
            self.move_to(window, start.x, start.y);
            self.request_size(window, cw, ch);
        }
    }

    /// The drag ended: followers stop, the group's zones take the moved splits, and a side no
    /// member followed takes `w` out of the group.
    pub(crate) fn end_group_resize(&mut self) {
        let partners = std::mem::take(&mut self.wm.snap.partners);
        let resize = self.wm.snap.resize.take();
        let last = self.wm.snap.last.take();
        for p in &partners {
            self.set_resizing(p.window, false);
        }
        let (Some((w, start, edges)), Some(r)) = (resize, last) else {
            return;
        };
        let followed: Vec<Edges> = partners.iter().map(|p| p.side).collect();
        let lone = crate::snap::lone_side(start, r, edges, &followed);
        let centre = (start.x + start.width / 2, start.y + start.height / 2);
        let Some(out) = self.output_at(f64::from(centre.0), f64::from(centre.1)) else {
            return;
        };
        let work = self.usable_area(Some(&out));
        let mut moves: Vec<(bool, i32, i32)> = Vec::new();
        let mut done: Vec<Edges> = Vec::new();
        for p in &partners {
            if done.contains(&p.side) {
                continue;
            }
            done.push(p.side);
            let vertical = p.side == Edges::LEFT || p.side == Edges::RIGHT;
            let (from, to) = (
                crate::snap::side_at(start, p.side),
                crate::snap::side_at(r, p.side),
            );
            moves.push((vertical, from, to));
        }
        self.wm.snap.move_splits(w, work, &moves);
        if lone {
            self.wm.snap.leave_group(w);
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
    pub fn move_target(
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
    pub fn release_on(&mut self, w: WindowId, target: Target) {
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
    pub fn active_zones(&self, out: &Output) -> (Vec<Rect>, bool) {
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

    /// Super and an arrow: the focused window steps across the active layout, on to the next
    /// output, then maximized or back to its pre-snap size.
    pub fn key_snap(&mut self, dir: Dir) {
        if self.snap_assist_open() {
            self.close_overview();
        }
        let Some(w) = self.focused else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snap::{ActiveLayout, ZoneLayout};

    const WORK: Rect = Rect::new(0, 0, 1920, 1032);

    fn all(_: WindowId) -> bool {
        true
    }

    fn k(output: &str) -> (String, usize) {
        (output.to_string(), 0)
    }

    fn halves() -> ZoneLayout {
        ZoneLayout::parse("1|1").unwrap()
    }

    #[test]
    fn a_grouped_window_is_tiled_until_it_maximizes() {
        assert!(tiled(false, false, Some(1)));
        assert!(!tiled(true, false, Some(1)), "maximized has its own state");
        assert!(!tiled(false, true, Some(1)), "and fullscreen");
        assert!(!tiled(false, false, None), "out of every group");
    }

    #[test]
    fn groups_record_their_arrangement() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let g = s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
        s.join_group(a, b, 1);
        assert_eq!(s.group_of(b), Some(g));
        let r: Vec<_> = s.recents.iter().cloned().collect();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].members, vec![(0, a), (1, b)]);
        s.active
            .insert(k("DP-1"), ActiveLayout::new(halves(), true, WORK));
        s.key_zone(b, "DP-1", WORK, 0, false, all);
        assert_eq!(
            s.group_of(b),
            Some(g),
            "a step inside its own group keeps it"
        );
    }

    #[test]
    fn a_drop_into_the_active_layout_joins_its_group() {
        let (a, b, c, d) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let mut s = Snapping::default();
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
        let g = s.group_of(a);
        s.apply(b, "DP-1", quarters.clone(), true, true, WORK, 2, all);
        assert_eq!(s.group_of(b), g, "the quarters' group");
        assert_eq!(s.free_zones(b, 4, |_| true), vec![1, 3]);
        s.apply(c, "DP-1", quarters.clone(), true, true, WORK, 2, all);
        assert_eq!(s.group_of(c), g);
        assert_eq!(s.group_of(b), None, "covered, out of the group");
        s.apply(d, "DP-1", halves(), true, true, WORK, 0, all);
        assert_ne!(s.group_of(d), g, "another layout starts a group");
        s.apply(b, "DP-1", halves(), true, false, WORK, 1, all);
        assert_ne!(
            s.group_of(b),
            s.group_of(d),
            "a suggestion's pick starts its own"
        );
    }

    #[test]
    fn a_layout_with_no_member_on_screen_is_not_active() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
        // `a` minimized, or another desktop shown.
        let away = |m: WindowId| m != a;
        let (layout, explicit, _) = s.active_target("DP-1", WORK, &away);
        assert_eq!(layout, halves(), "the halves, as with no layout");
        assert!(!explicit, "no zone targets");
        assert_eq!(
            s.outlined_target(
                "DP-1",
                WORK,
                crate::snap::BAND,
                true,
                None,
                crate::snap::EVERY_SIDE,
                &mut true,
                500,
                300,
                &away
            ),
            None,
            "a release off the edges drops nowhere"
        );
        assert!(
            (s.outlined_target(
                "DP-1",
                WORK,
                crate::snap::BAND,
                true,
                None,
                crate::snap::EVERY_SIDE,
                &mut true,
                500,
                300,
                &all
            ))
            .is_some(),
            "with `a` on screen it drops into a quarter"
        );
        s.apply(b, "DP-1", quarters, true, true, WORK, 1, away);
        assert_ne!(s.group_of(b), s.group_of(a), "`b` joins no hidden group");
    }

    #[test]
    fn a_member_moved_to_another_desktop_leaves_its_group() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let g = s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
        s.join_group(a, b, 1);
        s.join_group(a, c, 1);
        let on = |b_at: Option<usize>| move |w: WindowId| if w == b { b_at } else { Some(0) };
        s.leave_other_desktops(on(None));
        assert_eq!(s.group_of(b), Some(g), "on every desktop: stays");
        s.leave_other_desktops(on(Some(0)));
        assert_eq!(s.group_of(b), Some(g), "still on the group's desktop");
        s.leave_other_desktops(on(Some(1)));
        assert_eq!(s.group_of(b), None, "moved to the second desktop");
        assert_eq!(s.group_of(a), Some(g));
        assert_eq!(s.group_of(c), Some(g));
        s.leave_other_desktops(|_| Some(1));
        assert_eq!(s.group_of(a), None);
        assert!(
            !s.active.contains_key(&k("DP-1")),
            "the layout went with the last member"
        );
    }

    #[test]
    fn each_desktop_keeps_its_own_layout() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
        // The second desktop shown: `a` is off screen.
        s.desktop = 1;
        let away = |m: WindowId| m != a;
        s.apply(b, "DP-1", thirds.clone(), true, true, WORK, 2, away);
        assert_eq!(s.active_target("DP-1", WORK, &away).0, thirds);
        s.desktop = 0;
        let back = |m: WindowId| m != b;
        let (layout, explicit, _) = s.active_target("DP-1", WORK, &back);
        assert_eq!(layout, quarters, "the first desktop's pick stayed");
        assert!(explicit);
        s.forget_window(b);
        assert!(
            !s.active.contains_key(&("DP-1".into(), 1)),
            "the second desktop's layout went with its last window"
        );
        assert!(s.active.contains_key(&k("DP-1")));
    }

    #[test]
    fn a_suggestions_pick_brings_back_only_members_that_can_fill_a_zone() {
        let (a, b, c, d) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let mut s = Snapping::default();
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        let sug = Suggestion {
            layout: thirds,
            // `c` claims zone 1 first; `d` cannot fill one.
            filled: vec![(1, c), (1, b), (2, d), (0, a)],
        };
        let fills = |m: WindowId| m != d;
        let SuggestionPick { zones, members } =
            (s.apply_suggestion(a, "DP-1", &sug, WORK, 0, all, fills)).unwrap();
        assert_eq!(members, vec![(c, zones[1])], "one member per zone");
        assert_eq!(s.group_of(c), s.group_of(a));
        assert_eq!(s.group_of(b), None);
        assert_eq!(s.group_of(d), None, "it claims no zone");
        assert_eq!(s.free_zones(a, 3, all), vec![2]);
        assert!(
            (s.apply_suggestion(a, "DP-1", &sug, WORK, 5, all, fills)).is_none(),
            "a zone the layout lacks"
        );
    }

    #[test]
    fn a_drop_joins_the_group_cut_as_the_active_layout() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
        // A suggestion's pick of the same layout starts a newer group.
        s.apply(b, "DP-1", quarters.clone(), true, false, WORK, 1, all);
        // The older group's border moves, and the active layout's with it.
        s.move_splits(a, WORK, &[(true, 960, 1100)]);
        let outline = s.active[&k("DP-1")].clone().zones(WORK).to_vec();
        assert_eq!(outline[3].x, 1100);
        let landed = s.apply(c, "DP-1", quarters, true, true, WORK, 3, all);
        assert_eq!(landed, Some(outline), "it lands where the outline showed");
        assert_eq!(s.group_of(c), s.group_of(a), "the group cut that way");
    }

    #[test]
    fn a_corner_drop_outlines_where_it_lands_with_two_groups() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 1, all);
        // A suggestion's pick of the same layout starts a newer group.
        s.apply(b, "DP-1", quarters.clone(), true, false, WORK, 0, all);
        // The older group's border moves, and the active layout's with it.
        s.move_splits(a, WORK, &[(true, 960, 1100)]);
        // A release in the top-right corner.
        let (target, outline) = s
            .outlined_target(
                "DP-1",
                WORK,
                crate::snap::BAND,
                true,
                None,
                crate::snap::EVERY_SIDE,
                &mut true,
                1919,
                0,
                &all,
            )
            .unwrap();
        assert_eq!(target, Target::Corner { quarter: 2 });
        assert_eq!(
            outline,
            Rect::new(1100, 0, 820, 516),
            "the moved group's cut"
        );
        let (layout, _, zone) = s.target_layout("DP-1", WORK, target, &all).unwrap();
        let (joins, _) = s.landing("DP-1", &layout, true, WORK, &all);
        assert_eq!(joins, s.group_of(a));
        let landed = s.apply(c, "DP-1", layout, true, true, WORK, zone, all);
        assert_eq!(
            landed.map(|z| z[2]),
            Some(outline),
            "it lands on the outline"
        );
        assert_eq!(s.group_of(c), joins, "in the group the outline showed");
    }

    #[test]
    fn a_suggestion_tile_outlines_where_its_pick_lands() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 1, all);
        s.apply(b, "DP-1", quarters.clone(), true, false, WORK, 0, all);
        s.move_splits(a, WORK, &[(true, 960, 1100)]);
        let (joins, tile) = s.landing("DP-1", &quarters, false, WORK, &all);
        assert_eq!(joins, None, "a suggestion starts its own group");
        assert_eq!(tile[2].x, 1100, "cut as the active layout is");
        let landed = s.apply(c, "DP-1", quarters, true, false, WORK, 2, all);
        assert_eq!(landed, Some(tile));
    }

    #[test]
    fn a_key_step_joins_the_group_a_drop_joins() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
        s.apply(b, "DP-1", quarters, true, false, WORK, 1, all);
        s.move_splits(a, WORK, &[(true, 960, 1100)]);
        let outline = s.active[&k("DP-1")].clone().zones(WORK).to_vec();
        // b steps out of its own, newer group into a's, cut as the
        // active layout is.
        assert_eq!(
            s.key_zone(b, "DP-1", WORK, 2, false, all).map(|l| l.zones),
            Some(outline)
        );
        assert_eq!(s.group_of(b), s.group_of(a));
        assert_eq!(
            s.key_zone(b, "DP-1", WORK, 4, false, all),
            None,
            "no fifth zone"
        );
    }

    #[test]
    fn super_up_and_down_step_by_group_zone_after_a_split_move() {
        let (x, y, z) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let half_quarters = ZoneLayout::parse("1|1:2").unwrap();
        for (w, zone) in [(x, 0), (y, 1), (z, 2)] {
            s.apply(
                w,
                "DP-1",
                half_quarters.clone(),
                true,
                true,
                WORK,
                zone,
                all,
            );
        }
        s.move_splits(y, WORK, &[(true, 960, 1150)]);
        let (_, _, zones) = s.active_target("DP-1", WORK, &all);
        // The clients' own sizes leave the frames short of the moved split.
        let y_frame = Rect::new(1139, 0, 781, 516);
        let z_frame = Rect::new(1139, 516, 781, 516);
        assert_eq!(crate::snap::zone_of(&zones, y_frame), Some(1));
        let step = |w: WindowId, frame: Rect, dir: Dir| {
            let at = s.member_zone(w, "DP-1", &all);
            let cy = frame.y + frame.height / 2;
            crate::snap::key_step(WORK, &zones, at, false, false, cy, dir)
        };
        assert_eq!(step(y, y_frame, Dir::Down), KeyStep::Zone(2));
        assert_eq!(step(z, z_frame, Dir::Up), KeyStep::Zone(1));
    }

    #[test]
    fn a_super_arrow_step_swaps_with_the_member_holding_the_zone() {
        let (x, y, z) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let half_quarters = ZoneLayout::parse("1|1:2").unwrap();
        for (w, zone) in [(x, 0), (y, 1), (z, 2)] {
            s.apply(
                w,
                "DP-1",
                half_quarters.clone(),
                true,
                true,
                WORK,
                zone,
                all,
            );
        }
        s.move_splits(y, WORK, &[(true, 960, 1150)]);
        let g = s.group_of(x);
        let zones = |s: &Snapping| [x, y, z].map(|w| s.member_zone(w, "DP-1", &all));
        let step = |s: &mut Snapping, w: WindowId, dir: Dir| {
            let (_, _, rects) = s.active_target("DP-1", WORK, &all);
            let at = s.member_zone(w, "DP-1", &all);
            let KeyStep::Zone(i) = crate::snap::key_step(WORK, &rects, at, false, true, 0, dir)
            else {
                panic!("{dir:?} steps into a zone");
            };
            s.key_zone(w, "DP-1", WORK, i, false, all).unwrap().swapped
        };
        assert_eq!(step(&mut s, y, Dir::Down), Some((z, 1)), "z takes y's zone");
        assert_eq!(zones(&s), [Some(0), Some(2), Some(1)]);
        assert_eq!(step(&mut s, y, Dir::Up), Some((z, 2)));
        assert_eq!(zones(&s), [Some(0), Some(1), Some(2)]);
        assert_eq!(step(&mut s, z, Dir::Up), Some((y, 2)), "z moves up, y down");
        assert_eq!(zones(&s), [Some(0), Some(2), Some(1)]);
        assert!(
            [x, y, z].iter().all(|&w| s.group_of(w) == g),
            "all stay grouped"
        );
    }

    #[test]
    fn super_left_and_right_swap_halves() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
        s.apply(b, "DP-1", halves(), true, true, WORK, 1, all);
        let l = s.key_zone(a, "DP-1", WORK, 1, false, all).unwrap();
        assert_eq!(l.swapped, Some((b, 0)), "Right: a and b trade halves");
        assert_eq!(s.member_zone(b, "DP-1", &all), Some(0));
        let l = s.key_zone(a, "DP-1", WORK, 0, false, all).unwrap();
        assert_eq!(l.swapped, Some((b, 1)), "Left: back again");
        assert_eq!(s.group_of(a), s.group_of(b));
    }

    #[test]
    fn a_super_arrow_snap_leaves_free_only_the_zones_no_member_holds() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let l = s.key_zone(a, "DP-1", WORK, 0, false, all).unwrap();
        assert_eq!(s.free_zones(a, l.zones.len(), all), vec![1]);
        let l = s.key_zone(b, "DP-1", WORK, 1, false, all).unwrap();
        assert_eq!(s.free_zones(b, l.zones.len(), all), vec![]);
        let l = s.key_zone(a, "DP-1", WORK, 1, false, all).unwrap();
        assert_eq!(l.swapped, Some((b, 0)));
        assert_eq!(s.free_zones(a, l.zones.len(), all), vec![], "a swap");
    }

    #[test]
    fn a_step_from_outside_a_zone_covers_the_occupant() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
        s.apply(b, "DP-1", halves(), true, true, WORK, 1, all);
        let l = s.key_zone(c, "DP-1", WORK, 1, false, all).unwrap();
        assert_eq!(l.swapped, None, "a free window");
        assert_eq!(s.group_of(b), None, "b leaves, covered");
        let l = s.key_zone(a, "DP-1", WORK, 1, true, all).unwrap();
        assert_eq!(l.swapped, None, "a maximized window");
        assert_eq!(s.group_of(c), None, "c leaves, covered");
    }

    #[test]
    fn a_window_outside_the_active_layouts_groups_has_no_member_zone() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        s.apply(a, "DP-1", halves(), true, true, WORK, 1, all);
        assert_eq!(s.member_zone(a, "DP-1", &all), Some(1));
        assert_eq!(s.member_zone(a, "DP-2", &all), None, "another output");
        assert_eq!(s.member_zone(b, "DP-1", &all), None, "in no group");
        s.apply(b, "DP-1", thirds, true, true, WORK, 2, all);
        assert_eq!(s.member_zone(b, "DP-1", &all), Some(2));
        assert_eq!(
            s.member_zone(a, "DP-1", &all),
            None,
            "its halves are no longer the active layout"
        );
    }

    #[test]
    fn a_drop_covers_only_an_occupant_on_screen() {
        let (a, b, c, d) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let mut s = Snapping::default();
        s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
        s.join_group(a, b, 1);
        // b is minimized or on another desktop: it keeps its zone.
        let shown = |m: WindowId| m != b;
        s.apply(c, "DP-1", halves(), true, true, WORK, 1, shown);
        assert_eq!(s.group_of(b), s.group_of(a));
        assert_eq!(s.group_of(c), s.group_of(a));
        s.apply(d, "DP-1", halves(), true, true, WORK, 0, shown);
        assert_eq!(s.group_of(a), None, "a on screen is covered");
        assert_eq!(s.group_of(d), s.group_of(b));
    }

    #[test]
    fn a_drop_into_a_layout_on_another_output_starts_a_group_there() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
        s.join_group(a, b, 1);
        let first = s.group_of(a);
        s.apply(b, "DP-2", halves(), false, true, WORK, 0, all);
        let second = s.group_of(b);
        assert!(second.is_some() && second != first, "not DP-1's group");
        assert_eq!(s.member_zone(b, "DP-2", &all), Some(0));
        assert_eq!(s.group_of(a), first, "DP-1 keeps its group");
        assert!(s.active.contains_key(&k("DP-1")));
        s.apply(a, "DP-2", halves(), false, true, WORK, 1, all);
        assert_eq!(s.group_of(a), second, "joins the group on DP-2");
        assert!(
            !s.active.contains_key(&k("DP-1")),
            "DP-1's layout went with its last window"
        );
    }

    #[test]
    fn a_cancelled_move_puts_the_window_back_in_its_group() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
        s.join_group(a, b, 1);
        let g = s.group_of(b);
        s.begin_move(b);
        s.leave_group(b);
        s.cancel_move(b);
        assert_eq!(s.group_of(b), g);
        assert_eq!(s.free_zones(a, 2, |_| true), Vec::<usize>::new());
        // Alone in its group, the group and the active layout went with
        // it; both come back.
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        s.apply(c, "DP-2", thirds.clone(), true, true, WORK, 2, all);
        let g = s.group_of(c);
        s.begin_move(c);
        s.leave_group(c);
        assert!(!s.active.contains_key(&k("DP-2")));
        s.cancel_move(c);
        assert_eq!(s.group_of(c), g);
        assert_eq!(s.active.get(&k("DP-2")).map(|l| &l.layout), Some(&thirds));
        assert_eq!(
            s.recents.iter().next().map(|r| r.members.clone()),
            Some(vec![(2, c)])
        );
        s.cancel_move(c);
        assert_eq!(s.group_of(c), g, "a second cancel changes nothing");
    }

    #[test]
    fn a_released_or_closed_window_has_no_move_to_cancel() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
        s.join_group(a, b, 1);
        s.begin_move(b);
        s.end_move();
        s.leave_group(b);
        s.cancel_move(b);
        assert_eq!(s.group_of(b), None, "released: nothing to put back");
        s.begin_move(a);
        s.forget_window(a);
        s.cancel_move(a);
        assert_eq!(s.group_of(a), None, "closed: not brought back");
        assert!(!s.active.contains_key(&k("DP-1")));
    }

    #[test]
    fn a_follower_riding_two_sides_takes_both_from_its_first_frame() {
        let (tr, bl) = (WindowId::next(), WindowId::next());
        let tr_start = Rect::new(960, 0, 960, 516);
        let tr_restore = Rect::new(300, 200, 640, 480);
        let partners = [
            Partner {
                window: tr,
                side: Edges::RIGHT,
                edge: Edges::LEFT,
                start: tr_start,
                snapped: Some(tr_restore),
            },
            Partner {
                window: bl,
                side: Edges::BOTTOM,
                edge: Edges::TOP,
                start: Rect::new(0, 516, 960, 516),
                snapped: None,
            },
            // Picked second, after the first side had moved it and taken its restore frame.
            Partner {
                window: tr,
                side: Edges::BOTTOM,
                edge: Edges::BOTTOM,
                start: Rect::new(1000, 0, 920, 516),
                snapped: None,
            },
        ];
        let by = partner_sides(&partners);
        assert_eq!(by.len(), 2, "each follower once");
        let (w, start, sides, snapped) = &by[0];
        assert_eq!((*w, *start), (tr, tr_start), "its first entry's frame");
        assert_eq!(
            *snapped,
            Some(tr_restore),
            "a cancel gives back the restore frame from before the first side"
        );
        let sized = Rect::new(0, 0, 1040, 600);
        assert_eq!(
            followed(*start, sides, sized),
            (Rect::new(1040, 0, 880, 600), Edges::LEFT | Edges::BOTTOM)
        );
        assert_eq!(
            followed(by[1].1, &by[1].2, sized),
            (Rect::new(0, 600, 960, 432), Edges::TOP)
        );
    }

    #[test]
    fn a_window_shares_a_group_only_with_its_members() {
        let (a, b, c, d) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let mut s = Snapping::default();
        s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
        s.join_group(a, b, 1);
        s.new_group(
            c,
            "DP-1",
            &ZoneLayout::parse("1|1|1").unwrap(),
            Splits::new(&ZoneLayout::parse("1|1|1").unwrap(), WORK),
            0,
        );
        assert!(s.shares_group(a, a));
        assert!(s.shares_group(a, b), "a member");
        assert!(!s.shares_group(a, c), "another group's");
        assert!(!s.shares_group(a, d), "in no group");
        assert!(!s.shares_group(d, WindowId::next()), "neither grouped");
    }

    #[test]
    fn free_zones_count_only_the_groups_own_members() {
        let (a, b, old0, old1, shut) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let mut s = Snapping::default();
        // Halves from before sit on the quarters' top corners.
        s.new_group(old0, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
        s.join_group(old0, old1, 1);
        let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
        s.new_group(a, "DP-1", &quarters, Splits::new(&quarters, WORK), 0);
        let live = |m: WindowId| m != shut;
        assert_eq!(
            s.free_zones(a, 4, live),
            vec![1, 2, 3],
            "the halves fill none"
        );
        s.join_group(a, b, 3);
        s.join_group(a, shut, 1);
        assert_eq!(
            s.free_zones(a, 4, live),
            vec![1, 2],
            "a member not on screen fills nothing"
        );
        assert_eq!(s.free_zones(WindowId::next(), 2, live), vec![0, 1]);
    }

    #[test]
    fn the_active_layout_goes_with_the_last_grouped_window() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let work = Rect::new(0, 0, 1920, 1032);
        s.active
            .insert(k("DP-1"), ActiveLayout::new(halves(), true, work));
        s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
        s.join_group(a, b, 1);
        s.leave_group(a);
        assert!(s.active.contains_key(&k("DP-1")), "b still holds a zone");
        s.forget_window(b);
        assert!(!s.active.contains_key(&k("DP-1")));
        assert_eq!(
            s.recents.iter().count(),
            0,
            "the group's entry emptied with it"
        );
    }

    #[test]
    fn a_new_layout_stays_when_its_window_was_the_only_one_grouped() {
        let a = WindowId::next();
        let mut s = Snapping::default();
        let work = Rect::new(0, 0, 1920, 1032);
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        s.apply(a, "DP-1", halves(), true, false, work, 0, all);
        let zones = s.apply(a, "DP-1", thirds.clone(), true, false, work, 2, all);
        assert_eq!(zones, Some(thirds.zones(work)));
        assert_eq!(s.active.get(&k("DP-1")).map(|l| &l.layout), Some(&thirds));
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].layout, thirds);
        assert_eq!(s.groups[0].members, vec![(2, a)]);
    }

    #[test]
    fn outputs_gone_drop_their_layouts() {
        let a = WindowId::next();
        let mut s = Snapping::default();
        let work = Rect::new(0, 0, 1920, 1032);
        s.active
            .insert(k("DP-1"), ActiveLayout::new(halves(), true, work));
        s.active
            .insert(k("DP-2"), ActiveLayout::new(halves(), true, work));
        s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
        s.new_group(
            WindowId::next(),
            "DP-2",
            &halves(),
            Splits::new(&halves(), WORK),
            0,
        );
        s.outputs_gone(&["DP-1".to_string()]);
        assert!(s.active.contains_key(&k("DP-1")));
        assert!(!s.active.contains_key(&k("DP-2")));
    }

    #[test]
    fn a_refit_moves_the_groups_on_every_layout_of_the_output() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut s = Snapping::default();
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        let old = Rect::new(0, 0, 1920, 1032);
        let new = Rect::new(0, 48, 1920, 1032);
        s.apply(a, "DP-1", halves(), true, false, old, 0, all);
        s.join_group(a, b, 1);
        s.apply(c, "DP-1", thirds.clone(), true, false, old, 2, all);
        let mut moves = s.refit("DP-1", new);
        moves.sort_by_key(|m| m.1.x);
        let (h, t) = (halves().zones(new), thirds.zones(new));
        assert_eq!(moves, vec![(a, h[0]), (b, h[1]), (c, t[2])]);
        assert!(s.refit("DP-1", new).is_empty(), "laid on the new area once");
    }

    #[test]
    fn a_group_keeps_its_own_splits_through_a_refit() {
        let (a, b, c, d) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let mut s = Snapping::default();
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        s.apply(a, "DP-1", halves(), true, false, WORK, 0, all);
        s.join_group(a, b, 1);
        s.move_splits(a, WORK, &[(true, 960, 1100)]);
        assert_eq!(
            s.active[&k("DP-1")].clone().zones(WORK)[0].right(),
            1100,
            "the active layout's split moves with its group"
        );
        // Thirds become active; the halves keep their moved split.
        s.apply(c, "DP-1", thirds.clone(), true, false, WORK, 0, all);
        s.join_group(c, d, 1);
        s.move_splits(c, WORK, &[(true, 640, 700)]);
        s.move_splits(a, WORK, &[(true, 1100, 1200)]);
        let mut active = s.active[&k("DP-1")].clone();
        assert_eq!(
            active.zones(WORK)[1],
            Rect::new(700, 0, 580, 1032),
            "the halves' resize leaves the thirds' split"
        );
        // The bar moves to the left side, 60 wide: 1200 of 1920 is 1163
        // of 1860 (1162.5 rounded up) from x 60, 700 is 678, 1280 is 1240.
        let moved = Rect::new(60, 0, 1860, 1032);
        let mut moves = s.refit("DP-1", moved);
        moves.sort_by_key(|m| (m.1.x, m.1.width));
        assert_eq!(
            moves,
            vec![
                (c, Rect::new(60, 0, 678, 1032)),
                (a, Rect::new(60, 0, 1163, 1032)),
                (d, Rect::new(738, 0, 562, 1032)),
                (b, Rect::new(1223, 0, 697, 1032)),
            ]
        );
        assert!(
            s.refit("DP-1", moved).is_empty(),
            "laid on the new area once"
        );
        let back = s.refit("DP-1", WORK);
        assert!(
            back.contains(&(b, Rect::new(1200, 0, 720, 1032))),
            "back on the first area, the split is where the drag left it"
        );
    }
}
