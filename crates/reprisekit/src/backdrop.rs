//! Geometry for a translucent look's blurred backdrop: rects covering a
//! rectangle with cut corners, and the draws of one blur step. Pure.

use crate::raster::{Canvas, Rect};

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

/// A corner table for radius `N + 1`: row `i` loses the pixels whose centres
/// lie outside the circle. Inverse of `corner_radius`.
pub const fn round_corner<const N: usize>() -> [i32; N] {
    let r2 = 2 * (N as i64 + 1);
    let mut out = [0; N];
    let mut row = 0;
    while row < N {
        let dy = r2 - 2 * row as i64 - 1;
        let mut k = 0;
        while k < N as i64 {
            let dx = r2 - 2 * k - 1;
            if dx * dx + dy * dy <= r2 * r2 {
                break;
            }
            k += 1;
        }
        out[row] = k as i32;
        row += 1;
    }
    out
}

pub fn grow(r: Rect, by: i32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.width + 2 * by, r.height + 2 * by)
}

/// What a translucent look's glass adds to the blurred backdrop `b` under its chrome, channels 0..1:
///
/// ```text
/// b' = b + gain·(colour·luma(b) - mean·b - (colour - mean)/2)
/// ```
///
/// clamped to 0..1. The chrome's one-alpha glass over `b'` shows its colour in step with the
/// backdrop's brightness (`glass::tint::lift`). The compositor's glass shader runs the same sum.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lift {
    pub colour: [f32; 3],
    pub mean: f32,
    pub gain: f32,
}

impl Lift {
    pub const NONE: Lift = Lift {
        colour: [0.0; 3],
        mean: 0.0,
        gain: 0.0,
    };

    pub fn over(&self, b: [f32; 3]) -> [f32; 3] {
        let l = 0.299 * b[0] + 0.587 * b[1] + 0.114 * b[2];
        std::array::from_fn(|i| {
            let c = self.colour[i];
            let d = c * l - self.mean * b[i] - (c - self.mean) * 0.5;
            (b[i] + self.gain * d).clamp(0.0, 1.0)
        })
    }

    /// An opaque `0xAARRGGBB` backdrop pixel, lifted and rounded.
    pub fn px(&self, px: u32) -> u32 {
        let ch = |shift: u32| ((px >> shift) & 0xFF) as f32 / 255.0;
        let [r, g, b] = self
            .over([ch(16), ch(8), ch(0)])
            .map(|v| (v * 255.0).round() as u32);
        px & 0xFF00_0000 | r << 16 | g << 8 | b
    }

    /// Lifts the pixels of `c` that `art`, about to be laid at (`x`, `y`), covers with any alpha:
    /// the backdrop a CPU-drawn preview lays its glass art over. Both canvases at scale 1.
    pub fn under(&self, c: &mut Canvas, x: i32, y: i32, art: &Canvas) {
        if self.gain == 0.0 {
            return;
        }
        let a = art.bounds();
        for ay in 0..a.height {
            for ax in 0..a.width {
                if art.get(ax, ay) >> 24 != 0 && c.bounds().contains(x + ax, y + ay) {
                    let px = c.get(x + ax, y + ay);
                    c.put_raw(x + ax, y + ay, self.px(px));
                }
            }
        }
    }
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
    fn a_round_corner_table_steps_its_radius() {
        let t: [i32; 25] = round_corner();
        assert_eq!(corner_radius(&t), 26.0);
        assert!(
            t.windows(2).all(|w| w[0] >= w[1]),
            "insets shrink toward the straight edge"
        );
        assert_eq!(t[24], 0);
        for (row, &got) in t.iter().enumerate() {
            let (r, cy) = (26.0_f64, row as f64 + 0.5);
            let outside = (0..26)
                .filter(|&k| (r - f64::from(k) - 0.5).powi(2) + (r - cy).powi(2) > r * r)
                .count();
            assert_eq!(got, outside as i32, "row {row}");
        }
        let small: [i32; 5] = round_corner();
        assert_eq!(small, [4, 2, 1, 1, 0], "radius 6");
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
