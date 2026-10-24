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
    /// The two share at least one pixel.
    pub fn intersects(self, o: Rect) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
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
    pub(super) fn axes(self) -> (Fill, Fill) {
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
pub(super) fn nine_axis(
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
