//! Snap groups: forming, joining and leaving them, and sizing their members together.

use reprisekit::classic::frame::Edges;

use crate::raster::Rect;
use crate::snap::{ActiveLayout, Arrangement, Splits, Suggestion, Target, ZoneLayout};
use crate::window::WindowId;

use super::{Group, KeyLanding, MoveUndo, Partner, SizeEnd, Snapping, SuggestionPick};

impl Group {
    fn has_live(&self, live: &dyn Fn(WindowId) -> bool) -> bool {
        self.members.iter().any(|&(_, m)| live(m))
    }
}

/// A follower, the frame it started from and the (dragged side, own side) pairs it rides.
type Follower = (WindowId, Rect, Vec<(Edges, Edges)>);

/// Each follower once; its frame is its first entry's.
pub(super) fn partner_sides(partners: &[Partner]) -> Vec<Follower> {
    let mut out: Vec<Follower> = Vec::new();
    for p in partners {
        match out.iter_mut().find(|o| o.0 == p.window) {
            Some(o) => o.2.push((p.side, p.edge)),
            None => out.push((p.window, p.start, vec![(p.side, p.edge)])),
        }
    }
    out
}

/// A follower that started at `start`, its sides moved onto the lines of `r` it rides
/// (`sides`, as `partner_sides` gives them), and the edges of its own that moved.
pub(super) fn followed(start: Rect, sides: &[(Edges, Edges)], r: Rect) -> (Rect, Edges) {
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
    /// on every desktop, or gone) stays. A group with every member on every desktop moves to the
    /// desktop shown, its layout with it unless that desktop has one of its own there.
    pub fn settle_desktops(&mut self, desktop_of: impl Fn(WindowId) -> Option<usize>) {
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
        let current = self.desktop;
        for i in 0..self.groups.len() {
            let g = &self.groups[i];
            if g.desktop == current || g.members.iter().any(|&(_, m)| desktop_of(m).is_some()) {
                continue;
            }
            let old = (g.output.clone(), g.desktop);
            let layout = (self
                .active
                .get(&old)
                .filter(|a| a.layout == g.layout)
                .cloned())
            .unwrap_or_else(|| ActiveLayout {
                layout: g.layout.clone(),
                explicit: false,
                splits: g.splits.clone(),
            });
            self.active
                .entry((g.output.clone(), current))
                .or_insert(layout);
            self.groups[i].desktop = current;
            self.drop_unheld_layout(&old);
        }
    }

    /// The size ends: the followers it brought along, and what it did if it moved its window.
    pub(super) fn end_resize(&mut self) -> (Vec<WindowId>, Option<SizeEnd>) {
        let partners = std::mem::take(&mut self.partners);
        let followers = partner_sides(&partners).into_iter().map(|f| f.0).collect();
        let (Some((window, start, edges)), Some(r)) = (self.resize.take(), self.last.take()) else {
            return (followers, None);
        };
        let followed: Vec<Edges> = partners.iter().map(|p| p.side).collect();
        let off_zone = crate::snap::lone_side(start, r, edges, &followed);
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
        let end = SizeEnd {
            window,
            start,
            moves,
            off_zone,
        };
        (followers, Some(end))
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
