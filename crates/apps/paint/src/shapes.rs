//! The pixels each drawing tool covers, as point lists in picture coordinates. A shape's pixel is
//! in when its centre is inside the shape.

use reprisekit::raster::Rect;

pub type Pts = Vec<(i32, i32)>;

/// A brush tip; `size` is its width in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tip {
    Round,
    Square,
    /// A line rising to the right.
    Slash,
    /// A line falling to the right.
    Backslash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Brush {
    pub tip: Tip,
    pub size: i32,
}

impl Brush {
    pub const fn square(size: i32) -> Brush {
        Brush {
            tip: Tip::Square,
            size,
        }
    }

    /// The pixels the tip covers around `(0, 0)`.
    pub fn offsets(self) -> Pts {
        let n = self.size.max(1);
        let lo = -(n - 1) / 2;
        let hi = lo + n;
        match self.tip {
            Tip::Square => (lo..hi)
                .flat_map(|y| (lo..hi).map(move |x| (x, y)))
                .collect(),
            Tip::Round => {
                let c = (lo + hi) as f64 / 2.0 - 0.5;
                let r2 = (n as f64 / 2.0).powi(2);
                (lo..hi)
                    .flat_map(|y| (lo..hi).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let (dx, dy) = (x as f64 - c, y as f64 - c);
                        dx * dx + dy * dy <= r2
                    })
                    .collect()
            }
            Tip::Slash => (0..n).map(|i| (lo + i, hi - 1 - i)).collect(),
            Tip::Backslash => (0..n).map(|i| (lo + i, lo + i)).collect(),
        }
    }
}

/// Every pixel of the straight line from `a` to `b`, both ends in.
pub fn line(a: (i32, i32), b: (i32, i32)) -> Pts {
    let (dx, dy) = ((b.0 - a.0).abs(), -(b.1 - a.1).abs());
    let (sx, sy) = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    let (mut x, mut y, mut err) = (a.0, a.1, dx + dy);
    let mut out = Vec::with_capacity((dx - dy + 1) as usize);
    loop {
        out.push((x, y));
        if (x, y) == b {
            return out;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// The joined lines through `pts`.
pub fn polyline(pts: &[(i32, i32)]) -> Pts {
    match pts {
        [] => Vec::new(),
        [p] => vec![*p],
        _ => pts.windows(2).flat_map(|w| line(w[0], w[1])).collect(),
    }
}

/// `path` with the brush's tip stamped on each pixel, duplicates removed.
pub fn stamp(path: &[(i32, i32)], brush: Brush) -> Pts {
    let tip = brush.offsets();
    let mut out: Pts = path
        .iter()
        .flat_map(|&(x, y)| tip.iter().map(move |&(dx, dy)| (x + dx, y + dy)))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The pixels with centres inside the shape `inside` tests, over `r`.
fn covered(r: Rect, inside: impl Fn(f64, f64) -> bool) -> Pts {
    (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| inside(x as f64 + 0.5, y as f64 + 0.5))
        .collect()
}

/// The rectangle with corners `a` and `b`, both in.
pub fn span(a: (i32, i32), b: (i32, i32)) -> Rect {
    let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
    let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
    Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1)
}

/// `b` moved so the rectangle from `a` is square, keeping its direction from `a`.
pub fn squared(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    let n = (b.0 - a.0).abs().max((b.1 - a.1).abs());
    let sign = |d: i32| if d < 0 { -1 } else { 1 };
    (a.0 + sign(b.0 - a.0) * n, a.1 + sign(b.1 - a.1) * n)
}

/// `b` moved onto the nearest of the horizontal, vertical and 45 degree lines through `a`.
pub fn snapped(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (ax, ay) = (dx.abs(), dy.abs());
    if ay * 2 < ax {
        (b.0, a.1)
    } else if ax * 2 < ay {
        (a.0, b.1)
    } else {
        squared(a, b)
    }
}

fn in_ellipse(r: Rect, t: f64) -> impl Fn(f64, f64) -> bool {
    let (cx, cy) = (
        r.x as f64 + r.width as f64 / 2.0,
        r.y as f64 + r.height as f64 / 2.0,
    );
    let (a, b) = (r.width as f64 / 2.0 - t, r.height as f64 / 2.0 - t);
    move |x, y| a > 0.0 && b > 0.0 && ((x - cx) / a).powi(2) + ((y - cy) / b).powi(2) <= 1.0
}

/// A rounded rectangle's corner radius for a box this size.
pub fn round_radius(r: Rect) -> f64 {
    (r.width.min(r.height) as f64 / 2.0).min(8.0)
}

fn in_round_rect(r: Rect, t: f64) -> impl Fn(f64, f64) -> bool {
    let rad = (round_radius(r) - t).max(0.0);
    let (x0, y0) = (r.x as f64 + t, r.y as f64 + t);
    let (x1, y1) = (r.right() as f64 - t, r.bottom() as f64 - t);
    move |x, y| {
        if x < x0 || x > x1 || y < y0 || y > y1 {
            return false;
        }
        let cx = x.clamp(x0 + rad, x1 - rad);
        let cy = y.clamp(y0 + rad, y1 - rad);
        (x - cx).powi(2) + (y - cy).powi(2) <= rad * rad
    }
}

fn in_rect(r: Rect, t: f64) -> impl Fn(f64, f64) -> bool {
    move |x, y| {
        x > r.x as f64 + t
            && x < r.right() as f64 - t
            && y > r.y as f64 + t
            && y < r.bottom() as f64 - t
    }
}

/// The shapes drawn by dragging out a box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxShape {
    Rect,
    Ellipse,
    RoundRect,
}

/// The shape's outline `width` pixels thick, inside `r`.
pub fn outline(shape: BoxShape, r: Rect, width: i32) -> Pts {
    let t = width.max(1) as f64;
    match shape {
        BoxShape::Rect => {
            let (o, i) = (in_rect(r, 0.0), in_rect(r, t));
            covered(r, move |x, y| o(x, y) && !i(x, y))
        }
        BoxShape::Ellipse => {
            let (o, i) = (in_ellipse(r, 0.0), in_ellipse(r, t));
            covered(r, move |x, y| o(x, y) && !i(x, y))
        }
        BoxShape::RoundRect => {
            let (o, i) = (in_round_rect(r, 0.0), in_round_rect(r, t));
            covered(r, move |x, y| o(x, y) && !i(x, y))
        }
    }
}

/// The shape's whole area; with `inset`, only what lies inside a `inset` px outline.
pub fn interior(shape: BoxShape, r: Rect, inset: i32) -> Pts {
    let t = inset as f64;
    match shape {
        BoxShape::Rect => covered(r, in_rect(r, t)),
        BoxShape::Ellipse => covered(r, in_ellipse(r, t)),
        BoxShape::RoundRect => covered(r, in_round_rect(r, t)),
    }
}

/// The pixels inside the closed polygon through `pts`, by the even-odd rule.
pub fn polygon_fill(pts: &[(i32, i32)]) -> Pts {
    if pts.len() < 3 {
        return Vec::new();
    }
    let (y0, y1) = (
        pts.iter().map(|p| p.1).min().unwrap_or(0),
        pts.iter().map(|p| p.1).max().unwrap_or(0),
    );
    let c = |p: (i32, i32)| (p.0 as f64 + 0.5, p.1 as f64 + 0.5);
    let mut out = Vec::new();
    for y in y0..=y1 {
        let yc = y as f64 + 0.5;
        let mut xs: Vec<f64> = (0..pts.len())
            .filter_map(|i| {
                let (a, b) = (c(pts[i]), c(pts[(i + 1) % pts.len()]));
                ((a.1 <= yc) != (b.1 <= yc)).then(|| a.0 + (yc - a.1) * (b.0 - a.0) / (b.1 - a.1))
            })
            .collect();
        xs.sort_by(f64::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            let from = (pair[0] - 0.5).ceil() as i32;
            let to = (pair[1] - 0.5).floor() as i32;
            out.extend((from..=to).map(|x| (x, y)));
        }
    }
    out
}

/// The closed outline through `pts`.
pub fn polygon_outline(pts: &[(i32, i32)], width: i32) -> Pts {
    let mut ring = pts.to_vec();
    if let Some(&first) = pts.first() {
        ring.push(first);
    }
    stamp(&polyline(&ring), Brush::square(width))
}

/// The cubic curve from `a` to `b` pulled toward `c1` and `c2`, as joined lines.
pub fn curve(a: (i32, i32), c1: (i32, i32), c2: (i32, i32), b: (i32, i32)) -> Pts {
    let f = |p: (i32, i32)| (p.0 as f64, p.1 as f64);
    let (p0, p1, p2, p3) = (f(a), f(c1), f(c2), f(b));
    let len = [p0, p1, p2, p3]
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum::<f64>();
    let steps = (len / 4.0).ceil().clamp(1.0, 512.0) as usize;
    let pts: Vec<(i32, i32)> = (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            let x = w[0] * p0.0 + w[1] * p1.0 + w[2] * p2.0 + w[3] * p3.0;
            let y = w[0] * p0.1 + w[1] * p1.1 + w[2] * p2.1 + w[3] * p3.1;
            (x.round() as i32, y.round() as i32)
        })
        .collect();
    polyline(&pts)
}

/// A small xorshift generator, so a spray is the same on every run.
#[derive(Clone, Copy, Debug)]
pub struct Spray(pub u32);

impl Spray {
    fn next(&mut self) -> u32 {
        let mut x = self.0.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// `n` pixels scattered inside the disc of `radius` around `at`.
    pub fn burst(&mut self, at: (i32, i32), radius: i32, n: usize) -> Pts {
        let r = radius.max(1);
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let dx = (self.next() % (2 * r as u32 + 1)) as i32 - r;
            let dy = (self.next() % (2 * r as u32 + 1)) as i32 - r;
            if dx * dx + dy * dy <= r * r {
                out.push((at.0 + dx, at.1 + dy));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(pts: &[(i32, i32)], w: i32, h: i32) -> Vec<String> {
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| if pts.contains(&(x, y)) { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_line_covers_both_ends_without_gaps() {
        assert_eq!(
            grid(&line((0, 0), (4, 2)), 5, 3),
            ["#....", ".##..", "...##"]
        );
        assert_eq!(line((2, 2), (2, 2)), [(2, 2)]);
        assert_eq!(line((3, 0), (0, 0)).len(), 4);
    }

    #[test]
    fn brush_tips_cover_their_size() {
        assert_eq!(Brush::square(1).offsets(), [(0, 0)]);
        assert_eq!(Brush::square(3).offsets().len(), 9);
        let round = Brush {
            tip: Tip::Round,
            size: 5,
        };
        assert_eq!(
            grid(
                &round
                    .offsets()
                    .iter()
                    .map(|(x, y)| (x + 2, y + 2))
                    .collect::<Vec<_>>(),
                5,
                5
            ),
            [".###.", "#####", "#####", "#####", ".###."],
            "a round tip drops the corners"
        );
        let slash = Brush {
            tip: Tip::Slash,
            size: 3,
        };
        assert_eq!(slash.offsets(), [(-1, 1), (0, 0), (1, -1)]);
    }

    #[test]
    fn rectangle_outline_is_width_thick_inside_the_box() {
        let r = Rect::new(0, 0, 5, 4);
        assert_eq!(
            grid(&outline(BoxShape::Rect, r, 1), 5, 4),
            ["#####", "#...#", "#...#", "#####"]
        );
        assert_eq!(
            grid(&outline(BoxShape::Rect, r, 2), 5, 4),
            ["#####", "#####", "#####", "#####"]
        );
        assert_eq!(
            grid(&interior(BoxShape::Rect, r, 1), 5, 4),
            [".....", ".###.", ".###.", "....."]
        );
    }

    #[test]
    fn ellipse_fills_its_box_without_the_corners() {
        let r = Rect::new(0, 0, 7, 5);
        assert_eq!(
            grid(&interior(BoxShape::Ellipse, r, 0), 7, 5),
            [".#####.", "#######", "#######", "#######", ".#####."]
        );
        let ring = outline(BoxShape::Ellipse, r, 1);
        assert!(ring.contains(&(0, 2)) && ring.contains(&(3, 0)));
        assert!(!ring.contains(&(3, 2)), "the centre stays clear");
    }

    #[test]
    fn rounded_rectangle_rounds_only_the_corners() {
        let r = Rect::new(0, 0, 20, 20);
        let all = interior(BoxShape::RoundRect, r, 0);
        assert!(!all.contains(&(0, 0)) && all.contains(&(10, 0)) && all.contains(&(0, 10)));
    }

    #[test]
    fn polygon_fills_by_pixel_centre() {
        let tri = polygon_fill(&[(0, 0), (4, 0), (0, 4)]);
        assert_eq!(
            grid(&tri, 5, 5),
            ["#####", "####.", "###..", "##...", "....."]
        );
        let ring = polygon_outline(&[(0, 0), (3, 0), (3, 2), (0, 2)], 1);
        assert_eq!(grid(&ring, 4, 3), ["####", "#..#", "####"]);
    }

    #[test]
    fn shift_constraints() {
        assert_eq!(squared((0, 0), (5, -2)), (5, -5));
        assert_eq!(snapped((0, 0), (9, 2)), (9, 0));
        assert_eq!(snapped((0, 0), (1, 9)), (0, 9));
        assert_eq!(snapped((0, 0), (5, 6)), (6, 6));
    }

    #[test]
    fn a_curve_with_controls_on_the_line_is_the_line() {
        let c = curve((0, 0), (2, 0), (4, 0), (6, 0));
        assert_eq!(grid(&c, 7, 1), ["#######"]);
        let bent = curve((0, 4), (0, 0), (8, 0), (8, 4));
        assert!(bent.iter().all(|p| p.1 <= 4) && bent.iter().any(|p| p.1 == 1));
    }

    #[test]
    fn spray_stays_in_its_disc() {
        let mut s = Spray(7);
        let pts = s.burst((10, 10), 3, 40);
        assert_eq!(pts.len(), 40);
        assert!(
            pts.iter()
                .all(|(x, y)| (x - 10).pow(2) + (y - 10).pow(2) <= 9)
        );
        assert_eq!(
            Spray(7).burst((10, 10), 3, 40),
            pts,
            "a seed sprays the same"
        );
    }
}
