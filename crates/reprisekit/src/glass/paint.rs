//! Reprise Glass's paint: premultiplied pixels over the blurred backdrop (opaque with
//! transparency off). The frame is in [`super::frame`], caption buttons in [`super::buttons`],
//! the Start orb in [`super::orb`], the bar, tray shade, Show desktop strip and lit slots in
//! [`super::bar`], task buttons in [`super::taskbutton`]. Here: the caption title's white glow
//! (computed once per title and kept) and the colour a task button glows in.

use crate::glass::{Settings, bar, orb, taskbutton};
use crate::mutex::lock;
use crate::raster::Canvas;
use crate::style::{GlowMask, Part, PartImage, TaskGlow, TitleGlow};
use crate::theme::Color;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// The Start orb, 54x40: a 36 px disc of clear glass tinted in the glass hue, filling rows 2 to
/// 37. The glow round it comes from its icon ([`super::orb::bloom`]).
pub fn start_button(s: &Settings, scale: i32) -> PartImage {
    orb::orb(s, orb::Bar::Full, scale)
}

/// The 40 px bar's orb above the bar, 54x6, clear in every state; only the icon's glow reaches
/// there.
pub fn start_above(s: &Settings, scale: i32) -> PartImage {
    orb::above(s, orb::Bar::Full, scale)
}

/// The small bar's orb above the bar, 54x13: the disc's top 5 rows (centred at (27, 13) of the
/// 30 px bar). It lies over whatever is above the bar, not glass.
pub fn small_orb_above(s: &Settings, scale: i32) -> PartImage {
    orb::above(s, orb::Bar::Small, scale)
}

/// Task buttons that glow in their icon's colour; labelled buttons' pairs go through tint maps.
pub fn task_glow(_s: &Settings) -> Option<TaskGlow> {
    Some(TaskGlow {
        colour: dominant_colour,
        label_edges: Some(super::taskbutton::glowing_label_edge),
    })
}

/// The small bar's task buttons, glowing as [`task_glow`]'s do, without labelled pairs.
pub(crate) fn small_task_glow(_s: &Settings) -> Option<TaskGlow> {
    Some(TaskGlow {
        colour: dominant_colour,
        label_edges: None,
    })
}

/// The colour a task button glows in: the saturation-weighted mean of `icon`'s saturated (HSV
/// at least 0.25), mostly opaque (alpha at least 200) pixels. None when there are none, so a
/// grey icon lights a spot without a hue.
pub fn dominant_colour(icon: &Canvas) -> Option<Color> {
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for &px in &icon.pixels {
        let a = px >> 24;
        if a < 200 {
            continue;
        }
        let rgb = [16, 8, 0].map(|sh| (((px >> sh) & 0xFF) * 255) as f32 / a as f32);
        let hi = rgb[0].max(rgb[1]).max(rgb[2]);
        let lo = rgb[0].min(rgb[1]).min(rgb[2]);
        if hi <= 0.0 || (hi - lo) / hi < 0.25 {
            continue;
        }
        let sat = (hi - lo) / hi;
        for (s, v) in sum.iter_mut().zip(rgb) {
            *s += v * sat;
        }
        weight += sat;
    }
    let ch = |i: usize| (sum[i] / weight).round().min(255.0) as u8;
    (weight > 0.0).then(|| Color::rgb(ch(0), ch(1), ch(2)))
}

/// The small 30 px bar's parts (`[taskbar] small_icons`), each painted at `scale`.
pub fn small_bar(s: &Settings, scale: i32) -> [(Part, PartImage); 11] {
    let h = super::SMALL_TASKBAR_HEIGHT;
    let button = (super::SMALL_ICON_BUTTON.0, h);
    [
        (Part::Taskbar, bar::taskbar(s, h, scale)),
        (Part::StartButton, orb::orb(s, orb::Bar::Small, scale)),
        (Part::StartAbove, small_orb_above(s, scale)),
        (Part::TaskButton, taskbutton::buttons(s, button, scale)),
        (Part::TaskStack, taskbutton::stacks(s, button, false, scale)),
        (Part::TrayArea, bar::tray_area(s, h, scale)),
        (Part::ShowDesktop, bar::show_desktop(s, h, scale)),
        (Part::SlotEdge, bar::slot_edge(s, h, scale)),
        (
            Part::SlotLight,
            bar::slot_light(super::SMALL_ICON_BUTTON.0, scale),
        ),
        (Part::SlotBand, bar::slot_band(s, h, scale)),
        (Part::PagerTile, taskbutton::pager_tiles(s, button, scale)),
    ]
}

/// The white glow under a caption title: alpha is `peak * min(1, gain * blur(dilate(text)))` in
/// [`GLOW`]'s shape. The coverage is spread 4 px every way so the glow runs across gaps between
/// words, then blurred by three box blurs approximating a Gaussian of the state's sigma. Only
/// the band's glass rows: from row 2 of a restored band, row 0 of a maximized one, to 2 rows
/// above the client.
pub const TITLE_GLOW: TitleGlow = TitleGlow {
    reach: glow_reach,
    color: Color::rgb(0xFF, 0xFF, 0xFF),
    rows: [(2, 2), (0, 2)],
    alpha: title_glow,
};

/// The soft black edge under a dark Glass title: [`TITLE_GLOW`]'s rows in [`EDGE`]'s shape,
/// blurred about 3 px with no spread, so the white title reads over pale glass.
pub const TITLE_GLOW_DARK: TitleGlow = TitleGlow {
    reach: edge_reach,
    color: Color::rgb(0, 0, 0),
    alpha: title_edge,
    ..TITLE_GLOW
};

/// Under white labels a client draws over its frame's glass in dark mode (menu titles, toolbar
/// buttons): [`TITLE_GLOW_DARK`]'s edge over a faint dark cloud ([`HALO`]), so
/// they read over the pale glass and its sheen.
pub const LABEL_HALO_DARK: TitleGlow = TitleGlow {
    reach: halo_reach,
    color: Color::rgb(0, 0, 0),
    alpha: label_halo,
    ..TITLE_GLOW
};

struct GlowShape {
    /// Spread before the blur, at scale 1.
    spread: i32,
    /// How strongly the blurred coverage saturates to the peak.
    gain: f32,
    /// (peak alpha, blur sigma in px) for a focused window's title, then another's.
    states: [(f32, f32); 2],
}

const GLOW: GlowShape = GlowShape {
    spread: 4,
    gain: 1.33,
    states: [(0.479, 5.0), (0.329, 4.5)],
};

/// [`TITLE_GLOW_DARK`]'s shape. Another window's glass is paler, so its edge runs darker (0.85
/// against 0.75).
const EDGE: GlowShape = GlowShape {
    spread: 0,
    gain: 3.0,
    states: [(0.75, 1.4), (0.85, 1.4)],
};

/// The faint dark cloud of [`LABEL_HALO_DARK`]: blurred about 2 px, so it fades out within a
/// menu bar's rows.
const HALO: GlowShape = GlowShape {
    spread: 0,
    gain: 1.5,
    states: [(0.35, 2.0), (0.35, 2.0)],
};

impl GlowShape {
    /// How far past the text the halo reaches at `scale`: the spread plus the three boxes'
    /// radii, the wider state's.
    fn reach(&self, scale: i32) -> i32 {
        let s = scale.max(1);
        self.states
            .iter()
            .map(|&(_, sigma)| {
                let blur: usize = box_radii(sigma * s as f32).iter().sum();
                self.spread * s + blur as i32
            })
            .max()
            .unwrap_or(0)
    }
}

/// How far [`TITLE_GLOW`] reaches past the text; 4 + 13 = 17 px at scale 1.
pub fn glow_reach(scale: i32) -> i32 {
    GLOW.reach(scale)
}

/// How far [`TITLE_GLOW_DARK`] reaches: 3 px at scale 1.
pub fn edge_reach(scale: i32) -> i32 {
    EDGE.reach(scale)
}

/// How far [`LABEL_HALO_DARK`] reaches at `scale`: its cloud's reach.
pub fn halo_reach(scale: i32) -> i32 {
    HALO.reach(scale)
}

/// Radii of three box blurs whose sum has variance `sigma`^2 as near as odd widths allow: the
/// narrower odd width for the first passes, the wider for the rest.
fn box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut lo = ideal.floor() as i32;
    if lo % 2 == 0 {
        lo -= 1;
    }
    let l = lo as f32;
    let narrow = ((12.0 * sigma * sigma - n * l * l - 4.0 * n * l - 3.0 * n) / (-4.0 * l - 4.0))
        .round()
        .clamp(0.0, n) as usize;
    std::array::from_fn(|i| {
        let width = if i < narrow { lo } else { lo + 2 };
        (width as usize - 1) / 2
    })
}

/// [`TITLE_GLOW`]'s alpha for `m`, kept per coverage, state, row span and scale so a redraw for
/// a hover, press or move reuses it.
pub fn title_glow(m: &GlowMask) -> Arc<[u8]> {
    static GLOWS: Mutex<GlowCache> = Mutex::new(GlowCache::new());
    lock(&GLOWS).get(m, |m| glow_alpha(&GLOW, m))
}

/// [`TITLE_GLOW_DARK`]'s alpha for `m`, kept like [`title_glow`]'s.
pub fn title_edge(m: &GlowMask) -> Arc<[u8]> {
    static EDGES: Mutex<GlowCache> = Mutex::new(GlowCache::new());
    lock(&EDGES).get(m, |m| glow_alpha(&EDGE, m))
}

/// [`LABEL_HALO_DARK`]'s alpha for `m`, kept like [`title_glow`]'s.
pub fn label_halo(m: &GlowMask) -> Arc<[u8]> {
    static HALOS: Mutex<GlowCache> = Mutex::new(GlowCache::new());
    lock(&HALOS).get(m, |m| {
        let edge = glow_alpha(&EDGE, m);
        let mut cloud = glow_alpha(&HALO, m);
        for (c, e) in cloud.iter_mut().zip(edge) {
            *c = (*c).max(e);
        }
        cloud
    })
}

/// How many glows [`title_glow`], [`title_edge`] and [`label_halo`] each keep, oldest dropped first. An entry
/// holds two bytes per mask pixel: about 9 KB for a short title, 300 KB across a maximized 3840
/// px window at scale 2, so at most about 5 MB for 16.
const GLOW_CACHE_CAP: usize = 16;

/// Everything a glow depends on: the coverage (title, face, size, clipping), its rows, the
/// window's state and the scale.
struct GlowKey {
    coverage: Box<[u8]>,
    width: usize,
    rows: (usize, usize),
    active: bool,
    scale: i32,
}

impl GlowKey {
    fn of(m: &GlowMask) -> GlowKey {
        GlowKey {
            coverage: m.coverage.into(),
            width: m.width,
            rows: m.rows,
            active: m.active,
            scale: m.scale,
        }
    }

    fn matches(&self, m: &GlowMask) -> bool {
        (self.width, self.rows, self.active, self.scale) == (m.width, m.rows, m.active, m.scale)
            && *self.coverage == *m.coverage
    }
}

struct GlowCache {
    entries: VecDeque<(GlowKey, Arc<[u8]>)>,
}

impl GlowCache {
    const fn new() -> GlowCache {
        GlowCache {
            entries: VecDeque::new(),
        }
    }

    fn get(&mut self, m: &GlowMask, compute: impl FnOnce(&GlowMask) -> Vec<u8>) -> Arc<[u8]> {
        if let Some(i) = self.entries.iter().position(|(k, _)| k.matches(m)) {
            let hit = self.entries.remove(i).expect("position is in range");
            let glow = hit.1.clone();
            self.entries.push_front(hit);
            return glow;
        }
        let glow: Arc<[u8]> = compute(m).into();
        self.entries.truncate(GLOW_CACHE_CAP - 1);
        self.entries.push_front((GlowKey::of(m), glow.clone()));
        glow
    }
}

/// A halo's alpha in `shape` for `m`, 0..255 per pixel. Each pass costs a constant per pixel at
/// any scale; the spread skips all-0 rows and the passes down all-0 columns.
fn glow_alpha(shape: &GlowShape, m: &GlowMask) -> Vec<u8> {
    let (w, h) = (m.width, m.height);
    let s = m.scale.max(1);
    let (peak, sigma) = shape.states[usize::from(!m.active)];
    let spread = (shape.spread * s) as usize;
    let radii = box_radii(sigma * s as f32);
    let mut img: Vec<f32> = m.coverage[..w * h]
        .iter()
        .map(|&c| c as f32 / 255.0)
        .collect();
    let mut tmp = Lines::default();
    // Spread across then down, then blur down then across: the blur's passes are linear, so the
    // down passes may run first.
    for y in 0..h {
        tmp.on_row(&mut img, w, y, spread, |line, t| t.dilate(line, spread));
    }
    // Down the columns a whole row at a time, so every step runs along a row. A column the
    // spread left all 0 stays 0 down there, so only the inked columns go.
    let mut any = vec![false; w];
    for row in img.chunks_exact(w.max(1)) {
        for (a, &v) in any.iter_mut().zip(row) {
            *a |= v != 0.0;
        }
    }
    let inked: Vec<usize> = (0..w).filter(|&x| any[x]).collect();
    let mut band = Vec::with_capacity(inked.len() * h);
    for row in img.chunks_exact(w.max(1)) {
        band.extend(inked.iter().map(|&x| row[x]));
    }
    tmp.dilate_down(&mut band, inked.len(), spread);
    for &r in &radii {
        tmp.box_down(&mut band, inked.len(), r);
    }
    for (y, row) in band.chunks_exact(inked.len().max(1)).enumerate() {
        for (&x, &v) in inked.iter().zip(row) {
            img[y * w + x] = v;
        }
    }
    let rows = m.rows.0.min(h)..m.rows.1.min(h);
    for &r in &radii {
        tmp.box_across(&mut img, w, rows.clone(), r);
    }
    let mut out = vec![0u8; w * h];
    for y in rows {
        for x in 0..w {
            let g = peak * (shape.gain * img[y * w + x]).min(1.0);
            out[y * w + x] = round_byte(g * 255.0);
        }
    }
    out
}

#[derive(Default)]
struct Lines {
    a: Vec<f32>,
    b: Vec<f32>,
    c: Vec<f32>,
}

impl Lines {
    /// Run `f` on the span of row `y` of the `w`-wide image `img` within `r` of its nonzero
    /// values, none when it is all 0: a dilation by up to `r` keeps the rest at 0.
    fn on_row(
        &mut self,
        img: &mut [f32],
        w: usize,
        y: usize,
        r: usize,
        f: impl FnOnce(&mut [f32], &mut Lines),
    ) {
        let row = &mut img[y * w..(y + 1) * w];
        let Some(first) = row.iter().position(|&v| v != 0.0) else {
            return;
        };
        let last = row.iter().rposition(|&v| v != 0.0).unwrap_or(first);
        f(
            &mut row[first.saturating_sub(r)..(last + 1 + r).min(w)],
            self,
        );
    }

    /// Raise each value of `line` to the largest within `r` of it, 0 past the ends, in constant
    /// steps per value: padded by `r`, the line is cut into blocks 2r+1 long, so a window spans
    /// at most two and its maximum combines one block's running maximum from the start with the
    /// next's from the end.
    fn dilate(&mut self, line: &mut [f32], r: usize) {
        if r == 0 {
            return;
        }
        let k = 2 * r + 1;
        let n = line.len() + 2 * r;
        let (padded, from_start, to_end) = (&mut self.a, &mut self.b, &mut self.c);
        padded.clear();
        padded.resize(r, 0.0);
        padded.extend_from_slice(line);
        padded.resize(n, 0.0);
        from_start.resize(n, 0.0);
        to_end.resize(n, 0.0);
        for start in (0..n).step_by(k) {
            let end = (start + k).min(n);
            let mut m = 0.0f32;
            for i in start..end {
                m = m.max(padded[i]);
                from_start[i] = m;
            }
            let mut m = 0.0f32;
            for i in (start..end).rev() {
                m = m.max(padded[i]);
                to_end[i] = m;
            }
        }
        for (i, out) in line.iter_mut().enumerate() {
            *out = to_end[i].max(from_start[i + k - 1]);
        }
    }

    /// [`Lines::dilate`] down the columns of `img`, a row at a time.
    fn dilate_down(&mut self, img: &mut [f32], w: usize, r: usize) {
        if r == 0 || w == 0 {
            return;
        }
        let h = img.len() / w;
        let k = 2 * r + 1;
        let n = h + 2 * r;
        let (padded, from_start, to_end) = (&mut self.a, &mut self.b, &mut self.c);
        padded.clear();
        padded.resize(r * w, 0.0);
        padded.extend_from_slice(img);
        padded.resize(n * w, 0.0);
        from_start.resize(n * w, 0.0);
        to_end.resize(n * w, 0.0);
        for start in (0..n).step_by(k) {
            let end = (start + k).min(n);
            from_start[start * w..(start + 1) * w]
                .copy_from_slice(&padded[start * w..(start + 1) * w]);
            for i in start + 1..end {
                let (done, rest) = from_start.split_at_mut(i * w);
                let prev = &done[(i - 1) * w..];
                for ((o, &p), &v) in rest[..w]
                    .iter_mut()
                    .zip(prev)
                    .zip(&padded[i * w..(i + 1) * w])
                {
                    *o = p.max(v);
                }
            }
            to_end[(end - 1) * w..end * w].copy_from_slice(&padded[(end - 1) * w..end * w]);
            for i in (start..end - 1).rev() {
                let (head, done) = to_end.split_at_mut((i + 1) * w);
                let next = &done[..w];
                for ((o, &p), &v) in head[i * w..]
                    .iter_mut()
                    .zip(next)
                    .zip(&padded[i * w..(i + 1) * w])
                {
                    *o = p.max(v);
                }
            }
        }
        for y in 0..h {
            let (a, b) = (
                &to_end[y * w..(y + 1) * w],
                &from_start[(y + k - 1) * w..(y + k) * w],
            );
            for ((o, &p), &q) in img[y * w..(y + 1) * w].iter_mut().zip(a).zip(b) {
                *o = p.max(q);
            }
        }
    }

    /// Average each value over the 2r+1 down its column around it, 0 past the ends, with one
    /// running sum per column.
    fn box_down(&mut self, img: &mut [f32], w: usize, r: usize) {
        if w == 0 {
            return;
        }
        let h = img.len() / w;
        let (src, sum) = (&mut self.a, &mut self.b);
        src.clear();
        src.extend_from_slice(img);
        sum.clear();
        sum.resize(w, 0.0);
        for y in 0..(r + 1).min(h) {
            for (s, &v) in sum.iter_mut().zip(&src[y * w..(y + 1) * w]) {
                *s += v;
            }
        }
        let k = 1.0 / (2 * r + 1) as f32;
        for y in 0..h {
            for (o, &s) in img[y * w..(y + 1) * w].iter_mut().zip(sum.iter()) {
                *o = s * k;
            }
            if y + r + 1 < h {
                for (s, &v) in sum.iter_mut().zip(&src[(y + r + 1) * w..(y + r + 2) * w]) {
                    *s += v;
                }
            }
            if y >= r {
                for (s, &v) in sum.iter_mut().zip(&src[(y - r) * w..(y - r + 1) * w]) {
                    *s -= v;
                }
            }
        }
    }

    /// Average each value of `rows` over the 2r+1 along its row around it, 0 past the ends, with
    /// one running sum per row. [`LANES`] rows run side by side, so no sum waits on another.
    fn box_across(&mut self, img: &mut [f32], w: usize, rows: std::ops::Range<usize>, r: usize) {
        if w == 0 {
            return;
        }
        let src = &mut self.a;
        for y0 in rows.clone().step_by(LANES) {
            let n = LANES.min(rows.end - y0);
            src.clear();
            src.extend_from_slice(&img[y0 * w..(y0 + n) * w]);
            let out = &mut img[y0 * w..(y0 + n) * w];
            if n == LANES {
                box_rows::<LANES>(src, out, w, r);
            } else {
                for (src, out) in src.chunks_exact(w).zip(out.chunks_exact_mut(w)) {
                    box_rows::<1>(src, out, w, r);
                }
            }
        }
    }
}

/// [`Lines::box_across`] on the `N` rows of `src`, `w` wide, into `out`.
fn box_rows<const N: usize>(src: &[f32], out: &mut [f32], w: usize, r: usize) {
    let k = (2 * r + 1) as f32;
    let mut sum = [0.0f32; N];
    for (l, s) in sum.iter_mut().enumerate() {
        *s = src[l * w..l * w + (r + 1).min(w)].iter().sum();
    }
    for i in 0..w {
        for (l, s) in sum.iter().enumerate() {
            out[l * w + i] = *s / k;
        }
        if i + r + 1 < w {
            for (l, s) in sum.iter_mut().enumerate() {
                *s += src[l * w + i + r + 1];
            }
        }
        if i >= r {
            for (l, s) in sum.iter_mut().enumerate() {
                *s -= src[l * w + i - r];
            }
        }
    }
}

/// `v` in 0..=255 rounded half away from zero as `f32::round` does, without its libm call.
fn round_byte(v: f32) -> u8 {
    let whole = v as u8;
    whole + u8::from(v - f32::from(whole) >= 0.5)
}

/// Rows [`Lines::box_across`] sums side by side.
const LANES: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::frame::{END, caption};
    use crate::raster::{Insets, Rect};
    use crate::style::{Part, State};

    fn px(img: &PartImage, state: State, x: i32, y: i32) -> u32 {
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    /// Channel `shift` of premultiplied `px` over grey 128, all three.
    fn rgb_over_grey(px: u32) -> [i32; 3] {
        [16, 8, 0].map(|shift| over_grey(px, shift))
    }

    /// A 150x30 coverage mask with a "word" of 1 px stems every 3 px on
    /// rows 10..19 over each of `spans` (x from, x to).
    fn words(spans: &[(usize, usize)]) -> Vec<u8> {
        let (w, h) = (150, 30);
        let mut m = vec![0u8; w * h];
        for &(x0, x1) in spans {
            for y in 10..19 {
                for x in (x0..x1).step_by(3) {
                    m[y * w + x] = 255;
                }
            }
        }
        m
    }

    fn mask(coverage: &[u8], active: bool) -> GlowMask<'_> {
        GlowMask {
            coverage,
            width: 150,
            height: 30,
            rows: (2, 28),
            active,
            scale: 1,
        }
    }

    #[test]
    fn the_title_glow_peaks_over_the_text_and_keeps_to_the_glass_rows() {
        let text = words(&[(30, 120)]);
        for (active, peak) in [(true, 122), (false, 84)] {
            let g = glow_alpha(&GLOW, &mask(&text, active));
            let at = |x: usize, y: usize| g[y * 150 + x];
            assert_eq!(at(75, 14), peak, "active {active}: the peak over the text");
            assert!(at(75, 2) > 0, "active {active}: row 2 glows");
            assert!(at(75, 27) > 0, "active {active}: row 27 glows");
            for y in [0, 1, 28, 29] {
                assert_eq!(at(75, y), 0, "active {active}: row {y} has no glow");
            }
            assert_eq!(at(0, 14), 0, "active {active}: nothing past its reach");
            assert_eq!(at(149, 14), 0, "active {active}: nothing past its reach");
        }
    }

    #[test]
    fn the_dark_title_edge_keeps_within_3_px_of_the_strokes() {
        assert_eq!(edge_reach(1), 3);
        // Strokes at x 30, 33 .. 117 down rows 10 to 18.
        let text = words(&[(30, 120)]);
        let (lit, dim) = (
            glow_alpha(&EDGE, &mask(&text, true)),
            glow_alpha(&EDGE, &mask(&text, false)),
        );
        let at = |g: &[u8], x: usize, y: usize| g[y * 150 + x];
        assert_eq!(at(&lit, 75, 14), 191, "over the text, the full 0.75");
        assert_eq!(
            at(&dim, 75, 14),
            217,
            "the full 0.85 for another window, on its paler glass"
        );
        assert!(at(&lit, 76, 14) > 0, "between two strokes");
        assert!(at(&lit, 120, 14) > 0 && at(&lit, 75, 7) > 0, "3 px out");
        assert_eq!(
            at(&lit, 121, 14),
            0,
            "nothing 4 px right of the last stroke"
        );
        assert_eq!(at(&lit, 75, 6), 0, "nothing 4 px above the text");
    }

    #[test]
    fn the_title_glow_runs_on_across_a_word_gap() {
        // Two words 6 px apart: the spread closes the gap before the blur.
        let text = words(&[(30, 70), (76, 120)]);
        let g = glow_alpha(&GLOW, &mask(&text, true));
        for x in 70..76 {
            assert_eq!(g[14 * 150 + x], 122, "x {x} in the gap is at the peak");
        }
    }

    #[test]
    fn the_glow_ends_at_its_reach_at_every_scale() {
        assert_eq!(glow_reach(1), 17, "4 px of spread and 13 of blur");
        for scale in 1..=3 {
            let reach = glow_reach(scale) as usize;
            let (w, h) = (2 * reach + 40, 30 * scale as usize);
            let mut text = vec![0u8; w * h];
            for y in h / 3..2 * h / 3 {
                for x in reach..reach + 40 {
                    text[y * w + x] = 255;
                }
            }
            let g = glow_alpha(
                &GLOW,
                &GlowMask {
                    coverage: &text,
                    width: w,
                    height: h,
                    rows: (0, h),
                    active: true,
                    scale,
                },
            );
            for y in 0..h {
                assert_eq!(
                    g[y * w],
                    0,
                    "scale {scale}: row {y} is clear at the left edge"
                );
                assert_eq!(g[y * w + w - 1], 0, "scale {scale}: and at the right edge");
            }
            let first = (0..w).position(|x| g[h / 2 * w + x] > 0).unwrap();
            // The last steps of the blur's tail round to 0.
            assert!(
                first <= 3 * scale as usize,
                "scale {scale}: the glow reaches within 3 px of its edge, lit from {first}"
            );
        }
    }

    #[test]
    fn three_boxes_spread_like_the_gaussian() {
        for sigma in [4.5, 5.0, 9.0, 10.0, 15.0] {
            let var: f32 = box_radii(sigma)
                .iter()
                .map(|&r| ((2 * r + 1).pow(2) - 1) as f32 / 12.0)
                .sum();
            assert!(
                (var.sqrt() - sigma).abs() < 0.2,
                "sigma {sigma}: the boxes spread {}",
                var.sqrt()
            );
        }
    }

    /// The (2r+1)-square dilation, each output the maximum of its window.
    fn dilate_naive(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
        let mut out = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                    for xx in x.saturating_sub(r)..(x + r + 1).min(w) {
                        out[y * w + x] = f32::max(out[y * w + x], src[yy * w + xx]);
                    }
                }
            }
        }
        out
    }

    /// Two words 80 px apart, so the passes down skip the blank columns between them.
    #[test]
    fn the_glow_of_two_words_far_apart_keeps_its_pixels() {
        let (w, h) = (150, 30);
        let mut m = vec![0u8; w * h];
        for y in 10..19 {
            for x in (2..30).step_by(3) {
                m[y * w + x] = 255;
            }
            for x in (110..140).step_by(2) {
                m[y * w + x] = 180;
            }
        }
        let pins = [
            (&GLOW, true, 1, 0x83ab99027f25b786),
            (&GLOW, true, 2, 0x82ba80b9f956da60),
            (&GLOW, false, 1, 0xf00edb05f30cb9a5),
            (&GLOW, false, 2, 0x860999805b5335de),
            (&EDGE, true, 1, 0xcaca423e4264463e),
            (&EDGE, true, 2, 0x81ff1a6a36e333aa),
            (&EDGE, false, 1, 0x4b4c08a6667ba05e),
            (&EDGE, false, 2, 0xc60acb970c76a39e),
            (&HALO, true, 1, 0x5e33b77d9b7bf04f),
            (&HALO, true, 2, 0x058c3f450382a4da),
        ];
        for (shape, active, scale, pin) in pins {
            let g = glow_alpha(
                shape,
                &GlowMask {
                    coverage: &m,
                    width: w,
                    height: h,
                    rows: (2, 28),
                    active,
                    scale,
                },
            );
            let fnv = g.iter().fold(0xcbf29ce484222325u64, |f, &b| {
                (f ^ u64::from(b)).wrapping_mul(0x100000001b3)
            });
            assert_eq!(
                fnv, pin,
                "spread {} active {active} scale {scale}",
                shape.spread
            );
        }
    }

    #[test]
    fn a_byte_rounds_half_away_from_zero_as_f32_round_does() {
        for k in 0..=255u8 {
            let k = f32::from(k);
            for v in [
                k,
                k + 0.5,
                (k + 0.5).next_down(),
                (k + 0.5).next_up(),
                k.next_up(),
            ] {
                if v <= 255.0 {
                    assert_eq!(round_byte(v), v.round() as u8, "{v}");
                }
            }
        }
        for i in 0..=1_000_000u32 {
            let v = i as f32 * 255.0 / 1_000_000.0;
            assert_eq!(round_byte(v), v.round() as u8, "{v}");
        }
    }

    #[test]
    fn the_running_dilation_is_the_window_maximum() {
        let (w, h) = (37, 11);
        let src: Vec<f32> = (0..w * h)
            .map(|i| ((i * 7919) % 101) as f32 / 100.0)
            .collect();
        for r in [1, 3, 4, 8, 20] {
            let want = dilate_naive(&src, w, h, r);
            let mut got = src.clone();
            let mut t = Lines::default();
            for y in 0..h {
                t.on_row(&mut got, w, y, r, |l, t| t.dilate(l, r));
            }
            t.dilate_down(&mut got, w, r);
            assert_eq!(got, want, "radius {r}");
        }
    }

    /// [`glow_alpha`] with a true Gaussian of the state's sigma (4 sigma
    /// wide) in place of the three boxes.
    fn glow_gaussian(shape: &GlowShape, m: &GlowMask) -> Vec<u8> {
        let (w, h) = (m.width, m.height);
        let (peak, sigma) = shape.states[usize::from(!m.active)];
        let sigma = sigma * m.scale as f32;
        let text: Vec<f32> = m.coverage.iter().map(|&c| c as f32 / 255.0).collect();
        let d = dilate_naive(&text, w, h, (shape.spread * m.scale) as usize);
        let r = (4.0 * sigma + 0.5) as i32;
        let k: Vec<f32> = (-r..=r)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum: f32 = k.iter().sum();
        let blur = |src: &[f32], dx: usize, dy: usize| -> Vec<f32> {
            let mut out = vec![0.0; w * h];
            for y in 0..h as i32 {
                for x in 0..w as i32 {
                    let mut acc = 0.0;
                    for (j, kv) in (-r..=r).zip(&k) {
                        let (xx, yy) = (x + j * dx as i32, y + j * dy as i32);
                        if (0..w as i32).contains(&xx) && (0..h as i32).contains(&yy) {
                            acc += kv * src[yy as usize * w + xx as usize];
                        }
                    }
                    out[y as usize * w + x as usize] = acc / sum;
                }
            }
            out
        };
        let b = blur(&blur(&d, 1, 0), 0, 1);
        (0..w * h)
            .map(|i| {
                let y = i / w;
                if (m.rows.0..m.rows.1).contains(&y) {
                    (peak * (shape.gain * b[i]).min(1.0) * 255.0).round() as u8
                } else {
                    0
                }
            })
            .collect()
    }

    #[test]
    fn the_box_glow_stays_within_2_percent_of_the_gaussian_one() {
        for spans in [&[(30, 120)][..], &[(30, 70), (76, 120)], &[(40, 43)]] {
            let text = words(spans);
            for active in [true, false] {
                let m = mask(&text, active);
                let (got, want) = (glow_alpha(&GLOW, &m), glow_gaussian(&GLOW, &m));
                let worst = got
                    .iter()
                    .zip(&want)
                    .map(|(&a, &b)| (a as i32 - b as i32).abs())
                    .max()
                    .unwrap();
                assert!(worst <= 5, "{spans:?} active {active}: off by {worst}/255");
            }
        }
    }

    #[test]
    fn a_glow_is_worked_out_once_per_title_and_state() {
        let mut cache = GlowCache::new();
        let mut computed = 0;
        let mut get = |cache: &mut GlowCache, m: &GlowMask| {
            cache.get(m, |m| {
                computed += 1;
                glow_alpha(&GLOW, m)
            })
        };
        let (a, b) = (words(&[(30, 120)]), words(&[(30, 90)]));
        let first = get(&mut cache, &mask(&a, true));
        let again = get(&mut cache, &mask(&a, true));
        assert!(
            Arc::ptr_eq(&first, &again),
            "the same title gets the kept glow"
        );
        get(&mut cache, &mask(&a, false));
        get(&mut cache, &mask(&b, true));
        let maximized = GlowMask {
            rows: (0, 20),
            ..mask(&a, true)
        };
        get(&mut cache, &maximized);
        get(&mut cache, &mask(&a, true));
        assert_eq!(computed, 4, "one per title, state and row span");
        assert_eq!(cache.entries.len(), 4);

        // Past its capacity the least recently used glow goes.
        let others: Vec<Vec<u8>> = (0..GLOW_CACHE_CAP)
            .map(|i| words(&[(10 + i, 20 + i)]))
            .collect();
        for o in &others {
            cache.get(&mask(o, true), |m| glow_alpha(&GLOW, m));
        }
        assert_eq!(cache.entries.len(), GLOW_CACHE_CAP);
        let mut recomputed = false;
        cache.get(&mask(&a, true), |m| {
            recomputed = true;
            glow_alpha(&GLOW, m)
        });
        assert!(recomputed, "the oldest glow was dropped");
    }

    #[test]
    fn every_taskbar_part_has_its_size() {
        let s = Settings::default();
        let states = |p: PartImage| (p.states.len(), p.atlas.width, p.atlas.height);
        assert_eq!(
            states(bar::taskbar(&s, 40, 1)),
            (1, 106, 40),
            "the bar: left end and middle"
        );
        assert_eq!(states(start_button(&s, 1)), (3, 54, 3 * 40));
        assert_eq!(
            bar::tray_area(&s, 40, 1).sizing,
            Insets::new(bar::TRAY_RAMP, 2, 0, 0)
        );
    }

    #[test]
    fn every_small_bar_part_has_its_size() {
        let parts = small_bar(&Settings::default(), 1);
        let states = |p: &PartImage| (p.states.clone(), p.atlas.width, p.atlas.height);
        let (n, h, c, ch, p) = (
            State::Normal,
            State::Hot,
            State::Checked,
            State::CheckedHot,
            State::Pressed,
        );
        let want = [
            (Part::Taskbar, (vec![n], 106u32, 30u32)),
            (Part::StartButton, (vec![n, h, p], 54, 90)),
            (Part::StartAbove, (vec![n, h, p], 54, 39)),
            (Part::TaskButton, (vec![n, h, c, ch, p], 44, 150)),
            (Part::TaskStack, (vec![n, n, c, c], 16, 30)),
            (Part::TrayArea, (vec![n], bar::TRAY_RAMP as u32 + 2, 30)),
            (Part::ShowDesktop, (vec![n, h], 15, 60)),
            (Part::SlotEdge, (vec![h, p], 2, 60)),
            (Part::SlotLight, (vec![h, p], 44, 14)),
            (Part::SlotBand, (vec![p], 1, 30)),
            (Part::PagerTile, (vec![h, c, ch], 44, 90)),
        ];
        for ((part, img), (wpart, w)) in parts.iter().zip(want) {
            assert_eq!(*part, wpart);
            assert_eq!(states(img), w, "{part:?}: its frames");
        }
        assert_eq!(parts[3].1.sizing, Insets::new(4, 3, 4, 3));
        let (last, tile) = parts.last().unwrap();
        assert_eq!(
            (*last, tile.sizing),
            (Part::PagerTile, Insets::new(4, 3, 4, 3))
        );
        assert_eq!(parts[5].1.sizing, Insets::new(bar::TRAY_RAMP, 2, 0, 0));
    }

    /// A 16x16 icon: its left half `left` at `alpha`, its right half
    /// opaque `right`.
    fn two_halves(left: Color, alpha: u8, right: Color) -> Canvas {
        let mut c = Canvas::new(16, 16);
        let pre = |v: u8| (u32::from(v) * u32::from(alpha) + 127) / 255;
        let l = u32::from(alpha) << 24 | pre(left.r) << 16 | pre(left.g) << 8 | pre(left.b);
        for y in 0..16 {
            for x in 0..8 {
                c.put_raw(x, y, l);
            }
        }
        c.fill_rect(Rect::new(8, 0, 8, 16), right);
        c
    }

    #[test]
    fn an_icons_colour_is_the_mean_of_its_saturated_opaque_pixels() {
        let grey = Color::rgb(0x80, 0x80, 0x80);
        let red = Color::rgb(0xFF, 0, 0);
        assert_eq!(dominant_colour(&two_halves(grey, 255, grey)), None);
        assert_eq!(
            dominant_colour(&two_halves(Color::rgb(0xC0, 0xA0, 0xA0), 255, grey)),
            None,
            "a pink under 0.25 saturation counts as grey"
        );
        assert_eq!(
            dominant_colour(&two_halves(red, 255, grey)),
            Some(red),
            "half red, half grey"
        );
        assert_eq!(
            dominant_colour(&two_halves(red, 199, grey)),
            None,
            "a red under alpha 200 does not count"
        );
        assert_eq!(
            dominant_colour(&two_halves(red, 220, grey)),
            Some(red),
            "a mostly opaque red counts at its own colour"
        );
        // Red at saturation 1 and pale red (0xFF, 0x80, 0x80) at about
        // 0.5: the mean leans two to one toward the red.
        let pale = Color::rgb(0xFF, 0x80, 0x80);
        assert_eq!(
            dominant_colour(&two_halves(red, 255, pale)),
            Some(Color::rgb(0xFF, 0x2B, 0x2B))
        );
    }

    #[test]
    fn an_opaque_bar_is_opaque_and_its_parts_draw_over_it() {
        let s = Settings {
            transparency: false,
            ..Settings::default()
        };
        let big = bar::taskbar(&s, 40, 1);
        for (x, y) in [(0, 0), (50, 1), (20, 20), (105, 39)] {
            assert_eq!(big.atlas.get(x, y) >> 24, 0xFF, "({x}, {y})");
        }
        assert_eq!(
            big.atlas.get(105, 20),
            0xFFB2_CCE7,
            "plain glass is (179, 207, 232)"
        );
        let small = bar::taskbar(&s, 30, 1);
        for (x, y) in [(0, 0), (50, 1), (20, 15), (105, 29)] {
            assert_eq!(small.atlas.get(x, y) >> 24, 0xFF, "small ({x}, {y})");
        }
        let small_strip = bar::show_desktop(&s, 30, 1);
        for (x, y) in [(0, 0), (7, 15), (14, 59)] {
            assert_eq!(
                small_strip.atlas.get(x, y) >> 24,
                0xFF,
                "small strip ({x}, {y})"
            );
        }
    }

    #[test]
    fn a_cut_corner_takes_no_click_and_the_outline_beside_it_does() {
        let style = crate::glass::style(&Settings::default());
        let top = Rect::new(0, 0, 500, 30);
        for state in [State::Normal, State::Inactive] {
            let at = |x, y| style.opaque_at(Part::Caption, state, top, 1, x, y);
            assert!(!at(4, 0) && at(5, 0), "top row: 5 px cut");
            assert!(!at(0, 5) && at(0, 6), "left column: 6 rows cut");
            assert!(!at(495, 0) && at(494, 0), "top-right");
        }
        let bottom = Rect::new(0, 300, 500, 8);
        let at = |x, y| style.opaque_at(Part::FrameBottom, State::Normal, bottom, 1, x, y);
        assert!(!at(4, 307) && at(5, 307), "bottom-left");
        assert!(!at(499, 302) && at(499, 301), "bottom-right");
    }

    /// Composite channel `shift` of premultiplied `px` over grey 128.
    fn over_grey(px: u32, shift: u32) -> i32 {
        let a = (px >> 24) as i32;
        ((px >> shift) & 0xFF) as i32 + ((255 - a) * 128 + 127) / 255
    }

    /// Ruby at the intensity it comes at.
    fn ruby(transparency: bool) -> Settings {
        let w = crate::glass::SWATCHES[7];
        assert_eq!(w.name, "ruby");
        Settings {
            color: w.color,
            intensity: w.intensity,
            transparency,
            ..Settings::default()
        }
    }

    #[test]
    fn a_ruby_caption_is_red_glass_with_the_same_lines_and_glyphs() {
        let s = ruby(true);
        let (sky, red) = (caption(&Settings::default(), 1), caption(&s, 1));
        let [r, g, b] = rgb_over_grey(px(&red, State::Normal, END + 1, 10));
        assert!(
            r > g + 60 && r > b + 60,
            "red glass over grey: ({r}, {g}, {b})"
        );
        let [r, g, b] = rgb_over_grey(px(&red, State::Inactive, END + 1, 10));
        assert!(
            r > b + 20,
            "the inactive glass reddens too: ({r}, {g}, {b})"
        );
        assert_eq!(
            px(&red, State::Normal, 0, 0),
            0,
            "the cut corner stays clear"
        );
        let outline = |c: &PartImage| rgb_over_grey(px(c, State::Normal, 100, 0));
        let (a, b) = (outline(&sky), outline(&red));
        assert!(
            (0..3).all(|i| (a[i] - b[i]).abs() <= 25),
            "the outline stays dark: {a:?} to {b:?}"
        );
    }

    #[test]
    fn opaque_ruby_glass_is_opaque_and_its_corners_clear() {
        let s = ruby(false);
        let c = caption(&s, 1);
        let [r, g, b] = rgb_over_grey(px(&c, State::Normal, END + 1, 10));
        assert!(r > g + 60 && r > b + 60, "opaque red: ({r}, {g}, {b})");
        assert_eq!(px(&c, State::Normal, 0, 0), 0, "the cut corner stays clear");
        for img in [bar::taskbar(&s, 40, 1), bar::show_desktop(&s, 40, 1)] {
            let (w, h) = (img.atlas.width as i32, img.atlas.height as i32);
            for y in 0..h {
                for x in 0..w {
                    assert_eq!(img.atlas.get(x, y) >> 24, 0xFF, "({x}, {y}) is opaque");
                }
            }
        }
    }

    #[test]
    fn the_orb_on_either_bar_takes_the_glass_colour() {
        let (d, s) = (Settings::default(), ruby(true));
        // The orb follows the glass colour's hue.
        assert_ne!(
            start_button(&d, 1).atlas.pixels,
            start_button(&s, 1).atlas.pixels
        );
        let (small_d, small_s) = (small_bar(&d, 1), small_bar(&s, 1));
        for i in [1, 2] {
            assert_ne!(
                small_d[i].1.atlas.pixels, small_s[i].1.atlas.pixels,
                "{:?} on the small bar",
                small_d[i].0
            );
        }
        assert!(small_task_glow(&s).is_some(), "small buttons glow too");
        assert!(task_glow(&s).is_some(), "buttons glow in any glass colour");
    }
}
