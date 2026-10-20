//! Reprise Glass's window frame, drawn from the glass settings: the caption
//! band (restored and maximized), the side borders and the bottom border, for
//! the focused window and for the others.
//!
//! The frame is one piece of [`tint::plain`] glass round the client, with the
//! caption's sheen at each end of the band and down the top of each side, a
//! dark outline and a light line inside it, corners cut by [`CORNER`] (square
//! on the Square frame), and the client's light and dark edges. Each line is a
//! layer over the glass (the light ones over the sheened glass, so they
//! brighten where the sheen does) that puts the line on [`art::LINES`] over
//! Sky's plain glass. The sheen is [`art::SHEEN`]'s, the same in every glass
//! colour, its alpha stopping at [`DENSE_CEILING`] ([`Glass`]). With
//! transparency off the frame is made opaque.
//!
//! Parts are painted at the draw scale and nine-gridded: the caption's ends
//! fixed and its middle stretched, each side stretched whole so its sheen ends
//! a quarter of the way down at any height, the bottom's corners fixed. At a
//! finer scale the frame is the scale-1 design that many times over: lines
//! that many pixels wide, sheens sampled at each pixel's own place, and cut
//! corners the round curve ([`corner_radius`]) scaled too, each ring covering
//! each pixel as far as it reaches. The title, its glow and the caption
//! buttons are laid over the band by the frame's host.
//!
//! [`DENSE_CEILING`]: super::panel::DENSE_CEILING

use std::ops::Range;
use std::sync::Mutex;

use super::panel::{self, Corners, Ends, Glass};
use super::tint::{self, Recolour, Surface};
use super::{CORNER, Frame, Settings, art};
use crate::backdrop::corner_radius;
use crate::raster::{Canvas, Fill, Insets, Rect};
use crate::shape::{self, Px, centre, mix, over, pack, smooth};
use crate::style::{Layout, PartImage, State};

/// The caption band's rows: outline, light line, 26 glass rows, the client's two edges.
pub const BAND: i32 = 30;

/// A maximized window's caption band: 20 glass rows over the client's two edges.
pub const MAX_BAND: i32 = 22;

/// The side and bottom borders' width: outline, light line, 4 px of glass, the client's two edges.
pub const SIDE: i32 = 8;

/// Each fixed end of the caption band, wide enough to hold its sheen whole.
pub const END: i32 = 240;

/// The caption's sheen at both ends: half strength `147 - 1.7·y` px in from
/// either end on row `y`, fading from full to none across 180 px. Both ends
/// alike, so a band narrower than both meets its halves without a seam.
const CAPTION_SHEEN: Ends = Ends {
    reach: 147.0,
    slope: 1.7,
    width: 180.0,
    fade: (BAND as f64, BAND as f64 + 1.0),
};

/// The first glass row under the light line catches 0.3 more at the end, none 64 px in.
const TOP_ROW_SHEEN: (f64, f64) = (0.3, 64.0);

/// A side border's sheen down its height, as shares of the caption's: `top`
/// from the band down to `from` of the height, then rising (or falling) as
/// the 1.5th power of the way to `end` at [`SIDE_EDGE`], where it stops.
struct SideSheen {
    top: f64,
    end: f64,
    from: f64,
}

/// The focused side sheen brightens to twice the caption's before it stops; an unfocused one's fades.
const SIDE_SHEEN: [SideSheen; 2] = [
    SideSheen {
        top: 0.94,
        end: 2.1,
        from: 0.09,
    },
    SideSheen {
        top: 0.885,
        end: 0.58,
        from: 0.14,
    },
];

/// Where a side's sheen stops, as a share of the side's height, in a hard edge.
const SIDE_EDGE: f64 = 0.245;

/// Under its edge the side's sheen leaves 0.08 of the caption's, easing out over a tenth of the height.
const SIDE_TAIL: (f64, f64) = (0.08, 0.1);

/// The side strip's design height; it stretches whole to the window's.
const SIDE_ROWS: i32 = 462;

impl SideSheen {
    fn at(&self, t: f64) -> f64 {
        if t < SIDE_EDGE {
            let u = ((t - self.from) / (SIDE_EDGE - self.from)).clamp(0.0, 1.0);
            self.top + (self.end - self.top) * u.powf(1.5)
        } else {
            let (tail, over) = SIDE_TAIL;
            tail * (1.0 - smooth((t - SIDE_EDGE) / over))
        }
    }
}

/// How much of the light line shows where it turns along a cut corner's diagonal, over the outline.
const TURN: f64 = 0.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ring {
    Off,
    Outline,
    Light,
    /// The light line where it turns along a cut corner's diagonal: [`TURN`] over the outline.
    Turn,
    /// Glass in the step of a cut corner's light line: half light line.
    Inner,
    Glass,
    ClientLight,
    ClientDark,
    /// Where the client's light and dark edges meet at a corner: half of each.
    ClientCorner,
}

/// Pixel (`x`, `y`) of a `w` x `h` frame whose corners `cut` cuts and whose
/// client starts `band` rows down: outline, light line, glass, and round the
/// client its dark edge with its light edge outside. Where the light line
/// touches a cut corner only across a diagonal it turns, partly outline, and
/// the glass in each step is half light line. The light edge leaves out its
/// corner pixels, the dark edge's corners are half light.
fn ring(cut: &'static [i32], band: i32, x: i32, y: i32, w: i32, h: i32) -> Ring {
    let corners = Corners::Cut {
        rows: cut,
        bottom: true,
    };
    let rings = |x, y| panel::rings(corners, x, y, w, h);
    match rings(x, y) {
        [0.0, ..] => return Ring::Off,
        [_, 0.0, _] => return Ring::Outline,
        [_, _, 0.0] => {
            let diagonal = [(-1, -1), (1, -1), (-1, 1), (1, 1)];
            let off = |(dx, dy)| rings(x + dx, y + dy)[0] == 0.0;
            // Only a cut leaves a diagonal off inside the frame's own edge.
            let inside = x > 0 && y > 0 && x < w - 1 && y < h - 1;
            return if inside && diagonal.into_iter().any(off) {
                Ring::Turn
            } else {
                Ring::Light
            };
        }
        _ => {}
    }
    let light = |(dx, dy)| matches!(rings(x + dx, y + dy), [_, l, g] if l == 1.0 && g == 0.0);
    let beside = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    if !cut.is_empty() && beside.into_iter().filter(|&d| light(d)).count() >= 2 {
        return Ring::Inner;
    }
    let out = |v: i32, lo: i32, hi: i32| {
        if v < lo {
            lo - v
        } else if v >= hi {
            v - hi + 1
        } else {
            0
        }
    };
    let (dx, dy) = (out(x, SIDE, w - SIDE), out(y, band, h - SIDE));
    match (dx.max(dy), dx == dy) {
        (0, _) => Ring::Off,
        (1, true) => Ring::ClientCorner,
        (1, false) => Ring::ClientDark,
        (2, false) => Ring::ClientLight,
        _ => Ring::Glass,
    }
}

/// The layer that, laid over `plain`, gives `line`. Its colour is left as it
/// works out (a level below 0 where the line is darker than a layer can be),
/// so over `plain` it lands on `line` exactly and over other glass it moves
/// by the share of the glass it lets through.
fn layer(line: u32, plain: u32) -> Px {
    let ((la, lp), (pa, pp)) = (tint::split(line), tint::split(plain));
    let a = ((la - pa) / (1.0 - pa / 255.0)).clamp(0.0, 255.0);
    let keep = 1.0 - a / 255.0;
    (a, std::array::from_fn(|i| lp[i] - pp[i] * keep))
}

/// The whole window frame the parts are cut from, in design pixels: `w` x `h`,
/// corners cut by `cut` (none on Square), client `band` rows down. Without
/// `client` it is a sheet: glass where the client and its edges would be.
#[derive(Clone, Copy)]
struct Whole {
    cut: &'static [i32],
    band: i32,
    w: i32,
    h: i32,
    client: bool,
}

impl Whole {
    /// The frame's [`ring`] at design pixel (`x`, `y`), glass over the client on a sheet.
    fn ring(&self, x: i32, y: i32) -> Ring {
        let r = ring(self.cut, self.band, x, y, self.w, self.h);
        if self.client {
            return r;
        }
        match r {
            Ring::ClientLight | Ring::ClientDark | Ring::ClientCorner => Ring::Glass,
            Ring::Off if self.inside(x, y) => Ring::Glass,
            r => r,
        }
    }

    fn inside(&self, x: i32, y: i32) -> bool {
        let corners = Corners::Cut {
            rows: self.cut,
            bottom: true,
        };
        panel::rings(corners, x, y, self.w, self.h)[0] > 0.0
    }
}

struct Paint {
    glass: Glass,
    /// The outline, the light line, the client's light and dark edges, each as a layer.
    layers: [Px; 4],
    finish: Recolour,
    side: &'static SideSheen,
}

impl Paint {
    fn new(s: &Settings, surface: Surface) -> Paint {
        let k = usize::from(surface == Surface::Inactive);
        Paint {
            glass: Glass::new(s, surface, 0.0),
            layers: art::LINES[k].map(|line| layer(line, art::PLAIN[k])),
            finish: Recolour::new(s, surface),
            side: &SIDE_SHEEN[k],
        }
    }

    /// A pixel that is `ring`, over glass carrying `sheen` of the caption's sheen.
    fn px(&self, ring: Ring, sheen: f64) -> u32 {
        let plain = self.glass.plain;
        let glass = || self.glass.at(sheen, 0.0);
        let [outline, light, client_light, client_dark] = self.layers;
        let px = match ring {
            Ring::Off => return 0,
            Ring::Outline => over(outline, plain),
            Ring::Light => over(light, glass()),
            Ring::Turn => mix(over(outline, plain), over(light, glass()), TURN),
            Ring::Inner => mix(glass(), over(light, glass()), 0.5),
            Ring::Glass => glass(),
            Ring::ClientLight => over(client_light, glass()),
            Ring::ClientDark => over(client_dark, plain),
            Ring::ClientCorner => mix(over(client_dark, plain), over(client_light, glass()), 0.5),
        };
        self.finish.finish(pack(px))
    }

    /// Pixel (`x`, `y`) of frame `f` painted at `d` pixels per design pixel,
    /// over glass carrying `sheen` of the caption's sheen. At `d` = 1 the
    /// frame's own [`ring`]. Finer, each line is `d` px wide and a cut corner
    /// is the curve of [`corner_radius`] times `d`, each ring laid as far as
    /// it covers the pixel, so the edge is antialiased.
    fn px_at(&self, f: Whole, d: i32, x: i32, y: i32, sheen: f64) -> u32 {
        if d == 1 {
            return self.px(f.ring(x, y), sheen);
        }
        let k = f64::from(d);
        let r = f64::from(corner_radius(f.cut)) * k;
        let (w, h) = (f64::from(f.w) * k, f64::from(f.h) * k);
        let on = |i: f64| {
            let inset = i * k;
            shape::rounded(
                f64::from(x) - inset,
                f64::from(y) - inset,
                w - 2.0 * inset,
                h - 2.0 * inset,
                (r - inset).max(0.0),
            )
        };
        let [o, l, g] = [on(0.0), on(1.0), on(2.0)];
        if o == 0.0 {
            return 0;
        }
        let plain = self.glass.plain;
        let glass = self.glass.at(sheen, 0.0);
        let [outline, light, client_light, client_dark] = self.layers;
        // Inside the light line: the square frame's rings, a design pixel each.
        let inner = match ring(&[], f.band, x / d, y / d, f.w, f.h) {
            _ if !f.client => glass,
            Ring::Off => (0.0, [0.0; 3]),
            Ring::ClientLight => over(client_light, glass),
            Ring::ClientDark => over(client_dark, plain),
            Ring::ClientCorner => mix(over(client_dark, plain), over(client_light, glass), 0.5),
            _ => glass,
        };
        let layers = [
            (over(outline, plain), o - l),
            (over(light, glass), l - g),
            (inner, g),
        ];
        // As if the frame covered it whole, opaque if transparency is off, then cut to its coverage.
        let whole: Px = (
            layers.iter().map(|(p, c)| p.0 * c).sum::<f64>() / o,
            std::array::from_fn(|i| layers.iter().map(|(p, c)| p.1[i] * c).sum::<f64>() / o),
        );
        let px = self.finish.finish(pack(whole));
        if o >= 1.0 {
            px
        } else {
            pack(shape::scale(shape::unpack(px), o))
        }
    }
}

fn cut(s: &Settings) -> &'static [i32] {
    match s.frame {
        Frame::Rounded => CORNER,
        Frame::Square => &[],
    }
}

/// The focused window's surface, then every other window's, with their frame states.
const SURFACES: [(Surface, State); 2] = [
    (Surface::Active, State::Normal),
    (Surface::Inactive, State::Inactive),
];

/// One frame per surface, Normal then Inactive, each `w` x `h` design pixels
/// painted at `d` per design pixel, stacked; pixel (`x`, `y`) counted at `d` from `px`.
fn frames(
    s: &Settings,
    (w, h): (i32, i32),
    d: i32,
    px: impl Fn(&Paint, i32, i32) -> u32,
) -> PartImage {
    let (w, h) = (w * d, h * d);
    let mut c = Canvas::new(w as u32, 2 * h as u32);
    for (i, (surface, _)) in SURFACES.into_iter().enumerate() {
        let p = Paint::new(s, surface);
        for y in 0..h {
            for x in 0..w {
                c.put_raw(x, i as i32 * h + y, px(&p, x, y));
            }
        }
    }
    PartImage::new(c, &SURFACES.map(|(_, st)| st), Layout::Vertical).at_scale(d)
}

/// The caption's sheen at (`x`, `y`) of a band `w` wide: the stronger of its
/// two ends', plus the extra row 2 catches.
fn caption_sheen(x: f64, y: f64, w: i32) -> f64 {
    let far = f64::from(w - 1) - x;
    let ends = CAPTION_SHEEN
        .at_point(x, y)
        .max(CAPTION_SHEEN.at_point(far, y));
    let top = if y.round() as i32 == 2 {
        let (more, over) = TOP_ROW_SHEEN;
        more * (1.0 - x.min(far) / over).max(0.0)
    } else {
        0.0
    };
    ends + top
}

/// What lies under the caption buttons (`super::buttons`): a caption band for
/// one setting, unrounded and not yet opaque.
pub(super) struct Under(Paint);

impl Under {
    pub(super) fn new(s: &Settings, surface: Surface) -> Under {
        Under(Paint::new(s, surface))
    }

    /// The band `d` design px in from its right end at row `y`, both anywhere
    /// between design pixels: its glass with the sheen from that end (row 2
    /// catching its extra), the light line on row 1. With `maximized`, a
    /// maximized band's plain glass.
    pub(super) fn at(&self, d: f64, y: f64, maximized: bool) -> Px {
        let p = &self.0;
        if maximized {
            return p.glass.at(0.0, 0.0);
        }
        let row = y.round() as i32;
        let mut sheen = CAPTION_SHEEN.at_point(d, y);
        if row == 2 {
            let (more, reach) = TOP_ROW_SHEEN;
            sheen += more * (1.0 - d / reach).max(0.0);
        }
        let glass = p.glass.at(sheen, 0.0);
        if row == 1 {
            over(p.layers[1], glass)
        } else {
            glass
        }
    }
}

/// The caption band, [`BAND`] rows, Normal then Inactive, painted at `d`:
/// drawn [`END`] px at each end and 2 in the middle. Ends stay fixed and the
/// plain middle stretches; in a rect narrower than both ends they share it.
pub fn caption(s: &Settings, d: i32) -> PartImage {
    let w = 2 * END + 2;
    let f = Whole {
        cut: cut(s),
        band: BAND,
        w,
        h: BAND + SIDE + 2,
        client: true,
    };
    frames(s, (w, BAND), d, |p, x, y| {
        let sheen = caption_sheen(centre(x, d), centre(y, d), w);
        p.px_at(f, d, x, y, sheen)
    })
    .sizing(Insets::new(END, 0, END, 0))
    .split_ends()
}

/// A maximized window's caption band, [`MAX_BAND`] rows, Normal then Inactive,
/// painted at `d`, stretched across.
pub fn max_caption(s: &Settings, d: i32) -> PartImage {
    frames(s, (2, MAX_BAND), d, |p, _, y| {
        let ring = match MAX_BAND - 1 - y / d {
            0 => Ring::ClientDark,
            1 => Ring::ClientLight,
            _ => Ring::Glass,
        };
        p.px(ring, 0.0)
    })
}

/// A side border, [`SIDE`] px wide, Normal then Inactive, on the `right` or
/// left, painted at `d`: [`SIDE_ROWS`] tall, stretched whole to the window's height.
fn side(s: &Settings, right: bool, d: i32) -> PartImage {
    let (w, h) = (2 * SIDE + 2, BAND + SIDE + 2);
    let x0 = if right { w - SIDE } else { 0 };
    frames(s, (SIDE, SIDE_ROWS), d, |p, x, y| {
        let t = (f64::from(y) + 0.5) / f64::from(SIDE_ROWS * d);
        p.px(ring(&[], BAND, x0 + x / d, BAND, w, h), p.side.at(t))
    })
    .fill(Fill::StretchCentred)
}

pub fn frame_left(s: &Settings, d: i32) -> PartImage {
    side(s, false, d)
}

pub fn frame_right(s: &Settings, d: i32) -> PartImage {
    side(s, true, d)
}

/// The window's bottom border, [`SIDE`] rows, Normal then Inactive, painted at
/// `d`: corners [`SIDE`] px each and fixed, the middle stretched.
pub fn frame_bottom(s: &Settings, d: i32) -> PartImage {
    let (w, h) = (2 * SIDE + 2, BAND + SIDE + 2);
    let f = Whole {
        cut: cut(s),
        band: BAND,
        w,
        h,
        client: true,
    };
    frames(s, (w, SIDE), d, |p, x, y| {
        p.px_at(f, d, x, y + (h - SIDE) * d, 0.0)
    })
    .sizing(Insets::new(SIDE, 0, SIDE, 0))
    .clip_ends()
}

/// A window's sheet, [`SHEET`] px square, Normal then Inactive, painted at
/// `d`: plain glass with outline and light line and cut corners, no caption,
/// sheen or client edges. Its [`SIDE`] px edges stay fixed, the middle
/// stretches. A peek shows each window it hides as one.
pub fn sheet(s: &Settings, d: i32) -> PartImage {
    let f = Whole {
        cut: cut(s),
        band: BAND,
        w: SHEET,
        h: SHEET,
        client: false,
    };
    frames(s, (SHEET, SHEET), d, |p, x, y| p.px_at(f, d, x, y, 0.0))
        .sizing(Insets::new(SIDE, SIDE, SIDE, SIDE))
}

pub const SHEET: i32 = 2 * SIDE + 1;

/// The client's light and dark edges inside the frame, in design px.
const CLIENT_EDGES: i32 = 2;

/// Rows of [`client_glass`], its outset row included: the caption's sheen carried down under
/// the client is gone by the last, which a deeper band stretches.
pub const CLIENT_ROWS: i32 = 110;

/// [`CAPTION_SHEEN`] carried on down under the client's top, no longer cut at the band's foot.
const CLIENT_SHEEN: Ends = Ends {
    fade: (
        (BAND - CLIENT_EDGES + CLIENT_ROWS) as f64,
        (BAND - CLIENT_EDGES + CLIENT_ROWS) as f64 + 1.0,
    ),
    ..CAPTION_SHEEN
};

/// The frame's glass under a restored window's top, Normal then Inactive, painted at `d`: the
/// caption's glass and the sheen at each end carried down past the band, [`CLIENT_ROWS`] tall,
/// laid [`CLIENT_EDGES`] past the left, top and right over the client's edges to give way to
/// the glass there. Like the caption, its ends stay fixed and its plain middle stretches.
pub fn client_glass(s: &Settings, d: i32) -> PartImage {
    let w = 2 * END + 2;
    let (left, top) = (SIDE - CLIENT_EDGES, BAND - CLIENT_EDGES);
    let far = w + 2 * left - 1;
    frames(s, (w, CLIENT_ROWS), d, |p, x, y| {
        let (x, y) = (
            centre(x, d) + f64::from(left),
            centre(y, d) + f64::from(top),
        );
        let sheen = CLIENT_SHEEN
            .at_point(x, y)
            .max(CLIENT_SHEEN.at_point(f64::from(far) - x, y));
        p.px(Ring::Glass, sheen)
    })
    .sizing(Insets::new(END, CLIENT_ROWS - 1, END, 0))
    .split_ends()
    .outset(client_outset())
}

/// A maximized window's glass under its top: [`max_caption`]'s plain glass, one design pixel
/// stretched, Normal then Inactive, painted at `d`.
pub fn max_client_glass(s: &Settings, d: i32) -> PartImage {
    frames(s, (1, 1), d, |p, _, _| p.px(Ring::Glass, 0.0)).outset(client_outset())
}

/// How far in from each side the glass under a restored window's top turns from the side
/// border's sheen to the caption's.
const EDGE_FADE: f64 = 24.0;

/// What draws, over [`client_glass`] in `band` (a restored window's top rows, device pixels at
/// `d` per design pixel, the client `client_h` device pixels tall), each side's
/// [`EDGE_FADE`] px and the client edges beside them: the side border's sheen on that row,
/// which runs down the window's height, turning into [`client_glass`]'s own. The last
/// [`EDGES_KEPT`] strips painted are kept, so a frame redrawn at its size only copies them.
pub fn client_glass_edges(s: &Settings) -> impl Fn(&mut Canvas, Rect, i32, State, i32) + use<> {
    let paints = SURFACES.map(|(surface, _)| Paint::new(s, surface));
    let kept: Mutex<Vec<(EdgesKey, Vec<u32>)>> = Mutex::default();
    move |c, band, client_h, state, d| {
        let inactive = state == State::Inactive;
        let key = (band.width, band.height, client_h, inactive, d);
        let (o, fade) = (CLIENT_EDGES * d, (EDGE_FADE * f64::from(d)) as i32);
        let left = -o..fade.min(band.width);
        let right = (band.width - fade).max(fade)..band.width + o;
        let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
        let at = match kept.iter().position(|(k, _)| *k == key) {
            Some(i) => i,
            None => {
                let p = &paints[usize::from(inactive)];
                let strip = edge_strip(p, band, client_h, d, (&left, &right));
                kept.push((key, strip));
                if kept.len() > EDGES_KEPT {
                    kept.remove(0);
                }
                kept.len() - 1
            }
        };
        let mut px = kept[at].1.iter();
        for row in -o..band.height {
            for col in left.clone().chain(right.clone()) {
                let p = *px.next().expect("a pixel per strip cell");
                c.put_raw(band.x + col, band.y + row, p);
            }
        }
    }
}

/// Strips [`client_glass_edges`] keeps: a few windows' sizes, focused and not.
const EDGES_KEPT: usize = 8;

/// A kept strip's band width and height, client height, inactive and scale.
type EdgesKey = (i32, i32, i32, bool, i32);

/// [`client_glass_edges`]' pixels row by row over the `left` then `right` columns.
fn edge_strip(
    p: &Paint,
    band: Rect,
    client_h: i32,
    d: i32,
    (left, right): (&Range<i32>, &Range<i32>),
) -> Vec<u32> {
    let fw = f64::from(band.width / d + 2 * SIDE);
    let o = CLIENT_EDGES * d;
    let mut out = Vec::new();
    for row in -o..band.height {
        let y = centre(row, d) + f64::from(BAND);
        let side = p
            .side
            .at((f64::from(row) + 0.5) / f64::from(client_h.max(1)));
        for col in left.clone().chain(right.clone()) {
            let x = centre(col, d) + f64::from(SIDE);
            let own = CLIENT_SHEEN
                .at_point(x, y)
                .max(CLIENT_SHEEN.at_point(fw - 1.0 - x, y));
            let inward = (x - f64::from(SIDE)).min(fw - 1.0 - f64::from(SIDE) - x);
            let w = match row < 0 {
                true => 0.0,
                false => 1.0 - smooth(inward / EDGE_FADE),
            };
            let sheen = own + (side - own) * w;
            out.push(p.px(Ring::Glass, sheen));
        }
    }
    out
}

fn client_outset() -> Insets {
    Insets::new(CLIENT_EDGES, CLIENT_EDGES, CLIENT_EDGES, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Chrome, FrameGeometry, FrameState, Glow};
    use crate::glass::panel::DENSE_CEILING;
    use crate::glass::{Material, Mode, SWATCHES};
    use crate::raster::Rect;
    use crate::style::Part;
    use crate::text::TextRenderer;

    fn parts(s: &Settings) -> [(&'static str, PartImage); 5] {
        parts_at(s, 1)
    }

    fn parts_at(s: &Settings, d: i32) -> [(&'static str, PartImage); 5] {
        [
            ("caption", caption(s, d)),
            ("max caption", max_caption(s, d)),
            ("left", frame_left(s, d)),
            ("right", frame_right(s, d)),
            ("bottom", frame_bottom(s, d)),
        ]
    }

    /// Sky and Ruby, Frosted, Smoked and Molten, at intensity 42, 75 and 100.
    fn settings() -> Vec<Settings> {
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
        [SWATCHES[0].color, ruby]
            .into_iter()
            .flat_map(|color| {
                Material::ALL.into_iter().flat_map(move |material| {
                    [42, 75, 100].map(|intensity| Settings {
                        color,
                        material,
                        intensity,
                        ..Settings::default()
                    })
                })
            })
            .collect()
    }

    fn over_grey((a, p): Px) -> [f64; 3] {
        p.map(|v| v + (255.0 - a) * 128.0 / 255.0)
    }

    fn at(img: &PartImage, state: State, x: i32, y: i32) -> u32 {
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    #[test]
    fn sky_draws_the_frames_own_lines_glass_and_sheen() {
        let s = Settings::default();
        let (cap, max, left, bottom) = (
            caption(&s, 1),
            max_caption(&s, 1),
            frame_left(&s, 1),
            frame_bottom(&s, 1),
        );
        for (k, (_, state)) in SURFACES.into_iter().enumerate() {
            let [outline, light, client_light, client_dark] = art::LINES[k];
            let plain = art::PLAIN[k];
            let mid = END + 1;
            assert_eq!(at(&cap, state, mid, 0), outline, "{state:?}: the outline");
            assert_eq!(at(&cap, state, mid, 1), light, "{state:?}: the light line");
            assert_eq!(
                at(&cap, state, mid, 10),
                plain,
                "{state:?}: the plain glass"
            );
            assert_eq!(
                at(&cap, state, mid, 28),
                client_light,
                "{state:?}: the client's light edge"
            );
            assert_eq!(
                at(&cap, state, mid, 29),
                client_dark,
                "{state:?}: the client's dark edge"
            );
            for (x, y) in [(10, 10), (2 * END - 9, 10), (10, 2)] {
                assert!(
                    at(&cap, state, x, y) >> 24 > plain >> 24,
                    "{state:?}: the sheen at ({x}, {y})"
                );
            }
            assert!(
                at(&left, state, 3, 20) >> 24 > plain >> 24,
                "{state:?}: the side's sheen near its top"
            );
            assert_eq!(
                at(&left, state, 3, 400),
                plain,
                "{state:?}: plain glass lower down"
            );
            assert_eq!(
                [0, 1, 6, 7].map(|x| at(&left, state, x, 400)),
                [outline, light, client_light, client_dark],
                "{state:?}: the side's lines"
            );
            assert_eq!(
                [0, 1, 3, 6, 7].map(|y| at(&bottom, state, SIDE + 1, y)),
                [client_dark, client_light, plain, light, outline],
                "{state:?}: the bottom's lines and glass"
            );
            assert_eq!(
                [10, MAX_BAND - 2, MAX_BAND - 1].map(|y| at(&max, state, 0, y)),
                [plain, client_light, client_dark],
                "{state:?}: the maximized band"
            );
        }
    }

    #[test]
    fn sky_sheen_fades_through_its_points() {
        // Both caption ends mid-fade on rows 15 and 20; the left side near its top and 110 rows down.
        let s = Settings::default();
        let (cap, left) = (caption(&s, 1), frame_left(&s, 1));
        let state = State::Normal;
        assert_eq!(
            at(&cap, state, 10, 10),
            art::SHEEN[0],
            "the sheen at the end"
        );
        for (name, got, want) in [
            ("caption (100, 15)", at(&cap, state, 100, 15), 0x6F3E_4C5C),
            ("caption (140, 20)", at(&cap, state, 140, 20), 0x521A_2A3C),
            (
                "caption (2 * END - 100, 15), the right end",
                at(&cap, state, 2 * END - 100, 15),
                0x6E3D_4C5B,
            ),
            ("side (3, 20)", at(&left, state, 3, 20), 0x8256_6471),
            ("side (3, 110)", at(&left, state, 3, 110), 0xD1BB_C4CA),
        ] {
            assert_eq!(got, want, "{name}: {got:08X}");
        }
    }

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

    #[test]
    fn the_frames_parts_keep_their_pixels_at_sky() {
        for (frame, want) in [
            (
                Frame::Rounded,
                [
                    0x5D93_CD68_1261_1D25_u64,
                    0x89CD_5ACB_EE44_F915,
                    0x68FD_F885_915E_6F91,
                    0x8ECF_C16E_A3F7_5BD1,
                    0x30F0_5207_0564_7CAD,
                ],
            ),
            (
                Frame::Square,
                [
                    0x9E15_8DAA_BDD7_EEE1,
                    0x89CD_5ACB_EE44_F915,
                    0x68FD_F885_915E_6F91,
                    0x8ECF_C16E_A3F7_5BD1,
                    0x39C7_C816_AFBB_4631,
                ],
            ),
        ] {
            let s = Settings {
                frame,
                ..Settings::default()
            };
            for ((name, img), want) in parts(&s).into_iter().zip(want) {
                assert_eq!(fnv(&img), want, "{frame:?} {name}: the atlas");
            }
        }
    }

    #[test]
    fn a_sheet_is_the_frames_glass_and_lines_with_nothing_over_it() {
        for frame in [Frame::Rounded, Frame::Square] {
            let s = Settings {
                frame,
                ..Settings::default()
            };
            let img = sheet(&s, 1);
            for (k, (_, state)) in SURFACES.into_iter().enumerate() {
                let [outline, light, ..] = art::LINES[k];
                let plain = art::PLAIN[k];
                let last = SHEET - 1;
                let what = format!("{frame:?} {state:?}");
                assert_eq!(
                    [0, 1, 2, SIDE - 2, SIDE].map(|y| at(&img, state, SIDE, y)),
                    [outline, light, plain, plain, plain],
                    "{what}: the outline, the light line, then plain glass down the top"
                );
                assert_eq!(
                    [last, last - 1, last - 2].map(|y| at(&img, state, SIDE, y)),
                    [outline, light, plain],
                    "{what}: the same up from the bottom"
                );
                assert_eq!(
                    [2, SIDE - 2, SIDE - 1].map(|x| at(&img, state, x, SIDE)),
                    [plain; 3],
                    "{what}: glass where a side's client edges lie"
                );
                let corner = at(&img, state, 0, 0);
                match frame {
                    Frame::Rounded => assert_eq!(corner, 0, "{what}: the corner is cut"),
                    Frame::Square => assert_eq!(corner, outline, "{what}: the corner is square"),
                }
            }
        }
    }

    #[test]
    fn the_sheet_has_soft_cut_corners_and_two_states_at_scale_2() {
        let img = sheet(&Settings::default(), 2);
        assert!(
            (0..2 * SIDE).any(|x| {
                let a = at(&img, State::Normal, x, x) >> 24;
                a != 0 && a != 0xFF
            }),
            "partial-alpha pixels on the cut corner"
        );
        let mid = 2 * SIDE;
        assert_ne!(
            at(&img, State::Normal, mid, mid),
            at(&img, State::Inactive, mid, mid),
            "focused and unfocused glass differ"
        );
        assert_eq!(
            img.sizing,
            Insets::new(SIDE, SIDE, SIDE, SIDE),
            "edges in design px"
        );
    }

    #[test]
    fn the_sheet_keeps_its_pixels_in_each_material_and_frame() {
        for (mode, material, frame, want) in [
            (
                Mode::Light,
                Material::Frosted,
                Frame::Rounded,
                [0xFB7E_AE68_1F97_FEAC_u64, 0x8376_DA57_7E89_2AED],
            ),
            (
                Mode::Light,
                Material::Smoked,
                Frame::Square,
                [0x9935_69D2_DAD4_3239, 0x97F8_8FAB_346A_9EA5],
            ),
            (
                Mode::Light,
                Material::Molten,
                Frame::Rounded,
                [0xE571_96E4_329A_3B23, 0xFE08_FBCA_0A2E_9155],
            ),
            (
                Mode::Dark,
                Material::Frosted,
                Frame::Rounded,
                [0xFB7E_AE68_1F97_FEAC, 0x8376_DA57_7E89_2AED],
            ),
        ] {
            let s = Settings {
                mode,
                material,
                frame,
                ..Settings::default()
            };
            let got = [1, 2].map(|d| fnv(&sheet(&s, d)));
            assert_eq!(got, want, "{mode:?} {material:?} {frame:?}: {got:X?}");
        }
    }

    #[test]
    fn client_glass_edges_drawn_from_a_kept_strip_match_a_fresh_paint() {
        let s = Settings::default();
        let kept = client_glass_edges(&s);
        let draw = |edges: &dyn Fn(&mut Canvas, Rect, i32, State, i32), x, state, d| {
            let mut c = Canvas::new(700, 200);
            edges(&mut c, Rect::new(x, 10, 600, 80 * d), 400 * d, state, d);
            c.pixels
        };
        for d in [1, 2] {
            for state in [State::Normal, State::Inactive] {
                let fresh = draw(&client_glass_edges(&s), 4, state, d);
                assert_eq!(draw(&kept, 4, state, d), fresh, "first paint at {d}");
                assert_eq!(draw(&kept, 4, state, d), fresh, "kept strip at {d}");
            }
        }
        let shifted = draw(&kept, 9, State::Normal, 1);
        let at4 = draw(&kept, 4, State::Normal, 1);
        assert_eq!(
            shifted[5..],
            at4[..at4.len() - 5],
            "a strip lands wherever the band is"
        );
    }

    #[test]
    fn max_client_glass_is_the_frames_plain_glass_laid_over_the_client_edges() {
        for (k, (_, state)) in SURFACES.into_iter().enumerate() {
            for d in [1, 2] {
                let img = max_client_glass(&Settings::default(), d);
                assert!(
                    (0..d).all(|x| (0..d).all(|y| at(&img, state, x, y) == art::PLAIN[k])),
                    "{state:?} at scale {d}: plain glass"
                );
            }
        }
        let img = max_client_glass(&Settings::default(), 1);
        assert_eq!(
            img.outset,
            Insets::new(CLIENT_EDGES, CLIENT_EDGES, CLIENT_EDGES, 0),
            "over the client's edges beside and above, none below"
        );
        let opaque = Settings {
            transparency: false,
            ..Settings::default()
        };
        assert_eq!(
            at(&max_client_glass(&opaque, 1), State::Normal, 0, 0),
            0xFFB0_C9E6,
            "opaque as the frame's plain glass is"
        );
    }

    #[test]
    fn client_glass_carries_the_captions_end_sheen_down_to_plain_glass() {
        let s = Settings::default();
        let cap = caption(&s, 1);
        for (k, (_, state)) in SURFACES.into_iter().enumerate() {
            let plain = art::PLAIN[k];
            for d in [1, 2] {
                let img = client_glass(&s, d);
                let px = |x: i32, y: i32| at(&img, state, x * d, y * d);
                let what = format!("{state:?} at scale {d}");
                assert_ne!(px(0, 0), plain, "{what}: the end's top is sheened");
                assert_eq!(px(END, 0), plain, "{what}: plain glass past the end");
                assert_eq!(px(0, CLIENT_ROWS - 1), plain, "{what}: the sheen is gone");
                assert_eq!(px(0, 0), px(2 * END + 1, 0), "{what}: both ends alike");
            }
            let img = client_glass(&s, 1);
            for x in [0, 20, 60, 120] {
                let (below, above) = (at(&img, state, x, 0), at(&cap, state, x + SIDE - 2, 27));
                let near = (0..4)
                    .all(|i| (below >> (8 * i) & 0xFF).abs_diff(above >> (8 * i) & 0xFF) <= 4);
                assert!(
                    near,
                    "{state:?} column {x}: {below:08X} carries on {above:08X}"
                );
            }
        }
        let img = client_glass(&s, 1);
        assert_eq!(
            img.outset,
            Insets::new(CLIENT_EDGES, CLIENT_EDGES, CLIENT_EDGES, 0),
            "over the client's edges beside and above, none below"
        );
        assert_eq!(
            img.sizing,
            Insets::new(END, CLIENT_ROWS - 1, END, 0),
            "fixed ends and rows, the last row stretching"
        );
    }

    #[test]
    fn client_glass_keeps_its_pixels_in_each_material() {
        let got = [Material::Frosted, Material::Smoked, Material::Molten].map(|material| {
            let s = Settings {
                material,
                ..Settings::default()
            };
            [1, 2].map(|d| [fnv(&client_glass(&s, d)), fnv(&max_client_glass(&s, d))])
        });
        let want = [
            [
                [0x4787_5A5E_F15B_DA6D, 0xD6C9_6FCC_DFB0_0A84],
                [0x13D7_BAC6_2753_11CD, 0x3CD3_AEE4_9B07_8B15],
            ],
            [
                [0x1AB6_329F_A262_8915, 0x7542_D690_08B5_0B29],
                [0x4F79_DFDF_84D1_2091, 0x0C48_F624_358D_9065],
            ],
            [
                [0x3925_0BDD_7575_F885, 0xF755_0E91_5CDB_520B],
                [0x3765_9B57_6106_0319, 0x7ECF_8D47_39DC_8475],
            ],
        ];
        assert_eq!(
            got, want,
            "Frosted, Smoked, Molten, sheened then plain: {got:X?}"
        );
    }

    #[test]
    fn opaque_plain_glass_lands_on_its_opaque_colour() {
        let s = Settings {
            transparency: false,
            ..Settings::default()
        };
        let c = caption(&s, 1);
        assert_eq!(
            at(&c, State::Normal, END + 1, 10),
            0xFFB0_C9E6,
            "plain glass is (176, 204, 231)"
        );
        assert_eq!(at(&c, State::Normal, 0, 0), 0, "the cut corner stays clear");
    }

    #[test]
    fn the_glass_moves_as_the_plain_glass_does() {
        // Glass points of each part: caption middle, sheen at both ends and row 2, a side's sheen, foot, bottom, maximized band.
        let points: [(usize, i32, i32); 11] = [
            (0, END + 1, 10),
            (0, 10, 10),
            (0, 2 * END - 9, 20),
            (0, 10, 2),
            (0, 100, 15),
            (1, 0, 10),
            (2, 3, 20),
            (2, 3, 110),
            (3, 4, 300),
            (4, SIDE + 1, 3),
            (4, 3, 2),
        ];
        let sky = Settings::default();
        let from = parts(&sky);
        let mut capped = 0;
        for s in settings() {
            let to = parts(&s);
            for (surface, state) in SURFACES {
                let (a_sky, p_sky) = tint::plain(&sky, surface);
                let (a_s, p_s) = tint::plain(&s, surface);
                let moved: [f64; 3] = std::array::from_fn(|i| {
                    over_grey((a_s, p_s))[i] - over_grey((a_sky, p_sky))[i]
                });
                for (part, x, y) in points {
                    let what = format!(
                        "{} ({x}, {y}) {state:?}, {:?} {:?} at {}",
                        from[part].0, s.color, s.material, s.intensity
                    );
                    let (a0, c0) = tint::split(at(&from[part].1, state, x, y));
                    let (a1, c1) = tint::split(at(&to[part].1, state, x, y));
                    // What the sheen adds over the plain glass here, in Sky, where nothing caps it.
                    let added = a0 - a_sky;
                    assert!(a_sky + added < DENSE_CEILING, "{what}: Sky is not capped");
                    // Whether Sky's sheen colour over this plain glass would pass the alpha (within rounding).
                    let past = (0..3).any(|i| c0[i] - p_sky[i] + p_s[i] > a1 - 1.0);
                    if a_s + added > DENSE_CEILING || past {
                        // Added alpha stops at the ceiling; the sheen's colour scales down alike on every channel that would pass the alpha.
                        let want = if a_s + added > DENSE_CEILING {
                            capped += 1;
                            a_s.max(DENSE_CEILING).round()
                        } else {
                            a_s + added
                        };
                        assert!((a1 - want).abs() <= 1.0, "{what}: alpha {a1}, want {want}");
                        panel::sheen_share(
                            &what,
                            std::array::from_fn(|i| c0[i] - p_sky[i]),
                            std::array::from_fn(|i| c1[i] - p_s[i]),
                        );
                        continue;
                    }
                    let (g0, g1) = (over_grey((a0, c0)), over_grey((a1, c1)));
                    for i in 0..3 {
                        assert!(
                            (g1[i] - g0[i] - moved[i]).abs() <= 1.5,
                            "{what} channel {i}: moves {} over grey, as the plain glass's {}",
                            g1[i] - g0[i],
                            moved[i]
                        );
                    }
                }
            }
        }
        assert!(capped > 0, "intensity 100 takes some sheens to the ceiling");
    }

    #[test]
    fn no_setting_clips_the_frames_colour() {
        // Caption and side sheens for both surfaces in every setting, before rounding: no channel outside 0..alpha for `pack` to clamp.
        let w = 2 * END + 2;
        let mut clipped = Vec::new();
        for s in panel::every_setting() {
            for (surface, _) in SURFACES {
                let p = Paint::new(&s, surface);
                let band = (2..BAND).flat_map(|y| {
                    (0..w).map(move |x| caption_sheen(f64::from(x), f64::from(y), w))
                });
                let sides =
                    (0..SIDE_ROWS).map(|y| p.side.at((f64::from(y) + 0.5) / f64::from(SIDE_ROWS)));
                let worst = band
                    .chain(sides)
                    .map(|sheen| panel::clip(p.glass.at(sheen, 0.0)))
                    .fold(0.0, f64::max);
                if worst > 1e-9 {
                    clipped.push(format!(
                        "{surface:?}, {:?} {:?} {}: {worst:.1}",
                        s.color, s.material, s.intensity
                    ));
                }
            }
        }
        assert!(clipped.is_empty(), "{clipped:#?}");
    }

    #[test]
    fn with_transparency_on_no_setting_makes_the_frame_opaque() {
        for s in settings() {
            for (name, img) in parts(&s) {
                let densest = img.atlas.pixels.iter().map(|&px| px >> 24).max();
                assert!(
                    densest < Some(0xFF),
                    "{name}, {:?} {:?} at {}: densest {densest:?}",
                    s.color,
                    s.material,
                    s.intensity
                );
            }
        }
    }

    #[test]
    fn with_transparency_off_the_frame_is_opaque_but_its_cut_corners() {
        for s in settings() {
            let s = Settings {
                transparency: false,
                ..s
            };
            for (name, img) in parts(&s) {
                assert!(
                    img.atlas
                        .pixels
                        .iter()
                        .all(|&px| px == 0 || px >> 24 == 0xFF),
                    "{name}, {:?} {:?} at {}: every pixel opaque or clear",
                    s.color,
                    s.material,
                    s.intensity
                );
            }
            for state in [State::Normal, State::Inactive] {
                assert_eq!(
                    at(&caption(&s, 1), state, 0, 0),
                    0,
                    "the cut corner stays clear"
                );
                assert_eq!(at(&caption(&s, 1), state, 5, 0) >> 24, 0xFF, "the outline");
            }
        }
    }

    #[test]
    fn rounded_corners_cut_the_staircase_and_square_ones_none() {
        let rounded = Settings::default();
        let square = Settings {
            frame: Frame::Square,
            ..rounded
        };
        for state in [State::Normal, State::Inactive] {
            let (cap, bottom) = (caption(&rounded, 1), frame_bottom(&rounded, 1));
            let (w, h) = (2 * END + 2, SIDE);
            for (i, &cut) in CORNER.iter().chain(&[0]).enumerate() {
                let i = i as i32;
                let corners = [
                    (&cap, 0, i, 1),
                    (&cap, w - 1, i, -1),
                    (&bottom, 0, h - 1 - i, 1),
                    (&bottom, 2 * SIDE + 1, h - 1 - i, -1),
                ];
                for (n, (img, x0, y, dx)) in corners.into_iter().enumerate() {
                    for d in 0..cut {
                        assert_eq!(
                            at(img, state, x0 + dx * d, y),
                            0,
                            "{state:?} corner {n}, row {i}: pixel {d} is clear"
                        );
                    }
                    assert_ne!(
                        at(img, state, x0 + dx * cut, y) >> 24,
                        0,
                        "{state:?} corner {n}, row {i}: pixel {cut} is the outline"
                    );
                }
            }
            // Square: the outline and the light line run straight into each corner.
            let (cap, bottom) = (caption(&square, 1), frame_bottom(&square, 1));
            let (w, b) = (2 * END + 2, 2 * SIDE + 2);
            for (x, along) in [(0, 7), (w - 1, w - 8)] {
                assert_eq!(
                    at(&cap, state, x, 0),
                    at(&cap, state, along, 0),
                    "{state:?}: the top outline at x {x}"
                );
            }
            assert_eq!(
                at(&cap, state, 1, 1),
                at(&cap, state, 7, 1),
                "{state:?}: the light line"
            );
            for (x, along) in [(0, 6), (b - 1, b - 7)] {
                assert_eq!(
                    at(&bottom, state, x, SIDE - 1),
                    at(&bottom, state, along, SIDE - 1),
                    "{state:?}: the bottom outline at x {x}"
                );
            }
        }
    }

    #[test]
    fn a_caption_keeps_its_ends_at_any_width() {
        let style = crate::glass::style(&Settings::default());
        let band = |w: i32, state| {
            let mut c = Canvas::new(w as u32, BAND as u32);
            style.draw(&mut c, Rect::new(0, 0, w, BAND), Part::Caption, state, 1);
            c
        };
        for state in [State::Normal, State::Inactive] {
            let wide = band(2000, state);
            for w in [300, 420, 800, 2000] {
                let c = band(w, state);
                // The left end from the left edge, the right from the right; a narrow band shares it half and half.
                let (left, right) = (END.min(w / 2), END.min(w - w / 2));
                for y in 0..BAND {
                    for x in 0..left {
                        assert_eq!(
                            c.get(x, y),
                            wide.get(x, y),
                            "{w} px {state:?}: ({x}, {y}) of the left end"
                        );
                    }
                    for d in 0..right {
                        assert_eq!(
                            c.get(w - 1 - d, y),
                            wide.get(1999 - d, y),
                            "{w} px {state:?}: ({d}, {y}) in from the right end"
                        );
                    }
                    for x in left..w - right {
                        assert_eq!(
                            c.get(x, y),
                            wide.get(1000, y),
                            "{w} px {state:?}: ({x}, {y}) the plain middle"
                        );
                    }
                }
            }
        }
    }

    fn rects(w: i32, h: i32) -> [(Part, Rect); 5] {
        [
            (Part::Caption, Rect::new(0, 0, w, BAND)),
            (Part::MaxCaption, Rect::new(0, 0, w, MAX_BAND)),
            (Part::FrameLeft, Rect::new(0, BAND, SIDE, h - BAND - SIDE)),
            (
                Part::FrameRight,
                Rect::new(w - SIDE, BAND, SIDE, h - BAND - SIDE),
            ),
            (Part::FrameBottom, Rect::new(0, h - SIDE, w, SIDE)),
        ]
    }

    const R2: i32 = 14;

    /// Whether device pixel (`x`, `y`) of a `w` x `h` frame lies in one of the corners' [`R2`] boxes.
    fn in_corner(x: i32, y: i32, w: i32, h: i32) -> bool {
        (x < R2 || x >= w - R2) && (y < R2 || y >= h - R2)
    }

    #[test]
    fn the_style_at_scale_2_paints_the_frame_at_2() {
        let style = crate::glass::style(&Settings::default());
        let two = style.at_scale(2);
        for part in [
            Part::Caption,
            Part::MaxCaption,
            Part::FrameLeft,
            Part::FrameRight,
            Part::FrameBottom,
        ] {
            assert_eq!(two.part(part).unwrap().scale, 2, "{part:?}");
            assert_eq!(style.part(part).unwrap().scale, 1, "{part:?}");
        }
        assert!(std::sync::Arc::ptr_eq(&two, &style.at_scale(2)), "kept");
        assert!(std::sync::Arc::ptr_eq(&style, &style.at_scale(1)));
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass).scaled(2);
        let scaled = theme.look.style().unwrap();
        assert_eq!(
            scaled.part(Part::Caption).unwrap().scale,
            2,
            "a scaled theme's"
        );
    }

    #[test]
    fn a_frame_at_scale_2_is_its_scale_1_design_twice_over() {
        // Each scale-2 pixel is within a level either side of the scale-1
        // pixels nearest it (2 x 2 design pixels), so a sheen runs smooth and
        // a line's edge falls between two pixels. Where no sheen reaches those
        // pixels are one colour, exactly it. Rounded corners are covered by
        // `rounded_corners_at_scale_2_are_round_and_antialiased`.
        let (w, h) = (640, 400);
        for frame in Frame::ALL {
            let style = crate::glass::style(&Settings {
                frame,
                ..Settings::default()
            });
            let two = style.at_scale(2);
            for state in [State::Normal, State::Inactive] {
                for (part, r) in rects(w, h) {
                    let mut one = Canvas::new(w as u32, h as u32);
                    style.draw(&mut one, r, part, state, 1);
                    let mut big = Canvas::new(2 * w as u32, 2 * h as u32);
                    let r2 = Rect::new(2 * r.x, 2 * r.y, 2 * r.width, 2 * r.height);
                    two.draw(&mut big, r2, part, state, 2);
                    let near = |v: i32, len: i32| {
                        let c = centre(v, 2);
                        [c.floor() as i32, c.ceil() as i32].map(|n| n.clamp(0, len - 1))
                    };
                    for y in r2.y..r2.bottom() {
                        for x in r2.x..r2.right() {
                            let round = frame == Frame::Rounded && part != Part::MaxCaption;
                            if round && in_corner(x, y, 2 * w, 2 * h) {
                                continue;
                            }
                            let got = big.get(x, y);
                            let around: Vec<u32> = near(y, h)
                                .into_iter()
                                .flat_map(|sy| near(x, w).map(|sx| one.get(sx, sy)))
                                .collect();
                            let what = format!("{frame:?} {part:?} {state:?} ({x}, {y})");
                            let (dx, dy) = (x / 2, y / 2);
                            let sheenless = match part {
                                Part::Caption => dx >= END - 2 && dx < w - END + 2,
                                Part::FrameLeft | Part::FrameRight => {
                                    let t = f64::from(dy - BAND) / f64::from(h - BAND - SIDE);
                                    t > SIDE_EDGE + SIDE_TAIL.1 + 0.01
                                }
                                _ => true,
                            };
                            if sheenless && around.iter().all(|&p| p == around[0]) {
                                assert_eq!(got, around[0], "{what}: {got:08X}");
                                continue;
                            }
                            for sh in [24, 16, 8, 0] {
                                let ch = |p: u32| ((p >> sh) & 0xFF) as i32;
                                let lo = around.iter().map(|&p| ch(p)).min().unwrap();
                                let hi = around.iter().map(|&p| ch(p)).max().unwrap();
                                assert!(
                                    (lo - 1..=hi + 1).contains(&ch(got)),
                                    "{what}: {got:08X} channel {sh} outside {lo}..={hi}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rounded_corners_at_scale_2_are_round_and_antialiased() {
        // The corner table's curve, radius 7 design px (14 at scale 2): clear outside, whole inside, partly covered along it; all four corners alike.
        let s = Settings::default();
        let (cap, bottom) = (caption(&s, 2), frame_bottom(&s, 2));
        let (cw, bh, bw) = (2 * (2 * END + 2), 2 * SIDE, 2 * (2 * SIDE + 2));
        for (k, (_, state)) in SURFACES.into_iter().enumerate() {
            let plain = art::PLAIN[k] >> 24;
            let mut partial = std::collections::BTreeSet::new();
            for y in 0..R2 {
                for x in 0..R2 {
                    let px = at(&cap, state, x, y);
                    let (dx, dy) = (f64::from(R2 - x) - 0.5, f64::from(R2 - y) - 0.5);
                    let dist = dx.hypot(dy);
                    let what = format!("{state:?} ({x}, {y}): {px:08X}");
                    if dist >= f64::from(R2) + 0.71 {
                        assert_eq!(px, 0, "{what}: outside the curve");
                    } else if dist <= f64::from(R2) - 0.71 {
                        assert!(px >> 24 >= plain, "{what}: inside the curve");
                    } else {
                        partial.insert(px >> 24);
                    }
                    assert_eq!(
                        at(&cap, state, cw - 1 - x, y),
                        px,
                        "{what}: the top corners alike"
                    );
                    let low = at(&bottom, state, x, bh - 1 - y);
                    assert_eq!(
                        at(&bottom, state, bw - 1 - x, bh - 1 - y),
                        low,
                        "{what}: the bottom corners alike"
                    );
                    assert_eq!(low == 0, px == 0, "{what}: the bottom's curve the top's");
                }
            }
            assert!(
                partial.len() >= 8,
                "{state:?}: the curve's edge in many levels, not a step: {partial:?}"
            );
        }
        let square = caption(
            &Settings {
                frame: Frame::Square,
                ..s
            },
            2,
        );
        for state in [State::Normal, State::Inactive] {
            for (x, y) in [(0, 0), (1, 1), (0, R2), (R2, 0)] {
                assert_eq!(
                    at(&square, state, x, y),
                    at(&square, state, 40, 0),
                    "{state:?} ({x}, {y}): the outline"
                );
            }
        }
    }

    #[test]
    fn the_frames_parts_at_scale_2_keep_their_pixels_at_sky() {
        for (frame, want) in [
            (
                Frame::Rounded,
                [
                    0xCEBA_77A7_1E8E_5905_u64,
                    0x091B_3BA0_520C_0105,
                    0xC8DC_F957_A856_12B5,
                    0x1772_08CE_FB37_AAC5,
                    0x0D09_9A17_9FF6_05C9,
                ],
            ),
            (
                Frame::Square,
                [
                    0xFE9C_16E4_BE11_A91D,
                    0x091B_3BA0_520C_0105,
                    0xC8DC_F957_A856_12B5,
                    0x1772_08CE_FB37_AAC5,
                    0x09BD_F4D0_EB8D_77F5,
                ],
            ),
        ] {
            let s = Settings {
                frame,
                ..Settings::default()
            };
            let got = parts_at(&s, 2).map(|(_, img)| fnv(&img));
            assert_eq!(
                got, want,
                "{frame:?}: caption, max caption, left, right, bottom, {got:#018X?}"
            );
        }
    }

    #[test]
    fn with_transparency_off_a_frame_at_scale_2_is_opaque_but_its_corners_edge() {
        // Opaque or clear everywhere but along a Rounded corner's curve, where a pixel is its opaque colour cut to coverage.
        for s in settings() {
            for frame in Frame::ALL {
                let s = Settings {
                    transparency: false,
                    frame,
                    ..s
                };
                for (name, img) in parts_at(&s, 2) {
                    let (w, h) = (img.atlas.width as i32, img.atlas.height as i32 / 2);
                    let (top, fh) = match name {
                        "bottom" => (2 * (BAND + 2), 2 * (BAND + SIDE + 2)),
                        _ => (0, 2 * (BAND + SIDE + 2)),
                    };
                    let corners = frame == Frame::Rounded && matches!(name, "caption" | "bottom");
                    for y in 0..2 * h {
                        for x in 0..w {
                            let px = img.atlas.get(x, y);
                            let a = px >> 24;
                            let edge = corners && in_corner(x, top + y % h, w, fh);
                            let what = format!("{name} {frame:?} ({x}, {y}): {px:08X}");
                            if edge {
                                assert!(
                                    [16, 8, 0].iter().all(|sh| (px >> sh) & 0xFF <= a),
                                    "{what}: premultiplied"
                                );
                            } else {
                                assert!(a == 0 || a == 0xFF, "{what}: opaque or clear");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Under dark Glass a window with no icon shows the light window glyph on its caption, whose glass keeps its colour.
    #[test]
    fn a_caption_without_an_icon_shows_the_light_glyph_under_dark_glass() {
        let theme = crate::interfaces::theme_with(
            crate::interface::Interface::Glass,
            crate::blue::Variant::default(),
            &Settings {
                mode: Mode::Dark,
                ..Settings::default()
            },
        );
        let style = theme.look.style().unwrap();
        let text = TextRenderer::load(&style.caption_text.font());
        let g = FrameGeometry::layout(&theme.metrics, 300, 100, false, Chrome::WINDOW);
        let draw = |icon: Option<&Canvas>| {
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            let st = FrameState {
                title: "",
                active: true,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon,
                tabbed: false,
            };
            crate::style::frame::draw_frame(&mut c, style, &theme, &text, &g, &st);
            c
        };
        let px = theme.icon_size();
        let light = crate::icon::chrome_fallback(&theme, px);
        let dark = crate::icon::fallback(&theme, px);
        assert!(light.pixels != dark.pixels, "the scheme's glyph is dark");
        assert!(
            draw(None).pixels == draw(Some(&light)).pixels,
            "the caption draws the light glyph"
        );
    }

    #[test]
    fn the_title_glow_keeps_to_the_bands_glass_rows_unchanged() {
        // The glow's rows restored: the band's glass rows, under the light line to over the client's edges.
        let cap = caption(&Settings::default(), 1);
        let (mid, state) = (END + 1, State::Normal);
        assert_eq!(
            at(&cap, state, mid, 1),
            art::LINES[0][1],
            "row 1 is the light line"
        );
        assert_eq!(
            at(&cap, state, mid, BAND - 2),
            art::LINES[0][2],
            "row 28 the client's light edge"
        );
        // A title of strokes every third pixel along rows 10 to 18, and its glow's alpha FNV-1a hash per mode.
        let (w, h) = (150, BAND as usize);
        let mut title = vec![0u8; w * h];
        for y in 10..19 {
            for x in (30..120).step_by(3) {
                title[y * w + x] = 255;
            }
        }
        let fnv = |a: &[u8]| {
            a.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        };
        for (mode, color, reach, hashes) in [
            (
                Mode::Light,
                0xFF,
                [17, 36],
                [0xb7fe_a99c_cd02_fb89, 0x21dc_c1ff_c30e_cb4d],
            ),
            (
                Mode::Dark,
                0,
                [3, 7],
                [0xa8f9_28b4_df90_e186, 0x726d_3fbe_69cf_e537],
            ),
        ] {
            let st = crate::glass::style(&Settings {
                mode,
                ..Settings::default()
            });
            let g = st.caption_title.glow.unwrap();
            assert_eq!(
                g.rows,
                [(2, 2), (0, 2)],
                "{mode:?}: the glass rows, restored and maximized"
            );
            assert_eq!(
                g.color,
                crate::theme::Color::rgb(color, color, color),
                "{mode:?}"
            );
            assert_eq!(
                [(g.reach)(1), (g.reach)(2)],
                reach,
                "{mode:?}: its reach at scale 1 and 2"
            );
            for (active, hash) in [true, false].into_iter().zip(hashes) {
                let mask = crate::style::GlowMask {
                    coverage: &title,
                    width: w,
                    height: h,
                    rows: (2, h - 2),
                    active,
                    scale: 1,
                };
                assert_eq!(
                    fnv(&(g.alpha)(&mask)),
                    hash,
                    "{mode:?}, focused {active}: the glow"
                );
            }
        }
    }
}
