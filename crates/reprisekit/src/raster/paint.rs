use super::geometry::nine_axis;
use super::{Canvas, Fill, Insets, Rect};
use crate::theme::{Color, ColorScheme};

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

impl Canvas {
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
    /// border `m` (see [`nine_source`](super::nine_source)), composed over what is there. Builds
    /// only the part of `to` inside the canvas and clip, and with `border_only`
    /// only the bands round the skipped centre.
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
        let rows: Vec<(i32, bool)> = (vis.y..vis.bottom())
            .map(|y| {
                let (s, mid) = nine_axis(
                    y - to.y,
                    to.height,
                    from.y,
                    from.height,
                    m.top,
                    m.bottom,
                    fill_y,
                    k,
                );
                (s.div_euclid(k), mid)
            })
            .collect();
        let (w, h) = (cols.len(), rows.len());
        let mut piece = |x0: usize, x1: usize, y0: usize, y1: usize| {
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            let mut out = Canvas::new((x1 - x0) as u32, (y1 - y0) as u32);
            for (row, &(sy, _)) in out.pixels.chunks_exact_mut(x1 - x0).zip(&rows[y0..y1]) {
                for (px, &(sx, _)) in row.iter_mut().zip(&cols[x0..x1]) {
                    *px = src.get(sx, sy);
                }
            }
            self.blit_over(vis.x + x0 as i32, vis.y + y0 as i32, &out);
        };
        // The middle rows and columns are one run each, so a skipped centre leaves four bands.
        let run = |v: &[(i32, bool)]| {
            let a = v.iter().position(|e| e.1)?;
            Some((a, a + v[a..].iter().take_while(|e| e.1).count()))
        };
        match (border_only, run(&cols), run(&rows)) {
            (true, Some((cx0, cx1)), Some((ry0, ry1))) => {
                piece(0, w, 0, ry0);
                piece(0, cx0, ry0, ry1);
                piece(cx1, w, ry0, ry1);
                piece(0, w, ry1, h);
            }
            _ => piece(0, w, 0, h),
        }
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

pub(super) fn in_rounded(r: Rect, radius: i32, x: i32, y: i32) -> bool {
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
