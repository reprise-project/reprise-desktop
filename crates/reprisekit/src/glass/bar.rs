//! Reprise Glass's taskbar body, drawn from the glass settings at the bar's height: the bar, the notification area's shade, the veil under labelled buttons, the Show desktop strip, and a lit or held slot's edges, foot light and band.
//!
//! The bar is plain glass ([`tint::plain`]) under a dark top line and a light one. Denser parts (orb, notification area, labelled-row veil, held band, strip frame) cover a fixed share of what the bar lets through in the glass's own colour ([`Bar::shade`]), so they darken Smoked and Molten by the same share as Frosted. Lighter parts (strip sheen and lines, foot light, lit edges) are white or grey laid over as they are. With transparency off the bar and strip are opaque.
//!
//! Parts are drawn for the bar's design height (40 px, or 30 for the small bar); the nine-grid keeps edge rows fixed on other heights. At `d` pixels per design pixel, lines and edges are `d` px wide and shades, bells and sheen are evaluated at pixel centres ([`centre`]).

use super::Settings;
use super::art::glass;
use super::panel::DENSE_CEILING;
use super::tint::{self, Recolour, Surface};
use crate::raster::{Canvas, Insets};
use crate::shape::{Px, centre, over, pack, smooth};
use crate::style::{Layout, PartImage, State};
use crate::theme::Color;

/// The bar's dark top line and the light line under it, premultiplied, in
/// Sky at the default intensity: each runs the bar's width one pixel tall.
/// [`Recolour`] moves them with the glass colour.
pub const BAR_LINES: [u32; 2] = [BAR_TOP_LINE, BAR_LIGHT_LINE];
/// A deep blue-black line, about 70 % opaque.
const BAR_TOP_LINE: u32 = glass(Color::rgb(4, 14, 25), 178);
/// A blue-grey line, about 55 % opaque.
const BAR_LIGHT_LINE: u32 = glass(Color::rgb(196, 215, 237), 139);

/// The bar's plain glass, premultiplied, in Sky at the default intensity:
/// a dark navy at about a quarter of full alpha.
pub const BAR_PLAIN: u32 = glass(Color::rgb(0, 66, 153), 60);

/// The colour the bar lies over, opaque, when transparency is off: a pale
/// sky white.
pub const BAR_OPAQUE_UNDER: [u8; 3] = [233, 246, 255];

/// A layer of grey `ink` (0..255) at `share` (0..1) of full alpha.
fn ink(share: f64, ink: f64) -> Px {
    let share = share.clamp(0.0, 1.0);
    (255.0 * share, [ink * share; 3])
}

pub(super) fn white(share: f64) -> Px {
    ink(share, 255.0)
}

fn black(alpha: f64) -> Px {
    (alpha.clamp(0.0, 255.0), [0.0; 3])
}

// Each shade below is `k`, the share of what the bar lets through that it
// covers: over plain glass of alpha `a` it gives `a + (255 - a)·k`.

/// The glass under the orb: 0.285, full to x 54 and easing out by x 100.
const ORB_SHADE: f64 = 0.285;
const ORB_SHADE_END: (f64, f64) = (54.0, 100.0);

const LEFT: i32 = 104;

/// The notification area's shade: 0.256, rising evenly across
/// [`TRAY_RAMP`] px from its left edge.
const TRAY_SHADE: f64 = 0.256;

/// How far the notification area's shade rises from its left edge before
/// its plateau.
pub const TRAY_RAMP: i32 = 33;

/// The veil under a labelled row: 0.27, rising evenly across
/// [`VEIL_END`] px at either end of the row.
pub(super) const VEIL_SHADE: f64 = 0.27;
pub(super) const VEIL_END: i32 = 42;

/// A held slot's band: 0.414 at its middle row.
const BAND_SHADE: f64 = 0.414;

/// A lit slot's edges: a black column of alpha 72 then a column of grey
/// E8 at alpha 89, each at its strongest at mid-height.
const EDGE_DARK: f64 = 72.0;
const EDGE_LIGHT: (f64, f64) = (89.0, 232.0);

/// A lit slot's foot light: grey C8, whole at the middle of the slot's
/// foot and fading out over an ellipse 17 px across each way and 7.5 px
/// up.
const LIGHT_INK: f64 = 200.0;
const LIGHT_REACH: (f64, f64) = (17.0, 7.5);
const LIGHT_ROWS: i32 = 7;

pub const STRIP_WIDTH: i32 = 15;

/// A bell across `h` rows, 0 in the `inset` rows at each end and 1 at the
/// middle: `sin^1.5` of the way across, at row `y` ([`centre`]).
fn bell(y: f64, h: i32, inset: f64) -> f64 {
    let t = (y + 0.5 - inset) / (f64::from(h) - 2.0 * inset);
    if !(0.0..=1.0).contains(&t) {
        return 0.0;
    }
    (std::f64::consts::PI * t).sin().powf(1.5)
}

pub(super) struct Bar {
    plain: Px,
    lines: [u32; 2],
    finish: Recolour,
}

impl Bar {
    pub(super) fn new(s: &Settings) -> Bar {
        let re = Recolour::new(s, Surface::Taskbar);
        Bar {
            plain: tint::plain(s, Surface::Taskbar),
            lines: BAR_LINES.map(|px| re.px(px)),
            finish: re,
        }
    }

    /// Shade `k` (the share of what the bar lets through it covers), at
    /// most what takes the bar under it to [`DENSE_CEILING`], so the bar
    /// stays translucent however dense the setting.
    fn shade(&self, k: f64) -> f64 {
        capped(self.plain.0, k)
    }

    fn shaded(&self, k: f64) -> Px {
        let (a, p) = self.plain;
        (a + (255.0 - a) * self.shade(k), p)
    }

    pub(super) fn veil(&self, k: f64) -> Px {
        self.veil_over(0.0, k)
    }

    /// Shade `k` as a layer laid over the bar already under shade `under`
    /// (the labelled row's veil, say): over that it covers `k` of what is
    /// let through, at most what takes it to [`DENSE_CEILING`].
    pub(super) fn veil_over(&self, under: f64, k: f64) -> Px {
        let k = capped(self.shaded(under).0, k);
        (255.0 * k, self.plain.1.map(|v| v * k))
    }

    fn put(&self, c: &mut Canvas, x: i32, y: i32, px: Px) {
        c.put_raw(x, y, self.finish.finish(pack(px)));
    }
}

/// Share `k` of what glass of alpha `a` lets through, at most what takes
/// it to [`DENSE_CEILING`].
fn capped(a: f64, k: f64) -> f64 {
    let room = if a < DENSE_CEILING {
        (DENSE_CEILING - a) / (255.0 - a)
    } else {
        0.0
    };
    k.clamp(0.0, room)
}

/// The bar, `h` px tall: two lines, then the plain glass, denser under the orb by [`ORB_SHADE`]. Drawn 106 px wide: the left [`LEFT`] px fixed, the middle stretched, so a side bar keeps the lines crisp. A top bar draws it turned over.
pub fn taskbar(s: &Settings, h: i32, d: i32) -> PartImage {
    let bar = Bar::new(s);
    let w = LEFT + 2;
    let mut c = Canvas::new((w * d) as u32, (h * d) as u32);
    let (from, to) = ORB_SHADE_END;
    for x in 0..w * d {
        let under_orb = ORB_SHADE * (1.0 - smooth((centre(x, d) - from) / (to - from)));
        let glass = bar.shaded(under_orb);
        for y in 0..h * d {
            match y / d {
                0 => c.put_raw(x, y, bar.lines[0]),
                1 => c.put_raw(x, y, bar.lines[1]),
                _ => bar.put(&mut c, x, y, glass),
            }
        }
    }
    PartImage::new(c, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(LEFT, 2, 0, 0))
        .at_scale(d)
}

/// The notification area's shade, clear on the bar's lines, then [`TRAY_SHADE`] rising over [`TRAY_RAMP`] px from its left edge and flat after.
pub fn tray_area(s: &Settings, h: i32, d: i32) -> PartImage {
    let bar = Bar::new(s);
    let w = TRAY_RAMP + 2;
    let mut c = Canvas::new((w * d) as u32, (h * d) as u32);
    for x in 0..w * d {
        let rise = ((centre(x, d) + 1.0) / f64::from(TRAY_RAMP + 1)).clamp(0.0, 1.0);
        let px = pack(bar.veil(TRAY_SHADE * rise));
        for y in 2 * d..h * d {
            c.put_raw(x, y, px);
        }
    }
    PartImage::new(c, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(TRAY_RAMP, 2, 0, 0))
        .at_scale(d)
}

/// The veil under labelled buttons: [`VEIL_SHADE`] rising across a 42 px left end and falling across a 42 px right end, clear on the bar's lines.
pub fn label_row(s: &Settings, h: i32, d: i32) -> PartImage {
    let bar = Bar::new(s);
    let w = 2 * VEIL_END + 1;
    let mut c = Canvas::new((w * d) as u32, (h * d) as u32);
    for x in 0..w * d {
        let x_at = centre(x, d);
        let from_end = x_at.min(f64::from(w - 1) - x_at);
        let rise = ((from_end - 1.0) / f64::from(VEIL_END - 2)).clamp(0.0, 1.0);
        let px = pack(bar.veil(VEIL_SHADE * rise));
        for y in 2 * d..h * d {
            c.put_raw(x, y, px);
        }
    }
    PartImage::new(c, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(VEIL_END, 2, VEIL_END, 0))
        .at_scale(d)
}

/// A held slot's band at row `y` ([`centre`]) of an `h` px bar: up to
/// [`BAND_SHADE`] in the glass's colour, densest at mid-height and clear
/// over about a tenth of the bar at the top and bottom.
fn band(bar: &Bar, y: f64, h: i32) -> Px {
    bar.veil(BAND_SHADE * bell(y, h, f64::from(h) / 10.0 + 0.5))
}

pub fn slot_band(s: &Settings, h: i32, d: i32) -> PartImage {
    let bar = Bar::new(s);
    let mut c = Canvas::new(d as u32, (h * d) as u32);
    for y in 0..h * d {
        let px = pack(band(&bar, centre(y, d), h));
        for x in 0..d {
            c.put_raw(x, y, px);
        }
    }
    PartImage::new(c, &[State::Pressed], Layout::Vertical).at_scale(d)
}

/// The edges of a lit or held slot, 2 px wide: a dark column then a light one, strongest at mid-height, fading 2 px from the bar's top and foot. Held, it lies over the band.
pub fn slot_edge(s: &Settings, h: i32, d: i32) -> PartImage {
    let bar = Bar::new(s);
    let mut c = Canvas::new((2 * d) as u32, (2 * h * d) as u32);
    for y in 0..h * d {
        let y_at = centre(y, d);
        let k = bell(y_at, h, 2.0);
        let (light, grey) = EDGE_LIGHT;
        let lit = [black(EDGE_DARK * k), ink(light * k / 255.0, grey)];
        for x in 0..2 * d {
            let px = lit[(x / d) as usize];
            c.put_raw(x, y, pack(px));
            c.put_raw(x, h * d + y, pack(over(px, band(&bar, y_at, h))));
        }
    }
    PartImage::new(c, &[State::Hot, State::Pressed], Layout::Vertical).at_scale(d)
}

/// The light along a lit or held slot's foot, `w` px wide and [`LIGHT_ROWS`] tall: whole at the middle of the bottom row, fading over an ellipse [`LIGHT_REACH`] across and up. Turned over on a top bar.
pub fn slot_light(w: i32, d: i32) -> PartImage {
    let h = LIGHT_ROWS;
    let mut c = Canvas::new((w * d) as u32, (2 * h * d) as u32);
    let (rx, ry) = LIGHT_REACH;
    for y in 0..h * d {
        for x in 0..w * d {
            // Down the light, within its rows: it ends at its top row
            // and its brightest is its bottom row's middle.
            let y_at = centre(y, d).clamp(0.0, f64::from(h - 1));
            let dx = (centre(x, d) + 0.5 - (f64::from(w / 2) + 0.5)) / rx;
            let dy = (f64::from(h) - y_at - 0.5) / ry;
            let px = pack(ink(1.0 - dx.hypot(dy), LIGHT_INK));
            c.put_raw(x, y, px);
            c.put_raw(x, h * d + y, px);
        }
    }
    PartImage::new(c, &[State::Hot, State::Pressed], Layout::Vertical).at_scale(d)
}

/// The Show desktop strip's dark edge: the plain glass under these shades
/// on its top row, bottom row, left column and right column.
const STRIP_EDGE: [f64; 4] = [0.33, 0.38, 0.49, 0.46];

/// At rest, the strip's inside is the plain glass under shade 0.049 plus
/// up to [`STRIP_DEEPEN`] more on its upper rows, easing out from
/// [`STRIP_DEEPEN_FROM`] of its height to its foot.
const STRIP_SHADE: f64 = 0.049;
const STRIP_DEEPEN: f64 = 0.269;
const STRIP_DEEPEN_FROM: f64 = 0.52;

/// Under the pointer, the strip's inside is the plain glass under white
/// at 0.11, plus up to 0.22 more at its top easing out across
/// [`STRIP_GLOW_ROWS`] of its height.
const STRIP_GLOW: (f64, f64) = (0.11, 0.22);
const STRIP_GLOW_ROWS: f64 = 0.7;

/// The strip's sheen across its top left: white at `0.48 - 0.036·y -
/// 0.014·x`, cut along the slant `x + 1.78·y = 22.5`. Under the light
/// lines it is trimmed to [`SHEEN_ON_LINES`] of that, so the two don't
/// stack to a hot spot in the top left corner.
const SHEEN: (f64, f64, f64) = (0.48, 0.036, 0.014);
const SHEEN_CUT: (f64, f64) = (1.78, 22.5);
const SHEEN_ON_LINES: f64 = 0.8;

/// The strip's light lines inside its dark edge, in white: along the top
/// `0.41 - 0.003·x`, along the foot `0.275 + 0.0105·x`.
const LIGHT_TOP: (f64, f64) = (0.41, -0.003);
const LIGHT_FOOT: (f64, f64) = (0.275, 0.0105);

/// A light line down one side of the strip: `top` on its upper rows,
/// easing to `mid` between rows `fall`, then rising to `foot` from `rise`
/// rows below the middle to `foot_row` rows above the strip's foot.
struct Side {
    top: f64,
    mid: f64,
    foot: f64,
    fall: (f64, f64),
    rise: f64,
    foot_row: f64,
}

const LIGHT_LEFT: Side = Side {
    top: 0.41,
    mid: 0.17,
    foot: 0.24,
    fall: (10.8, 21.9),
    rise: 8.8,
    foot_row: 3.0,
};

const LIGHT_RIGHT: Side = Side {
    top: 0.31,
    mid: 0.12,
    foot: 0.365,
    fall: (5.8, 15.4),
    rise: -1.6,
    foot_row: 4.0,
};

impl Side {
    fn at(&self, y: f64, h: i32) -> f64 {
        let h = f64::from(h);
        let (f0, f1) = self.fall;
        let (r0, r1) = (h / 2.0 + self.rise, h - self.foot_row);
        self.mid
            + (self.top - self.mid) * (1.0 - smooth((y - f0) / (f1 - f0)))
            + (self.foot - self.mid) * smooth((y - r0) / (r1 - r0).max(1.0))
    }
}

fn strip_frame(bar: &Bar, c: &mut Canvas, top: i32, h: i32, hot: bool, d: i32) {
    let w = STRIP_WIDTH;
    let hf = f64::from(h);
    let inside = |y: f64| {
        if hot {
            let (base, more) = STRIP_GLOW;
            let glow = base + more * (1.0 - smooth((y - 2.0) / (hf * STRIP_GLOW_ROWS)));
            over(white(glow), bar.plain)
        } else {
            let from = hf * STRIP_DEEPEN_FROM;
            let deepen = 1.0 - smooth((y - from) / (hf - 2.0 - from));
            bar.shaded(STRIP_SHADE + STRIP_DEEPEN * deepen)
        }
    };
    for y in 0..h * d {
        let row = y / d;
        for x in 0..w * d {
            let col = x / d;
            let [t, b, l, r] = STRIP_EDGE;
            let edge = match (col, row) {
                (_, 0) => Some(t),
                (_, y) if y == h - 1 => Some(b),
                (0, _) => Some(l),
                (x, _) if x == w - 1 => Some(r),
                _ => None,
            };
            let px = match edge {
                Some(share) => bar.shaded(share),
                None => {
                    // A light line is a design pixel wide: along it the
                    // pixel's middle, across it its design pixel's.
                    let (xf, yf) = (centre(x, d), centre(y, d));
                    let (light, xf, yf) = if row == 1 {
                        (Some(LIGHT_TOP.0 + LIGHT_TOP.1 * xf), xf, 1.0)
                    } else if row == h - 2 {
                        (Some(LIGHT_FOOT.0 + LIGHT_FOOT.1 * xf), xf, f64::from(h - 2))
                    } else if col == 1 {
                        (Some(LIGHT_LEFT.at(yf, h)), 1.0, yf)
                    } else if col == w - 2 {
                        (Some(LIGHT_RIGHT.at(yf, h)), f64::from(w - 2), yf)
                    } else {
                        (None, xf, yf)
                    };
                    let (s0, sy, sx) = SHEEN;
                    let (slant, cut) = SHEEN_CUT;
                    let trim = if light.is_some() { SHEEN_ON_LINES } else { 1.0 };
                    let sheen = trim
                        * (s0 - sy * yf - sx * xf).max(0.0)
                        * smooth(cut - (xf + slant * yf) + 0.5);
                    let glass = over(white(sheen), inside(yf));
                    over(white(light.unwrap_or(0.0)), glass)
                }
            };
            bar.put(c, x, top + y, px);
        }
    }
}

/// The Show desktop strip, [`STRIP_WIDTH`] x `h`, at rest and under the pointer: a dark edge round a glass panel framed by a light line, with a sheen across its top left. At rest the glass is denser than the bar's, most on its upper rows; under the pointer it is lit white from the top. Each frame includes the bar's glass, so it replaces the bar; a top bar turns it over.
pub fn show_desktop(s: &Settings, h: i32, d: i32) -> PartImage {
    let bar = Bar::new(s);
    let mut c = Canvas::new((STRIP_WIDTH * d) as u32, (2 * h * d) as u32);
    strip_frame(&bar, &mut c, 0, h, false, d);
    strip_frame(&bar, &mut c, h * d, h, true, d);
    PartImage::new(c, &[State::Normal, State::Hot], Layout::Vertical)
        .sizing(Insets::new(0, 3, 0, 2))
        .at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Material, SWATCHES};

    fn stack(top: u32, under: u32) -> u32 {
        let mut c = Canvas::new(1, 1);
        c.put_raw(0, 0, under);
        let mut o = Canvas::new(1, 1);
        o.put_raw(0, 0, top);
        c.blit_over(0, 0, &o);
        c.get(0, 0)
    }

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

    /// `s`'s shades at mid-height, each with its `k`: the tray, veil and
    /// band laid over the plain bar, and the glass under the orb and the
    /// strip's left edge, which are whole bar pixels, as they are.
    fn shades_over_the_bar(s: &Settings) -> [(&'static str, f64, u32); 5] {
        let bar = taskbar(s, 40, 1).atlas.get(LEFT + 1, 20);
        [
            (
                "tray",
                TRAY_SHADE,
                stack(tray_area(s, 40, 1).atlas.get(TRAY_RAMP + 1, 20), bar),
            ),
            (
                "veil",
                VEIL_SHADE,
                stack(label_row(s, 40, 1).atlas.get(VEIL_END, 20), bar),
            ),
            (
                "band",
                BAND_SHADE,
                stack(slot_band(s, 40, 1).atlas.get(0, 19), bar),
            ),
            (
                "under the orb",
                ORB_SHADE,
                taskbar(s, 40, 1).atlas.get(20, 20),
            ),
            (
                "strip edge",
                STRIP_EDGE[2],
                show_desktop(s, 40, 1).atlas.get(0, 20),
            ),
        ]
    }

    #[test]
    fn every_shade_covers_the_same_share_of_what_the_bar_lets_through() {
        // Each covers a fixed share of the bar's remaining transmission,
        // in the glass's own colour, up to where the bar under it reaches
        // the ceiling. Intensity 100 takes the denser ones there. The bar's
        // alpha and the stacked shade are each rounded, hence 1.5.
        let mut capped = 0;
        for s in settings() {
            let bar = taskbar(&s, 40, 1).atlas.get(LEFT + 1, 20);
            let a = f64::from(bar >> 24);
            for (what, k, px) in shades_over_the_bar(&s) {
                let what = format!("{what}, {:?} {:?} at {}", s.color, s.material, s.intensity);
                let got = f64::from(px >> 24);
                let want = a + (255.0 - a) * k;
                if want <= DENSE_CEILING {
                    assert!(
                        (got - want).abs() <= 1.5,
                        "{what}: covers {k} of what the bar lets through, alpha {want:.1} within 1.5, got {got}"
                    );
                } else {
                    capped += 1;
                    assert!(
                        (got - DENSE_CEILING).abs() <= 1.0,
                        "{what}: the bar under it stops at {DENSE_CEILING}, got {got}"
                    );
                }
                for sh in [16, 8, 0] {
                    let (g, b) = ((px >> sh) & 0xFF, (bar >> sh) & 0xFF);
                    assert!(
                        g.abs_diff(b) <= 1,
                        "{what}: keeps the glass's colour, {bar:#010x} within a level, got {px:#010x}"
                    );
                }
            }
        }
        assert!(capped > 0, "some shades reach the ceiling at intensity 100");
    }

    #[test]
    fn the_shades_follow_the_material_darker_over_smoked_lighter_over_molten() {
        let alpha = |material| {
            let s = Settings {
                material,
                ..Settings::default()
            };
            shades_over_the_bar(&s)[0].2 >> 24
        };
        let (frosted, smoked, molten) = (
            alpha(Material::Frosted),
            alpha(Material::Smoked),
            alpha(Material::Molten),
        );
        assert!(
            molten < frosted && frosted < smoked,
            "the tray over the bar, alpha {molten} < {frosted} < {smoked}"
        );
    }

    #[test]
    fn a_smoked_tray_stays_lighter_than_black() {
        for intensity in [42, 75] {
            let s = Settings {
                material: Material::Smoked,
                intensity,
                ..Settings::default()
            };
            let tray = shades_over_the_bar(&s)[0].2 >> 24;
            assert!(
                tray < 240,
                "Smoked at {intensity}: the tray over the bar under alpha 240, got {tray}"
            );
        }
    }

    #[test]
    fn with_transparency_off_the_bar_and_strip_are_opaque() {
        for s in settings() {
            let s = Settings {
                transparency: false,
                ..s
            };
            for h in [40, 30] {
                for d in [1, 2] {
                    for img in [taskbar(&s, h, d), show_desktop(&s, h, d)] {
                        assert!(
                            img.atlas.pixels.iter().all(|&px| px >> 24 == 0xFF),
                            "{:?} {:?} at {}, {h} px at scale {d}: every pixel opaque",
                            s.color,
                            s.material,
                            s.intensity
                        );
                    }
                }
                let bar = taskbar(&s, h, 1).atlas.get(LEFT + 1, 20);
                let tray = tray_area(&s, h, 1).atlas.get(TRAY_RAMP + 1, 20);
                assert_eq!(
                    stack(tray, bar) >> 24,
                    0xFF,
                    "the tray over it stays opaque"
                );
            }
        }
    }

    #[test]
    fn with_transparency_on_the_bar_and_strip_stay_translucent() {
        for s in settings() {
            for h in [40, 30] {
                for d in [1, 2] {
                    for img in [taskbar(&s, h, d), show_desktop(&s, h, d)] {
                        let densest = img.atlas.pixels.iter().map(|&px| px >> 24).max();
                        assert!(
                            densest < Some(0xFF),
                            "{:?} {:?} at {}, {h} px at scale {d}: densest {densest:?}",
                            s.color,
                            s.material,
                            s.intensity
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_part_is_drawn_at_the_bars_height() {
        let s = Settings::default();
        let size = |p: &PartImage| (p.atlas.width as i32, p.atlas.height as i32, p.states.len());
        for (h, button) in [(40, 60), (30, 44)] {
            assert_eq!(size(&taskbar(&s, h, 1)), (LEFT + 2, h, 1), "{h} px bar");
            assert_eq!(
                size(&tray_area(&s, h, 1)),
                (TRAY_RAMP + 2, h, 1),
                "{h} px tray"
            );
            assert_eq!(
                size(&label_row(&s, h, 1)),
                (2 * VEIL_END + 1, h, 1),
                "{h} px veil"
            );
            assert_eq!(size(&slot_band(&s, h, 1)), (1, h, 1), "{h} px band");
            assert_eq!(size(&slot_edge(&s, h, 1)), (2, 2 * h, 2), "{h} px edges");
            assert_eq!(
                size(&slot_light(button, 1)),
                (button, 2 * LIGHT_ROWS, 2),
                "{h} px light"
            );
            assert_eq!(
                size(&show_desktop(&s, h, 1)),
                (STRIP_WIDTH, 2 * h, 2),
                "{h} px strip"
            );
        }
        // The two top lines stay fixed when the bar is drawn at another
        // height; the glass under them stretches.
        assert_eq!(taskbar(&s, 40, 1).sizing, Insets::new(LEFT, 2, 0, 0));
    }

    #[test]
    fn the_bar_keeps_its_lines_and_eases_out_of_the_glass_under_the_orb() {
        let s = Settings::default();
        let bar = taskbar(&s, 40, 1);
        let at = |x, y| bar.atlas.get(x, y);
        assert_eq!(at(50, 0), BAR_LINES[0], "the dark line");
        assert_eq!(at(50, 1), BAR_LINES[1], "the light line");
        assert_eq!(at(LEFT + 1, 20), BAR_PLAIN, "the plain glass");
        let alpha = |x| at(x, 20) >> 24;
        let (a, k) = (f64::from(BAR_PLAIN >> 24), ORB_SHADE);
        let want = (a + (255.0 - a) * k).round() as u32;
        assert_eq!(alpha(20), want, "denser glass under the orb, by {k}");
        assert!(
            (55..101).all(|x| alpha(x) >= alpha(x + 1)),
            "it eases out from x 54 to 100"
        );
        assert_eq!(alpha(100), alpha(LEFT + 1), "and is plain by x 100");
    }

    #[test]
    fn a_held_slots_edge_lies_over_its_band_and_its_light_is_brightest_at_the_middle_of_its_foot() {
        let s = Settings::default();
        let edge = slot_edge(&s, 40, 1);
        let band = slot_band(&s, 40, 1).atlas.get(0, 20);
        let (hot, held) = (edge.frame(State::Hot), edge.frame(State::Pressed));
        for x in 0..2 {
            let got = edge.atlas.get(held.x + x, held.y + 20);
            let want = stack(edge.atlas.get(hot.x + x, hot.y + 20), band);
            for sh in [24, 16, 8, 0] {
                let (g, w) = ((got >> sh) & 0xFF, (want >> sh) & 0xFF);
                assert!(
                    g.abs_diff(w) <= 1,
                    "column {x}: the lit edge over the band, {want:#010x} within a level, got {got:#010x}"
                );
            }
        }
        let light = slot_light(60, 1);
        let peak = light.atlas.get(30, LIGHT_ROWS - 1) >> 24;
        assert!(peak > 230, "near whole at the middle of its foot: {peak}");
        assert!(light.atlas.get(30, 0) >> 24 < 40, "faint at its top");
        assert_eq!(
            light.atlas.get(0, LIGHT_ROWS - 1),
            0,
            "clear past its reach"
        );
    }

    use crate::style::{Part, Style};
    use std::sync::Arc;

    const BAR_PARTS: [Part; 12] = [
        Part::Taskbar,
        Part::TrayArea,
        Part::LabelRow,
        Part::SlotBand,
        Part::SlotEdge,
        Part::SlotLight,
        Part::ShowDesktop,
        Part::TaskButton,
        Part::TaskStack,
        Part::TaskLabel,
        Part::TaskLabelEdge,
        Part::PagerTile,
    ];

    fn bar_styles(s: &Settings) -> [(i32, Arc<Style>); 2] {
        [
            (40, crate::glass::style(s)),
            (30, crate::glass::small_bar_style(s)),
        ]
    }

    #[test]
    fn the_style_at_scale_2_paints_the_bar_and_the_orb_at_2() {
        for (h, style) in bar_styles(&Settings::default()) {
            let two = style.at_scale(2);
            for part in BAR_PARTS {
                let (Some(one), Some(img)) = (style.part(part), two.part(part)) else {
                    continue;
                };
                assert_eq!((one.scale, img.scale), (1, 2), "{h} px {part:?}");
                assert_eq!(
                    (img.atlas.width, img.atlas.height),
                    (2 * one.atlas.width, 2 * one.atlas.height),
                    "{h} px {part:?}: twice the atlas"
                );
                assert_eq!(img.sizing, one.sizing, "{h} px {part:?}: design px");
            }
            for part in [Part::StartButton, Part::StartAbove] {
                assert_eq!(two.part(part).unwrap().scale, 2, "{h} px {part:?}");
            }
            let spot = two.task_spot().unwrap();
            assert_eq!((spot.button.base.scale, spot.button.gain.scale), (2, 2));
        }
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass).scaled(2);
        let scaled = theme.look.style().unwrap();
        assert_eq!(
            scaled.part(Part::Taskbar).unwrap().scale,
            2,
            "a scaled theme's"
        );
    }

    #[test]
    fn the_bars_parts_at_scale_2_are_their_scale_1_design_twice_over() {
        // The bar, the tray, the veil, the band, the edges, the foot light
        // and the strip, each frame drawn at 1 and painted and drawn at 2,
        // as `part_twice_over` holds them; the stretching ones wider than
        // their art.
        use crate::look::styled::fixture::part_twice_over;
        let rows = [
            Settings::default(),
            Settings {
                color: SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color,
                material: Material::Molten,
                intensity: 100,
                ..Settings::default()
            },
        ];
        for s in rows {
            for (h, button) in [(40, 60), (30, 44)] {
                type Painter = fn(&Settings, i32, i32) -> PartImage;
                let parts: [(&str, Painter, i32); 6] = [
                    ("bar", taskbar, 300),
                    ("tray", tray_area, 120),
                    ("veil", label_row, 200),
                    ("band", slot_band, 17),
                    ("edges", slot_edge, 2),
                    ("strip", show_desktop, STRIP_WIDTH),
                ];
                for (name, part, w) in parts {
                    let what = format!("{:?} {h} px {name}", s.material);
                    let (one, two) = (part(&s, h, 1), part(&s, h, 2));
                    let got = part_twice_over(&one, &two, (w, h), |_, _, _, _| false);
                    assert_eq!(got, Ok(()), "{what}");
                }
                let (one, two) = (slot_light(button, 1), slot_light(button, 2));
                let got = part_twice_over(&one, &two, (button, LIGHT_ROWS), |_, _, _, _| false);
                assert_eq!(got, Ok(()), "{:?} {h} px foot light", s.material);
            }
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
    fn the_bars_parts_at_scale_2_keep_their_pixels_at_sky() {
        // Each of the bar's own parts at scale 2 as an FNV-1a hash, the
        // 40 px bar's then the 30 px one's: the bar, the tray, the veil,
        // the band, the edges, the foot light and the strip.
        let s = Settings::default();
        let got = [(40, 60), (30, 44)].map(|(h, button)| {
            [
                taskbar(&s, h, 2),
                tray_area(&s, h, 2),
                label_row(&s, h, 2),
                slot_band(&s, h, 2),
                slot_edge(&s, h, 2),
                slot_light(button, 2),
                show_desktop(&s, h, 2),
            ]
            .map(|img| fnv(&img))
        });
        let want: [[u64; 7]; 2] = [
            [
                0x8B6C_3C1E_0008_98C5,
                0xAA75_4ACD_F116_504D,
                0x4FD3_CB86_E0D2_2B15,
                0x3346_87F0_4F50_C88D,
                0x63CA_E354_E03A_A51D,
                0xDBD0_91E8_856F_93F5,
                0x364F_0C29_AD81_54C7,
            ],
            [
                0x6A49_FCD7_02BC_C825,
                0x1084_5139_518A_6035,
                0xF40B_D5AE_EB53_1BC5,
                0xCB94_C4C1_FF3E_E68D,
                0xE033_CE2C_76D0_5A05,
                0xDE90_A15C_1F59_0BF5,
                0x28B6_D09D_E28B_1733,
            ],
        ];
        assert_eq!(got, want, "{got:#018X?}");
    }
}
