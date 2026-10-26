//! ARGB8888 CPU canvas and its fill, bevel and blit primitives.

use crate::textedit::{Preedit, TextSpot};
use crate::theme::Color;

mod blit;
mod encode;
mod geometry;
mod paint;
#[cfg(test)]
mod tests;

pub use encode::{to_png, write_preview};
pub use geometry::{Fill, Insets, Rect, fit_size, nine_source};
pub use paint::{EdgeStyle, rounded_border, rounded_fill};

/// A label tip a form drew no pixels for yet: its whole text and the pointer, in device px.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportedTip {
    pub text: String,
    pub at: (i32, i32),
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
    /// A form's label tip waiting for the host to draw it over everything ([`Canvas::report_tip`]).
    tip: Option<Box<ReportedTip>>,
    /// Rows at the top drawn clear for the frame's glass ([`Canvas::report_glass_top`]).
    glass_top: i32,
    /// What the frame changed ([`Canvas::report_damage`]); `None` is the whole canvas.
    damage: Option<Vec<Rect>>,
    /// The frame answers a caret blink and nothing else ([`Canvas::blink_only`]).
    blink_only: bool,
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
    // The blend's own result in these cases, without the four divisions.
    if sa == 0xFF || d == 0 {
        return s;
    }
    if s == 0 {
        return d;
    }
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
            tip: None,
            glass_top: 0,
            damage: None,
            blink_only: false,
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
            tip: None,
            glass_top: 0,
            damage: None,
            blink_only: false,
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

    /// The coverage of what `draw` inks into this clear canvas in the colour it is handed, one
    /// byte per device pixel: drawn white, read back from the red channel.
    pub fn ink_coverage(mut self, draw: impl FnOnce(&mut Canvas, Color)) -> Vec<u8> {
        draw(&mut self, Color::rgb(0xFF, 0xFF, 0xFF));
        self.pixels.iter().map(|&p| (p >> 16) as u8).collect()
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

    /// A form reports the tip of the ellipsized label the pointer rests on; the host draws it
    /// last, over the form's popups ([`crate::controls::form::draw_tip`]).
    pub fn report_tip(&mut self, tip: ReportedTip) {
        self.tip = Some(Box::new(tip));
    }

    pub fn take_tip(&mut self) -> Option<ReportedTip> {
        self.tip.take().map(|t| *t)
    }

    /// The top `rows` of a window's canvas are drawn clear for the frame's glass to show
    /// through ([`crate::look::client_glass`]); the host reads the deepest report of a frame.
    pub fn report_glass_top(&mut self, rows: i32) {
        self.glass_top = self.glass_top.max(rows);
    }

    pub fn take_glass_top(&mut self) -> i32 {
        std::mem::take(&mut self.glass_top)
    }

    /// An app that keeps its canvas between frames, or a control on a [`Canvas::blink_only`]
    /// frame, reports each logical-px rect it redrew; a frame with no report changed the whole
    /// canvas.
    pub fn report_damage(&mut self, r: Rect) {
        self.damage.get_or_insert_with(Vec::new).push(r);
    }

    /// `r` changed too: added to a partial damage report, while a whole-canvas one stays whole.
    pub fn widen_damage(&mut self, r: Rect) {
        if let Some(d) = self.damage.as_mut() {
            d.push(r);
        }
    }

    /// Whether the host draws this frame only to flip the caret: an edit whose caret is the one
    /// thing that changed reports its caret bar as the frame's damage.
    pub fn blink_only(&self) -> bool {
        self.blink_only
    }

    pub fn set_blink_only(&mut self, on: bool) {
        self.blink_only = on;
    }

    pub fn take_damage(&mut self) -> Option<Vec<Rect>> {
        self.damage.take()
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
}
