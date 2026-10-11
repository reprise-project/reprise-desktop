use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::draw_focus_ring;
use crate::look::Paint;
use crate::look::widgets::draw_field_frame;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;
use accesskit::{NodeId, Role};

const FILL: f64 = 0.75;
/// Well px within which a dragged tile's edge lines up with a neighbour's.
const SNAP_PX: f64 = 8.0;
pub const BADGE: i32 = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub number: u32,
    pub name: String,
    /// Place and logical size in the compositor's layout.
    pub pos: (i32, i32),
    pub size: (i32, i32),
}

impl Tile {
    pub fn rect(&self) -> Rect {
        Rect::new(self.pos.0, self.pos.1, self.size.0, self.size.1)
    }
}

/// Maps layout coordinates into the well: `x * scale + ox`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fit {
    pub scale: f64,
    pub ox: f64,
    pub oy: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Held {
    tile: usize,
    /// Where the press landed, in form coordinates.
    from: (i32, i32),
    start: (i32, i32),
    /// The press's fit, kept so the well does not zoom while a tile moves.
    fit: Fit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arrange {
    pub tiles: Vec<Tile>,
    pub selected: usize,
    pub enabled: bool,
    held: Option<Held>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub frame: Rect,
    pub inner: Rect,
    pub fit: Fit,
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

pub fn touches(a: Rect, b: Rect) -> bool {
    if overlaps(a, b) {
        return false;
    }
    let side = (a.right() == b.x || b.right() == a.x) && a.y < b.bottom() && b.y < a.bottom();
    let flat = (a.bottom() == b.y || b.bottom() == a.y) && a.x < b.right() && b.x < a.right();
    side || flat
}

pub fn whole(tiles: &[Tile]) -> bool {
    let r: Vec<Rect> = tiles.iter().map(Tile::rect).collect();
    for i in 0..r.len() {
        for j in i + 1..r.len() {
            if overlaps(r[i], r[j]) {
                return false;
            }
        }
    }
    let mut seen = vec![false; r.len()];
    let mut stack: Vec<usize> = (!r.is_empty()).then_some(0).into_iter().collect();
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        stack.extend((0..r.len()).filter(|&j| !seen[j] && touches(r[i], r[j])));
    }
    seen.iter().all(|s| *s)
}

fn bounds(tiles: &[Tile]) -> Option<Rect> {
    tiles.iter().map(Tile::rect).reduce(|a, b| a.union(&b))
}

fn align(v: i32, to: &[i32], snap: i32) -> i32 {
    to.iter()
        .copied()
        .filter(|t| (t - v).abs() <= snap)
        .min_by_key(|t| (t - v).abs())
        .unwrap_or(v)
}

/// Left of, right of, above and below `o` for a tile of `size`: `y` places the
/// first two, `x` the last two.
fn beside(o: Rect, (w, h): (i32, i32), x: i32, y: i32) -> [(i32, i32); 4] {
    [(o.x - w, y), (o.right(), y), (x, o.y - h), (x, o.bottom())]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// The next place in `dir` for a tile of `size` at `from` among `others`: against an
/// edge, lined up with it or keeping `from`'s offset, overlapping none. A place
/// straight along `dir` comes first, then the least sideways shift, then the nearest.
pub fn shove(others: &[Rect], size: (i32, i32), from: (i32, i32), dir: Dir) -> Option<(i32, i32)> {
    let (w, h) = size;
    let mut best: Option<((i32, i32), (i32, i32))> = None;
    for o in others {
        for x in [o.x, o.right() - w, from.0] {
            for y in [o.y, o.bottom() - h, from.1] {
                for at in beside(*o, size, x, y) {
                    let (dx, dy) = (at.0 - from.0, at.1 - from.1);
                    let (ahead, side) = match dir {
                        Dir::Left => (-dx, dy),
                        Dir::Right => (dx, dy),
                        Dir::Up => (-dy, dx),
                        Dir::Down => (dy, dx),
                    };
                    let r = Rect::new(at.0, at.1, w, h);
                    if ahead <= 0 || !touches(r, *o) || others.iter().any(|q| overlaps(r, *q)) {
                        continue;
                    }
                    let rank = (side.abs(), ahead);
                    if best.is_none_or(|(_, b)| rank < b) {
                        best = Some((at, rank));
                    }
                }
            }
        }
    }
    best.map(|(at, _)| at)
}

pub fn snap(others: &[Rect], size: (i32, i32), want: (i32, i32), snap: i32) -> Option<(i32, i32)> {
    let (w, h) = size;
    let mut best: Option<((i32, i32), i64)> = None;
    for o in others {
        let y = align(
            want.1.clamp(o.y - h + 1, o.bottom() - 1),
            &[o.y, o.bottom() - h],
            snap,
        );
        let x = align(
            want.0.clamp(o.x - w + 1, o.right() - 1),
            &[o.x, o.right() - w],
            snap,
        );
        for at in beside(*o, size, x, y) {
            let r = Rect::new(at.0, at.1, w, h);
            if others.iter().any(|q| overlaps(r, *q)) {
                continue;
            }
            let d = i64::from(at.0 - want.0).pow(2) + i64::from(at.1 - want.1).pow(2);
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((at, d));
            }
        }
    }
    best.map(|(at, _)| at)
}

pub fn settle(tiles: &mut [Tile], fixed: usize) {
    if fixed >= tiles.len() {
        return;
    }
    let mut placed = vec![fixed];
    let mut left: Vec<usize> = (0..tiles.len()).filter(|&i| i != fixed).collect();
    while !left.is_empty() {
        let rects: Vec<Rect> = placed.iter().map(|&i| tiles[i].rect()).collect();
        let fine = left.iter().position(|&i| {
            let r = tiles[i].rect();
            rects.iter().any(|p| touches(r, *p)) && !rects.iter().any(|p| overlaps(r, *p))
        });
        let k = fine.unwrap_or_else(|| {
            let gap = |i: usize| {
                let r = tiles[i].rect();
                rects
                    .iter()
                    .map(|p| {
                        let dx = (p.x - r.right()).max(r.x - p.right()).max(0);
                        let dy = (p.y - r.bottom()).max(r.y - p.bottom()).max(0);
                        i64::from(dx).pow(2) + i64::from(dy).pow(2)
                    })
                    .min()
                    .unwrap_or(0)
            };
            (0..left.len())
                .min_by_key(|&j| gap(left[j]))
                .expect("left is not empty")
        });
        let i = left.remove(k);
        if fine.is_none()
            && let Some(at) = snap(&rects, tiles[i].size, tiles[i].pos, 0)
        {
            tiles[i].pos = at;
        }
        placed.push(i);
    }
    if let Some(b) = bounds(tiles) {
        for t in tiles.iter_mut() {
            t.pos = (t.pos.0 - b.x, t.pos.1 - b.y);
        }
    }
}

fn fit(tiles: &[Tile], inner: Rect) -> Fit {
    let Some(b) = bounds(tiles).filter(|b| b.width > 0 && b.height > 0) else {
        return Fit {
            scale: 1.0,
            ox: f64::from(inner.x),
            oy: f64::from(inner.y),
        };
    };
    let scale = (f64::from(inner.width) * FILL / f64::from(b.width))
        .min(f64::from(inner.height) * FILL / f64::from(b.height));
    let cx = f64::from(inner.x) + f64::from(inner.width) / 2.0;
    let cy = f64::from(inner.y) + f64::from(inner.height) / 2.0;
    Fit {
        scale,
        ox: cx - (f64::from(b.x) + f64::from(b.width) / 2.0) * scale,
        oy: cy - (f64::from(b.y) + f64::from(b.height) / 2.0) * scale,
    }
}

pub fn layout(r: Rect, a: &Arrange) -> Layout {
    let inner = r.inset(2);
    Layout {
        frame: r,
        inner,
        fit: a.held.map_or_else(|| fit(&a.tiles, inner), |h| h.fit),
    }
}

impl Arrange {
    pub fn new(tiles: Vec<Tile>, selected: usize) -> Arrange {
        let selected = selected.min(tiles.len().saturating_sub(1));
        Arrange {
            tiles,
            selected,
            enabled: true,
            held: None,
        }
    }

    pub fn tile_rect(&self, lay: &Layout, k: usize) -> Rect {
        let Some(t) = self.tiles.get(k) else {
            return Rect::default();
        };
        let f = lay.fit;
        let x0 = (f64::from(t.pos.0) * f.scale + f.ox).round() as i32;
        let y0 = (f64::from(t.pos.1) * f.scale + f.oy).round() as i32;
        let x1 = (f64::from(t.pos.0 + t.size.0) * f.scale + f.ox).round() as i32;
        let y1 = (f64::from(t.pos.1 + t.size.1) * f.scale + f.oy).round() as i32;
        Rect::new(x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
    }

    pub fn hit(&self, lay: &Layout, x: i32, y: i32) -> Option<usize> {
        if !lay.inner.contains(x, y) {
            return None;
        }
        let picked = std::iter::once(self.selected).filter(|&k| k < self.tiles.len());
        picked
            .chain((0..self.tiles.len()).rev())
            .find(|&k| self.tile_rect(lay, k).contains(x, y))
    }

    pub fn press(&mut self, lay: &Layout, k: usize, x: i32, y: i32) {
        if k >= self.tiles.len() {
            return;
        }
        self.selected = k;
        self.held = Some(Held {
            tile: k,
            from: (x, y),
            start: self.tiles[k].pos,
            fit: lay.fit,
        });
    }

    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    pub fn drag(&mut self, x: i32, y: i32) -> bool {
        let Some(h) = self.held else {
            return false;
        };
        let s = h.fit.scale.max(1e-6);
        let want = (
            h.start.0 + (f64::from(x - h.from.0) / s).round() as i32,
            h.start.1 + (f64::from(y - h.from.1) / s).round() as i32,
        );
        let others: Vec<Rect> = (self.tiles.iter().enumerate())
            .filter(|(i, _)| *i != h.tile)
            .map(|(_, t)| t.rect())
            .collect();
        let size = self.tiles[h.tile].size;
        let to = snap(&others, size, want, (SNAP_PX / s).round() as i32).unwrap_or(h.start);
        let moved = self.tiles[h.tile].pos != to;
        self.tiles[h.tile].pos = to;
        moved
    }

    pub fn drop_held(&mut self) -> bool {
        let Some(h) = self.held.take() else {
            return false;
        };
        settle(&mut self.tiles, h.tile);
        true
    }

    pub fn cancel_held(&mut self) {
        if let Some(h) = self.held.take() {
            self.tiles[h.tile].pos = h.start;
        }
    }

    /// Move the picked tile to its next place in `dir` ([`shove`]), settling the rest
    /// around it as a drop does. False when nothing lies that way.
    pub fn shove(&mut self, dir: Dir) -> bool {
        let k = self.selected;
        if self.held.is_some() || k >= self.tiles.len() {
            return false;
        }
        let others: Vec<Rect> = (self.tiles.iter().enumerate())
            .filter(|(i, _)| *i != k)
            .map(|(_, t)| t.rect())
            .collect();
        let Some(to) = shove(&others, self.tiles[k].size, self.tiles[k].pos, dir) else {
            return false;
        };
        self.tiles[k].pos = to;
        settle(&mut self.tiles, k);
        true
    }

    pub fn step(&mut self, key: Step) -> bool {
        let n = self.tiles.len();
        if n == 0 {
            return false;
        }
        let to = match key {
            Step::Back => self.selected.saturating_sub(1),
            Step::Forward => (self.selected + 1).min(n - 1),
            Step::First => 0,
            Step::Last => n - 1,
        };
        let moved = to != self.selected;
        self.selected = to;
        moved
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Back,
    Forward,
    First,
    Last,
}

pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    a: &Arrange,
    lay: &Layout,
    cue: bool,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let well = lay.frame.scaled(s);
    c.fill_rect(well, cs.button_shadow);
    draw_field_frame(c, p, well, cs.button_shadow);
    let clip = lay.inner.scaled(s);
    let order = (0..a.tiles.len())
        .filter(|&k| k != a.selected)
        .chain(std::iter::once(a.selected).filter(|&k| k < a.tiles.len()));
    let least = (0..a.tiles.len())
        .map(|k| {
            let r = a.tile_rect(lay, k).scaled(s);
            r.width.min(r.height).max(0) as u32
        })
        .min()
        .unwrap_or(0);
    let size = (px * 2).min(least * 3 / 5).max(1);
    c.with_clip(clip, |c| {
        for k in order {
            let r = a.tile_rect(lay, k).scaled(s);
            let picked = k == a.selected;
            let (fill, ink) = if a.enabled {
                (cs.background, cs.desktop_text)
            } else {
                (cs.button_face, cs.gray_text)
            };
            let line = if picked && a.enabled {
                cs.hilight
            } else {
                cs.window_frame
            };
            let t = if picked { 2 * s } else { s };
            c.fill_rect(r, line);
            c.fill_rect(r.inset(t), fill);
            let number = a.tiles[k].number.to_string();
            text.draw(c, r, &number, true, ink, Align::Center, size);
            if picked && cue {
                draw_focus_ring(c, r.inset(t + s), s, cs.hilight_text);
            }
        }
    });
}

pub fn access(
    a: &Arrange,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash + Copy,
) -> NodeId {
    let mut n = access::named(Role::ListBox, lay.frame, "Monitor arrangement");
    access::focusable(&mut n, a.enabled);
    let id = tree.push(parent, (key, "arrange"), n);
    for (k, t) in a.tiles.iter().enumerate() {
        let r = a.tile_rect(lay, k);
        let mut o = access::named(Role::ListBoxOption, r, &format!("{}. {}", t.number, t.name));
        o.set_selected(a.selected == k);
        o.set_description(format!("At {}, {}", t.pos.0, t.pos.1));
        tree.push(id, (key, k), o);
    }
    id
}

pub fn draw_badge(theme: &Theme, text: &TextRenderer, number: u32) -> Canvas {
    let p = theme.paint();
    let (cs, s) = (p.cs, p.s);
    let mut c = Canvas::new((BADGE * s) as u32, (BADGE * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    let size = theme.fonts.size_px * 12;
    let b = c.bounds();
    text.draw(
        &mut c,
        b,
        &number.to_string(),
        true,
        cs.button_text,
        Align::Center,
        size,
    );
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(n: u32, pos: (i32, i32), size: (i32, i32)) -> Tile {
        Tile {
            number: n,
            name: format!("OUT-{n}"),
            pos,
            size,
        }
    }

    fn rects(tiles: &[Tile]) -> Vec<Rect> {
        tiles.iter().map(Tile::rect).collect()
    }

    /// A tile dragged near a neighbour lands against its nearest edge,
    /// its top lined up when within the snap distance, and never inside.
    #[test]
    fn a_dragged_tile_snaps_to_the_nearest_edge_and_lines_up() {
        let a = [Rect::new(0, 0, 1920, 1080)];
        assert_eq!(snap(&a, (1280, 1024), (1950, 20), 50), Some((1920, 0)));
        assert_eq!(snap(&a, (1280, 1024), (1950, 300), 50), Some((1920, 300)));
        assert_eq!(snap(&a, (1280, 1024), (100, 1200), 50), Some((100, 1080)));
        assert_eq!(
            snap(&a, (1280, 1024), (-1300, 40), 50),
            Some((-1280, 56)),
            "bottoms lined up"
        );
        assert_eq!(
            snap(&a, (1280, 1024), (500, 200), 0),
            Some((500, 1080)),
            "dropped inside, it leaves by the nearest edge"
        );
        assert_eq!(
            snap(&a, (1280, 1024), (4000, 1050), 0),
            Some((1920, 1050)),
            "far to the right, it comes back to the edge"
        );
        assert_eq!(snap(&[], (10, 10), (0, 0), 0), None);
    }

    /// With two neighbours a tile never lands overlapping either.
    #[test]
    fn a_snapped_place_overlaps_no_other_tile() {
        let others = [Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080)];
        let at = snap(&others, (1920, 1080), (1800, 0), 0).unwrap();
        let r = Rect::new(at.0, at.1, 1920, 1080);
        assert!(others.iter().all(|o| !overlaps(r, *o)), "{at:?}");
        assert!(others.iter().any(|o| touches(r, *o)), "{at:?}");
    }

    #[test]
    fn the_drop_closes_gaps_and_moves_the_layout_to_the_origin() {
        let mut t = vec![
            tile(1, (0, 0), (100, 100)),
            tile(2, (100, 0), (100, 100)),
            tile(3, (200, 0), (100, 100)),
        ];
        t[1].pos = (0, 100);
        settle(&mut t, 1);
        let r = rects(&t);
        assert_eq!(r[0], Rect::new(0, 0, 100, 100), "touching: it stays");
        assert_eq!(r[1], Rect::new(0, 100, 100, 100));
        assert!(
            touches(r[2], r[0]) || touches(r[2], r[1]),
            "the third came back: {:?}",
            r[2]
        );
        for i in 0..3 {
            for j in 0..3 {
                assert!(i == j || !overlaps(r[i], r[j]), "{i} and {j} overlap");
            }
        }
        let mut t = vec![tile(1, (-50, 20), (100, 100)), tile(2, (50, 20), (80, 60))];
        settle(&mut t, 0);
        assert_eq!((t[0].pos, t[1].pos), ((0, 0), (100, 0)));
    }

    #[test]
    fn an_overlap_is_settled_to_an_edge() {
        let mut t = vec![
            tile(1, (0, 0), (2560, 1440)),
            tile(2, (1920, 0), (1920, 1080)),
        ];
        assert!(!whole(&t));
        settle(&mut t, 0);
        assert_eq!(t[1].pos, (2560, 0));
        assert!(whole(&t));
        let apart = [tile(1, (0, 0), (10, 10)), tile(2, (11, 0), (10, 10))];
        assert!(!whole(&apart), "a 1 px gap");
        let corner = [tile(1, (0, 0), (10, 10)), tile(2, (10, 10), (10, 10))];
        assert!(!whole(&corner), "corners alone do not join");
        assert!(whole(&[]) && whole(&apart[..1]));
    }

    fn well() -> (Arrange, Layout) {
        let a = Arrange::new(
            vec![
                tile(1, (0, 0), (1920, 1080)),
                tile(2, (1920, 0), (1280, 1024)),
            ],
            0,
        );
        let lay = layout(Rect::new(20, 44, 358, 108), &a);
        (a, lay)
    }

    #[test]
    fn the_tiles_fit_the_well_and_share_their_edge() {
        let (a, lay) = well();
        assert_eq!(lay.inner, Rect::new(22, 46, 354, 104));
        let (r1, r2) = (a.tile_rect(&lay, 0), a.tile_rect(&lay, 1));
        assert_eq!(r1, Rect::new(83, 59, 139, 78));
        assert_eq!(r2, Rect::new(222, 59, 93, 74));
        assert_eq!(r1.right(), r2.x);
        assert_eq!(a.hit(&lay, r2.x + 5, r2.y + 5), Some(1));
        assert_eq!(a.hit(&lay, 24, 48), None, "the well's bare floor");
    }

    #[test]
    fn a_drag_moves_the_held_tile_and_the_drop_lets_go() {
        let (mut a, lay) = well();
        let r2 = a.tile_rect(&lay, 1);
        a.press(&lay, 1, r2.x + 4, r2.y + 4);
        assert_eq!(a.selected, 1);
        assert!(a.holding());
        assert!(a.drag(r2.x - 60, r2.y + 70));
        assert_eq!(a.tiles[1].pos.1, 1080);
        assert_eq!(layout(lay.frame, &a).fit, lay.fit, "no zoom mid-drag");
        assert!(a.drop_held());
        assert!(!a.holding() && !a.drop_held());
        assert!(touches(a.tiles[0].rect(), a.tiles[1].rect()));
    }

    #[test]
    fn ctrl_right_moves_a_tile_from_left_of_to_right_of_its_neighbour() {
        let (mut a, _) = well();
        assert!(a.shove(Dir::Right));
        let (r1, r2) = (a.tiles[0].rect(), a.tiles[1].rect());
        assert_eq!(
            r1,
            Rect::new(1280, 0, 1920, 1080),
            "right of 2, tops lined up"
        );
        assert_eq!(r2, Rect::new(0, 0, 1280, 1024));
        assert!(a.shove(Dir::Left), "and back");
        assert_eq!((a.tiles[0].pos, a.tiles[1].pos), ((0, 0), (1920, 0)));
    }

    #[test]
    fn ctrl_down_slides_a_tile_along_the_edge_then_below() {
        let (mut a, _) = well();
        a.selected = 1;
        assert!(a.shove(Dir::Down));
        assert_eq!(a.tiles[1].pos, (1920, 56), "bottoms lined up");
        assert!(a.shove(Dir::Down));
        assert_eq!(a.tiles[1].pos, (640, 1080), "below 1, right edges lined up");
        assert_eq!(a.tiles[0].pos, (0, 0));
        assert!(!a.shove(Dir::Down), "nothing lies further down");
    }

    #[test]
    fn a_shove_past_the_outer_edge_does_nothing() {
        let (mut a, _) = well();
        a.selected = 1;
        let before = a.tiles.clone();
        assert!(!a.shove(Dir::Right));
        assert_eq!(a.tiles, before);
        let mut one = Arrange::new(vec![tile(1, (0, 0), (1920, 1080))], 0);
        for d in [Dir::Left, Dir::Right, Dir::Up, Dir::Down] {
            assert!(!one.shove(d), "{d:?} with one monitor");
        }
    }

    #[test]
    fn a_shoved_tile_overlaps_none_and_keeps_the_layout_whole() {
        let mut a = Arrange::new(
            vec![
                tile(1, (0, 0), (100, 100)),
                tile(2, (100, 0), (100, 100)),
                tile(3, (200, 0), (100, 100)),
            ],
            1,
        );
        for d in [
            Dir::Down,
            Dir::Right,
            Dir::Up,
            Dir::Up,
            Dir::Left,
            Dir::Left,
        ] {
            a.shove(d);
            assert!(whole(&a.tiles), "after {d:?}: {:?}", a.tiles);
        }
    }

    #[test]
    fn the_keys_walk_the_pick() {
        let (mut a, _) = well();
        assert!(a.step(Step::Forward));
        assert!(!a.step(Step::Forward));
        assert!(a.step(Step::First));
        assert!(a.step(Step::Last));
        assert_eq!(a.selected, 1);
    }

    #[test]
    fn the_well_draws_its_tiles() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&theme.fonts);
        let (a, lay) = well();
        let mut c = Canvas::new(400, 200);
        draw(
            &mut c,
            &text,
            &theme.paint(),
            &a,
            &lay,
            false,
            theme.fonts.size_px,
        );
        assert_eq!(c.get(24, 48), cs.button_shadow.argb());
        let (r1, r2) = (a.tile_rect(&lay, 0), a.tile_rect(&lay, 1));
        assert_eq!(c.get(r1.x, r1.y), cs.hilight.argb());
        assert_eq!(c.get(r1.x + 1, r1.y + 1), cs.hilight.argb());
        assert_eq!(c.get(r1.x + 2, r1.y + 2), cs.background.argb());
        assert_eq!(c.get(r2.right() - 1, r2.y), cs.window_frame.argb());
        assert_eq!(c.get(r2.right() - 2, r2.y + 1), cs.background.argb());
        let ink = (r2.y..r2.bottom())
            .flat_map(|y| (r2.x..r2.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == cs.desktop_text.argb())
            .count();
        assert!(ink > 20, "the number 2 is drawn: {ink} px");
    }

    #[test]
    fn the_badge_is_a_raised_box_with_its_number() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&theme.fonts);
        let c = draw_badge(&theme, &text, 1);
        assert_eq!((c.width, c.height), (BADGE as u32, BADGE as u32));
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(BADGE - 1, BADGE - 1), cs.button_dk_shadow.argb());
        assert_eq!(c.get(8, 8), cs.button_face.argb());
        let mid = (70..130)
            .filter(|&y| c.get(BADGE / 2, y) == cs.button_text.argb())
            .count();
        assert!(mid > 10, "the stem of the 1 runs down the middle: {mid} px");
    }
}
