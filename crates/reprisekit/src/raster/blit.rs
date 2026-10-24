use super::{Canvas, Rect, fade_px, over_px, screen_px};
use crate::theme::Color;

impl Canvas {
    /// Resample to `w` x `h` logical pixels at the same scale: downscaling
    /// averages the covered source box, upscaling is nearest neighbour. An
    /// empty source or destination yields a transparent canvas.
    pub fn resample_box(&self, w: u32, h: u32) -> Canvas {
        if (w, h) == (self.width, self.height) {
            return self.clone();
        }
        let (sw, sh) = self.device_size();
        let s = self.scale as u32;
        let mut out = Canvas::resample_box_from(sw, sh, w * s, h * s, |x, y| {
            self.pixels[y as usize * sw as usize + x as usize]
        });
        (out.width, out.height, out.scale) = (w, h, self.scale);
        out
    }

    /// [`Canvas::resample_box`] over a `sw` x `sh` source read through
    /// `px(x, y)`, which returns premultiplied ARGB. A decoder resamples its
    /// own buffer this way without first copying it into a full-size canvas.
    pub fn resample_box_from(
        sw: u32,
        sh: u32,
        w: u32,
        h: u32,
        px: impl Fn(u32, u32) -> u32,
    ) -> Canvas {
        let mut out = Canvas::new(w, h);
        if sw == 0 || sh == 0 || w == 0 || h == 0 {
            return out;
        }
        let (sw, sh) = (sw as usize, sh as usize);
        for y in 0..h as usize {
            let y0 = y * sh / h as usize;
            let y1 = ((y + 1) * sh / h as usize).max(y0 + 1);
            for x in 0..w as usize {
                let x0 = x * sw / w as usize;
                let x1 = ((x + 1) * sw / w as usize).max(x0 + 1);
                let (mut a, mut r, mut g, mut b) = (0u32, 0u32, 0u32, 0u32);
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let p = px(sx as u32, sy as u32);
                        a += (p >> 24) & 0xFF;
                        r += (p >> 16) & 0xFF;
                        g += (p >> 8) & 0xFF;
                        b += p & 0xFF;
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as u32;
                let avg = |v: u32| (v + n / 2) / n;
                out.pixels[y * w as usize + x] = avg(a) << 24 | avg(r) << 16 | avg(g) << 8 | avg(b);
            }
        }
        out
    }

    /// Composite premultiplied `src` onto this canvas at logical (x, y),
    /// source-over, forcing the destination alpha to 0xFF (the canvas is
    /// opaque chrome).
    pub fn blit_canvas(&mut self, x: i32, y: i32, src: &Canvas) {
        let clip = self.area();
        self.blit_rows(x, y, src, clip, |s, d| {
            let a = (s >> 24) & 0xFF;
            if a == 0xFF {
                return s;
            }
            let blend = |sc: u32, dc: u32| -> u32 { sc + dc * (255 - a) / 255 };
            let r = blend((s >> 16) & 0xFF, (d >> 16) & 0xFF);
            let g = blend((s >> 8) & 0xFF, (d >> 8) & 0xFF);
            let b = blend(s & 0xFF, d & 0xFF);
            0xFF00_0000 | r << 16 | g << 8 | b
        });
    }

    /// Compose `src` over this canvas at logical (x, y) inside `clip`:
    /// `blend(source, dest)` per device pixel with non-zero source alpha. A
    /// source at another scale is point-sampled.
    fn blit_rows(
        &mut self,
        x: i32,
        y: i32,
        src: &Canvas,
        clip: Rect,
        blend: impl Fn(u32, u32) -> u32,
    ) {
        // An edit drawn on a scratch canvas reports through its blit.
        if let Some(spot) = &src.spot {
            self.spot = Some(Box::new((**spot).clone().offset(x, y)));
        }
        let dst = Rect::new(x, y, src.width as i32, src.height as i32)
            .intersect(&clip)
            .intersect(&self.area())
            .scaled(self.scale);
        if dst.is_empty() {
            return;
        }
        let (ds, ss) = (self.scale, src.scale);
        let (dw, sw) = (self.device_size().0 as usize, src.device_size().0 as usize);
        // The source device column or row under device `p` of this canvas,
        // for a source whose origin sits at device `o`.
        let at = |p: i32, o: i32| ((p - o) * ss / ds) as usize;
        let (ox, oy) = (x * ds, y * ds);
        let n = dst.width as usize;
        for py in dst.y..dst.bottom() {
            let d0 = py as usize * dw + dst.x as usize;
            let s0 = at(py, oy) * sw;
            let drow = &mut self.pixels[d0..d0 + n];
            if ss == ds {
                let s0 = s0 + at(dst.x, ox);
                for (d, &s) in drow.iter_mut().zip(&src.pixels[s0..s0 + n]) {
                    if s >> 24 != 0 {
                        *d = blend(s, *d);
                    }
                }
            } else {
                for (d, px) in drow.iter_mut().zip(dst.x..) {
                    let s = src.pixels[s0 + at(px, ox)];
                    if s >> 24 != 0 {
                        *d = blend(s, *d);
                    }
                }
            }
        }
    }

    /// Like `blit_canvas`, but composes the alpha channel too, for a
    /// translucent destination.
    pub fn blit_over(&mut self, x: i32, y: i32, src: &Canvas) {
        let clip = self.area();
        self.blit_over_clipped(x, y, src, clip);
    }

    /// `blit_over` with only the pixels inside `clip` touched: an icon
    /// half scrolled out of a list's rows, say.
    pub fn blit_over_clipped(&mut self, x: i32, y: i32, src: &Canvas, clip: Rect) {
        self.blit_rows(x, y, src, clip, over_px);
    }

    /// Screen `src` onto this canvas at logical (x, y) inside `clip`:
    /// `d = s + d - s * d / 255` per premultiplied channel, so light adds
    /// without darkening.
    pub fn blit_screen_clipped(&mut self, x: i32, y: i32, src: &Canvas, clip: Rect) {
        self.blit_rows(x, y, src, clip, screen_px);
    }

    /// `blit_over` of `src` faded by `num / den` as `fade` would, without
    /// the faded copy: a disabled icon drawn on every repaint.
    pub fn blit_over_faded(&mut self, x: i32, y: i32, src: &Canvas, num: u32, den: u32) {
        let clip = self.bounds();
        self.blit_rows(x, y, src, clip, |s, d| over_px(fade_px(s, num, den), d));
    }

    /// Blend a coverage mask in `color` at device pixel (x, y); `clip` is
    /// logical. `mask` is row-major and must hold `w * h` bytes (panics
    /// otherwise). With `antialias` false the mask is thresholded.
    #[expect(clippy::too_many_arguments)]
    pub fn blit_mask(
        &mut self,
        x: i32,
        y: i32,
        w: usize,
        h: usize,
        mask: &[u8],
        color: Color,
        antialias: bool,
        clip: Rect,
    ) {
        assert!(
            mask.len() >= w * h,
            "blit_mask: {}x{} mask needs {} bytes, got {}",
            w,
            h,
            w * h,
            mask.len()
        );
        let solid = color.argb();
        self.mask_pixels(x, y, w, h, mask, clip, |d, cov| {
            if !antialias {
                if cov >= 0x60 {
                    *d = solid;
                }
                return;
            }
            let dst = *d;
            let lerp = |dc: u32, s: u8| -> u32 {
                ((dc * (255 - cov as u32) + s as u32 * cov as u32) / 255) & 0xFF
            };
            let r = lerp((dst >> 16) & 0xFF, color.r);
            let g = lerp((dst >> 8) & 0xFF, color.g);
            let b = lerp(dst & 0xFF, color.b);
            *d = 0xFF00_0000 | r << 16 | g << 8 | b;
        });
    }

    /// [`Canvas::blit_mask`] for a translucent canvas: `color` at each
    /// pixel's coverage laid over the premultiplied pixel beneath, alpha
    /// included.
    #[expect(clippy::too_many_arguments)]
    pub fn blit_mask_over(
        &mut self,
        x: i32,
        y: i32,
        w: usize,
        h: usize,
        mask: &[u8],
        color: Color,
        clip: Rect,
    ) {
        assert!(mask.len() >= w * h, "blit_mask_over: {w}x{h} mask is short");
        self.mask_pixels(x, y, w, h, mask, clip, |d, cov| {
            let cov = cov as u32;
            let ch = |shift: u32, s: u8| {
                let dc = (*d >> shift) & 0xFF;
                (dc * (255 - cov) + s as u32 * cov + 127) / 255
            };
            *d = ch(24, 255) << 24 | ch(16, color.r) << 16 | ch(8, color.g) << 8 | ch(0, color.b);
        });
    }

    /// Run `f` on each device pixel under a `w` x `h` mask at device (x, y)
    /// inside logical `clip` whose coverage is non-zero.
    #[expect(clippy::too_many_arguments)]
    fn mask_pixels(
        &mut self,
        x: i32,
        y: i32,
        w: usize,
        h: usize,
        mask: &[u8],
        clip: Rect,
        mut f: impl FnMut(&mut u32, u8),
    ) {
        let dst = Rect::new(x, y, w as i32, h as i32).intersect(&self.device_area(clip));
        if dst.is_empty() {
            return;
        }
        let (ox, oy) = ((dst.x - x) as usize, (dst.y - y) as usize);
        let dw = self.device_size().0 as usize;
        for row in 0..dst.height as usize {
            let d0 = (dst.y as usize + row) * dw + dst.x as usize;
            let m0 = (oy + row) * w + ox;
            let n = dst.width as usize;
            let (drow, mrow) = (&mut self.pixels[d0..d0 + n], &mask[m0..m0 + n]);
            for (d, &cov) in drow.iter_mut().zip(mrow) {
                if cov != 0 {
                    f(d, cov);
                }
            }
        }
    }
}
