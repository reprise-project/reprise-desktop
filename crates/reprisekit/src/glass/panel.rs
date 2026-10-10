//! Reprise Glass's drawn panels (Start, Alt+Tab, task preview popup): [`draw`] paints a [`Panel`], [`part`] makes it a
//! nine-grid part. Painted at `d` pixels per design pixel, geometry evaluated at each pixel centre ([`centre`]).

use super::Settings;
use super::art;
use super::tint::{self, Recolour, Surface};
use crate::backdrop::corner_radius;
use crate::raster::{Canvas, Insets};
use crate::shape::{Px, centre, pack, premul, ringed, rounded, smooth};
use crate::style::{Layout, PartImage, State};
use crate::theme::Color;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub corners: Corners,
    /// How much of the window frame's outline and of its light line
    /// shows: at 1 each is the frame's own, below it is laid at that share
    /// over the plain glass.
    pub lines: (f64, f64),
    /// How many rows under the light line the caption's sheen fades out
    /// over, from full on the first; 0 for none.
    pub sheen_rows: i32,
    pub ends: Option<Ends>,
    /// How much alpha (in levels) the glass gains over the plain glass on
    /// its first row, falling evenly to none on its last.
    pub deepen: f64,
    /// The share of what the glass lets through that a layer of black
    /// glass under it covers; 0 for none.
    pub density: f64,
    pub shadow: Option<Shadow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corners {
    /// Each of the top rows leaves `rows[y]` pixels clear at either side,
    /// and with `bottom` each of the bottom rows the same counting up;
    /// else the bottom corners are square.
    Cut {
        rows: &'static [i32],
        bottom: bool,
    },
    Round(f64),
}

/// The caption's sheen at both ends of a panel: full at the ends, fading
/// towards the middle along a slant. On row `y` it is at half strength
/// `reach - slope·y` px in from either end, fading from full to none over
/// `width` px. Down the panel it eases out between rows `fade.0` and
/// `fade.1`; the rows below carry none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ends {
    pub reach: f64,
    pub slope: f64,
    pub width: f64,
    pub fade: (f64, f64),
}

/// The alpha (in levels) a panel's glass reaches at most where the sheen or
/// extra alpha lies over the plain glass, so it stays translucent however
/// dense the setting; denser plain glass keeps its own alpha.
pub(crate) const DENSE_CEILING: f64 = 240.0;

/// A soft black shadow: the rect it falls from grown by `grow` and moved by
/// `offset`, blurred by a gaussian of `sigma` px, `alpha` (0..255) where
/// whole. Corners are rounded over `radius` px (grown by `grow`), square at
/// 0. Its canvas reaches `margin` past the rect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub margin: Insets,
    pub alpha: f64,
    pub sigma: f64,
    pub offset: (f64, f64),
    pub grow: f64,
    pub radius: f64,
}

/// A light rounded box laid over a panel's cell or the Start panel's white
/// pane: a 1 px `outline`, a 1 px `line` inside it if it has one, and a
/// fill, the line and fill each from `.0` to `.1` over the rows `fade`
/// gives, corners rounded over `radius` px. Colours are plain RGB with
/// alpha, the same in every glass colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightBox {
    pub outline: (Color, u8),
    pub line: Option<((Color, u8), (Color, u8))>,
    pub fill: ((Color, u8), (Color, u8)),
    pub radius: f64,
    pub fade: Fade,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fade {
    Box,
    Ring,
}

impl Shadow {
    pub(super) fn at(&self, d: i32) -> Shadow {
        let k = f64::from(d);
        Shadow {
            margin: self.margin.scaled(d),
            alpha: self.alpha,
            sigma: self.sigma * k,
            offset: (self.offset.0 * k, self.offset.1 * k),
            grow: self.grow * k,
            radius: self.radius * k,
        }
    }
}

impl Ends {
    /// The sheen's strength (0..1) `d` design px in from its end on row
    /// `y`; both may be fractional, for a pixel of a part painted at a finer
    /// scale.
    pub(super) fn at_point(&self, d: f64, y: f64) -> f64 {
        let half = self.reach - self.slope * y;
        let (from, to) = self.fade;
        let down = 1.0 - smooth((y - from) / (to - from));
        smooth((half - d) / self.width + 0.5) * down
    }
}

fn on_cut(rows: &[i32], bottom: bool, x: i32, y: i32, w: i32, h: i32) -> bool {
    let cut = |row: i32| rows.get(row as usize).copied().unwrap_or(0);
    let cut = if bottom {
        cut(y).max(cut(h - 1 - y))
    } else {
        cut(y)
    };
    (0..h).contains(&y) && x >= cut && x < w - cut
}

/// How much of pixel (`x`, `y`) of a `w` x `h` panel shaped by `corners`
/// lies on the panel, on the panel inside its outline, and on its glass
/// inside the light line, each 0..1. Cut corners cover whole pixels: a
/// pixel is on the outline where a pixel beside it is off the panel, and
/// on the light line where one beside it is on the outline.
pub(super) fn rings(corners: Corners, x: i32, y: i32, w: i32, h: i32) -> [f64; 3] {
    match corners {
        Corners::Cut { rows, bottom } => {
            let n = rows.len() as i32 + 2;
            let below = if bottom { h - n } else { h - 2 };
            if x >= 2 && x < w - 2 && y >= n && y < below {
                return [1.0; 3];
            }
            let beside = [(-1, 0), (1, 0), (0, -1), (0, 1)];
            let on = |x, y| on_cut(rows, bottom, x, y, w, h);
            let outline =
                |x: i32, y: i32| on(x, y) && beside.iter().any(|&(dx, dy)| !on(x + dx, y + dy));
            if !on(x, y) {
                [0.0; 3]
            } else if outline(x, y) {
                [1.0, 0.0, 0.0]
            } else if beside.iter().any(|&(dx, dy)| outline(x + dx, y + dy)) {
                [1.0, 1.0, 0.0]
            } else {
                [1.0; 3]
            }
        }
        Corners::Round(r) => [0, 1, 2].map(|i| {
            let i = f64::from(i);
            rounded(
                f64::from(x) - i,
                f64::from(y) - i,
                f64::from(w) - 2.0 * i,
                f64::from(h) - 2.0 * i,
                r - i,
            )
        }),
    }
}

/// [`rings`] at pixel (`x`, `y`) of a `w` x `h` (design) panel painted at `d`
/// pixels per design pixel: at 1 the panel's own rings. Finer, the outline
/// and light line are `d` px wide and every corner is round (a cut corner
/// the curve its table steps, [`corner_radius`]; square at the bottom
/// without `bottom`), each ring `d` px tighter than the one round it.
pub(super) fn rings_at(corners: Corners, x: i32, y: i32, w: i32, h: i32, d: i32) -> [f64; 3] {
    if d == 1 {
        return rings(corners, x, y, w, h);
    }
    let k = f64::from(d);
    let (r, square_bottom) = match corners {
        Corners::Cut { rows, bottom } => (f64::from(corner_radius(rows)), !bottom),
        Corners::Round(r) => (r, false),
    };
    [0, 1, 2].map(|i| {
        let inset = f64::from(i) * k;
        let (x, y) = (f64::from(x) - inset, f64::from(y) - inset);
        let (w, h) = (
            f64::from(w) * k - 2.0 * inset,
            f64::from(h) * k - 2.0 * inset,
        );
        let r = (r - f64::from(i)) * k;
        if !square_bottom {
            rounded(x, y, w, h, r)
        } else if y + 1.0 > h {
            0.0
        } else {
            rounded(x, y, w, h + 2.0 * r, r)
        }
    })
}

#[cfg(test)]
pub(crate) fn on_glass(p: &Panel, x: i32, y: i32, w: i32, h: i32) -> bool {
    rings(p.corners, x, y, w, h) == [1.0; 3]
}

#[cfg(test)]
pub(crate) fn every_setting() -> Vec<Settings> {
    use super::{Material, SWATCHES};
    SWATCHES
        .iter()
        .flat_map(|sw| {
            Material::ALL.into_iter().flat_map(move |material| {
                (0..=100).step_by(5).map(move |intensity| Settings {
                    color: sw.color,
                    material,
                    intensity,
                    ..Settings::default()
                })
            })
        })
        .collect()
}

/// How far `p`'s furthest channel lies outside 0..=its alpha, before
/// [`pack`] clamps it: 0 for a pixel that keeps its colour.
#[cfg(test)]
pub(crate) fn clip((a, p): (f64, [f64; 3])) -> f64 {
    p.iter().fold(0.0, |w: f64, &v| w.max(v - a).max(-v))
}

/// The share (0..1) of Sky's sheen colour a glass keeps:
/// `adds`, the colour it adds over its plain glass, must be `sky_adds`,
/// what Sky's adds where nothing caps it, scaled alike on every channel
/// (both from pixels rounded to whole levels).
#[cfg(test)]
pub(crate) fn sheen_share(what: &str, sky_adds: [f64; 3], adds: [f64; 3]) -> f64 {
    let j = (0..3)
        .max_by(|&a, &b| sky_adds[a].total_cmp(&sky_adds[b]))
        .unwrap();
    if sky_adds[j] < 8.0 {
        for i in 0..3 {
            assert!(
                adds[i].abs() <= sky_adds[i].abs() + 2.0,
                "{what} channel {i}: adds {}, Sky's {}",
                adds[i],
                sky_adds[i]
            );
        }
        return 1.0;
    }
    let k = adds[j] / sky_adds[j];
    assert!(
        (-0.02..=1.02).contains(&k),
        "{what}: the sheen's colour at {k:.2} of Sky's"
    );
    for i in 0..3 {
        assert!(
            (adds[i] - k * sky_adds[i]).abs() <= 2.0,
            "{what} channel {i}: adds {}, {k:.2} of Sky's {}",
            adds[i],
            sky_adds[i]
        );
    }
    k
}

/// A frame surface's glass for one setting: the plain glass and what the
/// caption's sheen adds over it, worked out once, and the share of what it
/// lets through a layer of black glass under it covers. A panel's glass
/// is the focused window's.
pub(super) struct Glass {
    pub(super) plain: (f64, [f64; 3]),
    sheen: (f64, [f64; 3]),
    density: f64,
}

impl Glass {
    /// `surface`'s glass for `s`: [`Surface::Active`] or
    /// [`Surface::Inactive`], each with its own sheen ([`art::SHEEN`]).
    pub(super) fn new(s: &Settings, surface: Surface, density: f64) -> Glass {
        let k = usize::from(surface == Surface::Inactive);
        let (sa, sp) = tint::split(art::SHEEN[k]);
        let (pa, pp) = tint::split(surface.plain_px());
        Glass {
            plain: tint::plain(s, surface),
            sheen: (sa - pa, std::array::from_fn(|i| sp[i] - pp[i])),
            density,
        }
    }

    /// The glass, translucent, with `sheen` (0..1, or more where stronger
    /// than the caption's) of the caption's sheen and `deepen` levels more
    /// alpha, capped at [`DENSE_CEILING`], under the black layer: alpha and
    /// premultiplied colour, unrounded. The sheen's colour is added whole
    /// where it fits inside the alpha, else scaled down alike on every
    /// channel, so a sheen over light glass keeps its hue instead of
    /// clipping toward white.
    pub(super) fn at(&self, sheen: f64, deepen: f64) -> (f64, [f64; 3]) {
        let ((a, p), (sa, sp)) = (self.plain, self.sheen);
        let added = (sheen * sa + deepen).min((DENSE_CEILING - a).max(0.0));
        let alpha = (a + added).clamp(0.0, 255.0);
        let alpha = alpha + (255.0 - alpha) * self.density;
        let k = (0..3)
            .filter(|&i| sheen * sp[i] > 0.0)
            .map(|i| (alpha - p[i]) / (sheen * sp[i]))
            .fold(1.0, f64::min)
            .max(0.0);
        (alpha, std::array::from_fn(|i| p[i] + k * sheen * sp[i]))
    }

    pub(super) fn px(&self, sheen: f64, deepen: f64) -> u32 {
        pack(self.at(sheen, deepen))
    }
}

impl Panel {
    pub fn glass(&self, s: &Settings, sheen: f64) -> u32 {
        Glass::new(s, Surface::Active, self.density).px(sheen, 0.0)
    }

    fn sheen(&self, x: f64, y: f64, w: i32) -> f64 {
        let down = if self.sheen_rows > 0 {
            1.0 - smooth((y - 2.0) / f64::from(self.sheen_rows))
        } else {
            0.0
        };
        let far = f64::from(w - 1) - x;
        let ends = self
            .ends
            .map_or(0.0, |e| e.at_point(x, y).max(e.at_point(far, y)));
        down.max(ends)
    }

    fn deepen(&self, y: f64, h: i32) -> f64 {
        if self.deepen == 0.0 {
            return 0.0;
        }
        let t = (y - 2.0) / f64::from((h - 5).max(1));
        self.deepen * (1.0 - t.clamp(0.0, 1.0))
    }
}

/// `p` for `s`, the panel `w` x `h` design px painted at `d` pixels per
/// design pixel, on a canvas reaching its shadow's margin past it: the
/// shadow where the panel is not, then the outline, the light line and
/// the glass ([`rings_at`]). With transparency off the panel is made
/// opaque over the colour the window frame's is; the shadow stays as it
/// is.
pub fn draw(p: &Panel, s: &Settings, w: i32, h: i32, d: i32) -> Canvas {
    let m = p.shadow.map_or(Insets::ZERO, |sh| sh.margin).scaled(d);
    let mut c = match p.shadow {
        Some(sh) => shadow(&sh.at(d), w * d, h * d),
        None => Canvas::new((w * d) as u32, (h * d) as u32),
    };
    let re = Recolour::new(s, Surface::Active);
    let glass = Glass::new(s, Surface::Active, p.density);
    let translucent = Recolour::new(
        &Settings {
            transparency: true,
            ..*s
        },
        Surface::Active,
    );
    let line = |px: u32, k: f64| {
        let (la, lp) = tint::split(translucent.px(px));
        let (pa, pp) = glass.plain;
        let mix = |l: f64, g: f64| (k * l + (1.0 - k) * g).round() as u32;
        re.finish(
            mix(la, pa) << 24
                | mix(lp[0], pp[0]) << 16
                | mix(lp[1], pp[1]) << 8
                | mix(lp[2], pp[2]),
        )
    };
    let (outline, light) = (
        line(art::LINES[0][0], p.lines.0),
        line(art::LINES[0][1], p.lines.1),
    );
    for y in 0..h * d {
        let fy = centre(y, d);
        let deepen = p.deepen(fy, h);
        for x in 0..w * d {
            let glass = || re.finish(glass.px(p.sheen(centre(x, d), fy, w), deepen));
            let (cx, cy) = (m.left + x, m.top + y);
            let [o, l, g] = rings_at(p.corners, x, y, w, h, d);
            let whole = |k: f64| k == 0.0 || k == 1.0;
            let px = match (o, l, g) {
                _ if o == 0.0 => continue,
                _ if !(whole(o) && whole(l) && whole(g)) => {
                    let layers = [
                        (c.get(cx, cy), 1.0 - o),
                        (outline, o - l),
                        (light, l - g),
                        (glass(), g),
                    ];
                    let ch = |sh: u32| {
                        let v: f64 = layers
                            .iter()
                            .map(|&(px, k)| f64::from((px >> sh) & 0xFF) * k)
                            .sum();
                        (v.round() as u32).min(255) << sh
                    };
                    ch(24) | ch(16) | ch(8) | ch(0)
                }
                _ if l == 0.0 => outline,
                _ if g == 0.0 => light,
                _ => glass(),
            };
            c.put_raw(cx, cy, px);
        }
    }
    c
}

/// `p` for `s` as a part painted at `d`: drawn `w` x `h` design px and
/// nine-gridded with `fixed` px of the panel fixed at each side (its
/// shadow's margin on top), so the panel's middle row and column must be
/// the same across. A rect narrower than both ends together shares them,
/// each keeping its own pixels ([`PartImage::split_ends`]).
pub fn part(p: &Panel, s: &Settings, w: i32, h: i32, fixed: Insets, d: i32) -> PartImage {
    let m = p.shadow.map_or(Insets::ZERO, |sh| sh.margin);
    PartImage::new(draw(p, s, w, h, d), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(
            m.left + fixed.left,
            m.top + fixed.top,
            m.right + fixed.right,
            m.bottom + fixed.bottom,
        ))
        .at_scale(d)
        .split_ends()
}

fn phi(z: f64) -> f64 {
    // erf by Abramowitz and Stegun 7.1.26, good to 1.5e-7.
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    0.5 * (1.0 + erf.copysign(z))
}

/// `sh` falling from a `w` x `h` rect, on a canvas reaching its margin
/// past the rect: black, its alpha the share of the blurred, grown and
/// moved rect over each pixel.
pub fn shadow(sh: &Shadow, w: i32, h: i32) -> Canvas {
    if sh.radius > 0.0 {
        return rounded_shadow(sh, w, h);
    }
    let m = sh.margin;
    let (cw, ch) = (w + m.left + m.right, h + m.top + m.bottom);
    let along = |n: i32, lo: f64, hi: f64| -> Vec<f64> {
        (0..n)
            .map(|i| {
                let c = f64::from(i) + 0.5;
                phi((c - lo) / sh.sigma) - phi((c - hi) / sh.sigma)
            })
            .collect()
    };
    let (dx, dy) = sh.offset;
    let left = f64::from(m.left) - sh.grow + dx;
    let top = f64::from(m.top) - sh.grow + dy;
    let fx = along(cw, left, left + f64::from(w) + 2.0 * sh.grow);
    let fy = along(ch, top, top + f64::from(h) + 2.0 * sh.grow);
    let mut c = Canvas::new(cw as u32, ch as u32);
    for (y, ky) in fy.iter().enumerate() {
        for (x, kx) in fx.iter().enumerate() {
            let a = (sh.alpha * kx * ky).round().clamp(0.0, 255.0) as u32;
            c.put_raw(x as i32, y as i32, a << 24);
        }
    }
    c
}

/// [`shadow`] from a rect with rounded corners: a rounded rect's share of
/// a span along a row is no product of the two axes' shares, so its
/// coverage of each pixel ([`rounded`]) is blurred by a sampled gaussian,
/// across and then down.
fn rounded_shadow(sh: &Shadow, w: i32, h: i32) -> Canvas {
    let m = sh.margin;
    let (cw, ch) = (w + m.left + m.right, h + m.top + m.bottom);
    let (dx, dy) = sh.offset;
    let (x0, y0) = (
        f64::from(m.left) - sh.grow + dx,
        f64::from(m.top) - sh.grow + dy,
    );
    let (rw, rh) = (f64::from(w) + 2.0 * sh.grow, f64::from(h) + 2.0 * sh.grow);
    let cov: Vec<f64> = (0..ch)
        .flat_map(|y| (0..cw).map(move |x| (x, y)))
        .map(|(x, y)| {
            rounded(
                f64::from(x) - x0,
                f64::from(y) - y0,
                rw,
                rh,
                sh.radius + sh.grow,
            )
        })
        .collect();
    let reach = (3.0 * sh.sigma).ceil() as i32;
    let kernel: Vec<f64> = (-reach..=reach)
        .map(|d| (-f64::from(d * d) / (2.0 * sh.sigma * sh.sigma)).exp())
        .collect();
    let total: f64 = kernel.iter().sum();
    let blur = |src: &[f64], step: (i32, i32)| -> Vec<f64> {
        (0..ch)
            .flat_map(|y| (0..cw).map(move |x| (x, y)))
            .map(|(x, y)| {
                (-reach..=reach)
                    .map(|d| {
                        let (sx, sy) = (x + d * step.0, y + d * step.1);
                        let v = if (0..cw).contains(&sx) && (0..ch).contains(&sy) {
                            src[(sy * cw + sx) as usize]
                        } else {
                            0.0
                        };
                        v * kernel[(d + reach) as usize]
                    })
                    .sum::<f64>()
                    / total
            })
            .collect()
    };
    let blurred = blur(&blur(&cov, (1, 0)), (0, 1));
    let mut c = Canvas::new(cw as u32, ch as u32);
    for (i, k) in blurred.iter().enumerate() {
        let a = (sh.alpha * k).round().clamp(0.0, 255.0) as u32;
        c.put_raw(i as i32 % cw, i as i32 / cw, a << 24);
    }
    c
}

impl LightBox {
    fn faded(&self, pair: ((Color, u8), (Color, u8)), inset: i32, y: f64, h: i32) -> Px {
        let inset = match self.fade {
            Fade::Box => 0,
            Fade::Ring => inset,
        };
        let t = ((y - f64::from(inset)) / f64::from((h - 1 - 2 * inset).max(1))).clamp(0.0, 1.0);
        let ((a, p), (b, q)) = (premul(pair.0.0, pair.0.1), premul(pair.1.0, pair.1.1));
        (
            a + (b - a) * t,
            std::array::from_fn(|i| p[i] + (q[i] - p[i]) * t),
        )
    }

    pub(super) fn line_at(&self, y: f64, h: i32) -> Option<Px> {
        self.line.map(|pair| self.faded(pair, 1, y, h))
    }

    fn fill_at(&self, y: f64, h: i32) -> Px {
        let inset = if self.line.is_some() { 2 } else { 1 };
        self.faded(self.fill, inset, y, h)
    }
}

pub fn light_box(b: &LightBox, w: i32, h: i32, d: i32) -> Canvas {
    let n = if b.line.is_some() { 2 } else { 1 };
    let pixels = ringed(w, h, b.radius, n, d, |_, y, i| match (i, b.line_at(y, h)) {
        (0, _) => premul(b.outline.0, b.outline.1),
        (1, Some(line)) => line,
        _ => b.fill_at(y, h),
    });
    let cw = w * d;
    let mut c = Canvas::new(cw as u32, (h * d) as u32);
    for (i, &p) in pixels.iter().enumerate() {
        c.put_raw(i as i32 % cw, i as i32 / cw, pack(p));
    }
    c
}

/// Checks `two`, a drawing at scale 2, against `one`, the same at scale 1:
/// every pixel lies within the channel range of the 2 x 2 pixels of `one`
/// nearest its middle, a level either side for rounding. Where the 4 x 4
/// pixels round those are one colour, it is exactly that colour. The outer
/// half of the edge pixels is not checked. `skip(x, y)` leaves a pixel out
/// (a round corner, checked on its own; a pixel [`beside`] a layer).
#[cfg(test)]
pub(crate) fn assert_twice_over(
    what: &str,
    one: &Canvas,
    two: &Canvas,
    skip: impl Fn(i32, i32) -> bool,
) {
    let (w, h) = (one.width as i32, one.height as i32);
    assert_eq!(
        (two.width as i32, two.height as i32),
        (2 * w, 2 * h),
        "{what}: twice the size"
    );
    let near = |v: i32, len: i32| {
        let c = centre(v, 2);
        [c.floor() as i32, c.ceil() as i32].map(|n| n.clamp(0, len - 1))
    };
    let wide = |v: i32, len: i32| {
        let [a, b] = near(v, len);
        [a - 1, a, b, b + 1].map(|n| n.clamp(0, len - 1))
    };
    let inside = |v: i32, len: i32| (0.0..=f64::from(len - 1)).contains(&centre(v, 2));
    for y in (0..2 * h).filter(|&y| inside(y, h)) {
        for x in (0..2 * w).filter(|&x| inside(x, w) && !skip(x, y)) {
            let got = two.get(x, y);
            let around: Vec<u32> = near(y, h)
                .into_iter()
                .flat_map(|sy| near(x, w).map(|sx| one.get(sx, sy)))
                .collect();
            let flat = wide(y, h)
                .into_iter()
                .all(|sy| wide(x, w).iter().all(|&sx| one.get(sx, sy) == around[0]));
            if flat {
                assert_eq!(got, around[0], "{what} ({x}, {y}): {got:08X}");
                continue;
            }
            for sh in [24, 16, 8, 0] {
                let ch = |p: u32| ((p >> sh) & 0xFF) as i32;
                let lo = around.iter().map(|&p| ch(p)).min().unwrap();
                let hi = around.iter().map(|&p| ch(p)).max().unwrap();
                assert!(
                    (lo - 1..=hi + 1).contains(&ch(got)),
                    "{what} ({x}, {y}): {got:08X} channel {sh} outside {lo}..={hi} of {around:08X?}"
                );
            }
        }
    }
}

/// Whether (`x`, `y`) lies on the ring of pixels just outside `rect`
/// (both at scale 2): what lies under a layer shows there, and the
/// scale-1 pixels round it are partly the layer's, hiding it.
#[cfg(test)]
pub(crate) fn beside(rect: crate::raster::Rect, x: i32, y: i32) -> bool {
    let across = (rect.x - 1..=rect.right()).contains(&x);
    let down = (rect.y - 1..=rect.bottom()).contains(&y);
    (across && (y == rect.y - 1 || y == rect.bottom()))
        || (down && (x == rect.x - 1 || x == rect.right()))
}

/// Whether (`x`, `y`) lies in a corner's box of `rect` (both at scale 2)
/// that a corner reaching `r` design px in shapes at scale 2 or at scale
/// 1 beside it: `2 * ceil(r) + 1` px square.
#[cfg(test)]
pub(crate) fn in_corner(rect: crate::raster::Rect, r: f64, x: i32, y: i32) -> bool {
    let r = 2 * r.ceil() as i32 + 1;
    let (dx, dy) = (x - rect.x, y - rect.y);
    let along = |v: i32, len: i32| (0..r).contains(&v) || (len - r..len).contains(&v);
    along(dx, rect.width) && along(dy, rect.height)
}

/// Checks `c`'s top-left corner, rounded over `r` px from pixel (`x0`, `y0`):
/// clear where a pixel's middle lies `r + 0.71` or further from the
/// curve's centre, alpha at least `whole` where `r - 0.71` or nearer, and
/// partly covered along the curve in at least `levels` alphas.
#[cfg(test)]
pub(crate) fn assert_round_corner(
    what: &str,
    c: &Canvas,
    (x0, y0): (i32, i32),
    r: f64,
    whole: u32,
    levels: usize,
) {
    let mut partial = std::collections::BTreeSet::new();
    let n = r.ceil() as i32;
    for y in 0..n {
        for x in 0..n {
            let px = c.get(x0 + x, y0 + y);
            let dist = (r - f64::from(x) - 0.5).hypot(r - f64::from(y) - 0.5);
            if dist >= r + 0.71 {
                assert_eq!(px, 0, "{what} ({x}, {y}): {px:08X} outside the curve");
            } else if dist <= r - 0.71 {
                assert!(
                    px >> 24 >= whole,
                    "{what} ({x}, {y}): {px:08X} inside the curve"
                );
            } else {
                partial.insert(px >> 24);
            }
        }
    }
    assert!(
        partial.len() >= levels,
        "{what}: the curve's edge in many levels, not a step: {partial:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Material, SWATCHES, preview, startpanel, switcher};
    use crate::raster::Rect;
    use crate::style::Part;

    fn fnv(img: &PartImage) -> u64 {
        img.atlas
            .pixels
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    const CRISP: [Part; 30] = [
        Part::StartMagnifier,
        Part::StartClear,
        Part::StartTriangleLeft,
        Part::StartTriangleRight,
        Part::StartRowArrow,
        Part::StartPanel,
        Part::StartPane,
        Part::StartJumpPane,
        Part::StartHotLeft,
        Part::StartNewPin,
        Part::StartHotAll,
        Part::StartHotSplit,
        Part::StartHotJump,
        Part::StartLitResult,
        Part::StartHotRight,
        Part::StartShutdown,
        Part::StartShutdownArrow,
        Part::StartShutdownLight,
        Part::StartShutdownArrowLight,
        Part::StartSearchField,
        Part::StartPictureFrame,
        Part::SwitcherFrame,
        Part::SwitcherIconFrame,
        Part::SwitcherSelect,
        Part::SwitcherHot,
        Part::SwitcherShadow,
        Part::PreviewFrame,
        Part::PreviewThumb,
        Part::PreviewClose,
        Part::Tooltip,
    ];
    const KEPT: [Part; 4] = [
        Part::StartRuleLeft,
        Part::StartRuleRight,
        Part::StartJumpRule,
        Part::StartSearchRule,
    ];

    #[test]
    fn the_style_at_scale_2_paints_start_and_the_popups_at_2_and_their_rules_at_1() {
        let style = crate::glass::style(&Settings::default());
        let two = style.at_scale(2);
        for part in CRISP {
            assert_eq!(two.part(part).unwrap().scale, 2, "{part:?}");
            assert_eq!(style.part(part).unwrap().scale, 1, "{part:?}");
        }
        for part in KEPT {
            assert_eq!(two.part(part).unwrap().scale, 1, "{part:?} stays");
        }
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass).scaled(2);
        let scaled = theme.look.style().unwrap();
        for part in [Part::StartPanel, Part::SwitcherFrame, Part::PreviewFrame] {
            assert_eq!(scaled.part(part).unwrap().scale, 2, "a scaled theme's");
        }
    }

    fn bare_at_2(p: Panel, w: i32, h: i32) -> Canvas {
        let bare = Panel { shadow: None, ..p };
        draw(&bare, &Settings::default(), w, h, 2)
    }

    #[test]
    fn a_panels_corners_at_scale_2_are_round_and_antialiased() {
        // Alt+Tab's cut corners: radius 6 px (12 at scale 2), all four; the
        // popup's are rounded over 7 px; the Start panel's top ones over 6 and
        // its bottom ones square. Clear outside the curve, outline whole
        // inside it, partly covered along it.
        let plain = tint::plain(&Settings::default(), Surface::Active).0 as u32;
        for (name, p, w, h, r) in [
            ("Alt+Tab", switcher::PANEL, 448, 138, 12.0),
            ("preview", preview::PANEL, 235, 166, 14.0),
            ("Start", startpanel::PANEL, 60, 80, 12.0),
        ] {
            let c = bare_at_2(p, w, h);
            let (cw, ch) = (2 * w, 2 * h);
            assert_round_corner(name, &c, (0, 0), r, plain, 8);
            let n = r as i32;
            for y in 0..n {
                for x in 0..n {
                    let px = c.get(x, y);
                    assert_eq!(
                        c.get(cw - 1 - x, y),
                        px,
                        "{name} ({x}, {y}): left and right"
                    );
                    let (bl, br) = (c.get(x, ch - 1 - y), c.get(cw - 1 - x, ch - 1 - y));
                    assert_eq!(bl, br, "{name} ({x}, {y}): the bottom corners alike");
                    if name == "Start" {
                        assert_eq!(
                            bl,
                            c.get(n + 4, ch - 1 - x.min(y)),
                            "{name} ({x}, {y}): the bottom is square"
                        );
                    } else {
                        assert_eq!(bl == 0, px == 0, "{name} ({x}, {y}): the bottom's curve");
                    }
                }
            }
        }
    }

    #[test]
    fn a_popup_at_scale_2_is_its_scale_1_design_twice_over() {
        let s = Settings::default();
        for (name, p, w, h, r) in [
            ("Alt+Tab", switcher::PANEL, 448, 138, 7.0),
            ("preview", preview::PANEL, 235, 166, 7.0),
            ("Start", startpanel::PANEL, 60, 80, 7.0),
        ] {
            let m = p.shadow.map_or(Insets::ZERO, |sh| sh.margin);
            let panel = Rect::new(2 * m.left, 2 * m.top, 2 * w, 2 * h);
            assert_twice_over(
                name,
                &draw(&p, &s, w, h, 1),
                &draw(&p, &s, w, h, 2),
                |x, y| in_corner(panel, r, x, y) || beside(panel, x, y),
            );
        }
    }

    #[test]
    fn with_transparency_off_a_panel_at_scale_2_is_opaque_but_its_corners_edge() {
        let opaque = Settings {
            transparency: false,
            ..Settings::default()
        };
        for (name, p, w, h) in popups()
            .into_iter()
            .chain([("Start", startpanel::PANEL, 60, 80)])
        {
            let m = p.shadow.map_or(Insets::ZERO, |sh| sh.margin).scaled(2);
            let (c, glass) = (
                draw(&p, &opaque, w, h, 2),
                draw(&p, &Settings::default(), w, h, 2),
            );
            for y in 0..c.height as i32 {
                for x in 0..c.width as i32 {
                    let (px, py) = (x - m.left, y - m.top);
                    let on = rings_at(p.corners, px, py, w, h, 2)[0];
                    let got = c.get(x, y);
                    let what = format!("{name} ({px}, {py}): {got:08X}");
                    if on == 1.0 {
                        assert_eq!(got >> 24, 0xFF, "{what}: opaque");
                    } else if on == 0.0 {
                        assert_eq!(got, glass.get(x, y), "{what}: the shadow");
                    } else {
                        let a = got >> 24;
                        assert!(
                            [16, 8, 0].iter().all(|sh| (got >> sh) & 0xFF <= a),
                            "{what}: premultiplied"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_popups_parts_at_scale_2_keep_their_pixels_at_sky() {
        let two = crate::glass::style(&Settings::default()).at_scale(2);
        let got = [
            Part::SwitcherFrame,
            Part::SwitcherSelect,
            Part::SwitcherHot,
            Part::SwitcherShadow,
            Part::PreviewFrame,
            Part::PreviewThumb,
        ]
        .map(|part| fnv(two.part(part).unwrap()));
        assert_eq!(
            got,
            [
                0xED7EDB9C5DBF7AF2,
                0xB23D752F0CB35FA5,
                0x5742AB2DE12BD665,
                0x63EF886C669EFE25,
                0xFF2F93EC657C38E5,
                0xFCB51D7DA210E3E5,
            ],
            "Alt+Tab's frame, selection, hot box and picture shadow, the popup's frame and lit cell: {got:#018X?}"
        );
    }

    fn popups() -> [(&'static str, Panel, i32, i32); 2] {
        [
            ("Alt+Tab", switcher::PANEL, 448, 138),
            ("preview", preview::PANEL, 235, 166),
        ]
    }

    fn rubies() -> Vec<Settings> {
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
        Material::ALL
            .into_iter()
            .flat_map(|material| {
                [42, 75].map(|intensity| Settings {
                    color: ruby,
                    material,
                    intensity,
                    ..Settings::default()
                })
            })
            .collect()
    }

    fn over_grey((a, p): (f64, [f64; 3])) -> [f64; 3] {
        p.map(|v| v + (255.0 - a) * 128.0 / 255.0)
    }

    #[test]
    fn the_glass_moves_as_the_frames_plain_glass_does() {
        let sky = Settings::default();
        let plain = |s: &Settings| over_grey(tint::plain(s, Surface::Active));
        for (name, p, w, h) in popups() {
            let m = p.shadow.unwrap().margin;
            let from = draw(&p, &sky, w, h, 1);
            let points = [
                (w / 2, 3),
                (4, 6),
                (w - 5, 6),
                (w / 2, h / 2),
                (4, h - 8),
                (w - 5, h - 8),
            ];
            for s in rubies() {
                let to = draw(&p, &s, w, h, 1);
                let what = format!("{name}, {:?} at {}", s.material, s.intensity);
                let moved: [f64; 3] = std::array::from_fn(|i| plain(&s)[i] - plain(&sky)[i]);
                let (a_sky, p_sky) = tint::plain(&sky, Surface::Active);
                let (a_s, p_s) = tint::plain(&s, Surface::Active);
                for (x, y) in points {
                    let px = |c: &Canvas| tint::split(c.get(m.left + x, m.top + y));
                    let ((a0, c0), (a1, c1)) = (px(&from), px(&to));
                    let added = a0 - a_sky;
                    assert!(
                        a_sky + added < DENSE_CEILING,
                        "{name} ({x}, {y}): Sky is not capped"
                    );
                    if a_s + added > DENSE_CEILING {
                        // The added alpha stops at the ceiling; the colour
                        // the sheen adds is Sky's, scaled down alike on
                        // every channel where one would pass the alpha.
                        let want = a_s.max(DENSE_CEILING).round();
                        assert!(
                            (a1 - want).abs() <= 1.0,
                            "{what} ({x}, {y}): alpha {a1} stops at {want}"
                        );
                        sheen_share(
                            &format!("{what} ({x}, {y})"),
                            std::array::from_fn(|i| c0[i] - p_sky[i]),
                            std::array::from_fn(|i| c1[i] - p_s[i]),
                        );
                        continue;
                    }
                    let (g0, g1) = (over_grey(px(&from)), over_grey(px(&to)));
                    for i in 0..3 {
                        assert!(
                            (g1[i] - g0[i] - moved[i]).abs() <= 1.5,
                            "{what} ({x}, {y}) channel {i}: moves {} over grey, as the plain glass's {}",
                            g1[i] - g0[i],
                            moved[i]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn no_setting_clips_a_panels_colour() {
        // The Start panel's, Alt+Tab's and the preview's glass in every
        // setting, before it is rounded: no channel outside 0 to the alpha
        // for `pack` to clamp to white or flatten.
        let panels = [("Start", startpanel::PANEL, 411, 480)]
            .into_iter()
            .chain(popups());
        for (name, p, w, h) in panels {
            let mut inputs: Vec<(f64, f64)> = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .filter(|&(x, y)| on_glass(&p, x, y, w, h))
                .map(|(x, y)| {
                    (
                        p.sheen(f64::from(x), f64::from(y), w),
                        p.deepen(f64::from(y), h),
                    )
                })
                .collect();
            inputs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
            inputs.dedup();
            let mut clipped = Vec::new();
            for s in every_setting() {
                let glass = Glass::new(&s, Surface::Active, p.density);
                let worst = inputs
                    .iter()
                    .map(|&(sheen, deepen)| clip(glass.at(sheen, deepen)))
                    .fold(0.0, f64::max);
                if worst > 1e-9 {
                    clipped.push(format!(
                        "{:?} {:?} {}: {worst:.1}",
                        s.color, s.material, s.intensity
                    ));
                }
            }
            assert!(clipped.is_empty(), "{name}: {clipped:#?}");
        }
    }

    #[test]
    fn with_transparency_on_no_setting_makes_the_glass_opaque() {
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
        for (name, p, w, h) in popups() {
            let m = p.shadow.unwrap().margin;
            for color in [Settings::default().color, ruby] {
                for material in Material::ALL {
                    for intensity in 0..=100 {
                        let s = Settings {
                            color,
                            material,
                            intensity,
                            ..Settings::default()
                        };
                        let c = draw(&p, &s, w, h, 1);
                        let densest = (0..h)
                            .flat_map(|y| (0..w).map(move |x| (x, y)))
                            .filter(|&(x, y)| on_glass(&p, x, y, w, h))
                            .map(|(x, y)| c.get(m.left + x, m.top + y) >> 24)
                            .max()
                            .unwrap();
                        assert!(
                            densest < 0xFF,
                            "{name}, {color:?} {material:?} at {intensity}: densest glass {densest}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn with_transparency_off_the_panel_is_opaque_and_its_shadow_is_not() {
        let opaque = Settings {
            transparency: false,
            ..Settings::default()
        };
        for (name, p, w, h) in popups() {
            let m = p.shadow.unwrap().margin;
            let (c, glass) = (
                draw(&p, &opaque, w, h, 1),
                draw(&p, &Settings::default(), w, h, 1),
            );
            for y in 0..c.height as i32 {
                for x in 0..c.width as i32 {
                    let (px, py) = (x - m.left, y - m.top);
                    let on = rings(p.corners, px, py, w, h)[0];
                    if on == 1.0 {
                        assert_eq!(c.get(x, y) >> 24, 0xFF, "{name}: ({px}, {py}) is opaque");
                    } else if on == 0.0 {
                        assert_eq!(
                            c.get(x, y),
                            glass.get(x, y),
                            "{name}: ({px}, {py}) the shadow"
                        );
                        assert!(c.get(x, y) >> 24 < 0xFF, "{name}: ({px}, {py}) the shadow");
                    }
                }
            }
        }
        let start = draw(&startpanel::PANEL, &opaque, 30, 60, 1);
        assert!(
            start.pixels.iter().all(|&px| px == 0 || px >> 24 == 0xFF),
            "the Start panel is opaque but for its cut corners"
        );
    }

    #[test]
    fn the_glass_carries_no_streaks() {
        // The widest Alt+Tab panel: along every row of its glass it only
        // fades from each end to the middle, with no line across it. The
        // backdrop's streaks are the only ones, and `streaks = false`
        // leaves the panel clear of them.
        let s = Settings::default();
        let p = switcher::PANEL;
        let (w, h) = (1024, 231);
        let m = p.shadow.unwrap().margin;
        let c = draw(&p, &s, w, h, 1);
        let at = |x: i32, y: i32| tint::split(c.get(m.left + x, m.top + y));
        let glass = |x: i32, y: i32| rings(p.corners, x, y, w, h) == [1.0; 3];
        for y in 2..h - 2 {
            for x in (3..w / 2).filter(|&x| glass(x - 1, y)) {
                let (a, b) = (at(x - 1, y), at(x, y));
                assert!(
                    b.0 <= a.0 && (0..3).all(|i| b.1[i] <= a.1[i]),
                    "row {y}: the glass only fades towards the middle at x {x}"
                );
                let (a, b) = (at(w - x, y), at(w - 1 - x, y));
                assert!(
                    b.0 <= a.0 && (0..3).all(|i| b.1[i] <= a.1[i]),
                    "row {y}: and from the right end at x {}",
                    w - 1 - x
                );
            }
        }
        let p = preview::PANEL;
        let (w, h) = (669, 166);
        let m = p.shadow.unwrap().margin;
        let c = draw(&p, &s, w, h, 1);
        for y in 8..h - 8 {
            let row: Vec<u32> = (2..w - 2).map(|x| c.get(m.left + x, m.top + y)).collect();
            assert!(
                row.iter().all(|&px| px == row[0]),
                "the popup's row {y} is flat"
            );
        }
    }
}
