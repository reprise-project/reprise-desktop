//! The picture: opaque 0xFFRRGGBB pixels in rows, and the whole-picture operations of the Image
//! menu.

use reprisekit::raster::Rect;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
}

pub const WHITE: u32 = 0xFFFF_FFFF;
pub const BLACK: u32 = 0xFF00_0000;

impl Bitmap {
    pub fn new(w: i32, h: i32, fill: u32) -> Bitmap {
        let (w, h) = (w.max(1), h.max(1));
        Bitmap {
            w,
            h,
            px: vec![fill; (w * h) as usize],
        }
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.w, self.h)
    }

    fn at(&self, x: i32, y: i32) -> Option<usize> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then(|| (y * self.w + x) as usize)
    }

    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        self.at(x, y).map(|i| self.px[i])
    }

    /// Sets one pixel; points off the picture are dropped.
    pub fn set(&mut self, x: i32, y: i32, c: u32) {
        if let Some(i) = self.at(x, y) {
            self.px[i] = c;
        }
    }

    pub fn plot(&mut self, pts: &[(i32, i32)], c: u32) {
        for &(x, y) in pts {
            self.set(x, y, c);
        }
    }

    pub fn fill_rect(&mut self, r: Rect, c: u32) {
        let r = r.intersect(&self.bounds());
        for y in r.y..r.bottom() {
            let row = (y * self.w) as usize;
            self.px[row + r.x as usize..row + r.right() as usize].fill(c);
        }
    }

    /// The pixels under `r`, with `outside` where `r` leaves the picture.
    pub fn copy(&self, r: Rect, outside: u32) -> Bitmap {
        let mut out = Bitmap::new(r.width, r.height, outside);
        for y in 0..out.h {
            for x in 0..out.w {
                if let Some(c) = self.get(r.x + x, r.y + y) {
                    out.px[(y * out.w + x) as usize] = c;
                }
            }
        }
        out
    }

    /// Draws `src` with its top-left at `(x, y)`, skipping pixels `skip` says to leave.
    pub fn draw(&mut self, x: i32, y: i32, src: &Bitmap, skip: impl Fn(usize, u32) -> bool) {
        for sy in 0..src.h {
            for sx in 0..src.w {
                let i = (sy * src.w + sx) as usize;
                let c = src.px[i];
                if !skip(i, c) {
                    self.set(x + sx, y + sy, c);
                }
            }
        }
    }

    pub fn paste(&mut self, x: i32, y: i32, src: &Bitmap) {
        self.draw(x, y, src, |_, _| false);
    }

    /// Fills the 4-connected run of pixels the colour of `(x, y)` with `c`; the changed bounds,
    /// or None when nothing changed.
    pub fn flood(&mut self, x: i32, y: i32, c: u32) -> Option<Rect> {
        let target = self.get(x, y)?;
        if target == c {
            return None;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (x, y, x, y);
        let mut stack = vec![(x, y)];
        while let Some((sx, sy)) = stack.pop() {
            if self.get(sx, sy) != Some(target) {
                continue;
            }
            let mut l = sx;
            while self.get(l - 1, sy) == Some(target) {
                l -= 1;
            }
            let mut r = sx;
            while self.get(r + 1, sy) == Some(target) {
                r += 1;
            }
            for px in l..=r {
                self.set(px, sy, c);
                for ny in [sy - 1, sy + 1] {
                    if self.get(px, ny) == Some(target) {
                        stack.push((px, ny));
                    }
                }
            }
            (x0, x1, y0, y1) = (x0.min(l), x1.max(r), y0.min(sy), y1.max(sy));
        }
        Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    fn mapped(&self, w: i32, h: i32, from: impl Fn(i32, i32) -> (i32, i32)) -> Bitmap {
        let mut out = Bitmap::new(w, h, WHITE);
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = from(x, y);
                out.px[(y * w + x) as usize] = self.get(sx, sy).unwrap_or(WHITE);
            }
        }
        out
    }

    pub fn flipped_h(&self) -> Bitmap {
        self.mapped(self.w, self.h, |x, y| (self.w - 1 - x, y))
    }

    pub fn flipped_v(&self) -> Bitmap {
        self.mapped(self.w, self.h, |x, y| (x, self.h - 1 - y))
    }

    /// Turned clockwise by `quarters` right angles.
    pub fn rotated(&self, quarters: u32) -> Bitmap {
        let (w, h) = (self.w, self.h);
        match quarters % 4 {
            1 => self.mapped(h, w, |x, y| (y, h - 1 - x)),
            2 => self.mapped(w, h, |x, y| (w - 1 - x, h - 1 - y)),
            3 => self.mapped(h, w, |x, y| (w - 1 - y, x)),
            _ => self.clone(),
        }
    }

    /// Stretched to `w`x`h`, each new pixel taking the old one under its centre.
    pub fn resized(&self, w: i32, h: i32) -> Bitmap {
        let (w, h) = (w.max(1), h.max(1));
        let (sw, sh) = (self.w as i64, self.h as i64);
        self.mapped(w, h, |x, y| {
            (
                ((2 * x as i64 + 1) * sw / (2 * w as i64)) as i32,
                ((2 * y as i64 + 1) * sh / (2 * h as i64)) as i32,
            )
        })
    }

    /// Slanted by `h_deg` (rows shift right going down) and `v_deg` (columns shift down going
    /// right), grown to hold the result; uncovered pixels are `bg`.
    pub fn skewed(&self, h_deg: i32, v_deg: i32, bg: u32) -> Bitmap {
        let (th, tv) = (
            (h_deg as f64).to_radians().tan(),
            (v_deg as f64).to_radians().tan(),
        );
        let dx = (self.h as f64 * th.abs()).round() as i32;
        let dy = (self.w as f64 * tv.abs()).round() as i32;
        let (w, h) = (self.w + dx, self.h + dy);
        let mut out = Bitmap::new(w, h, bg);
        for y in 0..self.h {
            let sx = if th >= 0.0 {
                y as f64
            } else {
                (self.h - 1 - y) as f64
            };
            let ox = (sx * th.abs()).round() as i32;
            for x in 0..self.w {
                let sy = if tv >= 0.0 {
                    x as f64
                } else {
                    (self.w - 1 - x) as f64
                };
                let oy = (sy * tv.abs()).round() as i32;
                out.set(x + ox, y + oy, self.px[(y * self.w + x) as usize]);
            }
        }
        out
    }

    pub fn invert(&mut self) {
        self.invert_rect(self.bounds());
    }

    pub fn invert_rect(&mut self, r: Rect) {
        let r = r.intersect(&self.bounds());
        for y in r.y..r.bottom() {
            let row = (y * self.w) as usize;
            for p in &mut self.px[row + r.x as usize..row + r.right() as usize] {
                *p ^= 0x00FF_FFFF;
            }
        }
    }

    /// The picture on a `w`x`h` sheet, kept at the top-left, new area `bg`.
    pub fn with_size(&self, w: i32, h: i32, bg: u32) -> Bitmap {
        self.copy(Rect::new(0, 0, w.max(1), h.max(1)), bg)
    }

    /// The bounds of the pixels that differ from `other`, a picture the same size.
    pub fn differing(&self, other: &Bitmap) -> Option<Rect> {
        if (self.w, self.h) != (other.w, other.h) {
            return Some(self.bounds());
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
        for y in 0..self.h {
            let row = (y * self.w) as usize..((y + 1) * self.w) as usize;
            let (a, b) = (&self.px[row.clone()], &other.px[row]);
            if a == b {
                continue;
            }
            let first = a.iter().zip(b).position(|(p, q)| p != q).unwrap_or(0) as i32;
            let last = a.iter().zip(b).rposition(|(p, q)| p != q).unwrap_or(0) as i32;
            (x0, x1, y0, y1) = (x0.min(first), x1.max(last), y0.min(y), y1.max(y));
        }
        (x1 >= 0).then(|| Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    /// Flattened onto white from straight-alpha RGBA bytes.
    pub fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Bitmap {
        let px = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let a = p[3] as u32;
                let ch = |v: u8| (v as u32 * a + 255 * (255 - a) + 127) / 255;
                0xFF00_0000 | ch(p[0]) << 16 | ch(p[1]) << 8 | ch(p[2])
            })
            .collect();
        Bitmap {
            w: w as i32,
            h: h as i32,
            px,
        }
    }

    pub fn to_rgb(&self) -> Vec<u8> {
        self.px
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 0xFFFF_0000;

    fn from_rows(rows: &[&str]) -> Bitmap {
        let h = rows.len() as i32;
        let w = rows[0].len() as i32;
        let px = rows
            .iter()
            .flat_map(|r| r.chars().map(|c| if c == '#' { BLACK } else { WHITE }))
            .collect();
        Bitmap { w, h, px }
    }

    fn rows(b: &Bitmap) -> Vec<String> {
        (0..b.h)
            .map(|y| {
                (0..b.w)
                    .map(|x| match b.get(x, y) {
                        Some(BLACK) => '#',
                        Some(WHITE) => '.',
                        Some(R) => 'r',
                        _ => '?',
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn flood_fills_the_enclosed_run_only() {
        let mut b = from_rows(&[".#...", "#.#..", ".#..."]);
        let r = b.flood(1, 1, R);
        assert_eq!(
            rows(&b),
            [".#...", "#r#..", ".#..."],
            "the fill does not leak through the corners"
        );
        assert_eq!(r, Some(Rect::new(1, 1, 1, 1)));
        assert_eq!(
            b.flood(1, 1, R),
            None,
            "filling a colour with itself changes nothing"
        );
    }

    #[test]
    fn flood_wraps_around_an_obstacle() {
        let mut b = from_rows(&["......", ".####.", ".#..#.", "......"]);
        b.flood(0, 0, R);
        assert_eq!(rows(&b), ["rrrrrr", "r####r", "r#rr#r", "rrrrrr"]);
    }

    #[test]
    fn flips_and_turns_move_each_pixel() {
        let b = from_rows(&["#..", "..."]);
        assert_eq!(rows(&b.flipped_h()), ["..#", "..."]);
        assert_eq!(rows(&b.flipped_v()), ["...", "#.."]);
        assert_eq!(rows(&b.rotated(1)), [".#", "..", ".."]);
        assert_eq!(rows(&b.rotated(2)), ["...", "..#"]);
        assert_eq!(rows(&b.rotated(3)), ["..", "..", "#."]);
    }

    #[test]
    fn resize_doubles_and_halves_by_nearest_pixel() {
        let b = from_rows(&["#.", ".#"]);
        assert_eq!(rows(&b.resized(4, 2)), ["##..", "..##"]);
        let big = from_rows(&["##..", "##..", "..##", "..##"]);
        assert_eq!(rows(&big.resized(2, 2)), ["#.", ".#"]);
    }

    #[test]
    fn skew_shifts_rows_and_grows_the_sheet() {
        let b = from_rows(&["##", "##", "##"]);
        let s = b.skewed(45, 0, WHITE);
        assert_eq!(rows(&s), ["##...", ".##..", "..##."]);
    }

    #[test]
    fn invert_and_attributes_keep_the_corner() {
        let mut b = from_rows(&["#."]);
        b.invert();
        assert_eq!(rows(&b), [".#"]);
        assert_eq!(rows(&b.with_size(3, 2, R)), [".#r", "rrr"]);
        assert_eq!(rows(&b.with_size(1, 1, R)), ["."]);
    }

    #[test]
    fn differing_bounds_the_changed_pixels() {
        let a = Bitmap::new(5, 5, WHITE);
        let mut b = a.clone();
        assert_eq!(a.differing(&b), None);
        b.set(1, 3, BLACK);
        b.set(3, 1, BLACK);
        assert_eq!(a.differing(&b), Some(Rect::new(1, 1, 3, 3)));
    }

    #[test]
    fn alpha_flattens_onto_white() {
        let b = Bitmap::from_rgba(2, 1, &[255, 0, 0, 255, 0, 0, 0, 0]);
        assert_eq!(b.px, [R, WHITE]);
        assert_eq!(Bitmap::from_rgba(1, 1, &[0, 0, 0, 128]).px, [0xFF7F_7F7F]);
    }
}
