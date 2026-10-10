use crate::window::{SizeLimits, WindowId};
use reprisekit::classic::frame::Edges;
use reprisekit::raster::Rect;
use std::collections::VecDeque;

const SIDES: [Edges; 4] = [Edges::LEFT, Edges::RIGHT, Edges::TOP, Edges::BOTTOM];

pub fn sides(edges: Edges) -> impl Iterator<Item = Edges> {
    SIDES.into_iter().filter(move |&s| edges.contains(s))
}

/// A terminal or editor sizes itself in whole cells, so it may stop a
/// little short of its zone and still share the side.
pub const SNUG: i32 = 32;

/// The side of `b` that follows side `side` of `a` when a drag moves it:
/// `b`'s facing side when the two stand within [`SNUG`] of each other
/// along `side` and overlap across it; None otherwise.
pub fn follower(a: Rect, side: Edges, b: Rect) -> Option<Edges> {
    let across_x = a.x < b.right() && b.x < a.right();
    let across_y = a.y < b.bottom() && b.y < a.bottom();
    let near = |p: i32, q: i32| (p - q).abs() <= SNUG;
    match side {
        Edges::RIGHT if near(b.x, a.right()) && across_y => Some(Edges::LEFT),
        Edges::LEFT if near(b.right(), a.x) && across_y => Some(Edges::RIGHT),
        Edges::BOTTOM if near(b.y, a.bottom()) && across_x => Some(Edges::TOP),
        Edges::TOP if near(b.bottom(), a.y) && across_x => Some(Edges::BOTTOM),
        _ => None,
    }
}

pub fn side_at(r: Rect, side: Edges) -> i32 {
    match side {
        Edges::LEFT => r.x,
        Edges::RIGHT => r.right(),
        Edges::TOP => r.y,
        Edges::BOTTOM => r.bottom(),
        _ => 0,
    }
}

fn with_side(r: Rect, side: Edges, v: i32) -> Rect {
    match side {
        Edges::LEFT => Rect::new(v, r.y, r.right() - v, r.height),
        Edges::RIGHT => Rect::new(r.x, r.y, v - r.x, r.height),
        Edges::TOP => Rect::new(r.x, v, r.width, r.bottom() - v),
        Edges::BOTTOM => Rect::new(r.x, r.y, r.width, v - r.y),
        _ => r,
    }
}

/// The windows a drag of side `side` of `start` (the dragged window's
/// frame at the drag's start) sizes along with it, from `others`' frames,
/// as (index into `others`, its side that follows).
pub fn line_followers(start: Rect, side: Edges, others: &[Rect]) -> Vec<(usize, Edges)> {
    if !others.iter().any(|&b| follower(start, side, b).is_some()) {
        return Vec::new();
    }
    let line = side_at(start, side);
    let facing = match side {
        Edges::LEFT => Edges::RIGHT,
        Edges::RIGHT => Edges::LEFT,
        Edges::TOP => Edges::BOTTOM,
        Edges::BOTTOM => Edges::TOP,
        _ => return Vec::new(),
    };
    let on = |b: Rect, s: Edges| (side_at(b, s) - line).abs() <= SNUG;
    (others.iter().enumerate())
        .filter_map(|(i, &b)| {
            if on(b, facing) {
                Some((i, facing))
            } else if on(b, side) {
                Some((i, side))
            } else {
                None
            }
        })
        .collect()
}

pub fn follow(b: Rect, edge: Edges, line: i32) -> Rect {
    with_side(b, edge, line)
}

pub fn hold_for(a: Rect, side: Edges, b: Rect, edge: Edges, min: (i32, i32)) -> Rect {
    let v = side_at(a, side);
    let v = match edge {
        Edges::LEFT => v.min(b.right() - min.0),
        Edges::RIGHT => v.max(b.x + min.0),
        Edges::TOP => v.min(b.bottom() - min.1),
        Edges::BOTTOM => v.max(b.y + min.1),
        _ => v,
    };
    with_side(a, side, v)
}

pub fn lone_side(start: Rect, end: Rect, edges: Edges, followed: &[Edges]) -> bool {
    sides(edges).any(|s| !followed.contains(&s) && side_at(start, s) != side_at(end, s))
}

fn boundaries(start: i32, size: i32, weights: &[f64]) -> Vec<i32> {
    debug_assert!(!weights.is_empty());
    let total: f64 = weights.iter().sum();
    let mut out = Vec::with_capacity(weights.len() + 1);
    out.push(start);
    let mut cum = 0.0;
    for w in &weights[..weights.len() - 1] {
        cum += w;
        out.push(start + (size as f64 * cum / total).round() as i32);
    }
    out.push(start + size);
    out
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SnapTraits {
    pub mapped: bool,
    pub fullscreen: bool,
    pub resizable: bool,
    pub owned: bool,
    pub tab_hidden: bool,
}

pub fn snappable(t: SnapTraits) -> bool {
    t.mapped && !t.fullscreen && t.resizable && !t.owned && !t.tab_hidden
}

pub fn commit_size(client: (i32, i32), limits: SizeLimits) -> (i32, i32) {
    let fit = |v: i32, min: i32, max: Option<i32>| {
        let v = v.max(min);
        max.map_or(v, |m| v.min(m))
    };
    (
        fit(client.0, limits.min.0, limits.max.0),
        fit(client.1, limits.min.1, limits.max.1),
    )
}

pub fn drag_out_grab(cur: Rect, restored: Rect, press: (f64, f64)) -> (f64, f64) {
    let share =
        |p: f64, start: i32, len: i32| ((p - start as f64) / len.max(1) as f64).clamp(0.0, 1.0);
    let x = share(press.0, cur.x, cur.width) * restored.width as f64;
    let depth = press.1 - cur.y as f64;
    let y = if depth < restored.height as f64 {
        depth
    } else {
        share(press.1, cur.y, cur.height) * restored.height as f64
    };
    (x, y)
}

/// Spelt as band weights joined by `|` (columns) or `-` (rows), a band cut
/// further as `weight:parts`: "1|1:2" is a half and two quarters.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneLayout {
    pub columns: bool,
    pub bands: Vec<(f64, u32)>,
}

pub const ZONE_MAX: usize = 6;

impl ZoneLayout {
    pub fn parse(spec: &str) -> Result<ZoneLayout, String> {
        let spec = spec.trim();
        let columns = !spec.contains('-');
        if !columns && spec.contains('|') {
            return Err(format!("{spec:?} mixes | and -"));
        }
        let sep = if columns { '|' } else { '-' };
        let band = |band: &str| -> Result<(f64, u32), String> {
            let (w, parts) = band.split_once(':').unwrap_or((band, "1"));
            let (w, parts) = (w.trim(), parts.trim());
            let weight = (w.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0);
            let Some(weight) = weight else {
                return Err(format!("{spec:?}: {w:?} is not a weight > 0"));
            };
            match parts.parse::<u32>() {
                Ok(p) if (1..=ZONE_MAX as u32).contains(&p) => Ok((weight, p)),
                _ => Err(format!(
                    "{spec:?}: {parts:?} is not a count of parts from 1 to {ZONE_MAX}"
                )),
            }
        };
        let bands = spec.split(sep).map(band).collect::<Result<Vec<_>, _>>()?;
        if bands.len() > ZONE_MAX {
            return Err(format!("{spec:?}: at most {ZONE_MAX} bands"));
        }
        Ok(ZoneLayout { columns, bands })
    }

    pub fn zones(&self, work: Rect) -> Vec<Rect> {
        let weights: Vec<f64> = self.bands.iter().map(|b| b.0).collect();
        let (along, across) = if self.columns {
            ((work.x, work.width), (work.y, work.height))
        } else {
            ((work.y, work.height), (work.x, work.width))
        };
        let cuts = boundaries(along.0, along.1, &weights);
        let mut out = Vec::new();
        for (i, &(_, parts)) in self.bands.iter().enumerate() {
            let parts = boundaries(across.0, across.1, &vec![1.0; parts as usize]);
            for k in 0..parts.len() - 1 {
                let (a0, a1, b0, b1) = (cuts[i], cuts[i + 1], parts[k], parts[k + 1]);
                out.push(if self.columns {
                    Rect::new(a0, b0, a1 - a0, b1 - b0)
                } else {
                    Rect::new(b0, a0, b1 - b0, a1 - a0)
                });
            }
        }
        out
    }

    pub fn zone_count(&self) -> usize {
        self.bands.iter().map(|b| b.1 as usize).sum()
    }
}

pub fn picker_layouts(work: Rect) -> Vec<ZoneLayout> {
    layouts_of(work, &["1|1", "2|1", "1|1|1", "1|1:2", "1:2|1:2", "1|2|1"])
}

pub fn bar_layouts(work: Rect) -> Vec<ZoneLayout> {
    layouts_of(work, &["1|1", "2|1", "1|1:2", "1:2|1:2", "1|1|1", "1|2|1"])
}

fn layouts_of(work: Rect, specs: &[&str]) -> Vec<ZoneLayout> {
    let portrait = work.width < work.height;
    (specs.iter())
        .map(|s| {
            if portrait {
                s.replace('|', "-")
            } else {
                s.to_string()
            }
        })
        .filter_map(|s| ZoneLayout::parse(&s).ok())
        .collect()
}

/// How far in from the usable area's edges a title-bar drag snaps.
pub const BAND: i32 = 30;

/// Zone order: top-left, bottom-left, top-right, bottom-right.
pub fn quarters() -> ZoneLayout {
    static QUARTERS: std::sync::LazyLock<ZoneLayout> = std::sync::LazyLock::new(|| {
        ZoneLayout::parse("1:2|1:2").expect("the quarters' spec parses")
    });
    QUARTERS.clone()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Zone { zone: usize },
    Corner { quarter: usize },
    Maximize,
}

pub fn edge_zone(work: Rect, zones: &[Rect], left: bool, y: i32) -> Option<usize> {
    let y = y.clamp(work.y, work.bottom() - 1);
    zones.iter().position(|z| {
        let on_edge = if left {
            z.x == work.x
        } else {
            z.right() == work.right()
        };
        on_edge && z.y <= y && y < z.bottom()
    })
}

pub fn edge_target(work: Rect, zones: &[Rect], x: i32, y: i32) -> Option<Target> {
    let left = x < work.x + BAND;
    let right = x >= work.right() - BAND;
    let top = y < work.y + BAND;
    let bottom = y >= work.bottom() - BAND;
    let near_side = x < work.x + 2 * BAND || x >= work.right() - 2 * BAND;
    let near_end = y < work.y + 2 * BAND || y >= work.bottom() - 2 * BAND;
    if ((left || right) && near_end) || ((top || bottom) && near_side) {
        let col = usize::from(x >= work.x + work.width / 2);
        let row = usize::from(y >= work.y + work.height / 2);
        return Some(Target::Corner {
            quarter: col * 2 + row,
        });
    }
    if top {
        return Some(Target::Maximize);
    }
    if left || right {
        let zone = edge_zone(work, zones, left, y)?;
        return Some(Target::Zone { zone });
    }
    None
}

pub fn target_at(
    work: Rect,
    zones: &[Rect],
    explicit: bool,
    shift: bool,
    bar: Option<Rect>,
    x: i32,
    y: i32,
) -> Option<Target> {
    if bar.is_some_and(|b| b.contains(x, y)) {
        return None;
    }
    edge_target(work, zones, x, y).or_else(|| {
        if !explicit || !shift {
            return None;
        }
        let zone = zones.iter().position(|z| z.contains(x, y))?;
        Some(Target::Zone { zone })
    })
}

/// Nothing until the pointer has left every edge band, so a window grabbed
/// inside one (a maximized one by its caption) does not drop back.
#[allow(clippy::too_many_arguments)]
pub fn armed_target(
    armed: &mut bool,
    work: Rect,
    zones: &[Rect],
    explicit: bool,
    shift: bool,
    bar: Option<Rect>,
    x: i32,
    y: i32,
) -> Option<Target> {
    *armed |= edge_target(work, zones, x, y).is_none();
    if !*armed {
        return None;
    }
    target_at(work, zones, explicit, shift, bar, x, y)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveLayout {
    pub layout: ZoneLayout,
    pub explicit: bool,
    pub splits: Splits,
}

impl ActiveLayout {
    pub fn new(layout: ZoneLayout, explicit: bool, work: Rect) -> ActiveLayout {
        let splits = Splits::new(&layout, work);
        ActiveLayout {
            layout,
            explicit,
            splits,
        }
    }

    pub fn default_for(work: Rect) -> ActiveLayout {
        let halves = ZoneLayout {
            columns: true,
            bands: vec![(1.0, 1), (1.0, 1)],
        };
        ActiveLayout::new(halves, false, work)
    }

    pub fn zones(&mut self, work: Rect) -> &[Rect] {
        self.splits.zones(work)
    }

    pub fn move_split(&mut self, vertical: bool, from: i32, to: i32) {
        self.splits.move_split(vertical, from, to);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Splits {
    work: Rect,
    zones: Vec<Rect>,
    /// The area and zones the last cut or split move left; every other area
    /// scales from these, so going back to it restores them.
    base: (Rect, Vec<Rect>),
}

impl Splits {
    pub fn new(layout: &ZoneLayout, work: Rect) -> Splits {
        let zones = layout.zones(work);
        Splits {
            work,
            base: (work, zones.clone()),
            zones,
        }
    }

    pub fn work(&self) -> Rect {
        self.work
    }

    pub fn zones(&mut self, work: Rect) -> &[Rect] {
        if work != self.work {
            let (o, n) = (self.base.0, work);
            let scale = |v: i32, o0: i32, olen: i32, n0: i32, nlen: i32| {
                let olen = i64::from(olen.max(1));
                n0 + ((i64::from(v - o0) * i64::from(nlen) + olen / 2) / olen) as i32
            };
            let sx = |v| scale(v, o.x, o.width, n.x, n.width);
            let sy = |v| scale(v, o.y, o.height, n.y, n.height);
            self.zones = (self.base.1.iter())
                .map(|z| {
                    let (x0, x1, y0, y1) = (sx(z.x), sx(z.right()), sy(z.y), sy(z.bottom()));
                    Rect::new(x0, y0, x1 - x0, y1 - y0)
                })
                .collect();
            self.work = work;
        }
        &self.zones
    }

    pub fn move_split(&mut self, vertical: bool, from: i32, to: i32) {
        let w = self.work;
        let near = |v: i32| (v - from).abs() <= SNUG;
        for z in &mut self.zones {
            if vertical {
                let (mut a, mut b) = (z.x, z.right());
                if a != w.x && near(a) {
                    a = to;
                }
                if b != w.right() && near(b) {
                    b = to;
                }
                *z = Rect::new(a, z.y, b - a, z.height);
            } else {
                let (mut a, mut b) = (z.y, z.bottom());
                if a != w.y && near(a) {
                    a = to;
                }
                if b != w.bottom() && near(b) {
                    b = to;
                }
                *z = Rect::new(z.x, a, z.width, b - a);
            }
        }
        self.base = (self.work, self.zones.clone());
    }
}

pub fn neighbour(zones: &[Rect], i: usize, dir: Dir) -> Option<usize> {
    let z = *zones.get(i)?;
    let (mx, my) = (z.x + z.width / 2, z.y + z.height / 2);
    let across_y = |o: &Rect| o.y < z.bottom() && z.y < o.bottom();
    let across_x = |o: &Rect| o.x < z.right() && z.x < o.right();
    let touching = |o: &Rect| match dir {
        Dir::Left => o.right() == z.x && across_y(o),
        Dir::Right => o.x == z.right() && across_y(o),
        Dir::Up => o.bottom() == z.y && across_x(o),
        Dir::Down => o.y == z.bottom() && across_x(o),
    };
    let middle = |o: &Rect| match dir {
        Dir::Left | Dir::Right => o.y <= my && my < o.bottom(),
        Dir::Up | Dir::Down => o.x <= mx && mx < o.right(),
    };
    let found: Vec<usize> = (0..zones.len())
        .filter(|&k| k != i && touching(&zones[k]))
        .collect();
    found
        .iter()
        .copied()
        .find(|&k| middle(&zones[k]))
        .or(found.first().copied())
}

pub fn zone_of(zones: &[Rect], frame: Rect) -> Option<usize> {
    let area = |r: Rect| i64::from(r.width) * i64::from(r.height);
    let (best, overlap) = zones
        .iter()
        .enumerate()
        .map(|(i, z)| (i, area(z.intersect(&frame))))
        .max_by_key(|&(i, a)| (a, std::cmp::Reverse(i)))?;
    (overlap > 0 && overlap * 2 >= area(frame)).then_some(best)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStep {
    Zone(usize),
    NextOutput,
    Maximize,
    Unmaximize,
    Unsnap,
    Nothing,
}

pub fn key_step(
    work: Rect,
    zones: &[Rect],
    at: Option<usize>,
    maximized: bool,
    snapped: bool,
    y: i32,
    dir: Dir,
) -> KeyStep {
    let zone = |z: Option<usize>| z.map_or(KeyStep::Nothing, KeyStep::Zone);
    match (dir, at) {
        (Dir::Left | Dir::Right, Some(i)) if !maximized => {
            neighbour(zones, i, dir).map_or(KeyStep::NextOutput, KeyStep::Zone)
        }
        (Dir::Left | Dir::Right, _) => zone(edge_zone(work, zones, dir == Dir::Left, y)),
        (Dir::Up, _) if maximized => KeyStep::Nothing,
        (Dir::Up, Some(i)) => neighbour(zones, i, Dir::Up).map_or(KeyStep::Maximize, KeyStep::Zone),
        (Dir::Up, None) => KeyStep::Maximize,
        (Dir::Down, _) if maximized => KeyStep::Unmaximize,
        (Dir::Down, Some(i)) => {
            neighbour(zones, i, Dir::Down).map_or(KeyStep::Unsnap, KeyStep::Zone)
        }
        (Dir::Down, None) if snapped => KeyStep::Unsnap,
        (Dir::Down, None) => KeyStep::Nothing,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

pub const RECENTS_MAX: usize = 8;
pub const SUGGESTIONS_MAX: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Arrangement {
    pub group: u64,
    pub output: String,
    pub layout: ZoneLayout,
    /// (zone, window).
    pub members: Vec<(usize, WindowId)>,
}

impl Arrangement {
    fn same(&self, o: &Arrangement) -> bool {
        self.output == o.output
            && self.layout == o.layout
            && self.members.len() == o.members.len()
            && self.members.iter().all(|m| o.members.contains(m))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Recents {
    list: VecDeque<Arrangement>,
}

impl Recents {
    pub fn record(&mut self, a: Arrangement) {
        self.list.retain(|o| o.group != a.group && !o.same(&a));
        if a.members.is_empty() {
            return;
        }
        self.list.push_front(a);
        self.list.truncate(RECENTS_MAX);
    }

    pub fn forget_window(&mut self, w: WindowId) {
        let mut kept: VecDeque<Arrangement> = VecDeque::new();
        for mut a in self.list.drain(..) {
            a.members.retain(|&(_, m)| m != w);
            if !a.members.is_empty() && !kept.iter().any(|k| k.same(&a)) {
                kept.push_back(a);
            }
        }
        self.list = kept;
    }

    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = &Arrangement> {
        self.list.iter()
    }

    pub fn suggestions(
        &self,
        output: &str,
        w: WindowId,
        open: impl Fn(WindowId) -> bool,
    ) -> Vec<Suggestion> {
        let mut out: Vec<Suggestion> = Vec::new();
        for a in self.list.iter().filter(|a| a.output == output) {
            let filled: Vec<(usize, WindowId)> = (a.members.iter().copied())
                .filter(|&(_, m)| m != w && open(m))
                .collect();
            if filled.is_empty() || filled.len() >= a.layout.zone_count() {
                continue;
            }
            let s = Suggestion {
                layout: a.layout.clone(),
                filled,
            };
            if !out.contains(&s) {
                out.push(s);
            }
            if out.len() == SUGGESTIONS_MAX {
                break;
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub layout: ZoneLayout,
    /// (zone, window).
    pub filled: Vec<(usize, WindowId)>,
}

#[cfg(test)]
impl Suggestion {
    pub fn free(&self) -> Vec<usize> {
        (0..self.layout.zone_count())
            .filter(|z| !self.filled.iter().any(|f| f.0 == *z))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_of_a_side_no_member_follows_takes_the_window_off_its_zone() {
        let left = Rect::new(0, 0, 960, 1032);
        let wider = Rect::new(0, 0, 1000, 1032);
        let shorter = Rect::new(0, 0, 1000, 900);
        let right = [Edges::RIGHT];
        assert!(
            !lone_side(left, wider, Edges::RIGHT, &right),
            "the shared side moved with its neighbour"
        );
        assert!(
            lone_side(left, wider, Edges::RIGHT, &[]),
            "no member beside it"
        );
        assert!(
            lone_side(left, shorter, Edges::RIGHT | Edges::BOTTOM, &right),
            "the outer bottom side moved too"
        );
        assert!(
            !lone_side(left, left, Edges::LEFT, &[]),
            "a press on the edge that moved nothing"
        );
    }

    #[test]
    fn only_a_mapped_resizable_window_that_is_no_dialog_or_hidden_tab_snaps() {
        let ok = SnapTraits {
            mapped: true,
            resizable: true,
            ..SnapTraits::default()
        };
        assert!(snappable(ok));
        assert!(
            !snappable(SnapTraits {
                mapped: false,
                ..ok
            }),
            "unmapped"
        );
        assert!(
            !snappable(SnapTraits {
                fullscreen: true,
                ..ok
            }),
            "fullscreen"
        );
        assert!(
            !snappable(SnapTraits {
                resizable: false,
                ..ok
            }),
            "fixed size"
        );
        assert!(!snappable(SnapTraits { owned: true, ..ok }), "a dialog");
        assert!(
            !snappable(SnapTraits {
                tab_hidden: true,
                ..ok
            }),
            "a hidden tab: snap assist offers its group's shown tab only"
        );
    }

    #[test]
    fn a_neighbour_follows_the_edge_it_shares() {
        let left = Rect::new(0, 0, 960, 1050);
        let top_right = Rect::new(960, 0, 960, 525);
        let bottom_right = Rect::new(960, 525, 960, 525);
        assert_eq!(follower(left, Edges::RIGHT, top_right), Some(Edges::LEFT));
        assert_eq!(
            follower(left, Edges::RIGHT, bottom_right),
            Some(Edges::LEFT)
        );
        assert_eq!(follower(top_right, Edges::LEFT, left), Some(Edges::RIGHT));
        assert_eq!(
            follower(top_right, Edges::BOTTOM, bottom_right),
            Some(Edges::TOP)
        );
        assert_eq!(follower(left, Edges::LEFT, top_right), None, "far side");
        let short = Rect::new(0, 0, 951, 1043);
        assert_eq!(follower(short, Edges::RIGHT, top_right), Some(Edges::LEFT));
        assert_eq!(
            follower(Rect::new(0, 0, 927, 1050), Edges::RIGHT, top_right),
            None,
            "33 px apart"
        );
        assert_eq!(
            follower(top_right, Edges::LEFT, Rect::new(0, 525, 960, 525)),
            None,
            "touching only at a corner"
        );
        assert_eq!(sides(Edges::RIGHT | Edges::BOTTOM).count(), 2);
    }

    #[test]
    fn the_follower_takes_the_new_edge_and_holds_the_drag_at_its_minimum() {
        let left = Rect::new(0, 0, 1000, 1050);
        let right = Rect::new(960, 0, 960, 1050);
        assert_eq!(
            follow(right, Edges::LEFT, left.right()),
            Rect::new(1000, 0, 920, 1050)
        );
        assert_eq!(
            follow(Rect::new(0, 0, 960, 1050), Edges::RIGHT, 900),
            Rect::new(0, 0, 900, 1050)
        );
        let far = Rect::new(0, 0, 1800, 1050);
        assert_eq!(
            hold_for(far, Edges::RIGHT, right, Edges::LEFT, (300, 100)),
            Rect::new(0, 0, 1620, 1050)
        );
        assert_eq!(
            hold_for(left, Edges::RIGHT, right, Edges::LEFT, (300, 100)),
            left
        );
        let wide = Rect::new(100, 0, 1820, 1050);
        let half = Rect::new(0, 0, 960, 1050);
        assert_eq!(
            hold_for(wide, Edges::LEFT, half, Edges::RIGHT, (300, 100)),
            Rect::new(300, 0, 1620, 1050)
        );
        let tall = Rect::new(960, 0, 960, 1000);
        let below = Rect::new(960, 525, 960, 525);
        assert_eq!(
            hold_for(tall, Edges::BOTTOM, below, Edges::TOP, (300, 100)),
            Rect::new(960, 0, 960, 950)
        );
        let narrow = Rect::new(0, 0, 100, 516);
        assert_eq!(
            hold_for(narrow, Edges::RIGHT, half, Edges::RIGHT, (300, 100)),
            Rect::new(0, 0, 300, 516)
        );
    }

    #[test]
    fn a_shared_border_drag_takes_every_window_on_its_line() {
        let q = zl("1:2|1:2").zones(WORK);
        let others = [q[1], q[2], q[3]];
        assert_eq!(
            line_followers(q[0], Edges::RIGHT, &others),
            vec![(0, Edges::RIGHT), (1, Edges::LEFT), (2, Edges::LEFT)],
            "both rows follow the split between the columns"
        );
        assert_eq!(
            line_followers(q[0], Edges::BOTTOM, &others),
            vec![(0, Edges::TOP), (1, Edges::BOTTOM), (2, Edges::TOP)],
            "both columns follow the split between the rows"
        );
        assert!(
            line_followers(q[0], Edges::LEFT, &others).is_empty(),
            "the work area's edge"
        );
        assert!(
            line_followers(q[0], Edges::RIGHT, &[q[1]]).is_empty(),
            "nothing across the line to share it"
        );
        let short = Rect::new(0, 516, 951, 510);
        assert_eq!(
            line_followers(q[0], Edges::RIGHT, &[short, q[2]]),
            vec![(0, Edges::RIGHT), (1, Edges::LEFT)]
        );
        let halves = zl("1|1:2").zones(WORK);
        assert_eq!(
            line_followers(halves[1], Edges::BOTTOM, &[halves[0], halves[2]]),
            vec![(1, Edges::TOP)],
            "the left half has no side on the row split"
        );
    }

    #[test]
    fn a_zone_layout_reads_columns_rows_and_split_bands() {
        let z = ZoneLayout::parse("2|1").unwrap();
        let want = vec![(2.0, 1), (1.0, 1)];
        assert_eq!((z.columns, z.bands), (true, want));
        let z = ZoneLayout::parse(" 1 - 1:2 ").unwrap();
        assert_eq!((z.columns, z.bands), (false, vec![(1.0, 1), (1.0, 2)]));
        assert!(ZoneLayout::parse("1|1-1").is_err(), "mixed");
        assert!(ZoneLayout::parse("1||1").is_err(), "an empty band");
        assert!(ZoneLayout::parse("0|1").is_err());
        assert!(ZoneLayout::parse("1:0").is_err());
        assert!(ZoneLayout::parse("1:7").is_err());
        assert!(ZoneLayout::parse("1:x").is_err());
        assert!(ZoneLayout::parse("1|1|1|1|1|1|1").is_err(), "seven bands");
    }

    #[test]
    fn zones_tile_the_work_area_band_by_band() {
        let work = Rect::new(0, 30, 1920, 1050);
        let z = ZoneLayout::parse("1|1:2").unwrap().zones(work);
        assert_eq!(
            z,
            vec![
                Rect::new(0, 30, 960, 1050),
                Rect::new(960, 30, 960, 525),
                Rect::new(960, 555, 960, 525),
            ]
        );
        let z = ZoneLayout::parse("2-1:2").unwrap().zones(work);
        assert_eq!(
            z,
            vec![
                Rect::new(0, 30, 1920, 700),
                Rect::new(0, 730, 960, 350),
                Rect::new(960, 730, 960, 350),
            ]
        );
        let thirds = ZoneLayout::parse("1|1|1").unwrap();
        let area: i32 = (thirds.zones(Rect::new(0, 0, 1000, 10)).iter())
            .map(|r| r.width * r.height)
            .sum();
        assert_eq!(area, 10_000);
    }

    #[test]
    fn the_built_in_layouts_are_six_and_stack_on_a_portrait_screen() {
        let specs = |v: Vec<ZoneLayout>| {
            v.iter()
                .map(|l| (l.columns, l.bands.clone()))
                .collect::<Vec<_>>()
        };
        let wide = Rect::new(0, 0, 1920, 1032);
        let want = ["1|1", "2|1", "1|1|1", "1|1:2", "1:2|1:2", "1|2|1"];
        assert_eq!(
            specs(picker_layouts(wide)),
            specs(want.iter().map(|s| zl(s)).collect())
        );
        let bar = ["1|1", "2|1", "1|1:2", "1:2|1:2", "1|1|1", "1|2|1"];
        assert_eq!(
            specs(bar_layouts(wide)),
            specs(bar.iter().map(|s| zl(s)).collect())
        );
        let tall = picker_layouts(Rect::new(0, 0, 1080, 1890));
        assert_eq!(tall.len(), 6, "the wide middle on any screen");
        assert!(tall.iter().all(|l| !l.columns), "rows on a portrait screen");
        assert_eq!(tall[3], zl("1-1:2"));
    }

    #[test]
    fn the_committed_size_respects_the_limits() {
        let limits = |min, max| SizeLimits { min, max };
        let open = (None, None);
        assert_eq!(commit_size((500, 400), limits((1, 1), open)), (500, 400));
        assert_eq!(commit_size((500, 400), limits((600, 1), open)), (600, 400));
        let w300 = limits((1, 1), (Some(300), None));
        assert_eq!(commit_size((500, 400), w300), (300, 400));
        let h100 = limits((1, 1), (None, Some(100)));
        assert_eq!(commit_size((500, 400), h100), (500, 100));
    }

    #[test]
    fn a_window_dragged_out_of_its_fit_keeps_the_grab_on_the_frame() {
        let max = Rect::new(0, 0, 1920, 1050);
        let restored = Rect::new(300, 200, 800, 600);
        assert_eq!(drag_out_grab(max, restored, (480.0, 10.0)), (200.0, 10.0));
        assert_eq!(drag_out_grab(max, restored, (960.0, 840.0)), (400.0, 480.0));
    }

    const WORK: Rect = Rect::new(0, 0, 1920, 1032);

    fn zl(s: &str) -> ZoneLayout {
        ZoneLayout::parse(s).unwrap()
    }

    #[test]
    fn the_edges_snap_to_the_outer_zones_and_the_top_maximizes() {
        let halves = zl("1|1").zones(WORK);
        assert_eq!(
            edge_target(WORK, &halves, 0, 500),
            Some(Target::Zone { zone: 0 })
        );
        assert_eq!(
            edge_target(WORK, &halves, 29, 500),
            Some(Target::Zone { zone: 0 }),
            "the band is 30 px"
        );
        assert_eq!(edge_target(WORK, &halves, 30, 500), None);
        assert_eq!(
            edge_target(WORK, &halves, 1919, 500),
            Some(Target::Zone { zone: 1 })
        );
        assert_eq!(edge_target(WORK, &halves, 900, 0), Some(Target::Maximize));
        assert_eq!(
            edge_target(WORK, &halves, 900, 1031),
            None,
            "the bottom edge snaps nothing"
        );
        let stack = zl("1|1:2").zones(WORK);
        assert_eq!(
            edge_target(WORK, &stack, 1919, 700),
            Some(Target::Zone { zone: 2 })
        );
    }

    #[test]
    fn the_corners_give_quarters_and_reach_thirty_px_along_each_edge() {
        let halves = zl("1|1").zones(WORK);
        let q = |quarter| Some(Target::Corner { quarter });
        assert_eq!(edge_target(WORK, &halves, 0, 0), q(0));
        assert_eq!(
            edge_target(WORK, &halves, 5, 59),
            q(0),
            "30 px down the side"
        );
        assert_eq!(
            edge_target(WORK, &halves, 59, 5),
            q(0),
            "30 px along the top"
        );
        assert_eq!(
            edge_target(WORK, &halves, 5, 60),
            Some(Target::Zone { zone: 0 })
        );
        assert_eq!(edge_target(WORK, &halves, 0, 1031), q(1));
        assert_eq!(edge_target(WORK, &halves, 1919, 0), q(2));
        assert_eq!(edge_target(WORK, &halves, 1919, 1031), q(3));
    }

    #[test]
    fn an_edge_snap_follows_a_moved_split() {
        let mut a = ActiveLayout::new(zl("1|1"), false, WORK);
        a.move_split(true, 960, 1100);
        let zones = a.zones(WORK).to_vec();
        assert_eq!(
            zones,
            vec![Rect::new(0, 0, 1100, 1032), Rect::new(1100, 0, 820, 1032)]
        );
        assert_eq!(
            edge_target(WORK, &zones, 0, 400),
            Some(Target::Zone { zone: 0 })
        );
        // A split within SNUG of the dragged edge moves too; the outer edges never do.
        let mut q = ActiveLayout::new(zl("1:2|1:2"), true, WORK);
        q.move_split(true, 950, 800);
        assert_eq!(q.zones(WORK)[2], Rect::new(800, 0, 1120, 516));
        q.move_split(true, 0, 300);
        assert_eq!(q.zones(WORK)[0].x, 0, "the work area's edge stays");
        let mut q = ActiveLayout::new(zl("1:2|1:2"), true, WORK);
        q.move_split(false, 516, 400);
        let z = q.zones(WORK).to_vec();
        assert_eq!((z[0].bottom(), z[1].y), (400, 400));
        assert_eq!((z[2].bottom(), z[3].y), (400, 400));
        q.move_split(false, 1032, 900);
        assert_eq!(
            q.zones(WORK)[1].bottom(),
            1032,
            "the work area's foot stays"
        );
    }

    #[test]
    fn a_new_usable_area_scales_the_zones() {
        let mut a = ActiveLayout::new(zl("1|1"), false, WORK);
        a.move_split(true, 960, 1100);
        assert_eq!(a.splits.work(), WORK);
        // 1100 of 1920 is 1066 of 1860 (1066.125 rounded), from x 60.
        let moved = Rect::new(60, 0, 1860, 1080);
        assert_eq!(
            a.zones(moved).to_vec(),
            vec![Rect::new(60, 0, 1066, 1080), Rect::new(1126, 0, 794, 1080)],
            "the moved split keeps its share"
        );
        assert_eq!(a.splits.work(), moved);
        let mut q = ActiveLayout::new(zl("1:2|1:2"), true, WORK);
        let z = q.zones(Rect::new(0, 30, 1920, 1050)).to_vec();
        assert_eq!(z[0], Rect::new(0, 30, 960, 525), "rows scale too");
        assert_eq!(z[3].bottom(), 1080, "the far edges land on the area's");
    }

    #[test]
    fn zone_targets_need_an_explicit_layout_and_shift_and_the_edges_win() {
        let thirds = zl("1|1|1").zones(WORK);
        assert_eq!(target_at(WORK, &thirds, false, true, None, 960, 500), None);
        assert_eq!(
            target_at(WORK, &thirds, true, false, None, 960, 500),
            None,
            "a plain drag drops freely over the middle"
        );
        assert_eq!(
            target_at(WORK, &thirds, true, true, None, 960, 500),
            Some(Target::Zone { zone: 1 })
        );
        assert_eq!(
            target_at(WORK, &thirds, true, true, None, 10, 500),
            Some(Target::Zone { zone: 0 }),
            "the band, not the zone under it"
        );
        assert_eq!(
            target_at(WORK, &thirds, true, false, None, 10, 500),
            Some(Target::Zone { zone: 0 }),
            "edge snaps need no Shift"
        );
    }

    #[test]
    fn a_drag_drops_only_after_the_pointer_leaves_every_band() {
        let halves = zl("1|1").zones(WORK);
        let mut armed = false;
        assert_eq!(
            armed_target(&mut armed, WORK, &halves, false, false, None, 900, 5),
            None,
            "grabbed in the top band"
        );
        assert_eq!(
            armed_target(&mut armed, WORK, &halves, false, false, None, 300, 5),
            None,
            "slid along the top band"
        );
        assert!(!armed, "not armed inside a band");
        assert_eq!(
            armed_target(&mut armed, WORK, &halves, false, false, None, 900, 500),
            None
        );
        assert!(armed, "armed outside the bands");
        assert_eq!(
            armed_target(&mut armed, WORK, &halves, false, false, None, 900, 5),
            Some(Target::Maximize),
            "back in the top band"
        );
    }

    #[test]
    fn the_bar_hides_every_drop_under_it() {
        let halves = zl("1|1").zones(WORK);
        let bar = Some(Rect::new(513, -79, 894, 90));
        assert_eq!(
            target_at(WORK, &halves, true, true, bar, 900, 5),
            None,
            "the strip, not the top edge"
        );
        assert_eq!(
            target_at(WORK, &halves, true, true, bar, 300, 5),
            Some(Target::Maximize),
            "beside the bar"
        );
    }

    #[test]
    fn a_short_window_is_found_by_its_top_left() {
        let halves = zl("1|1").zones(WORK);
        assert_eq!(zone_of(&halves, halves[1]), Some(1));
        assert_eq!(
            zone_of(&halves, Rect::new(960, 0, 700, 990)),
            Some(1),
            "size limits keep it short"
        );
        assert_eq!(
            zone_of(&halves, Rect::new(961, 0, 700, 990)),
            Some(1),
            "a pixel off"
        );
    }

    #[test]
    fn a_window_is_found_by_the_zone_it_mostly_covers() {
        let z = zl("1|1:2").zones(WORK);
        let off = Rect::new(z[1].x - 11, z[1].y, z[1].width - 5, z[1].height - 7);
        assert_eq!(
            zone_of(&z, off),
            Some(1),
            "a few px off after a moved split"
        );
        let straddle = Rect::new(z[1].x, z[1].bottom() - 450, z[1].width, 1200);
        assert_eq!(zone_of(&z, straddle), None, "under half in each zone");
    }

    #[test]
    fn neighbours_touch_and_overlap() {
        let z = zl("1|1:2").zones(WORK);
        assert_eq!(
            neighbour(&z, 0, Dir::Right),
            Some(2),
            "the one holding zone 0's middle row"
        );
        assert_eq!(neighbour(&z, 1, Dir::Left), Some(0));
        assert_eq!(neighbour(&z, 1, Dir::Down), Some(2));
        assert_eq!(neighbour(&z, 2, Dir::Up), Some(1));
        assert_eq!(neighbour(&z, 0, Dir::Left), None);
        assert_eq!(neighbour(&z, 0, Dir::Up), None);
    }

    #[test]
    fn super_arrows_walk_halves() {
        let z = zl("1|1").zones(WORK);
        let step = |at, max, dir| key_step(WORK, &z, at, max, at.is_some(), 500, dir);
        assert_eq!(step(None, false, Dir::Left), KeyStep::Zone(0));
        assert_eq!(step(None, false, Dir::Right), KeyStep::Zone(1));
        assert_eq!(step(Some(0), false, Dir::Right), KeyStep::Zone(1));
        assert_eq!(step(Some(1), false, Dir::Right), KeyStep::NextOutput);
        assert_eq!(step(Some(0), false, Dir::Up), KeyStep::Maximize);
        assert_eq!(step(None, false, Dir::Up), KeyStep::Maximize);
        assert_eq!(step(None, true, Dir::Up), KeyStep::Nothing);
        assert_eq!(step(None, true, Dir::Down), KeyStep::Unmaximize);
        assert_eq!(step(Some(0), false, Dir::Down), KeyStep::Unsnap);
        assert_eq!(step(None, false, Dir::Down), KeyStep::Nothing);
        assert_eq!(
            key_step(WORK, &z, None, false, true, 500, Dir::Down),
            KeyStep::Unsnap,
            "a snapped window off every zone gets its size from before the snap back"
        );
        assert_eq!(
            step(None, true, Dir::Left),
            KeyStep::Zone(0),
            "out of maximized to the left half"
        );
    }

    #[test]
    fn super_arrows_walk_thirds_and_quarters() {
        let t = zl("1|1|1").zones(WORK);
        assert_eq!(
            key_step(WORK, &t, Some(0), false, true, 500, Dir::Right),
            KeyStep::Zone(1)
        );
        assert_eq!(
            key_step(WORK, &t, Some(1), false, true, 500, Dir::Right),
            KeyStep::Zone(2)
        );
        assert_eq!(
            key_step(WORK, &t, Some(2), false, true, 500, Dir::Right),
            KeyStep::NextOutput
        );
        assert_eq!(
            key_step(WORK, &t, Some(1), false, true, 500, Dir::Left),
            KeyStep::Zone(0)
        );
        let q = zl("1:2|1:2").zones(WORK);
        assert_eq!(
            key_step(WORK, &q, Some(1), false, true, 700, Dir::Up),
            KeyStep::Zone(0)
        );
        assert_eq!(
            key_step(WORK, &q, Some(0), false, true, 200, Dir::Down),
            KeyStep::Zone(1)
        );
        assert_eq!(
            key_step(WORK, &q, Some(0), false, true, 200, Dir::Up),
            KeyStep::Maximize
        );
        assert_eq!(
            key_step(WORK, &q, Some(1), false, true, 700, Dir::Down),
            KeyStep::Unsnap
        );
        assert_eq!(
            key_step(WORK, &q, Some(3), false, true, 700, Dir::Left),
            KeyStep::Zone(1)
        );
        assert_eq!(
            key_step(WORK, &q, None, false, false, 700, Dir::Right),
            KeyStep::Zone(3),
            "its row on the right edge"
        );
    }

    fn arr(group: u64, layout: &str, members: &[(usize, WindowId)]) -> Arrangement {
        Arrangement {
            group,
            output: "DP-1".into(),
            layout: zl(layout),
            members: members.to_vec(),
        }
    }

    #[test]
    fn recents_update_a_group_merge_twins_and_keep_eight() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut r = Recents::default();
        r.record(arr(1, "1|1", &[(0, a)]));
        r.record(arr(1, "1|1", &[(0, a), (1, b)]));
        assert_eq!(r.iter().count(), 1, "the group's entry is updated");
        r.record(arr(2, "1|1", &[(1, b), (0, a)]));
        assert_eq!(r.iter().count(), 1, "the same layout and members merge");
        assert_eq!(r.iter().next().unwrap().group, 2, "most recent first");
        r.record(arr(3, "2|1", &[(0, c)]));
        assert_eq!(r.iter().next().unwrap().group, 3);
        for g in 10..20 {
            r.record(arr(g, "1|1|1", &[(0, WindowId::next())]));
        }
        assert_eq!(r.iter().count(), RECENTS_MAX);
        let kept: Vec<u64> = r.iter().map(|a| a.group).collect();
        assert_eq!(kept, (12..20).rev().collect::<Vec<_>>(), "the oldest go");
    }

    #[test]
    fn a_closed_window_leaves_every_entry() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut r = Recents::default();
        r.record(arr(1, "1|1", &[(0, a), (1, b)]));
        r.record(arr(2, "2|1", &[(0, a)]));
        r.forget_window(a);
        let left: Vec<_> = r.iter().map(|x| x.members.clone()).collect();
        assert_eq!(left, vec![vec![(1, b)]], "the entry left empty goes");
    }

    #[test]
    fn a_suggestion_needs_an_open_member_and_a_free_zone() {
        let (a, b, me, shut) = (
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
            WindowId::next(),
        );
        let mut r = Recents::default();
        r.record(arr(1, "1|1", &[(0, a), (1, b)]));
        r.record(arr(2, "1|1:2", &[(0, a), (2, me)]));
        r.record(arr(3, "2|1", &[(0, shut)]));
        let open = |w: WindowId| w != shut;
        let s = r.suggestions("DP-1", me, open);
        assert_eq!(
            s.len(),
            1,
            "halves are full; the shut window's entry has nobody open"
        );
        assert_eq!(s[0].layout, zl("1|1:2"));
        assert_eq!(
            s[0].filled,
            vec![(0, a)],
            "the window being snapped is left out"
        );
        assert_eq!(s[0].free(), vec![1, 2]);
        assert!(
            r.suggestions("HDMI-A-1", me, open).is_empty(),
            "another output's"
        );
    }

    #[test]
    fn a_closed_member_leaves_the_suggestion() {
        let (a, b, me) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut r = Recents::default();
        r.record(Arrangement {
            group: 1,
            output: "DP-1".into(),
            layout: zl("1:2|1:2"),
            members: vec![(0, a), (1, b)],
        });
        let s = r.suggestions("DP-1", me, |w| w != b);
        assert_eq!(s[0].filled, vec![(0, a)]);
        assert_eq!(s[0].free(), vec![1, 2, 3]);
    }
}
