//! Geometry for a translucent look's blurred backdrop: rects covering a
//! rectangle with cut corners, and the draws of one blur step. Pure.

use crate::raster::Rect;

/// One blur step's four draws: offset in source texels and blend alpha
/// (1, 1/2, 1/3, 1/4 leave the mean).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    pub dx: f64,
    pub dy: f64,
    pub alpha: f32,
}

pub fn kawase_taps(offset: f64) -> [Tap; 4] {
    let o = offset;
    [
        Tap {
            dx: -o,
            dy: -o,
            alpha: 1.0,
        },
        Tap {
            dx: o,
            dy: -o,
            alpha: 0.5,
        },
        Tap {
            dx: -o,
            dy: o,
            alpha: 1.0 / 3.0,
        },
        Tap {
            dx: o,
            dy: o,
            alpha: 0.25,
        },
    ]
}

/// Rects covering a `w`x`h` rectangle with corners cut by `corner` (entry `i`
/// is the pixels row `i` from top or bottom loses per side); equal insets merge.
pub fn rounded_rects(w: i32, h: i32, corner: &[i32]) -> Vec<Rect> {
    let inset = |y: i32| {
        let i = y.min(h - 1 - y) as usize;
        corner.get(i).copied().unwrap_or(0)
    };
    let mut out: Vec<Rect> = Vec::new();
    let mut y = 0;
    while y < h {
        let i = inset(y);
        let mut end = y + 1;
        while end < h && inset(end) == i {
            end += 1;
        }
        if w - 2 * i > 0 {
            out.push(Rect::new(i, y, w - 2 * i, end - y));
        }
        y = end;
    }
    out
}

/// The radius of the round corner a stepped `corner` table approximates
/// (`corner_radius_fits_the_glass_corner`). 0 for an empty table.
pub fn corner_radius(corner: &[i32]) -> f32 {
    if corner.is_empty() {
        0.0
    } else {
        corner.len() as f32 + 1.0
    }
}

pub fn grow(r: Rect, by: i32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.width + 2 * by, r.height + 2 * by)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_radius_fits_the_glass_corner() {
        let table = crate::glass::CORNER;
        let r = corner_radius(table);
        assert_eq!(r, 7.0);
        let cut = |row: usize| {
            let dy = f64::from(r) - (row as f64 + 0.5);
            f64::from(r) - (f64::from(r).powi(2) - dy * dy).sqrt()
        };
        for (row, &want) in table.iter().enumerate() {
            assert_eq!(cut(row).ceil() as i32, want, "row {row}");
        }
        assert!(cut(table.len()) < 0.02, "the first row past the table");
        assert_eq!(corner_radius(&[]), 0.0, "a square corner");
    }

    #[test]
    fn taps_average_four_diagonal_samples() {
        let t = kawase_taps(1.5);
        let offs: Vec<(f64, f64)> = t.iter().map(|t| (t.dx, t.dy)).collect();
        assert_eq!(
            offs,
            vec![(-1.5, -1.5), (1.5, -1.5), (-1.5, 1.5), (1.5, 1.5)]
        );
        let alphas: Vec<f32> = t.iter().map(|t| t.alpha).collect();
        assert_eq!(
            alphas,
            vec![1.0, 0.5, 1.0 / 3.0, 0.25],
            "a running mean of four draws"
        );
    }

    #[test]
    fn a_square_corner_is_one_rect() {
        assert_eq!(rounded_rects(10, 6, &[]), vec![Rect::new(0, 0, 10, 6)]);
    }

    #[test]
    fn rounded_rects_cover_every_inside_pixel_once() {
        let (w, h) = (40, 30);
        let corner = [5, 3, 2, 1, 1, 1];
        let rects = rounded_rects(w, h, &corner);
        let mut hits = vec![0u8; (w * h) as usize];
        for r in &rects {
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    hits[(y * w + x) as usize] += 1;
                }
            }
        }
        for y in 0..h {
            let from_edge = y.min(h - 1 - y) as usize;
            let inset = corner.get(from_edge).copied().unwrap_or(0);
            for x in 0..w {
                let inside = x >= inset && x < w - inset;
                assert_eq!(hits[(y * w + x) as usize], inside as u8, "pixel ({x},{y})");
            }
        }
        assert_eq!(
            rects.len(),
            9,
            "rows of equal inset merge (5, 3, 2, 1 at each end, then the middle): {rects:?}"
        );
        assert_eq!(rects[0], Rect::new(5, 0, 30, 1), "the top row");
        assert_eq!(rects[3], Rect::new(1, 3, 38, 3), "rows 3 to 5 cut 1 px");
        assert_eq!(rects[8], Rect::new(5, 29, 30, 1), "the bottom row");
    }

    #[test]
    fn a_rect_shorter_than_two_corners_cuts_from_the_nearer_edge() {
        let rects = rounded_rects(20, 3, &[5, 3, 2]);
        assert_eq!(
            rects,
            vec![
                Rect::new(5, 0, 10, 1),
                Rect::new(3, 1, 14, 1),
                Rect::new(5, 2, 10, 1)
            ]
        );
    }

    #[test]
    fn grow_pads_every_side() {
        assert_eq!(grow(Rect::new(10, 20, 5, 6), 3), Rect::new(7, 17, 11, 12));
    }
}
