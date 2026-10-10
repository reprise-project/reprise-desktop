//! How much of a device pixel a shape covers, 0..1. Every function takes the pixel's top-left corner
//! (`x`, `y`) and the shape, in device pixels, and counts an 8x8 grid of points in the pixel where an
//! edge crosses it (a pixel wholly inside or outside answers at once).

use super::{Px, centre};

const GRID: i32 = 8;

/// The share of the 8x8 points in the pixel at (`x`, `y`) that `inside` holds.
fn grid_share(x: f64, y: f64, inside: impl Fn(f64, f64) -> bool) -> f64 {
    let n = GRID;
    let hits = (0..n * n)
        .filter(|k| {
            let px = x + (f64::from(k % n) + 0.5) / f64::from(n);
            let py = y + (f64::from(k / n) + 0.5) / f64::from(n);
            inside(px, py)
        })
        .count();
    hits as f64 / f64::from(n * n)
}

/// Whether the pixel at (`x`, `y`) lies wholly outside the box from (`x0`, `y0`) to (`x1`, `y1`).
fn outside(x: f64, y: f64, (x0, y0, x1, y1): (f64, f64, f64, f64)) -> bool {
    x + 1.0 <= x0 || y + 1.0 <= y0 || x >= x1 || y >= y1
}

/// How much of the pixel at (`x`, `y`) a `w` x `h` rect at the origin with corners rounded over `r` covers.
pub fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> f64 {
    if w <= 0.0 || h <= 0.0 || x + 1.0 <= 0.0 || y + 1.0 <= 0.0 || x >= w || y >= h {
        return 0.0;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let corner_x = x < r || x + 1.0 > w - r;
    let corner_y = y < r || y + 1.0 > h - r;
    let whole = x >= 0.0 && y >= 0.0 && x + 1.0 <= w && y + 1.0 <= h;
    if whole && !(corner_x && corner_y) {
        return 1.0;
    }
    grid_share(x, y, |px, py| {
        if px < 0.0 || py < 0.0 || px > w || py > h {
            return false;
        }
        let cx = px.clamp(r, w - r);
        let cy = py.clamp(r, h - r);
        (px - cx).powi(2) + (py - cy).powi(2) <= r * r
    })
}

/// [`rounded`] with each corner its own radius: top left, top right, bottom right, bottom left. Each is held to half the shorter side.
pub fn rounded4(x: f64, y: f64, w: f64, h: f64, r: [f64; 4]) -> f64 {
    if w <= 0.0 || h <= 0.0 || outside(x, y, (0.0, 0.0, w, h)) {
        return 0.0;
    }
    let r = r.map(|r| r.min(w / 2.0).min(h / 2.0).max(0.0));
    let centres = [
        (r[0], r[0]),
        (w - r[1], r[1]),
        (w - r[2], h - r[2]),
        (r[3], h - r[3]),
    ];
    let in_corner = |i: usize, px: f64, py: f64| match i {
        0 => px < centres[0].0 && py < centres[0].1,
        1 => px > centres[1].0 && py < centres[1].1,
        2 => px > centres[2].0 && py > centres[2].1,
        _ => px < centres[3].0 && py > centres[3].1,
    };
    let whole = x >= 0.0 && y >= 0.0 && x + 1.0 <= w && y + 1.0 <= h;
    let touches = |i: usize| {
        let (cx, cy) = centres[i];
        let xs = if i == 0 || i == 3 {
            x < cx
        } else {
            x + 1.0 > cx
        };
        let ys = if i < 2 { y < cy } else { y + 1.0 > cy };
        r[i] > 0.0 && xs && ys
    };
    if whole && !(0..4).any(touches) {
        return 1.0;
    }
    grid_share(x, y, |px, py| {
        if px < 0.0 || py < 0.0 || px > w || py > h {
            return false;
        }
        (0..4).all(|i| {
            if !in_corner(i, px, py) {
                return true;
            }
            let (cx, cy) = centres[i];
            (px - cx).powi(2) + (py - cy).powi(2) <= r[i] * r[i]
        })
    })
}

/// How much of the pixel at (`x`, `y`) the ellipse round (`cx`, `cy`) with radii `rx`, `ry` covers; a disc when equal.
pub fn ellipse(x: f64, y: f64, cx: f64, cy: f64, rx: f64, ry: f64) -> f64 {
    if rx <= 0.0 || ry <= 0.0 || outside(x, y, (cx - rx, cy - ry, cx + rx, cy + ry)) {
        return 0.0;
    }
    let inside = |px: f64, py: f64| ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) <= 1.0;
    let corners = [(x, y), (x + 1.0, y), (x, y + 1.0), (x + 1.0, y + 1.0)];
    if corners.iter().all(|&(px, py)| inside(px, py)) {
        return 1.0;
    }
    grid_share(x, y, inside)
}

/// Whether (`px`, `py`) lies inside the polygon through `pts`, by the even-odd rule.
fn in_polygon(px: f64, py: f64, pts: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let ((xi, yi), (xj, yj)) = (pts[i], pts[j]);
        if (yi > py) != (yj > py) && px < xi + (py - yi) * (xj - xi) / (yj - yi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn bounds(pts: &[(f64, f64)], m: f64) -> (f64, f64, f64, f64) {
    pts.iter().fold(
        (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        |(x0, y0, x1, y1), &(x, y)| (x0.min(x - m), y0.min(y - m), x1.max(x + m), y1.max(y + m)),
    )
}

/// How much of the pixel at (`x`, `y`) the polygon through `pts` (closed back to the first point) covers, by the even-odd rule.
pub fn polygon(x: f64, y: f64, pts: &[(f64, f64)]) -> f64 {
    if pts.len() < 3 || outside(x, y, bounds(pts, 0.0)) {
        return 0.0;
    }
    grid_share(x, y, |px, py| in_polygon(px, py, pts))
}

/// Whether (`px`, `py`) lies within `half` of the segment from `a` to `b`, its ends cut square.
fn on_segment(px: f64, py: f64, a: (f64, f64), b: (f64, f64), half: f64) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return false;
    }
    let t = ((px - a.0) * dx + (py - a.1) * dy) / len2;
    if !(0.0..=1.0).contains(&t) {
        return false;
    }
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    (px - qx).powi(2) + (py - qy).powi(2) <= half * half
}

pub(super) type Segment = ((f64, f64), (f64, f64), f64);

/// How much of the pixel at (`x`, `y`) the segments cover, each cut square at its ends, with a disc of `radius` at each `(point, radius)` join.
pub(super) fn segments(x: f64, y: f64, segs: &[Segment], joins: &[((f64, f64), f64)]) -> f64 {
    let near = segs.iter().any(|&(a, b, w)| {
        let m = w / 2.0;
        !outside(x, y, bounds(&[a, b], m))
    });
    if !near {
        return 0.0;
    }
    grid_share(x, y, |px, py| {
        segs.iter()
            .any(|&(a, b, w)| on_segment(px, py, a, b, w / 2.0))
            || joins
                .iter()
                .any(|&((jx, jy), r)| (px - jx).powi(2) + (py - jy).powi(2) <= r * r)
    })
}

/// How much of the pixel at (`x`, `y`) a line `width` wide along the polyline through `pts` covers:
/// ends cut square, each bend rounded (a disc `width` across at every inner point).
pub fn stroke(x: f64, y: f64, pts: &[(f64, f64)], width: f64) -> f64 {
    if pts.len() < 2 || width <= 0.0 {
        return 0.0;
    }
    let segs: Vec<_> = pts.windows(2).map(|p| (p[0], p[1], width)).collect();
    let joins: Vec<_> = pts[1..pts.len() - 1]
        .iter()
        .map(|&p| (p, width / 2.0))
        .collect();
    segments(x, y, &segs, &joins)
}

/// A `w` x `h` (design) box painted at `d` pixels per design pixel, corners rounded over `r`, as `n`
/// rings of 1 px (outermost first, each rounded 1 px tighter) round a fill, every length `d` times
/// over: each pixel's ring and fill coverage, unrounded, `(w * d) * (h * d)` of them, row by row.
/// `paint(x, y, i)` gives ring `i` at the pixel's middle (`x`, `y`) in design pixels ([`centre`]),
/// `paint(x, y, n)` the fill.
pub fn ringed(
    w: i32,
    h: i32,
    r: f64,
    n: usize,
    d: i32,
    paint: impl Fn(f64, f64, usize) -> Px,
) -> Vec<Px> {
    let k = f64::from(d);
    let mut out = Vec::with_capacity((w * h * d * d) as usize);
    for y in 0..h * d {
        for x in 0..w * d {
            let cov = |i: usize| {
                let i = i as f64;
                rounded(
                    f64::from(x) - i * k,
                    f64::from(y) - i * k,
                    f64::from(w) * k - 2.0 * i * k,
                    f64::from(h) * k - 2.0 * i * k,
                    (r - i) * k,
                )
            };
            let mut acc: Px = (0.0, [0.0; 3]);
            let mut outer = cov(0);
            for i in 0..=n {
                let inner = if i < n { cov(i + 1) } else { 0.0 };
                let k = outer - inner;
                if k > 0.0 {
                    let p = paint(centre(x, d), centre(y, d), i);
                    acc.0 += p.0 * k;
                    for c in 0..3 {
                        acc.1[c] += p.1[c] * k;
                    }
                }
                outer = inner;
            }
            out.push(acc);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(w: i32, h: i32, f: impl Fn(f64, f64) -> f64) -> Vec<f64> {
        (0..w * h)
            .map(|i| f(f64::from(i % w), f64::from(i / w)))
            .collect()
    }

    #[test]
    fn a_rounded_rect_covers_its_middle_and_edges_whole() {
        assert_eq!(rounded(5.0, 0.0, 20.0, 10.0, 4.0), 1.0);
        assert_eq!(rounded(0.0, 5.0, 20.0, 10.0, 4.0), 1.0);
        assert_eq!(rounded(10.0, 5.0, 20.0, 10.0, 4.0), 1.0);
    }

    #[test]
    fn a_rounded_rect_covers_nothing_outside() {
        assert_eq!(rounded(-1.0, 0.0, 20.0, 10.0, 4.0), 0.0);
        assert_eq!(rounded(20.0, 0.0, 20.0, 10.0, 4.0), 0.0);
        assert_eq!(rounded(0.0, 10.0, 20.0, 10.0, 4.0), 0.0);
        assert_eq!(rounded(0.0, 0.0, 0.0, 10.0, 4.0), 0.0);
    }

    #[test]
    fn a_rounded_corner_pixel_is_part_covered() {
        assert_eq!(rounded(0.0, 0.0, 20.0, 10.0, 4.0), 0.0, "past the curve");
        let k = rounded(1.0, 1.0, 20.0, 10.0, 4.0);
        assert!(k > 0.0 && k < 1.0, "on the curve {k}");
        assert_eq!(rounded(0.0, 0.0, 20.0, 10.0, 0.0), 1.0);
    }

    #[test]
    fn rounded4_with_one_radius_is_rounded() {
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (18.0, 9.0), (3.0, 2.0), (19.0, 0.0)] {
            assert_eq!(
                rounded4(x, y, 20.0, 10.0, [4.0; 4]),
                rounded(x, y, 20.0, 10.0, 4.0),
                "({x}, {y})"
            );
        }
    }

    #[test]
    fn rounded4_rounds_only_the_corners_given() {
        let r = [4.0, 0.0, 0.0, 0.0];
        assert!(rounded4(0.0, 0.0, 20.0, 10.0, r) < 1.0);
        assert_eq!(rounded4(19.0, 0.0, 20.0, 10.0, r), 1.0);
        assert_eq!(rounded4(19.0, 9.0, 20.0, 10.0, r), 1.0);
        assert_eq!(rounded4(0.0, 9.0, 20.0, 10.0, r), 1.0);
    }

    #[test]
    fn a_disc_of_radius_10_covers_its_area() {
        let sum: f64 = grid(24, 24, |x, y| ellipse(x, y, 12.0, 12.0, 10.0, 10.0))
            .iter()
            .sum();
        let area = std::f64::consts::PI * 100.0;
        assert!((sum - area).abs() / area < 0.01, "{sum} against {area}");
    }

    #[test]
    fn a_polygon_square_covers_whole_pixels() {
        let sq = [(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)];
        assert_eq!(polygon(3.0, 3.0, &sq), 1.0);
        assert_eq!(polygon(1.0, 3.0, &sq), 0.0);
        // A triangle half covers the pixel its hypotenuse cuts corner to corner.
        let tri = [(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)];
        let k = polygon(1.0, 2.0, &tri);
        assert!((k - 0.5).abs() < 0.07, "{k}");
    }

    #[test]
    fn a_polygon_ring_is_even_odd() {
        // An outer square traced on into an inner one leaves the inner one empty.
        let ring = [
            (0.0, 0.0),
            (8.0, 0.0),
            (8.0, 8.0),
            (0.0, 8.0),
            (0.0, 0.0),
            (2.0, 2.0),
            (6.0, 2.0),
            (6.0, 6.0),
            (2.0, 6.0),
            (2.0, 2.0),
        ];
        assert_eq!(polygon(1.0, 4.0, &ring), 1.0);
        assert_eq!(polygon(4.0, 4.0, &ring), 0.0);
    }

    #[test]
    fn a_horizontal_2_px_stroke_covers_exactly_2_rows() {
        let line = [(1.0, 5.0), (9.0, 5.0)];
        let rows: Vec<f64> = (0..10)
            .map(|y| stroke(4.0, f64::from(y), &line, 2.0))
            .collect();
        assert_eq!(rows, [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(stroke(0.0, 4.0, &line, 2.0), 0.0);
        assert_eq!(stroke(9.0, 4.0, &line, 2.0), 0.0);
        assert_eq!(stroke(1.0, 4.0, &line, 2.0), 1.0);
    }

    #[test]
    fn ringed_splits_a_pixel_between_ring_and_fill() {
        let out = ringed(10, 10, 3.0, 1, 1, |_, _, i| {
            if i == 0 {
                (255.0, [0.0; 3])
            } else {
                (255.0, [255.0; 3])
            }
        });
        let corner = rounded(0.0, 0.0, 10.0, 10.0, 3.0);
        assert!(
            (out[0].0 - 255.0 * corner).abs() < 1e-9,
            "the corner pixel's share"
        );
        assert_eq!(out[5 * 10 + 5], (255.0, [255.0; 3]), "the fill inside");
        assert_eq!(out[5 * 10], (255.0, [0.0; 3]), "the ring on the edge");
    }
}
