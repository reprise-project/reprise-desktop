//! The Reprise mark: an R drawn as one ribbon. Each size is its own drawing on its own pixel grid
//! (`logo/r16.png` and up), written by `scripts/logo/build.py`; edit the script, not the masks.

use std::sync::OnceLock;

use crate::raster::Canvas;
use crate::theme::Color;

const SIZES: [usize; 4] = [16, 24, 32, 48];
const PNGS: [&[u8]; 4] = [
    include_bytes!("logo/r16.png"),
    include_bytes!("logo/r24.png"),
    include_bytes!("logo/r32.png"),
    include_bytes!("logo/r48.png"),
];

fn drawn(i: usize) -> &'static [u8] {
    static MASKS: [OnceLock<Vec<u8>>; 4] = [const { OnceLock::new() }; 4];
    MASKS[i].get_or_init(|| {
        let size = SIZES[i];
        let mut decoder = png::Decoder::new(std::io::Cursor::new(PNGS[i]));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().expect("logo mask: png header");
        let mut buf = vec![0u8; reader.output_buffer_size().expect("logo mask: size")];
        let info = reader.next_frame(&mut buf).expect("logo mask: png data");
        assert_eq!(
            (
                info.color_type,
                info.bit_depth,
                info.width as usize,
                info.height as usize
            ),
            (png::ColorType::Grayscale, png::BitDepth::Eight, size, size),
            "logo mask r{size}.png is not {size}x{size} 8-bit grey",
        );
        buf.truncate(size * size);
        buf
    })
}

pub fn coverage(px: usize) -> Vec<u8> {
    let (src, n) = match SIZES.iter().rposition(|&s| px.is_multiple_of(s)) {
        Some(i) => (drawn(i), SIZES[i]),
        None => (drawn(SIZES.len() - 1), SIZES[SIZES.len() - 1]),
    };
    (0..px * px)
        .map(|i| src[(i / px) * n / px * n + (i % px) * n / px])
        .collect()
}

pub fn draw(c: &mut Canvas, x: i32, y: i32, px: i32, color: Color) {
    let s = c.scale();
    let dev = (px * s).max(0) as usize;
    let mask = coverage(dev);
    let clip = c.bounds();
    c.blit_mask_over(x * s, y * s, dev, dev, &mask, color, clip);
}

struct Ribbon {
    size: f64,
    w: f64,
    gap: f64,
    stem_x: f64,
    top_y: f64,
    base_y: f64,
    bowl_r: f64,
    bowl_x: f64,
    fold_x: f64,
    leg_end_x: f64,
}

const RIBBONS: [Ribbon; 4] = [
    Ribbon {
        size: 24.0,
        w: 4.0,
        gap: 1.0,
        stem_x: 3.0,
        top_y: 3.0,
        base_y: 21.0,
        bowl_r: 4.5,
        bowl_x: 16.5,
        fold_x: 9.0,
        leg_end_x: 21.0,
    },
    Ribbon {
        size: 32.0,
        w: 5.0,
        gap: 1.5,
        stem_x: 4.5,
        top_y: 4.5,
        base_y: 27.5,
        bowl_r: 6.0,
        bowl_x: 21.5,
        fold_x: 12.0,
        leg_end_x: 27.5,
    },
    Ribbon {
        size: 48.0,
        w: 8.0,
        gap: 1.5,
        stem_x: 6.0,
        top_y: 6.0,
        base_y: 42.0,
        bowl_r: 9.0,
        bowl_x: 33.0,
        fold_x: 18.0,
        leg_end_x: 42.0,
    },
    Ribbon {
        size: 64.0,
        w: 10.0,
        gap: 1.8,
        stem_x: 9.0,
        top_y: 9.0,
        base_y: 55.0,
        bowl_r: 12.0,
        bowl_x: 41.0,
        fold_x: 24.0,
        leg_end_x: 55.0,
    },
];

fn to_segment((x, y): (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 {
        0.0
    } else {
        (((x - a.0) * dx + (y - a.1) * dy) / len).clamp(0.0, 1.0)
    };
    (x - a.0 - t * dx).hypot(y - a.1 - t * dy)
}

impl Ribbon {
    fn inks(&self, p: (f64, f64)) -> bool {
        let h = self.w / 2.0;
        let cy = self.top_y + self.bowl_r;
        let low = cy + self.bowl_r;
        let leg = to_segment(p, (self.fold_x, low), (self.leg_end_x, self.base_y));
        if leg <= h {
            return true;
        }
        if leg < h + self.gap {
            return false;
        }
        let bowl = if p.0 >= self.bowl_x {
            ((p.0 - self.bowl_x).hypot(p.1 - cy) - self.bowl_r).abs()
        } else {
            f64::INFINITY
        };
        let stem = to_segment(p, (self.stem_x, self.base_y), (self.stem_x, self.top_y));
        let bar = to_segment(p, (self.stem_x, self.top_y), (self.bowl_x, self.top_y));
        let foot = to_segment(p, (self.bowl_x, low), (self.fold_x, low));
        bowl.min(stem).min(bar).min(foot) <= h
    }
}

pub fn path_coverage(px: usize) -> Vec<u8> {
    let r = RIBBONS
        .iter()
        .find(|r| r.size == px as f64)
        .unwrap_or(if px < 24 { &RIBBONS[0] } else { &RIBBONS[3] });
    let k = r.size / px as f64;
    const N: usize = 8;
    (0..px * px)
        .map(|i| {
            let (x, y) = ((i % px) as f64, (i / px) as f64);
            let mut n = 0;
            for sy in 0..N {
                for sx in 0..N {
                    let p = (
                        (x + (sx as f64 + 0.5) / N as f64) * k,
                        (y + (sy as f64 + 0.5) / N as f64) * k,
                    );
                    n += usize::from(r.inks(p));
                }
            }
            (n * 255 / (N * N)) as u8
        })
        .collect()
}

pub fn draw_path(c: &mut Canvas, x: i32, y: i32, px: i32, color: Color) {
    let s = c.scale();
    let dev = (px * s).max(0) as usize;
    let mask = path_coverage(dev);
    let clip = c.bounds();
    c.blit_mask_over(x * s, y * s, dev, dev, &mask, color, clip);
}

#[cfg(test)]
pub(crate) fn assert_drawn(c: &Canvas, x: i32, y: i32, px: i32, ink: Color) {
    assert_inked(c, x, y, px, &coverage(px as usize), ink);
}

#[cfg(test)]
pub(crate) fn assert_path_drawn(c: &Canvas, x: i32, y: i32, px: i32, ink: Color) {
    assert_inked(c, x, y, px, &path_coverage(px as usize), ink);
}

#[cfg(test)]
fn assert_inked(c: &Canvas, x: i32, y: i32, px: i32, m: &[u8], ink: Color) {
    let mut full = 0;
    for (i, &a) in m.iter().enumerate() {
        let (mx, my) = ((i % px as usize) as i32, (i / px as usize) as i32);
        if a == 255 {
            full += 1;
            assert_eq!(
                c.get(x + mx, y + my),
                ink.argb(),
                "the {px} px mark at ({x}, {y}) inks ({mx}, {my})"
            );
        }
    }
    assert!(full > 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows16() -> Vec<String> {
        coverage(16)
            .chunks(16)
            .map(|r| {
                r.iter()
                    .map(|&a| match a {
                        255 => '#',
                        0 => '.',
                        _ => '+',
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_16_px_mark_has_a_3_px_stem_and_a_1_px_gap_where_the_leg_folds() {
        let rows = rows16();
        for row in &rows[2..14] {
            assert_eq!(&row[1..4], "###", "{row}");
        }
        assert_eq!(rows[7], ".###..+#.#####+.");
        assert_eq!(rows[8], ".###.+###.####+.");
        assert_eq!(&rows[9][10..11], ".");
        assert!(rows[0].chars().chain(rows[15].chars()).all(|c| c == '.'));
    }

    #[test]
    fn every_drawn_size_decodes_square_and_inks_its_corners_clear() {
        for &n in &SIZES {
            let m = coverage(n);
            assert_eq!(m.len(), n * n);
            assert_eq!(m[0], 0, "{n}: top-left clear");
            assert_eq!(m[n * n - 1], 0, "{n}: bottom-right clear");
            assert!(
                m.iter().filter(|&&a| a == 255).count() > n * n / 4,
                "{n}: solid ink"
            );
        }
    }

    #[test]
    fn the_ribbon_draws_the_same_mark_as_the_masks() {
        for n in [24, 32, 48] {
            let (path, mask) = (path_coverage(n), coverage(n));
            let off: usize = path
                .iter()
                .zip(&mask)
                .map(|(&a, &b)| usize::from(a.abs_diff(b)))
                .sum();
            assert!(off < n * n * 26, "{n}: {off} levels off in all");
            assert!(path.iter().filter(|&&a| a == 255).count() > n * n / 4);
            assert_eq!(path[0], 0, "{n}: top-left clear");
        }
    }

    #[test]
    fn the_ribbon_at_an_unlaid_size_is_smooth_and_solid() {
        let m = path_coverage(72);
        assert!(m.iter().any(|&a| a > 0 && a < 255), "anti-aliased edges");
        assert_eq!(m[36 * 72 + 10], 255);
    }

    #[test]
    fn a_multiple_of_a_drawn_size_is_that_drawing_blown_up() {
        let m16 = coverage(16);
        let m64 = coverage(64);
        let m32 = coverage(32);
        for y in 0..64 {
            for x in 0..64 {
                assert_eq!(m64[y * 64 + x], m32[(y / 2) * 32 + x / 2]);
            }
        }
        let m80 = coverage(80);
        assert_eq!(m80[7 * 5 * 80 + 9 * 5], m16[7 * 16 + 9]);
    }

    #[test]
    fn draw_inks_the_mark_in_its_colour_at_the_canvas_scale() {
        let mut c = Canvas::with_scale(20, 20, 2);
        let ink = Color::rgb(0x0a, 0x24, 0x6a);
        draw(&mut c, 2, 2, 16, ink);
        assert_eq!(c.get(4, 10), 0xFF00_0000 | 0x0a246a);
        assert_eq!(c.get(2, 2), 0, "the mark's clear corner");
    }
}
