//! The 3D switcher (Super+Tab) model: windows tilted into a stack receding up and to the left, the front card at the lower right, the entry in front selected. The host lists the entries and steps them with [`crate::switcher::step`].
//! Entry `k` sits at depth `k - pos`, wrapped into -1 (exclusive) to `len - 1`; depth 0 is the front card and a card between -1 and 0 is on its way from front to back, so animating `pos` between whole numbers rolls the stack. Each card's picture maps to the screen by a [`Homography`]; hosts draw through the inverse and hit test with [`hit`].
//! On opening and closing, cards fly between the windows' places and the stack along each [`Pose`]; [`fly`] lays them out anywhere on that way.

use crate::raster::Rect;

/// A plane-to-plane perspective map: `(u, v)` to
/// `(m[0]·p / m[2]·p, m[1]·p / m[2]·p)` with `p = (u, v, 1)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography(pub [[f64; 3]; 3]);

impl Homography {
    /// Where `(u, v)` lands, or None behind the eye.
    pub fn map(&self, u: f64, v: f64) -> Option<(f64, f64)> {
        let [a, b, c] = self.0;
        let w = c[0] * u + c[1] * v + c[2];
        if w <= f64::EPSILON {
            return None;
        }
        Some((
            (a[0] * u + a[1] * v + a[2]) / w,
            (b[0] * u + b[1] * v + b[2]) / w,
        ))
    }

    /// The map `(u, v)` to `(sx·u + tx, sy·v + ty)`: a change of units or
    /// origin on either side of a card's map.
    pub fn affine(sx: f64, sy: f64, tx: f64, ty: f64) -> Homography {
        Homography([[sx, 0.0, tx], [0.0, sy, ty], [0.0, 0.0, 1.0]])
    }

    /// `first`, then `self`: `self.after(&first).map(p)` is
    /// `self.map(first.map(p))` wherever both are defined.
    pub fn after(&self, first: &Homography) -> Homography {
        let (a, b) = (self.0, first.0);
        Homography(std::array::from_fn(|r| {
            std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum())
        }))
    }

    /// The map back, or None when it collapses the plane.
    pub fn inverse(&self) -> Option<Homography> {
        let m = self.0;
        let cof = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let adj = [
            [cof(1, 2, 1, 2), -cof(0, 2, 1, 2), cof(0, 1, 1, 2)],
            [-cof(1, 2, 0, 2), cof(0, 2, 0, 2), -cof(0, 1, 0, 2)],
            [cof(1, 2, 0, 1), -cof(0, 2, 0, 1), cof(0, 1, 0, 1)],
        ];
        let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
        if det.abs() <= f64::EPSILON {
            return None;
        }
        Some(Homography(adj.map(|r| r.map(|x| x / det))))
    }
}

/// The stack's shape, in fractions of the area so it holds at any size.
///
/// The cards stand on one path that recedes up and to the left of the
/// front. At path step `p` a card's bottom-left corner lands on the
/// screen at `far + (front - far) · shrink^p` (each axis on its own, in
/// area widths and heights), so every step moves `shrink` of the one
/// before, and it stands `back.0 + back.1 · p` area heights behind the
/// screen, a window `wide_back` further for each area width it is wide.
/// The eye looks at the area's centre from `focal` area heights away.
///
/// Up to `full` cards take one step each; more share the run from a
/// front pushed `spread · √(cards - full)` steps forward to step
/// `far_step`, so a long stack packs tighter and starts nearer. `shown`
/// cards are drawn, the last fading; past them nothing.
///
/// A `w` wide window is drawn at `scale / (1 + w / (squeeze · area
/// width))` of its size at the front: every window shrinks, wide ones
/// more. Each card leans `tilt` degrees in its own plane (its bottom
/// toward the right), tips its top `pitch` degrees toward the eye, and
/// turns `yaw` degrees about its upright left edge, its right edge away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub focal: f64,
    pub front: (f64, f64),
    pub far: (f64, f64),
    pub shrink: (f64, f64),
    pub back: (f64, f64),
    pub wide_back: f64,
    pub scale: f64,
    pub squeeze: f64,
    pub yaw: f64,
    pub tilt: f64,
    pub pitch: f64,
    pub full: usize,
    pub spread: f64,
    pub far_step: f64,
    pub shown: usize,
}

pub const GEOMETRY: Geometry = Geometry {
    focal: 1.61,
    front: (0.456, 0.791),
    far: (0.016, 0.406),
    shrink: (0.653, 0.525),
    back: (0.315, 0.402),
    wide_back: -0.234,
    scale: 0.71,
    squeeze: 2.48,
    yaw: 26.0,
    tilt: -2.16,
    pitch: 10.6,
    full: 5,
    spread: 0.11,
    far_step: 4.2,
    shown: 10,
};

impl Geometry {
    fn step(&self, n: usize, depth: f64) -> f64 {
        let m = n.min(self.shown);
        if m <= self.full {
            return depth;
        }
        let start = -self.spread * ((m - self.full) as f64).sqrt();
        start + depth * (self.far_step - start) / (m - 1) as f64
    }
}

/// One drawn card: entry `entry` at `depth` (0 front, below 0 leaving),
/// drawn at `alpha`, its picture of `size` (logical, the window's)
/// mapped to the area's coordinates by `to_screen`, inside `bounds`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub entry: usize,
    pub depth: f64,
    pub alpha: f32,
    pub size: (i32, i32),
    pub to_screen: Homography,
    pub bounds: Rect,
}

/// The cards for entries of `sizes` at position `pos` in `area`, back to
/// front (draw order). Cards past the tail or fully faded are not drawn.
pub fn layout(area: Rect, sizes: &[(i32, i32)], pos: f64, g: &Geometry) -> Vec<Card> {
    fly(area, sizes, &vec![None; sizes.len()], 0, pos, 1.0, g)
}

/// The cards at position `pos` on their way between the desktop and the
/// stack, back to front: `out` 0 is every window flat where it lies on the
/// desktop, 1 the stack ([`layout`]), and each card's pose is the
/// [`Pose::lerp`] between the two. `flat` is each entry's rect on the
/// desktop (area coordinates), or None for one that is not there (a
/// minimized window, the desktop), which fades in at its stack pose
/// instead. A flat card fades toward its alpha in the stack. On the
/// desktop the entries lie in their order, `top` over the rest; the draw
/// order moves from that to the stack's as `out` grows.
pub fn fly(
    area: Rect,
    sizes: &[(i32, i32)],
    flat: &[Option<Rect>],
    top: usize,
    pos: f64,
    out: f64,
    g: &Geometry,
) -> Vec<Card> {
    let n = sizes.len();
    let mut cards: Vec<(f64, Card)> = sizes
        .iter()
        .enumerate()
        .filter_map(|(k, &size)| {
            let depth = depth(k, n, pos);
            let (place, stacked) = place(depth, n, g);
            let pose = Pose::stack(area, size, place, n, g);
            let (pose, alpha) = match flat.get(k).copied().flatten() {
                Some(r) => (
                    Pose::flat(r).lerp(&pose, out),
                    (1.0 + (f64::from(stacked) - 1.0) * out) as f32,
                ),
                None => (pose, (f64::from(stacked) * out) as f32),
            };
            (alpha > 0.0).then(|| {
                let rank = if k == top { -1.0 } else { k as f64 };
                let to_screen = pose.homography(area, size, g);
                let card = Card {
                    entry: k,
                    depth,
                    alpha,
                    size,
                    to_screen,
                    bounds: bounds(&to_screen, size),
                };
                ((1.0 - out) * rank + out * place, card)
            })
        })
        .collect();
    cards.sort_by(|a, b| b.0.total_cmp(&a.0));
    cards.into_iter().map(|(_, c)| c).collect()
}

/// The position that holds the stack still as entry `k` of `n` leaves
/// it at `pos`: the cards in front of it keep their depth and the ones
/// behind move up one, so the one after it takes its place. The result is
/// wrapped into `0..n - 1`; renumber the entries after `k` down one.
pub fn forget(pos: f64, n: usize, k: usize) -> f64 {
    if n <= 1 {
        return 0.0;
    }
    let p = pos.rem_euclid(n as f64);
    let p = if p > k as f64 { p - 1.0 } else { p };
    p.rem_euclid((n - 1) as f64)
}

/// [`forget`] for a roll under way from `pos` toward `to`: where it stands
/// now, and where it heads, about as many steps on as before, at the entry
/// `to` brought to the front (the one after it, if that one left).
///
/// Mid-roll one card is always on its way round between the front and the
/// back. When that card leaves, the rest cannot keep their depths on the
/// shorter round (the last would have to go round in its place): they
/// finish the step at once instead, each moving up less than one place.
pub fn forget_roll(pos: f64, to: f64, n: usize, k: usize) -> (f64, f64) {
    let mut now = forget(pos, n, k);
    if n <= 2 {
        return (now, now);
    }
    let m = (n - 1) as f64;
    if depth(k, n, pos) < 0.0 {
        now = now.ceil().rem_euclid(m);
    }
    let end = forget(to, n, k);
    let ahead = now + (to - pos);
    (now, end + ((ahead - end) / m).round() * m)
}

/// The position that holds the stack still as an entry joins `n` at index
/// `i` (the entries from `i` on renumber up one) at `pos`: the cards in
/// front of it keep their depth and the ones behind move back one. It
/// never takes the front: joining at the front entry's index, it goes to
/// the back. Wrapped into `0..n + 1`; [`forget`] takes it out again.
pub fn insert(pos: f64, n: usize, i: usize) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let p = pos.rem_euclid(n as f64);
    if p < i as f64 { p } else { p + 1.0 }
}

/// [`insert`] for a roll under way from `pos` toward `to`, which brings
/// entry `to` (wrapped) to the front: where it stands now, and where it
/// heads, about as many steps on as before, at that same entry.
pub fn insert_roll(pos: f64, to: f64, n: usize, i: usize) -> (f64, f64) {
    let now = insert(pos, n, i);
    if n == 0 {
        return (now, now);
    }
    let m = (n + 1) as f64;
    let target = to.round().rem_euclid(n as f64);
    let end = if target < i as f64 {
        target
    } else {
        target + 1.0
    };
    let ahead = now + (to - pos);
    (now, end + ((ahead - end) / m).round() * m)
}

pub fn hit(cards: &[Card], x: f64, y: f64) -> Option<usize> {
    cards.iter().rev().find_map(|c| {
        let (u, v) = c.to_screen.inverse()?.map(x, y)?;
        let (w, h) = (f64::from(c.size.0), f64::from(c.size.1));
        ((0.0..w).contains(&u) && (0.0..h).contains(&v)).then_some(c.entry)
    })
}

fn depth(k: usize, n: usize, pos: f64) -> f64 {
    if n <= 1 {
        return 0.0;
    }
    let d = (k as f64 - pos).rem_euclid(n as f64);
    if d > n as f64 - 1.0 { d - n as f64 } else { d }
}

/// Where a card at `depth` among `n` stands (a depth on the path) and how
/// opaque it is. A card leaving the front (depth -1 to 0) fades out in
/// the first half of its step, moving on forward, and fades in again at
/// the back in the second, sliding into the last place; a step back runs
/// the same way round. With more than `shown` cards the last shown one
/// is half faded and the ones behind it are not drawn.
fn place(depth: f64, n: usize, g: &Geometry) -> (f64, f32) {
    let tail = |d: f64| {
        if n <= g.shown {
            1.0
        } else {
            ((g.shown as f64 - d) / 2.0).clamp(0.0, 1.0)
        }
    };
    if depth >= 0.0 {
        return (depth, tail(depth) as f32);
    }
    let gone = -depth;
    if gone < 0.5 {
        (-gone, (1.0 - 2.0 * gone) as f32)
    } else {
        let d = n as f64 - gone;
        (d, ((2.0 * gone - 1.0) * tail(n as f64 - 1.0)) as f32)
    }
}

/// Where a card stands: its bottom-left corner (`x`, `y` in area
/// coordinates, `z` toward the eye in logical pixels, 0 on the screen),
/// the `scale` its picture is drawn at, and its turns in degrees: `tilt`
/// in its own plane, `pitch` its top toward the eye, `yaw` about its left
/// edge, its right edge away from the eye.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub scale: f64,
    pub yaw: f64,
    pub tilt: f64,
    pub pitch: f64,
}

impl Pose {
    /// A window lying where it is on the desktop, `rect`: on the screen,
    /// unturned, full size. Its [`Pose::homography`] maps a picture of the
    /// rect's size onto the rect.
    pub fn flat(rect: Rect) -> Pose {
        Pose {
            x: f64::from(rect.x),
            y: f64::from(rect.y) + f64::from(rect.height),
            z: 0.0,
            scale: 1.0,
            yaw: 0.0,
            tilt: 0.0,
            pitch: 0.0,
        }
    }

    pub fn stack(area: Rect, size: (i32, i32), depth: f64, n: usize, g: &Geometry) -> Pose {
        let (aw, ah) = (f64::from(area.width), f64::from(area.height));
        let (cx, cy) = (f64::from(area.x) + aw / 2.0, f64::from(area.y) + ah / 2.0);
        let w = f64::from(size.0);
        let p = g.step(n, depth);
        let on_screen = |front: f64, far: f64, shrink: f64| far + (front - far) * shrink.powf(p);
        let sx = f64::from(area.x) + on_screen(g.front.0, g.far.0, g.shrink.0) * aw;
        let sy = f64::from(area.y) + on_screen(g.front.1, g.far.1, g.shrink.1) * ah;
        let eye = g.focal * ah;
        let wide = g.wide_back * w / aw;
        let z = -(g.back.0 + g.back.1 * p + wide) * ah;
        let front_z = -(g.back.0 + wide) * ah;
        // Back along the eye's ray from where the corner lands on the
        // screen to its depth; the scale is the front's, at the depth the
        // window would stand in front.
        let k = (eye - z) / eye;
        Pose {
            x: cx + (sx - cx) * k,
            y: cy + (sy - cy) * k,
            z,
            scale: g.scale / (1.0 + w / (g.squeeze * aw)) * (eye - front_z) / eye,
            yaw: g.yaw,
            tilt: g.tilt,
            pitch: g.pitch,
        }
    }

    pub fn lerp(&self, to: &Pose, t: f64) -> Pose {
        let m = |a: f64, b: f64| a + (b - a) * t;
        Pose {
            x: m(self.x, to.x),
            y: m(self.y, to.y),
            z: m(self.z, to.z),
            scale: m(self.scale, to.scale),
            yaw: m(self.yaw, to.yaw),
            tilt: m(self.tilt, to.tilt),
            pitch: m(self.pitch, to.pitch),
        }
    }

    pub fn homography(&self, area: Rect, size: (i32, i32), g: &Geometry) -> Homography {
        let (aw, ah) = (f64::from(area.width), f64::from(area.height));
        let h = f64::from(size.1);
        let (cx, cy) = (f64::from(area.x) + aw / 2.0, f64::from(area.y) + ah / 2.0);
        let eye = g.focal * ah;
        let s = self.scale;
        let (st, ct) = self.tilt.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let (sy, cy_) = self.yaw.to_radians().sin_cos();
        // Every coordinate below is affine in the picture point (u, v):
        // [coefficient of u, of v, constant]. From the bottom-left corner
        // the point is (u, v - h); tilted in its plane, it is (a, b).
        let a = [ct, -st, st * h];
        let b = [st, ct, -ct * h];
        let lin = |p: [f64; 3], q: [f64; 3], x: f64, y: f64, c: f64| {
            [
                x * p[0] + y * q[0],
                x * p[1] + y * q[1],
                x * p[2] + y * q[2] + c,
            ]
        };
        let wx = lin(a, b, s * cy_, 0.0, self.x);
        let wy = lin(a, b, 0.0, s * cp, self.y);
        let wz = lin(a, b, -s * sy, -s * sp, self.z);
        // A world point lands at centre + eye·(world - centre) / (eye - z).
        let rw = [-wz[0], -wz[1], eye - wz[2]];
        let row = |c: f64, p: [f64; 3]| {
            [
                c * rw[0] + eye * p[0],
                c * rw[1] + eye * p[1],
                c * rw[2] + eye * (p[2] - c),
            ]
        };
        Homography([row(cx, wx), row(cy, wy), rw])
    }
}

/// The whole pixels round the four corners of a `size` picture mapped by
/// `m`; float noise under a millionth of a pixel does not reach the next.
pub fn bounds(m: &Homography, size: (i32, i32)) -> Rect {
    const NOISE: f64 = 1e-6;
    let (w, h) = (f64::from(size.0), f64::from(size.1));
    let pts: Vec<(f64, f64)> = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)]
        .iter()
        .filter_map(|&(u, v)| m.map(u, v))
        .collect();
    let min = |f: fn(&(f64, f64)) -> f64| pts.iter().map(f).fold(f64::INFINITY, f64::min);
    let max = |f: fn(&(f64, f64)) -> f64| pts.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
    let x0 = (min(|p| p.0) + NOISE).floor();
    let y0 = (min(|p| p.1) + NOISE).floor();
    let x1 = (max(|p| p.0) - NOISE).ceil();
    let y1 = (max(|p| p.1) - NOISE).ceil();
    Rect::new(x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1000, 800);
    const HD: Rect = Rect::new(0, 0, 1920, 1080);

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    fn card(cards: &[Card], entry: usize) -> Card {
        *cards.iter().find(|c| c.entry == entry).expect("card drawn")
    }

    fn corner(c: &Card) -> (f64, f64) {
        c.to_screen.map(0.0, f64::from(c.size.1)).unwrap()
    }

    #[test]
    fn the_front_card_of_five_stands_at_the_lower_right_of_centre() {
        let cards = layout(HD, &[(800, 500); 5], 0.0, &GEOMETRY);
        let m = card(&cards, 0).to_screen;
        for ((u, v), want) in [
            ((0.0, 0.0), (861.78, 556.31)),
            ((800.0, 0.0), (1275.04, 537.91)),
            ((800.0, 500.0), (1274.38, 804.39)),
            ((0.0, 500.0), (875.52, 854.28)),
        ] {
            let got = m.map(u, v).unwrap();
            assert!(
                (got.0 - want.0).abs() < 1.0 && (got.1 - want.1).abs() < 1.0,
                "corner {u},{v} at {got:?}, want {want:?}"
            );
        }
    }

    #[test]
    fn the_stack_recedes_up_and_left_in_ever_shorter_steps() {
        let cards = layout(HD, &[(800, 500); 5], 0.0, &GEOMETRY);
        let at: Vec<(f64, f64)> = (0..5).map(|k| corner(&card(&cards, k))).collect();
        let steps: Vec<(f64, f64)> = at
            .windows(2)
            .map(|p| (p[1].0 - p[0].0, p[1].1 - p[0].1))
            .collect();
        for s in &steps {
            assert!(
                s.0 < 0.0 && s.1 < 0.0,
                "each step goes up and left: {steps:?}"
            );
        }
        for s in steps.windows(2) {
            assert!(s[1].0.abs() < s[0].0.abs(), "x steps shrink: {steps:?}");
            assert!(s[1].1.abs() < s[0].1.abs(), "y steps shrink: {steps:?}");
        }
        let h = |k: usize| card(&cards, k).bounds.height;
        assert!(
            h(1) < h(0) && h(2) < h(1),
            "cards further back draw smaller"
        );
    }

    #[test]
    fn every_window_in_a_place_shares_its_bottom_left_corner() {
        let sizes = [(300, 200), (800, 500), (1600, 1000), (1920, 1080)];
        let corners: Vec<(f64, f64)> = sizes
            .iter()
            .map(|&s| corner(&card(&layout(HD, &[s, (800, 500)], 0.0, &GEOMETRY), 0)))
            .collect();
        for c in &corners {
            assert!(close(*c, corners[0]), "{corners:?}");
        }
    }

    #[test]
    fn every_window_shrinks_and_wide_ones_more() {
        let shown = |size: (i32, i32)| {
            let c = card(&layout(HD, &[size, (800, 500)], 0.0, &GEOMETRY), 0);
            let (top, bottom) = (c.to_screen.map(0.0, 0.0).unwrap(), corner(&c));
            (bottom.1 - top.1).hypot(bottom.0 - top.0) / f64::from(size.1)
        };
        let (small, mid, big) = (shown((300, 200)), shown((800, 500)), shown((1600, 1000)));
        assert!(small < 0.7, "a small window is drawn smaller too: {small}");
        assert!(big < mid && mid < small, "{small} {mid} {big}");
    }

    #[test]
    fn a_longer_stack_starts_nearer_and_ends_where_a_short_one_does() {
        let five = layout(HD, &[(800, 500); 5], 0.0, &GEOMETRY);
        let nine = layout(HD, &[(800, 500); 9], 0.0, &GEOMETRY);
        let (f5, f9) = (corner(&card(&five, 0)), corner(&card(&nine, 0)));
        assert!(f9.0 > f5.0 + 50.0 && f9.1 > f5.1 + 40.0, "{f5:?} {f9:?}");
        let (e5, e9) = (corner(&card(&five, 4)), corner(&card(&nine, 8)));
        assert!(
            (e5.0 - e9.0).abs() < 15.0 && (e5.1 - e9.1).abs() < 15.0,
            "{e5:?} {e9:?}"
        );
    }

    #[test]
    fn a_card_between_places_stands_between_them() {
        let sizes = [(800, 500); 5];
        let at = |pos: f64| corner(&card(&layout(HD, &sizes, pos, &GEOMETRY), 2));
        let (a, m, b) = (at(1.0), at(0.5), at(0.0));
        assert!(
            b.0 < m.0 && m.0 < a.0 && b.1 < m.1 && m.1 < a.1,
            "{b:?} {m:?} {a:?}"
        );
    }

    #[test]
    fn a_map_after_another_runs_the_first_then_itself() {
        let c = card(&layout(AREA, &[(640, 480)], 0.0, &GEOMETRY), 0);
        let texels = Homography::affine(2.0, 2.0, 0.0, 0.0);
        let view = Homography::affine(2.0, 2.0, 10.0, 20.0);
        let m = view.after(&c.to_screen).after(&texels);
        for (u, v) in [(0.0, 0.0), (320.0, 0.0), (0.0, 240.0), (160.0, 100.0)] {
            let (x, y) = c.to_screen.map(u * 2.0, v * 2.0).unwrap();
            assert!(
                close(m.map(u, v).unwrap(), (10.0 + 2.0 * x, 20.0 + 2.0 * y)),
                "texel {u},{v}"
            );
        }
    }

    #[test]
    fn the_inverse_takes_every_corner_back() {
        let sizes = [(640, 480), (300, 900), (1200, 700)];
        for c in layout(AREA, &sizes, 0.4, &GEOMETRY) {
            let back = c.to_screen.inverse().unwrap();
            let (w, h) = (f64::from(c.size.0), f64::from(c.size.1));
            for (u, v) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h), (w / 3.0, h / 2.0)] {
                let (x, y) = c.to_screen.map(u, v).unwrap();
                let (bu, bv) = back.map(x, y).unwrap();
                assert!(
                    (bu - u).abs() < 1e-6 && (bv - v).abs() < 1e-6,
                    "card {} at {u},{v}",
                    c.entry
                );
            }
        }
    }

    #[test]
    fn the_right_edge_turns_away_and_draws_shorter() {
        let c = card(&layout(AREA, &[(400, 300)], 0.0, &GEOMETRY), 0);
        let m = c.to_screen;
        let left = m.map(0.0, 300.0).unwrap().1 - m.map(0.0, 0.0).unwrap().1;
        let right = m.map(400.0, 300.0).unwrap().1 - m.map(400.0, 0.0).unwrap().1;
        assert!(right < left, "right edge {right} shorter than left {left}");
    }

    #[test]
    fn cards_come_back_to_front_with_the_front_entry_last() {
        let cards = layout(AREA, &[(400, 300); 4], 2.0, &GEOMETRY);
        let order: Vec<usize> = cards.iter().map(|c| c.entry).collect();
        assert_eq!(order, [1, 0, 3, 2]);
    }

    #[test]
    fn a_step_fades_the_front_card_out_forward_then_in_at_the_back() {
        let sizes = [(400, 300); 4];
        let front = corner(&card(&layout(AREA, &sizes, 0.0, &GEOMETRY), 0));
        let back = corner(&card(&layout(AREA, &sizes, 1.0, &GEOMETRY), 0));
        let early = layout(AREA, &sizes, 0.25, &GEOMETRY);
        let c = card(&early, 0);
        assert_eq!((c.depth, c.alpha), (-0.25, 0.5));
        let at = corner(&c);
        assert!(
            at.0 > front.0 && at.1 > front.1,
            "on forward: {at:?} {front:?}"
        );
        assert_eq!(early.last().unwrap().entry, 0, "drawn in front");
        let late = layout(AREA, &sizes, 0.75, &GEOMETRY);
        let c = card(&late, 0);
        assert_eq!(c.alpha, 0.5);
        let at = corner(&c);
        assert!(
            at.0 < back.0 + 1.0 && at.0 > back.0 - 100.0,
            "at the back: {at:?} {back:?}"
        );
        assert_eq!(late[0].entry, 0, "drawn behind every card");
        assert_eq!(card(&late, 1).depth, 0.25);
    }

    #[test]
    fn stepping_back_brings_the_last_entry_in_at_the_front() {
        let cards = layout(AREA, &[(400, 300); 3], -0.75, &GEOMETRY);
        let c = card(&cards, 2);
        assert_eq!((c.depth, c.alpha), (-0.25, 0.5));
        assert_eq!(cards.last().unwrap().entry, 2);
        assert_eq!(card(&cards, 0).depth, 0.75);
    }

    #[test]
    fn past_the_shown_cards_the_last_fades_and_the_rest_stop() {
        let cards = layout(AREA, &[(400, 300); 16], 0.0, &GEOMETRY);
        assert_eq!(cards.len(), 10);
        assert_eq!(card(&cards, 9).alpha, 0.5);
        assert_eq!(card(&cards, 8).alpha, 1.0);
        let nine = layout(AREA, &[(400, 300); 9], 0.0, &GEOMETRY);
        assert!(nine.iter().all(|c| c.alpha == 1.0), "nine are all whole");
    }

    #[test]
    fn one_entry_stays_in_front_whatever_the_position() {
        let cards = layout(AREA, &[(400, 300)], 0.7, &GEOMETRY);
        assert_eq!((cards[0].depth, cards[0].alpha), (0.0, 1.0));
    }

    #[test]
    fn hit_takes_the_frontmost_card_under_the_point() {
        let cards = layout(AREA, &[(400, 300); 2], 0.0, &GEOMETRY);
        let (front, back) = (card(&cards, 0), card(&cards, 1));
        let (x, y) = front.to_screen.map(200.0, 150.0).unwrap();
        assert_eq!(hit(&cards, x, y), Some(0));
        let (x, y) = back.to_screen.map(5.0, 5.0).unwrap();
        assert!(
            hit(&cards[1..], x, y).is_none(),
            "front card misses {x},{y}"
        );
        assert_eq!(hit(&cards, x, y), Some(1));
        assert_eq!(hit(&cards, 999.0, 2.0), None);
    }

    #[test]
    fn a_flat_pose_maps_the_picture_onto_its_rect() {
        let r = Rect::new(130, 75, 640, 480);
        let m = Pose::flat(r).homography(AREA, (640, 480), &GEOMETRY);
        for (u, v) in [
            (0.0, 0.0),
            (640.0, 0.0),
            (0.0, 480.0),
            (640.0, 480.0),
            (17.0, 301.0),
        ] {
            let (x, y) = m.map(u, v).unwrap();
            assert!(
                (x - 130.0 - u).abs() < 1e-6 && (y - 75.0 - v).abs() < 1e-6,
                "{u},{v} lands at its own pixel"
            );
        }
        assert_eq!(bounds(&m, (640, 480)), r);
    }

    #[test]
    fn a_lerp_runs_from_one_pose_to_the_other() {
        let a = Pose::flat(Rect::new(0, 0, 1100, 480));
        let b = Pose::stack(AREA, (1100, 480), 2.0, 4, &GEOMETRY);
        assert_eq!(a.lerp(&b, 0.0), a);
        assert_eq!(a.lerp(&b, 1.0), b);
        let m = a.lerp(&b, 0.25);
        let between = |p: f64, q: f64, x: f64| p.min(q) < x && x < p.max(q);
        assert!(between(a.x, b.x, m.x), "x {m:?}");
        assert!(between(a.y, b.y, m.y), "y {m:?}");
        assert!(between(a.z, b.z, m.z), "z {m:?}");
        assert!(between(a.scale, b.scale, m.scale), "scale {m:?}");
        assert!(between(a.yaw, b.yaw, m.yaw), "yaw {m:?}");
        assert!(between(a.tilt, b.tilt, m.tilt), "tilt {m:?}");
        assert!(between(a.pitch, b.pitch, m.pitch), "pitch {m:?}");
    }

    #[test]
    fn flown_all_the_way_out_the_cards_are_the_stack() {
        let sizes = [(400, 300), (640, 480), (300, 200)];
        let flat = [Some(Rect::new(10, 10, 400, 300)), None, None];
        assert_eq!(
            fly(AREA, &sizes, &flat, 0, 0.0, 1.0, &GEOMETRY),
            layout(AREA, &sizes, 0.0, &GEOMETRY)
        );
    }

    #[test]
    fn unflown_the_windows_lie_on_the_desktop_with_the_top_one_last() {
        let sizes = [(400, 300), (640, 480), (300, 200)];
        let flat = [
            Some(Rect::new(10, 10, 400, 300)),
            Some(Rect::new(200, 100, 640, 480)),
            None,
        ];
        let cards = fly(AREA, &sizes, &flat, 1, 0.0, 0.0, &GEOMETRY);
        let order: Vec<usize> = cards.iter().map(|c| c.entry).collect();
        assert_eq!(order, [0, 1], "the minimized one is not drawn; 1 on top");
        assert_eq!(card(&cards, 1).bounds, Rect::new(200, 100, 640, 480));
        assert_eq!(card(&cards, 1).alpha, 1.0);
        let half = fly(AREA, &sizes, &flat, 1, 0.0, 0.5, &GEOMETRY);
        assert_eq!(card(&half, 2).alpha, 0.5, "it fades in where it stacks");
        assert_eq!(
            card(&half, 2).to_screen,
            card(&layout(AREA, &sizes, 0.0, &GEOMETRY), 2).to_screen
        );
    }

    #[test]
    fn a_leaving_entry_moves_the_cards_behind_it_up_one() {
        let n = 5;
        for k in 0..n {
            for pos in [0.0, 0.3, 1.0, 2.6, 3.0, 3.4, 4.0, 4.7] {
                let before = layout(AREA, &vec![(400, 300); n], pos, &GEOMETRY);
                let after = layout(AREA, &vec![(400, 300); n - 1], forget(pos, n, k), &GEOMETRY);
                let gone = card(&before, k).depth;
                let gone = if gone < 0.0 { gone + n as f64 } else { gone };
                for j in (0..n).filter(|&j| j != k) {
                    let was = card(&before, j).depth;
                    let now = card(&after, if j > k { j - 1 } else { j }).depth;
                    let want = if was < gone { was } else { was - 1.0 };
                    // On the shorter stack's round: mid-roll, the last
                    // card may stand at -0.3 for 3.7, on its way round.
                    let off = (now - want).rem_euclid((n - 1) as f64);
                    assert!(
                        off < 1e-9 || off > (n - 1) as f64 - 1e-9,
                        "entry {j}, {k} leaving at {pos}: depth {now}, want {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_roll_under_way_heads_on_to_the_same_entry() {
        assert_eq!(forget_roll(1.0, 4.0, 5, 3), (1.0, 3.0));
        assert_eq!(forget_roll(1.0, 3.0, 5, 3), (1.0, 3.0));
        assert_eq!(forget_roll(4.5, 6.0, 5, 2), (3.5, 5.0));
        assert_eq!(forget_roll(4.0, 4.0, 5, 4), (0.0, 0.0));
        assert_eq!(forget_roll(0.3, 1.0, 5, 0), (0.0, 0.0));
    }

    #[test]
    fn a_joining_entry_moves_the_cards_behind_it_back_one() {
        let n = 4;
        for i in 0..=n {
            for pos in [0.0, 1.0, 2.0, 3.0, 0.3, 2.6] {
                let before = layout(AREA, &vec![(400, 300); n], pos, &GEOMETRY);
                let now = insert(pos, n, i);
                let after = layout(AREA, &vec![(400, 300); n + 1], now, &GEOMETRY);
                let new = card(&after, i).depth;
                if pos.fract() == 0.0 {
                    assert!(new > 0.0, "entry joining at {i} at {pos} takes the front");
                }
                for j in 0..n {
                    let was = card(&before, j).depth;
                    let is = card(&after, if j >= i { j + 1 } else { j }).depth;
                    let want = if was < 0.0 || was < new {
                        was
                    } else {
                        was + 1.0
                    };
                    assert!(
                        (is - want).abs() < 1e-9,
                        "entry {j}, joining at {i} at {pos}: depth {is}, want {want}"
                    );
                }
                let back = forget(now, n + 1, i);
                assert!((back - pos).abs() < 1e-9, "forget undoes it: {back}");
            }
        }
    }

    #[test]
    fn a_roll_under_way_as_an_entry_joins_heads_on_to_the_same_entry() {
        assert_eq!(insert_roll(0.0, 2.0, 4, 3), (0.0, 2.0));
        assert_eq!(insert_roll(0.0, 2.0, 4, 1), (0.0, 3.0));
        assert_eq!(insert_roll(3.0, 4.0, 4, 3), (4.0, 5.0));
    }

    #[test]
    fn a_card_leaving_mid_roll_sends_no_other_card_round() {
        let n = 5;
        for k in 0..n {
            for (pos, to) in [(0.3, 1.0), (0.7, 1.0), (2.4, 3.0), (1.6, 1.0), (4.2, 5.0)] {
                let before = layout(AREA, &vec![(400, 300); n], pos, &GEOMETRY);
                let (now, _) = forget_roll(pos, to, n, k);
                let after = layout(AREA, &vec![(400, 300); n - 1], now, &GEOMETRY);
                for j in (0..n).filter(|&j| j != k) {
                    let was = card(&before, j).depth;
                    let is = card(&after, if j > k { j - 1 } else { j }).depth;
                    if was < 0.0 {
                        continue;
                    }
                    assert!(
                        is >= 0.0 && is <= was + 1e-9 && was - is <= 1.0 + 1e-9,
                        "entry {j}, {k} leaving at {pos}: depth {was} to {is}"
                    );
                }
            }
        }
    }
}
