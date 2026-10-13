//! ARGB8888 CPU canvas and its fill, bevel and blit primitives.

use crate::textedit::{Preedit, TextSpot};
use crate::theme::{Color, ColorScheme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }
    pub fn right(&self) -> i32 {
        self.x + self.width
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
    pub fn inset(&self, n: i32) -> Rect {
        Rect::new(
            self.x + n,
            self.y + n,
            (self.width - 2 * n).max(0),
            (self.height - 2 * n).max(0),
        )
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(
            x,
            y,
            self.right().max(o.right()) - x,
            self.bottom().max(o.bottom()) - y,
        )
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }
    /// All four fields multiplied by `n` (`n <= 0` counts as 1).
    pub fn scaled(&self, n: i32) -> Rect {
        let n = n.max(1);
        Rect::new(self.x * n, self.y * n, self.width * n, self.height * n)
    }
}

/// The fixed border of a nine-grid, in pixels from each side: the corners
/// keep their size, the edges stretch along one axis, the centre along both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Insets {
    pub const ZERO: Insets = Insets::new(0, 0, 0, 0);

    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Insets {
        Insets {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn scaled(&self, n: i32) -> Insets {
        let n = n.max(1);
        Insets::new(self.left * n, self.top * n, self.right * n, self.bottom * n)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fill {
    #[default]
    Stretch,
    Tile,
    /// Stretched, each target pixel taking the source pixel under its
    /// centre, so a sharp edge lands where it scales to.
    StretchCentred,
    /// Tiled across and stretched down: a texture that repeats along a
    /// row while its rows fade from top to bottom.
    TileAcross,
}

impl Fill {
    fn axes(self) -> (Fill, Fill) {
        match self {
            Fill::TileAcross => (Fill::Tile, Fill::Stretch),
            f => (f, f),
        }
    }
}

/// The two margins of one axis fitted into `len`: as given when they fit,
/// else shrunk in proportion so they fill it exactly.
fn fit_margins(a: i32, b: i32, len: i32) -> (i32, i32) {
    if a + b <= len || a + b == 0 {
        return (a, b);
    }
    let a2 = a * len / (a + b);
    (a2, len - a2)
}

/// The source coordinate on one axis for target offset `d`: corners map
/// straight or in proportion, the middle stretches or tiles. `src0`,
/// `src_len`, `a`, `b` and the result are in a `k`-times magnified atlas;
/// for a `len` that is a multiple of `k` the result is the scale-1 grid
/// with every pixel repeated `k` times.
#[allow(clippy::too_many_arguments)]
fn nine_axis(
    d: i32,
    len: i32,
    src0: i32,
    src_len: i32,
    a: i32,
    b: i32,
    fill: Fill,
    k: i32,
) -> (i32, bool) {
    let a = a.clamp(0, src_len);
    let b = b.clamp(0, src_len - a);
    if k > 1 && a + b > len {
        let l1 = (len + k - 1) / k;
        let (s, mid) = nine_axis(
            (d / k).min(l1 - 1),
            l1,
            src0 / k,
            src_len / k,
            a / k,
            b / k,
            fill,
            1,
        );
        return (s * k, mid);
    }
    let (ta, tb) = fit_margins(a, b, len);
    if d < ta {
        (src0 + d * a / ta.max(1), false)
    } else if d >= len - tb {
        let off = d - (len - tb);
        (src0 + src_len - b + off * b / tb.max(1), false)
    } else {
        let (mid_src, mid_len) = (src_len - a - b, len - ta - tb);
        if mid_src <= 0 {
            return (src0 + a.min(src_len - 1), true);
        }
        let off = d - ta;
        let s = if mid_src == mid_len {
            off
        } else {
            let (q, ml) = (off / k, ((mid_len + k - 1) / k).max(1));
            match fill {
                // `TileAcross` is split by `Fill::axes` before it gets
                // here; as one axis it stretches.
                Fill::Stretch | Fill::TileAcross => k * (q * mid_src / (k * ml)),
                Fill::Tile => off % mid_src,
                Fill::StretchCentred => k * ((2 * q + 1) * mid_src / (2 * k * ml)),
            }
        };
        (src0 + a + s, true)
    }
}

/// The atlas pixel a nine-grid of `from` drawn into `to` shows at target
/// pixel (`x`, `y`), or None outside `to` or in a skipped centre. `from`
/// and `m` are in the atlas magnified `k` times; the result is in atlas
/// pixels.
#[allow(clippy::too_many_arguments)]
pub fn nine_source(
    from: Rect,
    to: Rect,
    m: Insets,
    fill: Fill,
    border_only: bool,
    k: i32,
    x: i32,
    y: i32,
) -> Option<(i32, i32)> {
    if !to.contains(x, y) {
        return None;
    }
    let (fill_x, fill_y) = fill.axes();
    let (sx, mid_x) = nine_axis(
        x - to.x,
        to.width,
        from.x,
        from.width,
        m.left,
        m.right,
        fill_x,
        k,
    );
    let (sy, mid_y) = nine_axis(
        y - to.y,
        to.height,
        from.y,
        from.height,
        m.top,
        m.bottom,
        fill_y,
        k,
    );
    if border_only && mid_x && mid_y {
        return None;
    }
    Some((sx.div_euclid(k), sy.div_euclid(k)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeStyle {
    WindowRaised,
    ButtonRaised,
    /// A sunken field: shadow then dark shadow at the top-left.
    ButtonSunken,
    /// A pushed button: dark shadow then shadow at the top-left, the
    /// pressed Start and task buttons.
    ButtonPushed,
    ThinSunken,
    /// One px hilight top/left, shadow bottom/right: a hot toolbar button.
    ThinRaised,
}

/// A canvas has a logical size, which drawing calls measure in, and a
/// scale: each logical pixel is `scale` x `scale` device pixels of
/// `pixels`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// Row-major premultiplied ARGB, `width * scale` by `height * scale`.
    pub pixels: Vec<u32>,
    /// What drawing may touch, inside the bounds; `None` is the whole canvas.
    /// Set only inside [`Canvas::with_clip`].
    clip: Option<Rect>,
    scale: i32,
    /// The input method's preedit, for the focused edit to show at its caret.
    /// Boxed like `spot`: canvases are held by value and rarely carry either.
    preedit: Option<Box<Preedit>>,
    spot: Option<Box<TextSpot>>,
}

/// One premultiplied pixel with all four channels scaled by `num / den`.
fn fade_px(p: u32, num: u32, den: u32) -> u32 {
    let ch = |shift: u32| ((p >> shift) & 0xFF) * num / den.max(1);
    (ch(24) << 24) | (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Premultiplied `s` over `d`, alpha included: `s + d * (255 - sa) / 255`
/// per channel.
fn over_px(s: u32, d: u32) -> u32 {
    let sa = (s >> 24) & 0xFF;
    let blend = |sc: u32, dc: u32| -> u32 { sc + dc * (255 - sa) / 255 };
    let a = blend(sa, (d >> 24) & 0xFF);
    let r = blend((s >> 16) & 0xFF, (d >> 16) & 0xFF);
    let g = blend((s >> 8) & 0xFF, (d >> 8) & 0xFF);
    let b = blend(s & 0xFF, d & 0xFF);
    a << 24 | r << 16 | g << 8 | b
}

/// Premultiplied `s` screened onto `d`, per channel `s + d - s * d / 255`
/// rounded: a valid premultiplied pixel never darker than `d`.
fn screen_px(s: u32, d: u32) -> u32 {
    let ch = |k: u32| {
        let (sc, dc) = ((s >> k) & 0xFF, (d >> k) & 0xFF);
        sc + dc - (sc * dc + 127) / 255
    };
    let a = ch(24);
    a << 24 | ch(16).min(a) << 16 | ch(8).min(a) << 8 | ch(0).min(a)
}

/// `a` moved toward each `(b, l)` by `l / 255`, the moves added up per
/// premultiplied channel, rounded and cut to 0..=255, and alpha raised to
/// the largest colour channel so the pixel stays premultiplied. One move
/// is `a` and `b` blended by `l`.
fn mix_px(a: u32, moves: impl Iterator<Item = (u32, u32)> + Clone) -> u32 {
    let ch = |shift: u32| {
        let x = ((a >> shift) & 0xFF) as i32;
        let moved: i32 = moves
            .clone()
            .map(|(b, l)| l as i32 * (((b >> shift) & 0xFF) as i32 - x))
            .sum();
        (255 * x + moved + 127).div_euclid(255).clamp(0, 255) as u32
    };
    let (r, g, b) = (ch(16), ch(8), ch(0));
    (ch(24).max(r).max(g).max(b) << 24) | (r << 16) | (g << 8) | b
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Canvas::with_scale(width, height, 1)
    }

    /// New fully transparent canvas `width` x `height` logical pixels at
    /// `scale` device pixels each (`scale <= 0` counts as 1).
    pub fn with_scale(width: u32, height: u32, scale: i32) -> Self {
        let scale = scale.max(1);
        let (dw, dh) = (width * scale as u32, height * scale as u32);
        Canvas {
            width,
            height,
            // Widened first: a u32 product wraps past 65k a side, and a
            // wrapped length is a canvas whose pixels are not there.
            pixels: vec![0; dw as usize * dh as usize],
            clip: None,
            scale,
            preedit: None,
            spot: None,
        }
    }

    /// A canvas over `pixels`, row-major premultiplied ARGB, which must
    /// hold `width * height` of them.
    pub fn from_pixels(width: u32, height: u32, pixels: Vec<u32>) -> Self {
        debug_assert_eq!(pixels.len(), width as usize * height as usize);
        Canvas {
            width,
            height,
            pixels,
            clip: None,
            scale: 1,
            preedit: None,
            spot: None,
        }
    }

    /// A transparent canvas for part of this one, drawn and then blitted
    /// back onto it: the same scale, and the preedit, so an edit drawn
    /// there shows it.
    pub fn scratch(&self, width: u32, height: u32) -> Canvas {
        let mut c = Canvas::with_scale(width, height, self.scale);
        c.preedit = self.preedit.clone();
        c
    }

    pub fn set_preedit(&mut self, preedit: Option<Preedit>) {
        self.preedit = preedit.map(Box::new);
    }

    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_deref()
    }

    /// The focused edit says where it is; one frame has one focused edit,
    /// so a later report replaces an earlier one.
    pub fn report_spot(&mut self, spot: TextSpot) {
        self.spot = Some(Box::new(spot));
    }

    pub fn take_spot(&mut self) -> Option<TextSpot> {
        self.spot.take().map(|s| *s)
    }

    pub fn scale(&self) -> i32 {
        self.scale
    }

    /// The size of `pixels`: the logical size times the scale.
    pub fn device_size(&self) -> (u32, u32) {
        let s = self.scale as u32;
        (self.width * s, self.height * s)
    }

    /// This canvas flipped left to right.
    pub fn mirrored(&self) -> Canvas {
        let mut out = self.clone();
        let w = self.device_size().0 as usize;
        if w > 0 {
            for row in out.pixels.chunks_mut(w) {
                row.reverse();
            }
        }
        out
    }

    /// These same device pixels read at `scale` (a `px * scale` icon read
    /// back as `px`): the logical size is the device size divided by it,
    /// and a side that does not divide evenly is resampled down to one
    /// that does.
    pub fn at_scale(self, scale: i32) -> Canvas {
        let s = scale.max(1) as u32;
        let (dw, dh) = self.device_size();
        let (w, h) = (dw / s, dh / s);
        let mut out = if (w * s, h * s) == (dw, dh) {
            self
        } else {
            let sw = dw as usize;
            Canvas::resample_box_from(dw, dh, w * s, h * s, |x, y| {
                self.pixels[y as usize * sw + x as usize]
            })
        };
        (out.width, out.height, out.scale, out.clip) = (w, h, s as i32, None);
        out
    }

    fn device_area(&self, r: Rect) -> Rect {
        r.intersect(&self.area()).scaled(self.scale)
    }

    /// Run `f` on this canvas's device pixels read as a scale-1 canvas, so
    /// art painted at the canvas's scale goes on pixel for pixel.
    pub fn in_device_px<T>(&mut self, f: impl FnOnce(&mut Canvas) -> T) -> T {
        let s = self.scale;
        if s == 1 {
            return f(self);
        }
        let (w, h, clip) = (self.width, self.height, self.clip);
        (self.width, self.height) = self.device_size();
        self.scale = 1;
        self.clip = clip.map(|r| r.scaled(s));
        let out = f(self);
        (self.width, self.height, self.scale, self.clip) = (w, h, s, clip);
        out
    }

    /// Run `f` with drawing limited to `r` (and any clip already set). The
    /// drawing methods honour it; code writing `pixels` directly does not.
    pub fn with_clip<T>(&mut self, r: Rect, f: impl FnOnce(&mut Canvas) -> T) -> T {
        let outer = self.clip;
        self.clip = Some(self.area().intersect(&r));
        let out = f(self);
        self.clip = outer;
        out
    }

    /// Run `f`, which draws in `r`, with `r`'s device rows upside down. A Top
    /// taskbar draws through it. `r` is cut to the bounds.
    pub fn with_rows_flipped<T>(&mut self, r: Rect, f: impl FnOnce(&mut Canvas) -> T) -> T {
        self.flip_rows(r);
        let out = f(self);
        self.flip_rows(r);
        out
    }

    /// Run `f` on a clear canvas of its own (this one's scale), `r.height`
    /// wide and `r.width` tall, and lay that picture over `r` on its side:
    /// row `k` goes down `r`'s column `k` (column `r.width - 1 - k` with
    /// `mirror`), column `j` along `r`'s row `j`. A side taskbar draws
    /// through it. The picture is cut to the bounds and clip; the clip is
    /// not turned and `f` draws with no clip.
    pub fn with_transposed<T>(
        &mut self,
        r: Rect,
        mirror: bool,
        f: impl FnOnce(&mut Canvas) -> T,
    ) -> T {
        let s = self.scale;
        let mut pic = Canvas::with_scale(r.height.max(0) as u32, r.width.max(0) as u32, s);
        let out = f(&mut pic);
        let (pw, ph) = pic.device_size();
        let (pw, ph) = (pw as i32, ph as i32);
        let area = self.area().scaled(s);
        let dw = self.device_size().0 as usize;
        for py in 0..ph {
            let dx = r.x * s + if mirror { ph - 1 - py } else { py };
            for px in 0..pw {
                let dy = r.y * s + px;
                if !area.contains(dx, dy) {
                    continue;
                }
                let v = pic.pixels[(py * pw + px) as usize];
                let i = dy as usize * dw + dx as usize;
                self.pixels[i] = over_px(v, self.pixels[i]);
            }
        }
        out
    }

    fn flip_rows(&mut self, r: Rect) {
        let d = r.intersect(&self.bounds()).scaled(self.scale);
        if d.is_empty() {
            return;
        }
        let dw = self.device_size().0 as usize;
        let (x0, x1) = (d.x as usize, d.right() as usize);
        let (mut a, mut b) = (d.y as usize, d.bottom() as usize - 1);
        while a < b {
            let (head, tail) = self.pixels.split_at_mut(b * dw);
            head[a * dw + x0..a * dw + x1].swap_with_slice(&mut tail[x0..x1]);
            a += 1;
            b -= 1;
        }
    }

    fn area(&self) -> Rect {
        let b = self.bounds();
        self.clip.map_or(b, |c| c.intersect(&b))
    }

    /// Multiply every pixel (premultiplied, so all four channels) by
    /// `num / den`: the see-through drag image.
    pub fn fade(&mut self, num: u32, den: u32) {
        for p in &mut self.pixels {
            *p = fade_px(*p, num, den);
        }
    }

    /// Multiply each pixel by `strength(x, y) / 256`, `x` and `y` in
    /// device px: a fade that varies across the canvas.
    pub fn fade_by(&mut self, strength: impl Fn(i32, i32) -> u32) {
        let w = self.device_size().0.max(1) as usize;
        for (i, p) in self.pixels.iter_mut().enumerate() {
            let s = strength((i % w) as i32, (i / w) as i32).min(256);
            *p = fade_px(*p, s, 256);
        }
    }

    /// Move the pixels of `r` (logical, cut to the bounds) toward `other`'s
    /// by `level` (0.0 keeps these, 1.0 takes `other`'s exactly). Both
    /// canvases have the same size and scale.
    pub fn mix(&mut self, other: &Canvas, r: Rect, level: f32) {
        self.mix_all(&[(other, level)], r);
    }

    /// [`Canvas::mix`] toward several canvases at once. The moves add up, so
    /// each canvas shows its own level whatever the order, cut to what a
    /// premultiplied pixel holds.
    pub fn mix_all(&mut self, others: &[(&Canvas, f32)], r: Rect) {
        let levels: Vec<(&Canvas, u32)> = others
            .iter()
            .map(|&(c, level)| {
                debug_assert_eq!(
                    (self.width, self.height, self.scale),
                    (c.width, c.height, c.scale)
                );
                (c, (level.clamp(0.0, 1.0) * 255.0).round() as u32)
            })
            .filter(|&(_, l)| l > 0)
            .collect();
        if levels.is_empty() {
            return;
        }
        let r = r.intersect(&self.bounds());
        let s = self.scale;
        let dw = (self.width as i32 * s) as usize;
        for y in r.y * s..(r.y + r.height) * s {
            for x in r.x * s..(r.x + r.width) * s {
                let i = y as usize * dw + x as usize;
                let moves = levels.iter().map(|&(c, l)| (c.pixels[i], l));
                self.pixels[i] = mix_px(self.pixels[i], moves);
            }
        }
    }

    /// The smallest logical rect holding every pixel where this canvas
    /// and `other` differ, empty when none does: all a [`Canvas::mix`]
    /// between them changes. Both have the same size and scale.
    pub fn differing_rect(&self, other: &Canvas) -> Rect {
        debug_assert_eq!(
            (self.width, self.height, self.scale),
            (other.width, other.height, other.scale)
        );
        let s = self.scale;
        let dw = (self.width as i32 * s).max(1) as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
        for (y, (a, b)) in self
            .pixels
            .chunks(dw)
            .zip(other.pixels.chunks(dw))
            .enumerate()
        {
            let Some(first) = a.iter().zip(b).position(|(p, q)| p != q) else {
                continue;
            };
            let last = a.iter().zip(b).rposition(|(p, q)| p != q).unwrap_or(first);
            (x0, x1) = (x0.min(first as i32), x1.max(last as i32));
            (y0, y1) = (y0.min(y as i32), y as i32);
        }
        if x1 < 0 {
            return Rect::default();
        }
        Rect::new(x0 / s, y0 / s, x1 / s - x0 / s + 1, y1 / s - y0 / s + 1)
    }

    /// The pixels under `r` as a canvas of `r`'s size at this scale, the
    /// rest of it transparent where `r` runs past the bounds.
    pub fn copy_rect(&self, r: Rect) -> Canvas {
        let mut c = self.scratch(r.width.max(0) as u32, r.height.max(0) as u32);
        let s = self.scale;
        let inside = r.intersect(&self.bounds()).scaled(s);
        if inside.is_empty() {
            return c;
        }
        let (sw, cw) = (
            (self.width as i32 * s) as usize,
            (c.width as i32 * s) as usize,
        );
        let n = inside.width as usize;
        for y in inside.y..inside.bottom() {
            let from = y as usize * sw + inside.x as usize;
            let to = (y - r.y * s) as usize * cw + (inside.x - r.x * s) as usize;
            c.pixels[to..to + n].copy_from_slice(&self.pixels[from..from + n]);
        }
        c
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }

    pub fn stride_bytes(&self) -> usize {
        self.device_size().0 as usize * 4
    }

    /// The canvas as a StatusNotifierItem `IconPixmap` buffer: ARGB32,
    /// big-endian per pixel, straight alpha, at its device size.
    pub fn to_argb_be(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|&p| {
                let a = p >> 24;
                let straight = |shift: u32| match a {
                    0 => 0,
                    _ => (((p >> shift) & 0xFF) * 255 + a / 2) / a,
                };
                (a << 24 | straight(16) << 16 | straight(8) << 8 | straight(0)).to_be_bytes()
            })
            .collect()
    }

    /// The top-left device pixel of logical (x, y); 0 for out-of-bounds
    /// coordinates.
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if !self.bounds().contains(x, y) {
            return 0;
        }
        let (s, dw) = (self.scale as usize, self.device_size().0 as usize);
        self.pixels[y as usize * s * dw + x as usize * s]
    }

    pub fn put(&mut self, x: i32, y: i32, c: Color) {
        self.put_raw(x, y, c.argb());
    }

    /// Store a premultiplied ARGB value as is, in every device pixel of
    /// logical (x, y).
    pub fn put_raw(&mut self, x: i32, y: i32, v: u32) {
        if self.scale > 1 {
            self.fill_raw(Rect::new(x, y, 1, 1), v);
        } else if self.area().contains(x, y) {
            self.pixels[(y as u32 * self.width + x as u32) as usize] = v;
        }
    }

    /// Paint a pixel map: `rows` of characters, one cell of `s` x `s`
    /// device pixels each from (`x`, `y`), in whatever colour `ink` gives
    /// a character; None leaves the cell alone.
    pub fn paint_map(
        &mut self,
        x: i32,
        y: i32,
        s: i32,
        rows: &[&str],
        ink: impl Fn(char) -> Option<Color>,
    ) {
        let s = s.max(1);
        for (row, line) in rows.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                if let Some(color) = ink(ch) {
                    let cell = Rect::new(x + col as i32 * s, y + row as i32 * s, s, s);
                    self.fill_rect(cell, color);
                }
            }
        }
    }

    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        self.fill_raw(r, c.argb());
    }

    pub fn clear_rect(&mut self, r: Rect) {
        self.fill_raw(r, 0);
    }

    fn fill_raw(&mut self, r: Rect, v: u32) {
        let r = self.device_area(r);
        if r.is_empty() {
            return;
        }
        let dw = self.device_size().0 as usize;
        for y in r.y..r.bottom() {
            let row = y as usize * dw;
            self.pixels[row + r.x as usize..row + r.right() as usize].fill(v);
        }
    }

    pub fn hgradient(&mut self, r: Rect, left: Color, right: Color) {
        let denom = (r.width - 1).max(1) as f32;
        self.fill_columns(r, |i| {
            let t = i as f32 / denom;
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            Color::rgb(
                mix(left.r, right.r),
                mix(left.g, right.g),
                mix(left.b, right.b),
            )
        });
    }

    /// The caption gradient: pixel `i` of `r` is `left` plus the
    /// difference times `i / r.width`, rounded down to the step *below* an
    /// exact one, so the run ends a step short of `right` and each colour
    /// starts a pixel later than plain flooring would put it.
    pub fn hgradient_stepped(&mut self, r: Rect, left: Color, right: Color) {
        let den = r.width;
        self.fill_columns(r, |i| {
            let mix = |a: u8, b: u8| {
                let step = if i == 0 {
                    0
                } else {
                    ((b as i32 - a as i32) * i - 1).div_euclid(den)
                };
                (a as i32 + step) as u8
            };
            Color::rgb(
                mix(left.r, right.r),
                mix(left.g, right.g),
                mix(left.b, right.b),
            )
        });
    }

    /// Fill `r` one row at a time, row `i` in `rows[i]`; rows past the
    /// list repeat its last colour. An empty list paints nothing.
    pub fn fill_rows(&mut self, r: Rect, rows: &[Color]) {
        let Some(&last) = rows.last() else { return };
        for i in 0..r.height {
            let c = rows.get(i as usize).copied().unwrap_or(last);
            self.fill_rect(Rect::new(r.x, r.y + i, r.width, 1), c);
        }
    }

    pub fn fill_cols(&mut self, r: Rect, cols: &[Color]) {
        let Some(&last) = cols.last() else { return };
        self.fill_columns(r, |i| cols.get(i as usize).copied().unwrap_or(last));
    }

    /// Draws `from` of the scale-1 atlas `src` into `to` as a nine-grid with
    /// border `m` (see [`nine_source`]), composed over what is there. Builds
    /// only the part of `to` inside the canvas and clip.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_nine(
        &mut self,
        src: &Canvas,
        from: Rect,
        to: Rect,
        m: Insets,
        fill: Fill,
        border_only: bool,
        k: i32,
    ) {
        if to.is_empty() || from.is_empty() {
            return;
        }
        let vis = to.intersect(&self.area());
        if vis.is_empty() {
            return;
        }
        let (fill_x, fill_y) = fill.axes();
        let cols: Vec<(i32, bool)> = (vis.x..vis.right())
            .map(|x| {
                let (s, mid) = nine_axis(
                    x - to.x,
                    to.width,
                    from.x,
                    from.width,
                    m.left,
                    m.right,
                    fill_x,
                    k,
                );
                (s.div_euclid(k), mid)
            })
            .collect();
        let mut out = Canvas::new(vis.width as u32, vis.height as u32);
        for (row, y) in out.pixels.chunks_exact_mut(vis.width as usize).zip(vis.y..) {
            let (sy, mid_y) = nine_axis(
                y - to.y,
                to.height,
                from.y,
                from.height,
                m.top,
                m.bottom,
                fill_y,
                k,
            );
            let sy = sy.div_euclid(k);
            let skip_mid = border_only && mid_y;
            for (px, &(sx, mid_x)) in row.iter_mut().zip(&cols) {
                if !(skip_mid && mid_x) {
                    *px = src.get(sx, sy);
                }
            }
        }
        self.blit_over(vis.x, vis.y, &out);
    }

    /// Fill `r` with logical column `i` (counted from `r.x`) in
    /// `color(i)`: one device row built once and copied into every row
    /// inside the canvas.
    fn fill_columns(&mut self, r: Rect, color: impl Fn(i32) -> Color) {
        let clip = self.device_area(r);
        if clip.is_empty() {
            return;
        }
        let s = self.scale;
        let row: Vec<u32> = (clip.x..clip.right())
            .map(|x| color(x / s - r.x).argb())
            .collect();
        let w = self.device_size().0 as usize;
        for y in clip.y..clip.bottom() {
            let d0 = y as usize * w + clip.x as usize;
            self.pixels[d0..d0 + row.len()].copy_from_slice(&row);
        }
    }

    /// This canvas turned a quarter turn counter-clockwise: the left edge
    /// becomes the bottom, so horizontal text ends up reading upward.
    pub fn rotate_ccw(&self) -> Canvas {
        let (w, h) = self.device_size();
        let (w, h) = (w as usize, h as usize);
        let mut out = Canvas::with_scale(self.height, self.width, self.scale);
        for (sy, row) in self.pixels.chunks_exact(w.max(1)).enumerate() {
            for (sx, &p) in row.iter().enumerate() {
                out.pixels[(w - 1 - sx) * h + sy] = p;
            }
        }
        out
    }

    pub fn vline(&mut self, x: i32, y0: i32, y1: i32, c: Color) {
        self.fill_rect(Rect::new(x, y0, 1, y1 - y0), c);
    }

    /// One 3D border ring `t` px thick: top/left in `tl`, bottom/right in
    /// `br`. Bottom/right is drawn last so it owns the corners.
    pub fn ring_t(&mut self, r: Rect, tl: Color, br: Color, t: i32) {
        self.fill_rect(Rect::new(r.x, r.y, r.width, t), tl);
        self.fill_rect(Rect::new(r.x, r.y, t, r.height), tl);
        self.fill_rect(Rect::new(r.x, r.bottom() - t, r.width, t), br);
        self.fill_rect(Rect::new(r.right() - t, r.y, t, r.height), br);
    }

    pub fn edge_t(&mut self, r: Rect, style: EdgeStyle, cs: &ColorScheme, t: i32) {
        let t = t.max(1);
        match style {
            EdgeStyle::WindowRaised => {
                self.ring_t(r, cs.button_face, cs.button_dk_shadow, t);
                self.ring_t(r.inset(t), cs.button_hilight, cs.button_shadow, t);
            }
            EdgeStyle::ButtonRaised => {
                self.ring_t(r, cs.button_hilight, cs.button_dk_shadow, t);
                self.ring_t(r.inset(t), cs.button_light, cs.button_shadow, t);
            }
            EdgeStyle::ButtonSunken => {
                self.ring_t(r, cs.button_shadow, cs.button_hilight, t);
                self.ring_t(r.inset(t), cs.button_dk_shadow, cs.button_light, t);
            }
            EdgeStyle::ButtonPushed => {
                self.ring_t(r, cs.button_dk_shadow, cs.button_hilight, t);
                self.ring_t(r.inset(t), cs.button_shadow, cs.button_light, t);
            }
            EdgeStyle::ThinSunken => self.ring_t(r, cs.button_shadow, cs.button_hilight, t),
            EdgeStyle::ThinRaised => self.ring_t(r, cs.button_hilight, cs.button_shadow, t),
        }
    }

    pub fn etched_hline_t(&mut self, x0: i32, x1: i32, y: i32, cs: &ColorScheme, t: i32) {
        let t = t.max(1);
        self.fill_rect(Rect::new(x0, y, x1 - x0, t), cs.button_shadow);
        self.fill_rect(Rect::new(x0, y + t, x1 - x0, t), cs.button_hilight);
    }

    /// The divider across a dialog or a group box: like `etched_hline_t`,
    /// but the hilight turns the right end as a group box's frame turns its
    /// corner (shadow to `x1 - 1`, hilight to `x1` and one pixel over it).
    pub fn etched_divider_t(&mut self, x0: i32, x1: i32, y: i32, cs: &ColorScheme, t: i32) {
        let t = t.max(1);
        self.fill_rect(Rect::new(x0, y, x1 - x0, t), cs.button_shadow);
        self.fill_rect(Rect::new(x0, y + t, x1 - x0 + t, t), cs.button_hilight);
        self.fill_rect(Rect::new(x1, y, t, t), cs.button_hilight);
    }

    pub fn etched_vline_t(&mut self, x: i32, y0: i32, y1: i32, cs: &ColorScheme, t: i32) {
        let t = t.max(1);
        self.fill_rect(Rect::new(x, y0, t, y1 - y0), cs.button_shadow);
        self.fill_rect(Rect::new(x + t, y0, t, y1 - y0), cs.button_hilight);
    }

    /// The etched frame of a group box in `r`, `t` px per line: a sunken
    /// rectangle around a raised one, each as two Ls (top-left, then
    /// bottom-right over it) so the shared corners take the bottom-right
    /// colour.
    pub fn etched_frame_t(&mut self, r: Rect, cs: &ColorScheme, t: i32) {
        let t = t.max(1);
        for (rect, tl, br) in [
            (r, cs.button_shadow, cs.button_hilight),
            (r.inset(t), cs.button_hilight, cs.button_shadow),
        ] {
            self.fill_rect(Rect::new(rect.x, rect.y, rect.width, t), tl);
            self.fill_rect(Rect::new(rect.x, rect.y, t, rect.height), tl);
            self.fill_rect(Rect::new(rect.x, rect.bottom() - t, rect.width, t), br);
            self.fill_rect(Rect::new(rect.right() - t, rect.y, t, rect.height), br);
        }
    }

    /// Checkerboard of `t` x `t` logical cells of `a` and `b`, phase
    /// locked to canvas coordinates.
    pub fn dither_rect_t(&mut self, r: Rect, a: Color, b: Color, t: i32) {
        let t = t.max(1) * self.scale;
        let r = self.device_area(r);
        let (a, b) = (a.argb(), b.argb());
        let w = self.device_size().0 as usize;
        for y in r.y..r.bottom() {
            let row = y as usize * w;
            for x in r.x..r.right() {
                self.pixels[row + x as usize] = if (x / t + y / t) % 2 == 0 { a } else { b };
            }
        }
    }

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
        let dst = Rect::new(x, y, w as i32, h as i32).intersect(&self.device_area(clip));
        if dst.is_empty() {
            return;
        }
        let (ox, oy) = ((dst.x - x) as usize, (dst.y - y) as usize);
        let dw = self.device_size().0 as usize;
        let solid = color.argb();
        for row in 0..dst.height as usize {
            let d0 = (dst.y as usize + row) * dw + dst.x as usize;
            let m0 = (oy + row) * w + ox;
            let n = dst.width as usize;
            let (drow, mrow) = (&mut self.pixels[d0..d0 + n], &mask[m0..m0 + n]);
            for (d, &cov) in drow.iter_mut().zip(mrow) {
                if cov == 0 {
                    continue;
                }
                if !antialias {
                    if cov >= 0x60 {
                        *d = solid;
                    }
                    continue;
                }
                let dst = *d;
                let lerp = |dc: u32, s: u8| -> u32 {
                    ((dc * (255 - cov as u32) + s as u32 * cov as u32) / 255) & 0xFF
                };
                let r = lerp((dst >> 16) & 0xFF, color.r);
                let g = lerp((dst >> 8) & 0xFF, color.g);
                let b = lerp(dst & 0xFF, color.b);
                *d = 0xFF00_0000 | r << 16 | g << 8 | b;
            }
        }
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
                if cov == 0 {
                    continue;
                }
                let cov = cov as u32;
                let ch = |shift: u32, s: u8| {
                    let dc = (*d >> shift) & 0xFF;
                    (dc * (255 - cov) + s as u32 * cov + 127) / 255
                };
                *d = ch(24, 255) << 24
                    | ch(16, color.r) << 16
                    | ch(8, color.g) << 8
                    | ch(0, color.b);
            }
        }
    }
}

/// A `w` x `h` picture's size fitted inside `max`, keeping its aspect; one that fits already
/// stays as it is.
pub fn fit_size(w: u32, h: u32, max: (u32, u32)) -> (u32, u32) {
    let (mw, mh) = max;
    if w == 0 || h == 0 || (w <= mw && h <= mh) {
        return (w, h);
    }
    let scale = (mw as f64 / w as f64).min(mh as f64 / h as f64);
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

/// Write `canvas` as an opaque RGB PNG named `name` under the workspace's
/// `target/` and print where. For the ignored preview tests and the
/// gallery; panics when the file cannot be written.
pub fn write_preview(canvas: &Canvas, name: &str) {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .ancestors()
        .find(|p| p.join("target").is_dir())
        .unwrap_or(manifest);
    let path = root.join("target").join(name);
    std::fs::write(&path, to_png(canvas)).unwrap();
    eprintln!("wrote {}", path.display());
}

/// `canvas` as an opaque RGB PNG file's bytes at its device size; alpha is
/// dropped. Fast compression: a screen-sized canvas encodes on the
/// compositor's loop.
pub fn to_png(canvas: &Canvas) -> Vec<u8> {
    let mut bytes = Vec::new();
    let (w, h) = canvas.device_size();
    let mut enc = png::Encoder::new(&mut bytes, w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let rgb: Vec<u8> = canvas
        .pixels
        .iter()
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    // Writing into a Vec only fails on a size png rejects, which a
    // canvas's u32 sides and matching pixel count never are.
    let mut w = enc.write_header().expect("png header");
    w.write_image_data(&rgb).expect("png data");
    w.finish().expect("png end");
    bytes
}

pub fn rounded_fill(c: &mut Canvas, r: Rect, radius: i32, color: Color) {
    let radius = radius.min(r.width / 2).min(r.height / 2).max(0);
    fill_where(c, r, color, |x, y| in_rounded(r, radius, x, y));
}

pub fn rounded_border(c: &mut Canvas, r: Rect, radius: i32, color: Color, t: i32) {
    let inner = Rect::new(r.x + t, r.y + t, r.width - 2 * t, r.height - 2 * t);
    let inner_r = (radius - t).max(0);
    fill_where(c, r, color, |x, y| {
        in_rounded(r, radius, x, y) && !in_rounded(inner, inner_r, x, y)
    });
}

/// Paint the device pixels of `r` inside the canvas whose logical pixel
/// has `inside(x, y)`, rows indexed directly after one clip.
fn fill_where(c: &mut Canvas, r: Rect, color: Color, inside: impl Fn(i32, i32) -> bool) {
    let r = c.device_area(r);
    let (v, w, s) = (color.argb(), c.device_size().0 as usize, c.scale);
    for y in r.y..r.bottom() {
        let row = y as usize * w;
        for x in r.x..r.right() {
            if inside(x / s, y / s) {
                c.pixels[row + x as usize] = v;
            }
        }
    }
}

fn in_rounded(r: Rect, radius: i32, x: i32, y: i32) -> bool {
    if !r.contains(x, y) {
        return false;
    }
    if radius <= 0 {
        return true;
    }
    let cx = if x < r.x + radius {
        r.x + radius
    } else if x >= r.right() - radius {
        r.right() - radius - 1
    } else {
        return true;
    };
    let cy = if y < r.y + radius {
        r.y + radius
    } else if y >= r.bottom() - radius {
        r.bottom() - radius - 1
    } else {
        return true;
    };
    let (dx, dy) = ((x - cx) as i64, (y - cy) as i64);
    dx * dx + dy * dy <= (radius as i64) * (radius as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_size_scales_down_keeping_aspect() {
        assert_eq!(fit_size(800, 600, (96, 96)), (96, 72));
    }

    #[test]
    fn fit_size_never_upscales() {
        assert_eq!(fit_size(50, 20, (96, 96)), (50, 20));
    }

    #[test]
    fn mix_keeps_its_own_pixels_at_zero_takes_the_others_at_one_and_blends_between() {
        let mut a = Canvas::new(4, 2);
        let mut b = Canvas::new(4, 2);
        a.pixels.fill(0x80_40_20_10);
        b.pixels.fill(0xFF_FF_FF_FF);
        let mut m = a.clone();
        m.mix(&b, m.bounds(), 0.0);
        assert_eq!(m.pixels, a.pixels);
        let mut m = a.clone();
        m.mix(&b, m.bounds(), 1.0);
        assert_eq!(m.pixels, b.pixels);
        let mut m = a.clone();
        m.mix(&b, Rect::new(0, 0, 2, 2), 0.5);
        assert_eq!(m.get(0, 0), 0xC0_A0_90_88, "each channel half way, rounded");
        assert_eq!(m.get(3, 1), 0x80_40_20_10, "outside the rect untouched");
    }

    #[test]
    fn mix_all_adds_each_move_so_each_canvas_shows_its_own_level() {
        let a = Canvas::from_pixels(1, 1, vec![0xFF_00_00_00]);
        let red = Canvas::from_pixels(1, 1, vec![0xFF_FF_00_00]);
        let green = Canvas::from_pixels(1, 1, vec![0xFF_00_FF_00]);
        let mut m = a.clone();
        m.mix_all(&[(&red, 0.5), (&green, 1.0)], m.bounds());
        assert_eq!(
            m.get(0, 0),
            0xFF_80_FF_00,
            "red half way, green all the way"
        );
        let mut n = a.clone();
        n.mix_all(&[(&green, 1.0), (&red, 0.5)], n.bounds());
        assert_eq!(n.pixels, m.pixels, "the order does not matter");
        let mut one = a.clone();
        one.mix(&red, one.bounds(), 0.5);
        let mut all = a.clone();
        all.mix_all(&[(&red, 0.5)], all.bounds());
        assert_eq!(all.pixels, one.pixels, "one canvas is a plain mix");
    }

    #[test]
    fn mix_all_keeps_the_sum_premultiplied() {
        // One move lifts red to 255, the other drops alpha to 128.
        let mut m = Canvas::from_pixels(1, 1, vec![0xFF_80_80_80]);
        let red = Canvas::from_pixels(1, 1, vec![0xFF_FF_80_80]);
        let clear = Canvas::from_pixels(1, 1, vec![0x80_80_80_80]);
        m.mix_all(&[(&red, 1.0), (&clear, 1.0)], m.bounds());
        assert_eq!(
            m.get(0, 0),
            0xFF_FF_80_80,
            "alpha rises to the red it holds"
        );
        let mut up = Canvas::from_pixels(1, 1, vec![0xFF_C0_C0_C0]);
        let white = Canvas::from_pixels(1, 1, vec![0xFF_FF_FF_FF]);
        up.mix_all(&[(&white, 1.0), (&white, 1.0)], up.bounds());
        assert_eq!(up.get(0, 0), 0xFF_FF_FF_FF, "a channel stops at 255");
        let mut down = Canvas::from_pixels(1, 1, vec![0xFF_40_40_40]);
        let black = Canvas::from_pixels(1, 1, vec![0xFF_00_00_00]);
        down.mix_all(&[(&black, 1.0), (&black, 1.0)], down.bounds());
        assert_eq!(down.get(0, 0), 0xFF_00_00_00, "a channel stops at 0");
    }

    #[test]
    fn copy_rect_takes_the_pixels_under_a_rect_in_device_pixels() {
        let mut a = Canvas::with_scale(3, 2, 2);
        for (i, p) in a.pixels.iter_mut().enumerate() {
            *p = 0xFF_00_00_00 | i as u32;
        }
        let c = a.copy_rect(Rect::new(1, 1, 3, 1));
        assert_eq!((c.width, c.height, c.scale()), (3, 1, 2));
        assert_eq!(c.pixels[..4], a.pixels[14..18], "device row 2 from x 2");
        assert_eq!(c.pixels[6..10], a.pixels[20..24], "device row 3 from x 2");
        assert_eq!(c.pixels[4..6], [0, 0], "past the right edge, transparent");
    }

    #[test]
    fn mix_cuts_a_rect_hanging_over_the_bounds() {
        let a = Canvas::new(4, 2);
        let mut b = Canvas::new(4, 2);
        b.pixels.fill(0xFF_00_00_FF);
        let mut m = a.clone();
        m.mix(&b, Rect::new(-2, 0, 4, 1), 1.0);
        assert_eq!(m.get(0, 0), 0xFF_00_00_FF);
        assert_eq!(m.get(1, 0), 0xFF_00_00_FF);
        assert_eq!(m.get(2, 0), 0, "past the rect untouched");
        assert_eq!(m.get(0, 1), 0, "row below untouched");
    }

    #[test]
    fn mix_works_in_device_pixels_at_scale_two() {
        let mut a = Canvas::with_scale(2, 1, 2);
        let mut b = Canvas::with_scale(2, 1, 2);
        b.pixels.fill(0xFF_00_00_FF);
        a.mix(&b, Rect::new(1, 0, 1, 1), 1.0);
        assert_eq!(a.pixels[0], 0, "logical x 0 is device x 0..2");
        assert_eq!(a.pixels[2], 0xFF_00_00_FF);
        assert_eq!(a.pixels[7], 0xFF_00_00_FF);
    }

    #[test]
    fn the_differing_rect_holds_every_changed_pixel_in_logical_px() {
        let a = Canvas::with_scale(6, 5, 2);
        let mut b = a.clone();
        assert!(a.differing_rect(&b).is_empty(), "the same pictures");
        // Device (5, 3) is logical (2, 1); device (8, 6) is logical (4, 3).
        b.pixels[3 * 12 + 5] = 0xFF_00_00_FF;
        b.pixels[6 * 12 + 8] = 0xFF_00_00_FF;
        assert_eq!(a.differing_rect(&b), Rect::new(2, 1, 3, 3));
        let mut m = a.clone();
        m.mix(&b, a.differing_rect(&b), 1.0);
        assert_eq!(m.pixels, b.pixels, "a mix inside it reaches the other");
    }

    /// A scratch canvas takes the preedit down; its blit brings the spot
    /// back up, moved by where it lands.
    #[test]
    fn a_scratch_canvas_carries_the_preedit_down_and_the_spot_up() {
        use crate::textedit::{Preedit, TextEdit};
        let mut c = Canvas::with_scale(40, 40, 2);
        c.set_preedit(Preedit::new("ka".into(), 1, 1));
        let mut sc = c.scratch(10, 10);
        assert_eq!((sc.scale(), sc.preedit()), (2, c.preedit()));
        let r = Rect::new(1, 2, 3, 4);
        sc.report_spot(TextEdit::new("x", false).spot(r, r));
        c.blit_canvas(5, 6, &sc);
        let spot = c.take_spot().expect("carried up");
        assert_eq!(spot.caret, Rect::new(6, 8, 3, 4));
        assert_eq!(c.take_spot(), None, "taken");
    }
    use crate::theme::{Color, ColorScheme};

    #[test]
    fn to_png_decodes_to_the_same_rgb_without_alpha() {
        let c = Canvas::from_pixels(
            2,
            2,
            vec![0xff3a_6ea5, 0x00ff_0000, 0x8000_ff00, 0xffff_ffff],
        );
        let png = to_png(&c);
        let mut dec = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        let mut buf = vec![0; dec.output_buffer_size().unwrap()];
        let info = dec.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(
            &buf[..info.buffer_size()],
            [0x3a, 0x6e, 0xa5, 0xff, 0, 0, 0, 0xff, 0, 0xff, 0xff, 0xff]
        );
    }

    fn col(c: &Canvas, x: i32, y0: i32, y1: i32) -> Vec<u32> {
        (y0..y1).map(|y| c.get(x, y)).collect()
    }

    #[test]
    fn window_raised_edge_columns() {
        // The left column of a window frame at x=0..4 is face, hilight,
        // face, face; the right column mirrored is face, face, shadow, dk_shadow.
        let cs = ColorScheme::default();
        let mut c = Canvas::new(20, 20);
        c.fill_rect(Rect::new(0, 0, 20, 20), cs.button_face);
        c.edge_t(Rect::new(0, 0, 20, 20), EdgeStyle::WindowRaised, &cs, 1);
        let row: Vec<u32> = (0..4).map(|x| c.get(x, 10)).collect();
        assert_eq!(
            row,
            vec![
                cs.button_face.argb(),
                cs.button_hilight.argb(),
                cs.button_face.argb(),
                cs.button_face.argb()
            ]
        );
        let row: Vec<u32> = (16..20).map(|x| c.get(x, 10)).collect();
        assert_eq!(
            row,
            vec![
                cs.button_face.argb(),
                cs.button_face.argb(),
                cs.button_shadow.argb(),
                cs.button_dk_shadow.argb()
            ]
        );
        // bottom-right corner is dk_shadow, top-right outer corner is dk_shadow too (drawn last)
        assert_eq!(c.get(19, 19), cs.button_dk_shadow.argb());
        assert_eq!(c.get(19, 0), cs.button_dk_shadow.argb());
        assert_eq!(c.get(0, 19), cs.button_dk_shadow.argb());
    }

    #[test]
    fn button_raised_edge_row() {
        let cs = ColorScheme::default();
        let mut c = Canvas::new(16, 14);
        c.fill_rect(Rect::new(0, 0, 16, 14), cs.button_face);
        c.edge_t(Rect::new(0, 0, 16, 14), EdgeStyle::ButtonRaised, &cs, 1);
        let row: Vec<u32> = (0..16).map(|x| c.get(x, 7)).collect();
        let mut expected = vec![cs.button_hilight.argb()];
        expected.extend(std::iter::repeat_n(cs.button_face.argb(), 14));
        expected[14] = cs.button_shadow.argb();
        expected.push(cs.button_dk_shadow.argb());
        assert_eq!(row, expected);
    }

    #[test]
    fn sunken_edge_is_inverse() {
        let cs = ColorScheme::default();
        let mut c = Canvas::new(10, 10);
        c.edge_t(Rect::new(0, 0, 10, 10), EdgeStyle::ButtonSunken, &cs, 1);
        assert_eq!(c.get(0, 5), cs.button_shadow.argb());
        assert_eq!(c.get(1, 5), cs.button_dk_shadow.argb());
        assert_eq!(c.get(8, 5), cs.button_light.argb());
        assert_eq!(c.get(9, 5), cs.button_hilight.argb());
        c.edge_t(Rect::new(0, 0, 10, 10), EdgeStyle::ButtonPushed, &cs, 1);
        assert_eq!(c.get(0, 5), cs.button_dk_shadow.argb());
        assert_eq!(c.get(1, 5), cs.button_shadow.argb());
        assert_eq!(c.get(5, 0), cs.button_dk_shadow.argb());
        assert_eq!(c.get(8, 5), cs.button_light.argb());
        assert_eq!(c.get(9, 5), cs.button_hilight.argb());
    }

    #[test]
    fn gradient_endpoints_and_midpoint() {
        let mut c = Canvas::new(101, 1);
        c.hgradient(
            Rect::new(0, 0, 101, 1),
            Color::rgb(0, 0, 0),
            Color::rgb(200, 100, 0),
        );
        assert_eq!(c.get(0, 0), Color::rgb(0, 0, 0).argb());
        assert_eq!(c.get(100, 0), Color::rgb(200, 100, 0).argb());
        assert_eq!(c.get(50, 0), Color::rgb(100, 50, 0).argb());
    }

    /// A gradient hanging off both sides keeps each column's colour from
    /// the full rect.
    #[test]
    fn clipped_gradients_match_the_unclipped_columns() {
        let (l, r) = (Color::rgb(0, 10, 200), Color::rgb(160, 90, 8));
        let mut full = Canvas::new(20, 3);
        full.hgradient(Rect::new(0, 0, 20, 3), l, r);
        let mut stepped = Canvas::new(20, 3);
        stepped.hgradient_stepped(Rect::new(0, 0, 20, 3), l, r);
        let mut c = Canvas::new(12, 2);
        let mut cs = Canvas::new(12, 2);
        c.hgradient(Rect::new(-5, -1, 20, 3), l, r);
        cs.hgradient_stepped(Rect::new(-5, -1, 20, 3), l, r);
        for y in 0..2 {
            for x in 0..12 {
                assert_eq!(c.get(x, y), full.get(x + 5, 0), "hgradient ({x}, {y})");
                assert_eq!(cs.get(x, y), stepped.get(x + 5, 0), "stepped ({x}, {y})");
            }
        }
    }

    #[test]
    fn paint_map_scales_cells_and_skips_unlinked_characters() {
        let mut c = Canvas::new(8, 6);
        let red = Color::rgb(255, 0, 0);
        c.paint_map(2, 1, 2, &["X.", ".X"], |ch| (ch == 'X').then_some(red));
        let r = red.argb();
        assert_eq!(c.get(2, 1), r);
        assert_eq!(c.get(3, 2), r); // the cell is 2x2
        assert_eq!(c.get(4, 1), 0); // '.' is left alone
        assert_eq!(c.get(4, 3), r);
        assert_eq!(c.get(5, 4), r);
        assert_eq!(c.get(2, 3), 0);
        c.paint_map(7, 5, 3, &["XX"], |_| Some(red));
        assert_eq!(c.get(7, 5), r);
    }

    #[test]
    fn drawing_is_clipped_to_canvas() {
        let mut c = Canvas::new(4, 4);
        c.fill_rect(Rect::new(-2, -2, 10, 10), Color::rgb(1, 1, 1));
        c.fill_rect(Rect::new(-5, 1, 55, 1), Color::rgb(2, 2, 2));
        c.vline(2, -5, 50, Color::rgb(3, 3, 3));
        assert_eq!(c.get(0, 0), Color::rgb(1, 1, 1).argb());
        assert_eq!(c.get(3, 1), Color::rgb(2, 2, 2).argb());
        assert_eq!(c.get(2, 3), Color::rgb(3, 3, 3).argb());
        assert_eq!(c.get(-1, 0), 0);
        assert_eq!(c.get(4, 0), 0);
    }

    #[test]
    fn with_clip_limits_drawing_and_lifts_after() {
        let mut c = Canvas::new(10, 10);
        let red = Color::rgb(255, 0, 0);
        c.with_clip(Rect::new(2, 2, 4, 4), |c| {
            // A nested clip only narrows the outer one.
            c.with_clip(Rect::new(0, 0, 5, 10), |c| c.fill_rect(c.bounds(), red));
        });
        let painted: Vec<(i32, i32)> = (0..10)
            .flat_map(|y| (0..10).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) != 0)
            .collect();
        let want: Vec<(i32, i32)> = (2..6).flat_map(|y| (2..5).map(move |x| (x, y))).collect();
        assert_eq!(painted, want, "only x 2..5, y 2..6");
        c.fill_rect(Rect::new(9, 9, 1, 1), red);
        assert_eq!(c.get(9, 9), red.argb(), "the clip is lifted after");
    }

    #[test]
    fn off_canvas_rects_are_ignored() {
        let mut c = Canvas::new(4, 4);
        let red = Color::rgb(255, 0, 0);
        c.fill_rect(Rect::new(1000, 1, 5, 1), red);
        c.fill_rect(Rect::new(1, 1000, 1, 5), red);
        c.fill_rect(Rect::new(-1000, 1, 5, 1), red);
        c.fill_rect(Rect::new(1, -1000, 1, 5), red);
        c.vline(1000, 0, 10, red);
        assert!(c.pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn dither_is_checkerboard() {
        let a = Color::rgb(10, 10, 10);
        let b = Color::rgb(20, 20, 20);
        let mut c = Canvas::new(4, 2);
        c.dither_rect_t(Rect::new(0, 0, 4, 2), a, b, 1);
        assert_eq!(col(&c, 0, 0, 2), vec![a.argb(), b.argb()]);
        assert_eq!(col(&c, 1, 0, 2), vec![b.argb(), a.argb()]);
    }

    #[test]
    #[should_panic(expected = "blit_mask: 3x2 mask needs 6 bytes, got 5")]
    fn a_short_mask_is_refused_up_front() {
        let mut c = Canvas::new(4, 4);
        let clip = c.bounds();
        c.blit_mask(0, 0, 3, 2, &[255; 5], Color::rgb(0, 0, 0), true, clip);
    }

    #[test]
    fn mask_blit_threshold_and_blend() {
        let mut c = Canvas::new(3, 1);
        c.fill_rect(Rect::new(0, 0, 3, 1), Color::rgb(0, 0, 0));
        let mask = [0u8, 100, 255];
        c.blit_mask(
            0,
            0,
            3,
            1,
            &mask,
            Color::rgb(255, 255, 255),
            false,
            Rect::new(0, 0, 3, 1),
        );
        assert_eq!(c.get(0, 0), Color::rgb(0, 0, 0).argb());
        assert_eq!(c.get(1, 0), Color::rgb(255, 255, 255).argb());
        assert_eq!(c.get(2, 0), Color::rgb(255, 255, 255).argb());
        let mut c = Canvas::new(1, 1);
        c.fill_rect(Rect::new(0, 0, 1, 1), Color::rgb(0, 0, 0));
        c.blit_mask(
            0,
            0,
            1,
            1,
            &[128],
            Color::rgb(255, 255, 255),
            true,
            Rect::new(0, 0, 1, 1),
        );
        let v = c.get(0, 0) & 0xFF;
        assert!((127..=129).contains(&v), "blend was {v}");
    }

    #[test]
    fn canvas_blit_is_premultiplied_source_over() {
        let mut c = Canvas::new(2, 1);
        c.put(0, 0, Color::rgb(0, 0, 0));
        let mut src = Canvas::new(1, 1);
        src.pixels[0] = 0x8080_0000;
        c.blit_canvas(0, 0, &src);
        assert_eq!(c.get(0, 0), 0xFF80_0000);
        let mut untouched = Canvas::new(1, 1);
        untouched.put(0, 0, Color::rgb(1, 2, 3));
        let transparent = Canvas::new(1, 1);
        untouched.blit_canvas(0, 0, &transparent);
        assert_eq!(untouched.get(0, 0), Color::rgb(1, 2, 3).argb());
        let mut small = Canvas::new(1, 1);
        small.blit_canvas(5, 5, &src);
    }

    /// The clipped row loop must match a per-pixel source-over at every
    /// edge: a source hanging off the top-left and bottom-right corners.
    #[test]
    fn partly_off_canvas_blits_match_a_per_pixel_reference() {
        let mut src = Canvas::new(5, 4);
        for (i, p) in src.pixels.iter_mut().enumerate() {
            let a = (i as u32 * 13) & 0xFF;
            *p = a << 24 | (a / 2) << 16 | (a / 3) << 8 | (a / 4);
        }
        for (x, y) in [(-2, -1), (4, 3), (0, 0), (-9, 0), (6, 0)] {
            let mut base = Canvas::new(6, 5);
            for (i, p) in base.pixels.iter_mut().enumerate() {
                *p = 0x8000_0000 | ((i as u32 * 7) & 0xFF) << 8;
            }
            let mut want = base.clone();
            for sy in 0..src.height as i32 {
                for sx in 0..src.width as i32 {
                    let (dx, dy) = (x + sx, y + sy);
                    if !want.bounds().contains(dx, dy) {
                        continue;
                    }
                    let s = src.get(sx, sy);
                    let sa = s >> 24;
                    if sa == 0 {
                        continue;
                    }
                    let d = want.get(dx, dy);
                    let bl = |sc: u32, dc: u32| sc + dc * (255 - sa) / 255;
                    let v = bl(sa, d >> 24) << 24
                        | bl((s >> 16) & 0xFF, (d >> 16) & 0xFF) << 16
                        | bl((s >> 8) & 0xFF, (d >> 8) & 0xFF) << 8
                        | bl(s & 0xFF, d & 0xFF);
                    want.put_raw(dx, dy, v);
                }
            }
            let mut got = base.clone();
            got.blit_over(x, y, &src);
            assert_eq!(got.pixels, want.pixels, "blit_over at ({x}, {y})");
            let mut m = base.clone();
            m.blit_mask(
                x,
                y,
                5,
                4,
                &[0x70; 20],
                Color::rgb(9, 8, 7),
                false,
                m.bounds(),
            );
            let mut mw = base.clone();
            for sy in 0..4 {
                for sx in 0..5 {
                    mw.put(x + sx, y + sy, Color::rgb(9, 8, 7));
                }
            }
            assert_eq!(m.pixels, mw.pixels, "blit_mask at ({x}, {y})");
        }
    }

    #[test]
    fn mirrored_flips_each_row() {
        let (one, two) = (Color::rgb(0, 0, 1), Color::rgb(0, 0, 2));
        let mut c = Canvas::new(3, 2);
        c.put(0, 0, one);
        c.put(2, 1, two);
        let m = c.mirrored();
        assert_eq!((m.get(2, 0), m.get(0, 1)), (one.argb(), two.argb()));
        assert_eq!(m.get(0, 0), 0, "the rest moves too");
        assert_eq!(m.mirrored(), c);
    }

    #[test]
    fn rows_flipped_land_drawing_upside_down_and_keep_what_lay_under() {
        let (one, two) = (Color::rgb(0, 0, 1), Color::rgb(0, 0, 2));
        let (under, mark) = (Color::rgb(0, 0, 3), Color::rgb(0, 0, 4));
        let mut c = Canvas::with_scale(4, 6, 2);
        c.fill_rect(c.bounds(), under);
        c.put(1, 2, mark);
        let r = Rect::new(1, 1, 2, 4);
        c.with_rows_flipped(r, |c| {
            c.fill_rect(Rect::new(1, 1, 2, 1), one);
            c.put(2, 2, two);
        });
        assert_eq!(
            (c.get(1, 4), c.get(2, 4)),
            (one.argb(), one.argb()),
            "r's first row lands on its last"
        );
        assert_eq!(c.get(2, 3), two.argb(), "its second row on its second last");
        assert_eq!(c.get(1, 2), mark.argb(), "what lay under keeps its place");
        assert_eq!(c.get(1, 1), under.argb());
        assert_eq!(c.get(0, 4), under.argb(), "outside r nothing moves");
        // At scale 2 the flip is by device row: both of (2, 3)'s rows.
        let d = |x: usize, y: usize| c.pixels[y * 8 + x];
        assert_eq!(
            (d(4, 6), d(4, 7), d(5, 7)),
            (two.argb(), two.argb(), two.argb())
        );
    }

    #[test]
    fn blit_over_faded_matches_a_faded_copy_blitted_over() {
        let mut src = Canvas::new(8, 4);
        for (i, p) in src.pixels.iter_mut().enumerate() {
            let a = (i as u32 * 37) & 0xFF;
            *p = (a << 24) | ((a * 3 / 4) << 16) | ((a / 2) << 8) | (a / 5);
        }
        let mut dst = Canvas::new(10, 10);
        dst.fill_rect(dst.bounds(), Color::rgb(200, 120, 40));
        let mut want = dst.clone();
        let mut faded = src.clone();
        faded.fade(1, 2);
        want.blit_over(1, 3, &faded);
        dst.blit_over_faded(1, 3, &src, 1, 2);
        assert_eq!(
            dst.pixels, want.pixels,
            "faded blit equals fade then blit_over"
        );
    }

    #[test]
    fn blit_over_clipped_touches_only_the_clip() {
        let mut src = Canvas::new(6, 6);
        src.fill_rect(src.bounds(), Color::rgb(200, 10, 20));
        let mut full = Canvas::new(10, 10);
        full.blit_over(2, 2, &src);
        let clip = Rect::new(3, 4, 3, 2);
        let mut clipped = Canvas::new(10, 10);
        clipped.blit_over_clipped(2, 2, &src, clip);
        for y in 0..10 {
            for x in 0..10 {
                let want = if clip.contains(x, y) {
                    full.get(x, y)
                } else {
                    0
                };
                assert_eq!(clipped.get(x, y), want, "pixel ({x}, {y})");
            }
        }
    }

    #[test]
    fn canvas_blit_over_composes_alpha() {
        let mut src = Canvas::new(1, 1);
        src.pixels[0] = 0x8080_8080;
        let mut transparent = Canvas::new(1, 1);
        transparent.blit_over(0, 0, &src);
        assert_eq!(transparent.get(0, 0), 0x8080_8080);
        let mut opaque = Canvas::new(1, 1);
        opaque.put(0, 0, Color::rgb(0, 0, 0));
        opaque.blit_over(0, 0, &src);
        assert_eq!(opaque.get(0, 0), 0xFF80_8080);
        let mut untouched = Canvas::new(1, 1);
        untouched.put(0, 0, Color::rgb(1, 2, 3));
        untouched.blit_over(0, 0, &Canvas::new(1, 1));
        assert_eq!(untouched.get(0, 0), Color::rgb(1, 2, 3).argb());
        let mut small = Canvas::new(1, 1);
        small.blit_over(5, 5, &src);
    }

    #[test]
    fn rect_helpers() {
        let r = Rect::new(2, 3, 10, 5);
        assert!(r.contains(2, 3));
        assert!(r.contains(11, 7));
        assert!(!r.contains(12, 7));
        assert_eq!(r.inset(1), Rect::new(3, 4, 8, 3));
        assert_eq!(r.intersect(&Rect::new(0, 0, 5, 5)), Rect::new(2, 3, 3, 2));
    }

    #[test]
    fn rect_scaled_multiplies_all_four() {
        assert_eq!(Rect::new(2, 3, 10, 5).scaled(2), Rect::new(4, 6, 20, 10));
        assert_eq!(Rect::new(2, 3, 10, 5).scaled(0), Rect::new(2, 3, 10, 5));
    }

    #[test]
    fn resample_box_averages_checkerboard_to_grey() {
        let white = 0xFFFF_FFFFu32;
        let black = 0xFF00_0000u32;
        let mut c = Canvas::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                c.pixels[(y * 4 + x) as usize] = if (x + y) % 2 == 0 { white } else { black };
            }
        }
        let d = c.resample_box(2, 2);
        assert_eq!(d.width, 2);
        assert_eq!(d.height, 2);
        assert!(
            d.pixels.iter().all(|&p| p == 0xFF80_8080),
            "{:x?}",
            d.pixels
        );
    }

    #[test]
    fn resample_box_upscale_is_nearest() {
        let mut c = Canvas::new(2, 1);
        c.pixels[0] = 0xFF11_2233;
        c.pixels[1] = 0xFF44_5566;
        let d = c.resample_box(4, 2);
        assert_eq!(
            d.pixels,
            vec![
                0xFF11_2233,
                0xFF11_2233,
                0xFF44_5566,
                0xFF44_5566,
                0xFF11_2233,
                0xFF11_2233,
                0xFF44_5566,
                0xFF44_5566,
            ]
        );
    }

    #[test]
    fn argb_be_is_big_endian_straight_alpha() {
        let c = Canvas::from_pixels(3, 1, vec![0xFF60_6060, 0, 0x8040_2000]);
        assert_eq!(
            c.to_argb_be(),
            vec![0xFF, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0x80, 0x80, 0x40, 0]
        );
    }

    #[test]
    fn resample_box_keeps_premultiplied_alpha_consistent() {
        let mut c = Canvas::new(2, 1);
        c.pixels[0] = 0x8080_0000;
        c.pixels[1] = 0;
        let d = c.resample_box(1, 1);
        assert_eq!(d.pixels[0], 0x4040_0000);
        assert_eq!(c.resample_box(2, 1), c);
        assert_eq!(Canvas::new(0, 0).resample_box(2, 2).pixels, vec![0; 4]);
    }

    #[test]
    fn thick_edge_rings_and_etched_lines() {
        let cs = ColorScheme::default();
        let mut c = Canvas::new(12, 12);
        c.edge_t(Rect::new(0, 0, 12, 12), EdgeStyle::ButtonRaised, &cs, 2);
        // outer ring 2 px hilight, inner ring 2 px light, on the left column
        assert_eq!(col(&c, 0, 4, 6), vec![cs.button_hilight.argb(); 2]);
        assert_eq!(col(&c, 1, 4, 6), vec![cs.button_hilight.argb(); 2]);
        assert_eq!(c.get(2, 6), cs.button_light.argb());
        assert_eq!(c.get(3, 6), cs.button_light.argb());
        assert_eq!(c.get(4, 6), 0);
        assert_eq!(c.get(8, 6), cs.button_shadow.argb());
        assert_eq!(c.get(9, 6), cs.button_shadow.argb());
        assert_eq!(c.get(10, 6), cs.button_dk_shadow.argb());
        assert_eq!(c.get(11, 6), cs.button_dk_shadow.argb());
        let mut c = Canvas::new(8, 8);
        c.etched_hline_t(0, 8, 2, &cs, 2);
        assert_eq!(
            col(&c, 3, 2, 6),
            vec![
                cs.button_shadow.argb(),
                cs.button_shadow.argb(),
                cs.button_hilight.argb(),
                cs.button_hilight.argb()
            ]
        );
        let mut c = Canvas::new(8, 8);
        c.etched_vline_t(2, 0, 8, &cs, 2);
        let row: Vec<u32> = (2..6).map(|x| c.get(x, 3)).collect();
        assert_eq!(
            row,
            vec![
                cs.button_shadow.argb(),
                cs.button_shadow.argb(),
                cs.button_hilight.argb(),
                cs.button_hilight.argb()
            ]
        );
    }

    #[test]
    fn dither_cells_follow_thickness() {
        let a = Color::rgb(10, 10, 10);
        let b = Color::rgb(20, 20, 20);
        let mut c = Canvas::new(4, 4);
        c.dither_rect_t(Rect::new(0, 0, 4, 4), a, b, 2);
        assert_eq!(
            col(&c, 0, 0, 4),
            vec![a.argb(), a.argb(), b.argb(), b.argb()]
        );
        assert_eq!(
            col(&c, 2, 0, 4),
            vec![b.argb(), b.argb(), a.argb(), a.argb()]
        );
        // t = 1 is the 1 px checkerboard: colours alternate every pixel
        let mut c = Canvas::new(4, 2);
        c.dither_rect_t(Rect::new(0, 0, 4, 2), a, b, 1);
        assert_eq!(col(&c, 0, 0, 2), vec![a.argb(), b.argb()]);
    }

    #[test]
    fn rounded_shapes_clip_to_the_canvas() {
        let red = Color::rgb(200, 0, 0);
        for r in [Rect::new(-3, -2, 14, 11), Rect::new(2, 1, 12, 9)] {
            let mut fill = Canvas::new(10, 8);
            rounded_fill(&mut fill, r, 4, red);
            let mut border = Canvas::new(10, 8);
            rounded_border(&mut border, r, 4, red, 2);
            let inner = Rect::new(r.x + 2, r.y + 2, r.width - 4, r.height - 4);
            for y in 0..8 {
                for x in 0..10 {
                    let inf = in_rounded(r, 4, x, y);
                    let inb = inf && !in_rounded(inner, 2, x, y);
                    let want = |b: bool| if b { red.argb() } else { 0 };
                    assert_eq!(fill.get(x, y), want(inf), "fill {r:?} ({x}, {y})");
                    assert_eq!(border.get(x, y), want(inb), "border {r:?} ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn rotate_ccw_turns_left_edge_into_bottom() {
        let mut c = Canvas::new(3, 2);
        c.put(0, 0, Color::rgb(1, 0, 0)); // top-left
        c.put(2, 1, Color::rgb(0, 1, 0)); // bottom-right
        let r = c.rotate_ccw();
        assert_eq!((r.width, r.height), (2, 3));
        assert_eq!(r.get(0, 2), Color::rgb(1, 0, 0).argb());
        assert_eq!(r.get(1, 0), Color::rgb(0, 1, 0).argb());
    }

    /// Device rows of a canvas as 0/1 per pixel against `ink`.
    fn device_rows(c: &Canvas, ink: u32) -> Vec<String> {
        let (w, _) = c.device_size();
        c.pixels
            .chunks(w as usize)
            .map(|row| {
                row.iter()
                    .map(|&p| if p == ink { '1' } else { '0' })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_fill_at_scale_2_covers_the_device_block_of_each_logical_pixel() {
        let ink = Color::rgb(9, 9, 9);
        let mut c = Canvas::with_scale(3, 2, 2);
        assert_eq!(c.device_size(), (6, 4));
        assert_eq!(c.bounds(), Rect::new(0, 0, 3, 2));
        c.fill_rect(Rect::new(1, 0, 1, 1), ink);
        c.put(2, 1, ink);
        assert_eq!(
            device_rows(&c, ink.argb()),
            ["001100", "001100", "000011", "000011"]
        );
        assert_eq!(c.get(1, 0), ink.argb());
        assert_eq!(c.get(0, 0), 0);
    }

    #[test]
    fn a_clip_at_scale_2_limits_drawing_to_its_device_pixels() {
        let ink = Color::rgb(9, 9, 9);
        let mut c = Canvas::with_scale(3, 1, 2);
        c.with_clip(Rect::new(2, 0, 1, 1), |c| c.fill_rect(c.bounds(), ink));
        assert_eq!(device_rows(&c, ink.argb()), ["000011", "000011"]);
    }

    #[test]
    fn a_scale_1_blit_onto_a_scale_2_canvas_doubles_each_pixel() {
        let (a, b) = (Color::rgb(1, 0, 0).argb(), Color::rgb(0, 1, 0).argb());
        let src = Canvas::from_pixels(2, 1, vec![a, b]);
        let mut c = Canvas::with_scale(4, 1, 2);
        c.blit_over(1, 0, &src);
        let row: Vec<u32> = c.pixels[..8].to_vec();
        assert_eq!(row, [0, 0, a, a, b, b, 0, 0]);
        assert_eq!(c.pixels[8..], c.pixels[..8]);
    }

    #[test]
    fn a_blit_at_the_same_scale_copies_device_pixels() {
        let px = [0xFF00_0001, 0xFF00_0002, 0xFF00_0003, 0xFF00_0004];
        let src = Canvas::from_pixels(2, 2, px.to_vec()).at_scale(2);
        assert_eq!((src.width, src.height, src.scale()), (1, 1, 2));
        let mut c = Canvas::with_scale(2, 2, 2);
        c.blit_canvas(1, 1, &src);
        let at = |x: usize, y: usize| c.pixels[y * 4 + x];
        assert_eq!([at(2, 2), at(3, 2), at(2, 3), at(3, 3)], px);
        assert_eq!(at(1, 2), 0);
    }

    #[test]
    fn resample_and_rotate_keep_the_scale() {
        let c = Canvas::with_scale(4, 2, 2);
        let r = c.resample_box(2, 2);
        assert_eq!(
            (r.width, r.height, r.scale(), r.device_size()),
            (2, 2, 2, (4, 4))
        );
        let t = c.rotate_ccw();
        assert_eq!(
            (t.width, t.height, t.scale(), t.device_size()),
            (2, 4, 2, (4, 8))
        );
    }

    #[test]
    fn at_scale_resamples_a_side_that_does_not_divide() {
        let c = Canvas::new(5, 4).at_scale(2);
        assert_eq!((c.width, c.height, c.device_size()), (2, 2, (4, 4)));
        assert_eq!(c.pixels.len(), 16);
    }

    #[test]
    fn a_png_of_a_scaled_canvas_has_its_device_size() {
        let png = to_png(&Canvas::with_scale(3, 2, 2));
        let dec = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        assert_eq!((dec.info().width, dec.info().height), (6, 4));
    }

    fn nine_atlas() -> Canvas {
        let mut a = Canvas::new(3, 3);
        let ink = |i: u8| Color::rgb(i * 20, 0, 0);
        for y in 0..3 {
            for x in 0..3 {
                a.fill_rect(Rect::new(x, y, 1, 1), ink((y * 3 + x) as u8 + 1));
            }
        }
        a
    }

    #[test]
    fn nine_grid_keeps_corners_and_stretches_edges() {
        let a = nine_atlas();
        let mut c = Canvas::new(5, 4);
        c.blit_nine(
            &a,
            Rect::new(0, 0, 3, 3),
            Rect::new(0, 0, 5, 4),
            Insets::new(1, 1, 1, 1),
            Fill::Stretch,
            false,
            1,
        );
        let red = |x, y| (c.get(x, y) >> 16) & 0xFF;
        // Corners 1, 3, 7, 9; top edge 2 repeated; centre 5 over 3x2.
        assert_eq!(
            [red(0, 0), red(4, 0), red(0, 3), red(4, 3)],
            [20, 60, 140, 180]
        );
        assert_eq!([red(1, 0), red(2, 0), red(3, 0)], [40, 40, 40]);
        assert_eq!([red(1, 1), red(3, 2)], [100, 100]);
    }

    #[test]
    fn nine_grid_tiles_the_middle() {
        // Middle columns alternate 10 and 30 between 1 px margins.
        let mut a = Canvas::new(4, 1);
        for (x, r) in [(0, 5), (1, 10), (2, 30), (3, 50)] {
            a.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(r, 0, 0));
        }
        let mut c = Canvas::new(7, 1);
        c.blit_nine(
            &a,
            Rect::new(0, 0, 4, 1),
            Rect::new(0, 0, 7, 1),
            Insets::new(1, 0, 1, 0),
            Fill::Tile,
            false,
            1,
        );
        let row: Vec<u32> = (0..7).map(|x| (c.get(x, 0) >> 16) & 0xFF).collect();
        assert_eq!(row, vec![5, 10, 30, 10, 30, 10, 50]);
    }

    #[test]
    fn a_nine_grid_tiled_across_stretches_down() {
        // Middle columns alternate 10 and 30, middle rows 1 then 2
        // (green), between 1 px margins of 5 and 50.
        let mut a = Canvas::new(4, 4);
        for (x, r) in [(0, 5), (1, 10), (2, 30), (3, 50)] {
            for (y, g) in [(0, 0), (1, 1), (2, 2), (3, 3)] {
                a.fill_rect(Rect::new(x, y, 1, 1), Color::rgb(r, g, 0));
            }
        }
        let mut c = Canvas::new(7, 6);
        c.blit_nine(
            &a,
            Rect::new(0, 0, 4, 4),
            Rect::new(0, 0, 7, 6),
            Insets::new(1, 1, 1, 1),
            Fill::TileAcross,
            false,
            1,
        );
        let row: Vec<u32> = (0..7).map(|x| (c.get(x, 2) >> 16) & 0xFF).collect();
        assert_eq!(row, vec![5, 10, 30, 10, 30, 10, 50], "tiled across");
        let col: Vec<u32> = (0..6).map(|y| (c.get(2, y) >> 8) & 0xFF).collect();
        assert_eq!(col, vec![0, 1, 1, 2, 2, 3], "stretched down");
        for y in 0..6 {
            for x in 0..7 {
                let (sx, sy) = nine_source(
                    Rect::new(0, 0, 4, 4),
                    Rect::new(0, 0, 7, 6),
                    Insets::new(1, 1, 1, 1),
                    Fill::TileAcross,
                    false,
                    1,
                    x,
                    y,
                )
                .expect("inside");
                assert_eq!(
                    c.get(x, y),
                    a.get(sx, sy),
                    "nine_source agrees at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn a_centred_stretch_puts_an_edge_where_it_scales_to() {
        // Two columns, 10 then 90, stretched to five: the edge scales to
        // 2.5, so the third pixel's centre is already past it.
        let mut a = Canvas::new(2, 1);
        a.fill_rect(Rect::new(0, 0, 1, 1), Color::rgb(10, 0, 0));
        a.fill_rect(Rect::new(1, 0, 1, 1), Color::rgb(90, 0, 0));
        let row = |fill| {
            let mut c = Canvas::new(5, 1);
            c.blit_nine(
                &a,
                Rect::new(0, 0, 2, 1),
                Rect::new(0, 0, 5, 1),
                Insets::ZERO,
                fill,
                false,
                1,
            );
            (0..5)
                .map(|x| (c.get(x, 0) >> 16) & 0xFF)
                .collect::<Vec<u32>>()
        };
        assert_eq!(row(Fill::StretchCentred), vec![10, 10, 90, 90, 90]);
        assert_eq!(row(Fill::Stretch), vec![10, 10, 10, 90, 90]);
    }

    #[test]
    fn a_clipped_nine_grid_equals_the_full_one_inside_the_clip() {
        // A 9x9 atlas, one colour per pixel, tiled or stretched into a frame
        // running past the canvas on the left and bottom.
        let mut a = Canvas::new(9, 9);
        for y in 0..9 {
            for x in 0..9 {
                a.fill_rect(
                    Rect::new(x, y, 1, 1),
                    Color::rgb(x as u8 * 25, y as u8 * 25, 7),
                );
            }
        }
        let to = Rect::new(-3, 2, 40, 31);
        for (fill, scale) in [Fill::Tile, Fill::Stretch]
            .into_iter()
            .flat_map(|f| [(f, 1), (f, 2)])
        {
            let draw = |c: &mut Canvas| {
                c.blit_nine(
                    &a,
                    Rect::new(0, 0, 9, 9),
                    to,
                    Insets::new(2, 3, 2, 3),
                    fill,
                    false,
                    1,
                )
            };
            let mut full = Canvas::with_scale(30, 30, scale);
            draw(&mut full);
            for clip in [
                Rect::new(0, 0, 30, 30),
                Rect::new(5, 7, 11, 2),
                Rect::new(0, 28, 30, 5),
                Rect::new(1, 3, 1, 1),
                Rect::new(40, 40, 5, 5),
            ] {
                let mut part = Canvas::with_scale(30, 30, scale);
                part.with_clip(clip, draw);
                for y in 0..30 {
                    for x in 0..30 {
                        let want = if clip.contains(x, y) {
                            full.get(x, y)
                        } else {
                            0
                        };
                        assert_eq!(
                            part.get(x, y),
                            want,
                            "({x}, {y}) at {scale}x, {fill:?}, {clip:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nine_grid_border_only_leaves_the_centre_alone() {
        let a = nine_atlas();
        let mut c = Canvas::new(5, 5);
        c.blit_nine(
            &a,
            Rect::new(0, 0, 3, 3),
            Rect::new(0, 0, 5, 5),
            Insets::new(1, 1, 1, 1),
            Fill::Stretch,
            true,
            1,
        );
        assert_eq!(c.get(2, 2), 0);
        assert_ne!(c.get(0, 2), 0);
    }

    #[test]
    fn nine_grid_shrinks_margins_into_a_small_rect() {
        // Margins of 2 + 2 into 2 px: one column of each side's corner.
        let mut a = Canvas::new(5, 1);
        for x in 0..5 {
            a.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
        }
        let mut c = Canvas::new(2, 1);
        c.blit_nine(
            &a,
            Rect::new(0, 0, 5, 1),
            Rect::new(0, 0, 2, 1),
            Insets::new(2, 0, 2, 0),
            Fill::Stretch,
            false,
            1,
        );
        assert_eq!(
            [(c.get(0, 0) >> 16) & 0xFF, (c.get(1, 0) >> 16) & 0xFF],
            [10, 40]
        );
        c.blit_nine(
            &a,
            Rect::new(0, 0, 5, 1),
            Rect::new(0, 0, 0, 0),
            Insets::new(2, 0, 2, 0),
            Fill::Stretch,
            false,
            1,
        );
    }

    #[test]
    fn transparent_atlas_pixels_stay_transparent() {
        let mut a = Canvas::new(3, 1);
        a.fill_rect(Rect::new(1, 0, 2, 1), Color::rgb(0, 0, 255));
        let mut c = Canvas::new(6, 1);
        c.blit_nine(
            &a,
            Rect::new(0, 0, 3, 1),
            Rect::new(0, 0, 6, 1),
            Insets::new(1, 0, 1, 0),
            Fill::Stretch,
            false,
            1,
        );
        assert_eq!(c.get(0, 0), 0);
        assert_eq!(c.get(1, 0), 0xFF00_00FF);
    }

    #[test]
    fn row_and_column_lists_repeat_their_last_entry() {
        let mut c = Canvas::new(3, 4);
        c.fill_rows(
            Rect::new(0, 0, 3, 4),
            &[Color::rgb(1, 0, 0), Color::rgb(2, 0, 0)],
        );
        let red = |c: &Canvas, x, y| (c.get(x, y) >> 16) & 0xFF;
        assert_eq!([red(&c, 1, 0), red(&c, 1, 1), red(&c, 1, 3)], [1, 2, 2]);
        c.fill_cols(Rect::new(0, 0, 3, 4), &[Color::rgb(7, 0, 0)]);
        assert_eq!(red(&c, 2, 3), 7);
        c.fill_rows(Rect::new(0, 0, 3, 4), &[]);
        assert_eq!(red(&c, 2, 3), 7);
    }

    #[test]
    fn a_nine_axis_magnified_k_times_doubles_the_scale_1_axis() {
        // Every fill, source length, margin pair and target length at k times
        // magnification: pixel d shows what pixel d / k shows at scale 1.
        for fill in [Fill::Stretch, Fill::Tile, Fill::StretchCentred] {
            for src_len in 1..8 {
                for a in 0..=src_len {
                    for b in 0..=src_len - a {
                        for len in 1..20 {
                            for k in 2..=3 {
                                for d in 0..len * k {
                                    let want = nine_axis(d / k, len, 3, src_len, a, b, fill, 1).0;
                                    let got = nine_axis(
                                        d,
                                        len * k,
                                        3 * k,
                                        src_len * k,
                                        a * k,
                                        b * k,
                                        fill,
                                        k,
                                    )
                                    .0
                                    .div_euclid(k);
                                    assert_eq!(
                                        got, want,
                                        "{fill:?} k {k}, source {src_len} ({a}, {b}), \
                                         length {len}, pixel {d}: source {want}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn margins_larger_than_the_frame_never_read_outside_it() {
        let mut atlas = Canvas::new(1, 4);
        atlas.fill_rect(Rect::new(0, 0, 1, 2), Color::rgb(20, 0, 0));
        atlas.fill_rect(Rect::new(0, 2, 1, 2), Color::rgb(30, 0, 0));

        let mut c = Canvas::new(1, 3);
        c.blit_nine(
            &atlas,
            Rect::new(0, 2, 1, 2),
            Rect::new(0, 0, 1, 3),
            Insets::new(0, 3, 0, 3),
            Fill::Stretch,
            false,
            1,
        );

        let red = |y| (c.get(0, y) >> 16) & 0xFF;
        assert_eq!(red(0), 30, "target pixel 0 is from second frame");
        assert_eq!(red(1), 30, "target pixel 1 is from second frame");
        assert_eq!(red(2), 30, "target pixel 2 is from second frame");
    }

    #[test]
    fn blit_nine_draws_what_nine_source_gives_each_pixel() {
        // A 7x6 atlas of opaque, half and clear pixels over a patterned canvas:
        // blit_nine must equal composing the per-pixel `nine_source` build.
        let mut atlas = Canvas::new(7, 6);
        for y in 0..6 {
            for x in 0..7 {
                let i = (y * 7 + x) as u32;
                let a = [0xFF, 0x80, 0xFF, 0x00][(i % 4) as usize];
                let c = |n: u32| (n * 37 + i * 11) % (a + 1);
                atlas.put_raw(x, y, (a << 24) | (c(1) << 16) | (c(2) << 8) | c(3));
            }
        }
        let backdrop = |scale| {
            let mut c = Canvas::with_scale(12, 10, scale);
            for y in 0..10 {
                for x in 0..12 {
                    c.put_raw(x, y, 0xFF00_0000 | (x as u32 * 20) << 8 | (y as u32 * 25));
                }
            }
            c
        };
        let froms = [
            Rect::new(0, 0, 7, 6),
            Rect::new(2, 1, 4, 5),
            Rect::new(1, 2, 1, 1),
        ];
        let margins = [
            Insets::ZERO,
            Insets::new(1, 1, 1, 1),
            Insets::new(2, 1, 3, 2),
            Insets::new(4, 4, 4, 4),
        ];
        let tos = [
            Rect::new(0, 0, 12, 10),
            Rect::new(2, 3, 5, 4),
            Rect::new(-3, -2, 9, 7),
            Rect::new(8, 6, 9, 8),
            Rect::new(1, 1, 2, 1),
            Rect::new(-2, 4, 20, 3),
        ];
        for fill in [Fill::Stretch, Fill::Tile, Fill::StretchCentred] {
            for border_only in [false, true] {
                for k in 1..=3 {
                    for scale in [1, 2] {
                        for from in froms {
                            for m in margins {
                                for to in tos {
                                    let (from_k, m_k) = (from.scaled(k), m.scaled(k));
                                    let mut got = backdrop(scale);
                                    got.blit_nine(&atlas, from_k, to, m_k, fill, border_only, k);
                                    let mut out = Canvas::new(12, 10);
                                    for y in 0..10 {
                                        for x in 0..12 {
                                            if let Some((sx, sy)) = nine_source(
                                                from_k,
                                                to,
                                                m_k,
                                                fill,
                                                border_only,
                                                k,
                                                x,
                                                y,
                                            ) {
                                                out.put_raw(x, y, atlas.get(sx, sy));
                                            }
                                        }
                                    }
                                    let mut want = backdrop(scale);
                                    want.blit_over(0, 0, &out);
                                    assert_eq!(
                                        got.pixels, want.pixels,
                                        "{fill:?}, border only {border_only}, k {k}, \
                                         scale {scale}, {from:?} {m:?} into {to:?}: \
                                         the per-pixel nine_source result"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_transposed_picture_lies_on_its_side_its_first_row_down_the_inner_column() {
        let (red, green, blue) = (
            Color::rgb(255, 0, 0),
            Color::rgb(0, 255, 0),
            Color::rgb(0, 0, 255),
        );
        let (a, b, c) = (red.argb(), green.argb(), blue.argb());
        // A 3x2 picture: row 0 is a, a, b; row 1 is c, c, c.
        let draw = |p: &mut Canvas| {
            p.fill_rect(Rect::new(0, 0, 3, 1), red);
            p.put(2, 0, green);
            p.fill_rect(Rect::new(0, 1, 3, 1), blue);
        };
        let mut right = Canvas::new(4, 5);
        right.with_transposed(Rect::new(1, 1, 2, 3), false, draw);
        assert_eq!(
            [right.get(1, 1), right.get(1, 2), right.get(1, 3)],
            [a, a, b],
            "row 0 down r's first column"
        );
        assert_eq!(
            [right.get(2, 1), right.get(2, 2), right.get(2, 3)],
            [c, c, c]
        );
        assert_eq!(
            (right.get(0, 1), right.get(1, 4)),
            (0, 0),
            "nothing outside r"
        );
        let mut left = Canvas::new(4, 5);
        left.with_transposed(Rect::new(1, 1, 2, 3), true, draw);
        assert_eq!(
            [left.get(2, 1), left.get(2, 2), left.get(2, 3)],
            [a, a, b],
            "mirrored, row 0 down r's last column"
        );
        assert_eq!([left.get(1, 1), left.get(1, 2), left.get(1, 3)], [c, c, c]);
        // At scale 2 each logical pixel turns as a whole 2x2 block.
        let mut two = Canvas::with_scale(4, 5, 2);
        two.with_transposed(Rect::new(1, 1, 2, 3), false, draw);
        assert_eq!(two.get(1, 3), b);
        assert_eq!(two.pixels[7 * 8 + 3], b, "the block's far device pixel");
        // It lies over what is there: a clear pixel of the picture leaves
        // the canvas's own.
        let grey = Color::rgb(9, 9, 9);
        let mut under = Canvas::new(4, 5);
        under.fill_rect(Rect::new(0, 0, 4, 5), grey);
        under.with_transposed(Rect::new(1, 1, 2, 3), false, |p| p.put(0, 0, red));
        assert_eq!((under.get(1, 1), under.get(2, 2)), (a, grey.argb()));
    }

    #[test]
    fn a_transposed_picture_blends_clips_and_mirrors_at_scale_2() {
        let (red, green, blue) = (
            Color::rgb(255, 0, 0),
            Color::rgb(0, 255, 0),
            Color::rgb(0, 0, 255),
        );
        let grey = Color::rgb(9, 9, 9);
        let mut under = Canvas::new(4, 5);
        under.fill_rect(Rect::new(0, 0, 4, 5), grey);
        under.with_transposed(Rect::new(1, 1, 2, 3), false, |p| {
            p.put_raw(1, 0, 0x8080_0000)
        });
        assert_eq!(under.get(1, 2), 0xFF84_0404, "half red over grey");
        // A rect running off the canvas's foot, under a clip cutting its
        // second column: only the part on the canvas and in the clip.
        let mut cut = Canvas::new(4, 5);
        cut.with_clip(Rect::new(0, 0, 2, 5), |c| {
            c.with_transposed(Rect::new(1, 3, 2, 3), false, |p| {
                p.fill_rect(Rect::new(0, 0, 3, 2), red)
            })
        });
        assert_eq!([cut.get(1, 3), cut.get(1, 4)], [red.argb(); 2]);
        assert_eq!([cut.get(2, 3), cut.get(2, 4)], [0, 0], "past the clip");
        let draw = |p: &mut Canvas| {
            p.fill_rect(Rect::new(0, 0, 3, 1), red);
            p.put(2, 0, green);
            p.fill_rect(Rect::new(0, 1, 3, 1), blue);
        };
        let mut two = Canvas::with_scale(4, 5, 2);
        two.with_transposed(Rect::new(1, 1, 2, 3), true, draw);
        assert_eq!(
            [two.get(2, 1), two.get(2, 2), two.get(2, 3)],
            [red.argb(), red.argb(), green.argb()]
        );
        assert_eq!([two.get(1, 1), two.get(1, 3)], [blue.argb(), blue.argb()]);
        assert_eq!(
            two.pixels[7 * 8 + 5],
            green.argb(),
            "the block's far device pixel"
        );
    }
}
