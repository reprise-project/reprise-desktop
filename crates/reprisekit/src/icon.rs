//! Window icons: conversion of 32 bpp DRM-format pixel data into an `IconSource` kept at its native size, box resampling to the pixel size the chrome needs, and the window glyph shown when a client provides no icon. Reading pixels out of buffers is the compositor's job.

use crate::raster::{Canvas, Rect};
use crate::theme::{Color, ColorScheme, Theme};
use std::borrow::Cow;

/// Logical icon edge; multiplied by the output scale through
/// `Theme::icon_size`.
pub const ICON_SIZE: i32 = 16;

/// Source size we ask clients for and prefer when several are offered:
/// downscaling 32 to 16 with a box filter looks better than upscaling 16,
/// and 32 covers scale 2 exactly.
pub const PREFERRED_SOURCE: i32 = 32;

// `DRM_FORMAT_*` values from <libdrm/drm_fourcc.h> (little-endian byte
// order); libdrm defines them through the function-like `fourcc_code()`
// macro, so bindgen never emits them as constants.
pub const DRM_FORMAT_ARGB8888: u32 = 0x3432_5241;
pub const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;
pub const DRM_FORMAT_XBGR8888: u32 = 0x3432_4258;
pub const DRM_FORMAT_ABGR8888: u32 = 0x3432_4241;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconSource {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl From<Canvas> for IconSource {
    fn from(c: Canvas) -> IconSource {
        IconSource {
            width: c.width,
            height: c.height,
            pixels: c.pixels,
        }
    }
}

impl IconSource {
    pub fn to_canvas(&self, size: i32) -> Canvas {
        let size = size.max(1) as u32;
        let sw = self.width as usize;
        Canvas::resample_box_from(self.width, self.height, size, size, |x, y| {
            self.pixels[y as usize * sw + x as usize]
        })
    }
}

/// Whether a candidate of edge `w` beats the best edge seen so far when
/// hunting for `PREFERRED_SOURCE`. Ties keep the earlier candidate.
pub fn closer_to_target(w: i32, best: Option<i32>) -> bool {
    match best {
        None => true,
        Some(b) => (w - PREFERRED_SOURCE).abs() < (b - PREFERRED_SOURCE).abs(),
    }
}

/// Largest side a client-supplied icon (xdg-toplevel-icon, _NET_WM_ICON)
/// may have: a client picks the size, and the copy runs on the main loop.
pub const MAX_CLIENT_ICON: usize = 512;

/// Convert a raw `w`x`h` pixel buffer (row pitch `stride` bytes) in one of
/// the four DRM_FORMAT_* 32 bpp layouts into premultiplied ARGB u32s.
/// X formats (no alpha channel) get alpha 0xFF; BGR variants swap R and B.
/// A colour channel above its alpha clamps to the alpha, so the blits'
/// source-over sums stay within 255.
/// Returns `None` for an unsupported format, a buffer too short for the
/// claimed dimensions, or a side past `MAX_CLIENT_ICON`.
pub fn convert(pixels: &[u8], w: usize, h: usize, stride: usize, format: u32) -> Option<Vec<u32>> {
    if w == 0 || h == 0 || w > MAX_CLIENT_ICON || h > MAX_CLIENT_ICON {
        return None;
    }
    let (bgr, has_alpha) = match format {
        DRM_FORMAT_ARGB8888 => (false, true),
        DRM_FORMAT_XRGB8888 => (false, false),
        DRM_FORMAT_ABGR8888 => (true, true),
        DRM_FORMAT_XBGR8888 => (true, false),
        _ => return None,
    };
    if stride < w * 4 || pixels.len() < stride.checked_mul(h)? {
        return None;
    }
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &pixels[y * stride..];
        for x in 0..w {
            let o = x * 4;
            let (b0, b1, b2, b3) = (row[o], row[o + 1], row[o + 2], row[o + 3]);
            let a = if has_alpha { b3 } else { 0xFF };
            let (r, g, b) = if bgr { (b0, b1, b2) } else { (b2, b1, b0) };
            let (r, g, b) = (r.min(a), g.min(a), b.min(a));
            out.push((a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32);
        }
    }
    Some(out)
}

/// Premultiply one straight-alpha ARGB8888 pixel, the `_NET_WM_ICON`
/// convention (native-endian ARGB, non-premultiplied).
pub fn premultiply(argb: u32) -> u32 {
    let a = (argb >> 24) & 0xFF;
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    let mul = |c: u32| (c * a + 127) / 255;
    (a << 24) | (mul(r) << 16) | (mul(g) << 8) | mul(b)
}

pub const HOT_LIFT: u32 = 118;

/// `icon` as a hot icon button draws it: every colour scaled by [`HOT_LIFT`], clamped per pixel at full with the other channels scaled by the same factor. Alpha is kept.
pub fn hot(icon: &Canvas) -> Canvas {
    let mut out = icon.clone();
    for p in &mut out.pixels {
        let a = *p >> 24;
        let (r, g, b) = ((*p >> 16) & 0xFF, (*p >> 8) & 0xFF, *p & 0xFF);
        let top = r.max(g).max(b);
        if top == 0 {
            continue;
        }
        // Premultiplied: a channel's full is the pixel's alpha.
        let (num, den) = if top * HOT_LIFT > a * 100 {
            (a, top)
        } else {
            (HOT_LIFT, 100)
        };
        let lift = |c: u32| ((c * num + den / 2) / den).min(a);
        *p = a << 24 | lift(r) << 16 | lift(g) << 8 | lift(b);
    }
    out
}

/// Coverage below this alpha is left out of [`is_symbolic`]'s test: its premultiplied channels
/// round too coarsely to tell a grey from a hue.
const SYMBOLIC_MIN_ALPHA: u32 = 0x40;
/// How far apart (per channel, 0..255) the greys of a symbolic icon may be.
const SYMBOLIC_SPREAD: u32 = 24;

/// Whether every pixel of `icon` with real coverage is the same grey, within [`SYMBOLIC_SPREAD`].
pub fn is_symbolic(icon: &Canvas) -> bool {
    symbolic_grey(icon).is_some()
}

/// The grey of a symbolic `icon` (see [`is_symbolic`]), 0..255.
fn symbolic_grey(icon: &Canvas) -> Option<u32> {
    let mut range = (u32::MAX, 0);
    for &p in &icon.pixels {
        let a = p >> 24;
        if a < SYMBOLIC_MIN_ALPHA {
            continue;
        }
        let [r, g, b] = [16, 8, 0].map(|sh| ((p >> sh) & 0xFF) * 255 / a);
        let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
        if hi - lo > SYMBOLIC_SPREAD {
            return None;
        }
        range = (range.0.min(lo), range.1.max(hi));
    }
    (range.0 != u32::MAX && range.1 - range.0 <= SYMBOLIC_SPREAD).then(|| (range.0 + range.1) / 2)
}

/// `icon` with every pixel's colour replaced by `ink`, its coverage kept.
pub fn inked(icon: &Canvas, ink: Color) -> Canvas {
    let mut out = icon.clone();
    for p in &mut out.pixels {
        *p = premultiply((*p & 0xFF00_0000) | (ink.argb() & 0x00FF_FFFF));
    }
    out
}

/// A ground at most this luminance is dark: a dark scheme's window, menu or face, not a light
/// scheme's mid-tone face.
const DARK_GROUND: f64 = 0.18;
/// A symbolic icon's grey reads on a ground at this contrast; a darker one is inked.
const SYMBOLIC_MIN_CONTRAST: f64 = 3.0;
/// An outline pixel reads on the ground at this contrast.
const EDGE_MIN_CONTRAST: f64 = 2.0;
/// An icon whose outline reads in less than this share of its pixels (out of 255) gets a halo.
const EDGE_MIN_SHARE: u32 = 128;
/// How far a halo's colour goes from the ground toward the ink, out of 255.
const HALO_MIX: u8 = 0x99;

/// `icon` as it is drawn on a `ground` with labels in `ink`, at `s` device pixels per design
/// pixel. On a light or mid-tone ground it is unchanged. On a dark one a symbolic icon too dark to read is
/// drawn in `ink`, and an icon whose outline vanishes into the ground gets a one design pixel
/// halo of the ground mixed toward `ink` around its shape.
pub fn on_ground(icon: &Canvas, ground: Color, ink: Color, s: i32) -> Cow<'_, Canvas> {
    if ground.luminance() > DARK_GROUND {
        return Cow::Borrowed(icon);
    }
    if let Some(grey) = symbolic_grey(icon) {
        let grey = grey as u8;
        return if Color::rgb(grey, grey, grey).contrast(ground) < SYMBOLIC_MIN_CONTRAST {
            Cow::Owned(inked(icon, ink))
        } else {
            Cow::Borrowed(icon)
        };
    }
    let (w, h) = device_size(icon);
    if outline_share(icon, w, h, ground) >= EDGE_MIN_SHARE {
        return Cow::Borrowed(icon);
    }
    Cow::Owned(haloed(
        icon,
        w,
        h,
        ground.toward(ink, HALO_MIX),
        s.max(1) as usize,
    ))
}

fn device_size(icon: &Canvas) -> (usize, usize) {
    let s = icon.scale().max(1) as usize;
    (icon.width as usize * s, icon.height as usize * s)
}

/// Out of 255, how many of `icon`'s outline pixels (mostly covered, beside a clear one) read on
/// `ground`; 255 for an icon with no outline.
fn outline_share(icon: &Canvas, w: usize, h: usize, ground: Color) -> u32 {
    let alpha = |x: isize, y: isize| -> u32 {
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize {
            0
        } else {
            icon.pixels[y as usize * w + x as usize] >> 24
        }
    };
    let (mut edge, mut seen) = (0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            let p = icon.pixels[y * w + x];
            let a = p >> 24;
            if a < 0x80 {
                continue;
            }
            let (xi, yi) = (x as isize, y as isize);
            let clear = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| alpha(xi + dx, yi + dy) < 0x40);
            if !clear {
                continue;
            }
            edge += 1;
            let over = |sh: u32, g: u8| {
                let c = (p >> sh) & 0xFF;
                (c + u32::from(g) * (255 - a) / 255).min(255) as u8
            };
            let shown = Color::rgb(over(16, ground.r), over(8, ground.g), over(0, ground.b));
            if shown.contrast(ground) >= EDGE_MIN_CONTRAST {
                seen += 1;
            }
        }
    }
    (seen * 255).checked_div(edge).unwrap_or(255)
}

/// `icon` over a `halo`-coloured copy of its shape grown by `r` pixels each way.
fn haloed(icon: &Canvas, w: usize, h: usize, halo: Color, r: usize) -> Canvas {
    let mut out = icon.clone();
    let rgb = halo.argb() & 0x00FF_FFFF;
    for y in 0..h {
        for x in 0..w {
            let mut a = 0;
            for ny in y.saturating_sub(r)..(y + r + 1).min(h) {
                for nx in x.saturating_sub(r)..(x + r + 1).min(w) {
                    a = a.max(icon.pixels[ny * w + nx] >> 24);
                }
            }
            let under = premultiply(a << 24 | rgb);
            let p = icon.pixels[y * w + x];
            let pa = p >> 24;
            let mix = |sh: u32| {
                let top = (p >> sh) & 0xFF;
                let bottom = (under >> sh) & 0xFF;
                (top + bottom * (255 - pa) / 255).min(255)
            };
            out.pixels[y * w + x] = mix(24) << 24 | mix(16) << 16 | mix(8) << 8 | mix(0);
        }
    }
    out
}

/// Classic window glyph drawn when a client has no icon: a dark outline, a
/// button-face frame, a 3 px active-title caption band and a white body at
/// 16 px, scaled to `size` by nearest neighbour so the pixel art stays crisp.
pub fn fallback(theme: &Theme, size: i32) -> Canvas {
    fallback_in(&theme.colors, size)
}

/// [`fallback`] on the look's own chrome (taskbar, caption, Alt+Tab panel): in the scheme that chrome keeps ([`crate::style::Style::chrome_colors`]) when it has one, else in `theme`'s.
pub fn chrome_fallback(theme: &Theme, size: i32) -> Canvas {
    let cs = theme.look.style().and_then(|s| s.chrome_colors.as_ref());
    fallback_in(cs.unwrap_or(&theme.colors), size)
}

fn fallback_in(cs: &ColorScheme, size: i32) -> Canvas {
    let mut c = Canvas::new(ICON_SIZE as u32, ICON_SIZE as u32);
    let bounds = c.bounds();
    c.fill_rect(bounds, cs.button_dk_shadow);
    c.fill_rect(bounds.inset(1), cs.button_face);
    c.fill_rect(bounds.inset(2), cs.window);
    c.fill_rect(Rect::new(2, 2, 12, 3), cs.active_title);
    if size == ICON_SIZE {
        return c;
    }
    let size = size.max(1) as u32;
    c.resample_box(size, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHARCOAL: Color = Color::rgb(0x2B, 0x2B, 0x2B);
    const LIGHT_INK: Color = Color::rgb(0xE6, 0xE6, 0xE6);

    /// A 16 px icon: a white 8x8 square with a one pixel black outline, at (4, 4).
    fn outlined() -> Canvas {
        let mut c = Canvas::new(16, 16);
        c.fill_rect(Rect::new(4, 4, 8, 8), Color::rgb(0, 0, 0));
        c.fill_rect(Rect::new(5, 5, 6, 6), Color::rgb(0xFF, 0xFF, 0xFF));
        c
    }

    #[test]
    fn a_light_or_mid_tone_ground_leaves_every_icon_as_it_is() {
        let mut grey = Canvas::new(16, 16);
        grey.fill_rect(Rect::new(2, 7, 12, 2), Color::rgb(0x80, 0x80, 0x80));
        for ground in [Color::rgb(0xFF, 0xFF, 0xFF), Color::rgb(131, 153, 177)] {
            for icon in [outlined(), grey.clone()] {
                let got = on_ground(&icon, ground, Color::rgb(0, 0, 0), 1);
                assert!(matches!(got, Cow::Borrowed(_)), "{ground:?}");
            }
        }
    }

    #[test]
    fn a_black_outline_on_charcoal_gets_a_halo_one_design_pixel_wide() {
        let icon = outlined();
        let halo = CHARCOAL.toward(LIGHT_INK, HALO_MIX).argb();
        for s in [1, 2] {
            let got = on_ground(&icon, CHARCOAL, LIGHT_INK, s);
            let at = |x: usize, y: usize| got.pixels[y * 16 + x];
            assert_eq!(at(4 - s as usize, 8), halo, "the halo at {s}");
            assert_eq!(at(3 - s as usize, 8), 0, "clear past the halo at {s}");
            assert_eq!(at(4, 8), 0xFF00_0000, "the outline stays black at {s}");
            assert_eq!(at(8, 8), 0xFFFF_FFFF, "the inside stays white at {s}");
        }
    }

    #[test]
    fn an_outline_that_reads_on_charcoal_gets_no_halo() {
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(4, 4, 8, 8), Color::rgb(0xF0, 0xC0, 0x40));
        assert!(matches!(
            on_ground(&icon, CHARCOAL, LIGHT_INK, 1),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn a_dark_symbolic_icon_on_charcoal_takes_the_ink_and_a_light_one_stays() {
        let mut dark = Canvas::new(16, 16);
        dark.fill_rect(Rect::new(2, 7, 12, 2), Color::rgb(0x23, 0x26, 0x29));
        let got = on_ground(&dark, CHARCOAL, LIGHT_INK, 1);
        assert_eq!(got.pixels[7 * 16 + 5], LIGHT_INK.argb());
        assert_eq!(got.pixels[0], 0, "clear stays clear");
        let mut light = Canvas::new(16, 16);
        light.fill_rect(Rect::new(2, 7, 12, 2), Color::rgb(0xD0, 0xD0, 0xD0));
        assert!(matches!(
            on_ground(&light, CHARCOAL, LIGHT_INK, 1),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn convert_refuses_oversized_and_empty_icons() {
        let px = vec![0u8; 4 * 4 * 4];
        assert!(convert(&px, 4, 4, 16, DRM_FORMAT_ARGB8888).is_some());
        assert!(convert(&px, 0, 4, 16, DRM_FORMAT_ARGB8888).is_none());
        // A sparse shm buffer can claim any size; the copy must not happen.
        assert!(
            convert(
                &px,
                MAX_CLIENT_ICON + 1,
                1,
                4 * (MAX_CLIENT_ICON + 1),
                DRM_FORMAT_ARGB8888
            )
            .is_none()
        );
        assert!(convert(&px, 1, MAX_CLIENT_ICON + 1, 4, DRM_FORMAT_ARGB8888).is_none());
    }
    use crate::theme::Theme;

    fn checkerboard(w: u32, h: u32) -> IconSource {
        let white = 0xFFFF_FFFFu32;
        let black = 0xFF00_0000u32;
        let pixels = (0..h)
            .flat_map(|y| (0..w).map(move |x| if (x + y) % 2 == 0 { white } else { black }))
            .collect();
        IconSource {
            width: w,
            height: h,
            pixels,
        }
    }

    #[test]
    fn to_canvas_box_filters_down_to_the_requested_size() {
        let c = checkerboard(32, 32).to_canvas(16);
        assert_eq!((c.width, c.height), (16, 16));
        assert!(c.pixels.iter().all(|&p| p == 0xFF80_8080));
        let c = checkerboard(32, 32).to_canvas(32);
        assert_eq!(c.get(0, 0), 0xFFFF_FFFF);
        assert_eq!(c.get(1, 0), 0xFF00_0000);
    }

    #[test]
    fn to_canvas_handles_empty_source() {
        let s = IconSource {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
        let c = s.to_canvas(16);
        assert_eq!((c.width, c.height), (16, 16));
        assert!(c.pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn convert_xrgb_forces_opaque() {
        let bytes = [0x10, 0x20, 0x30, 0xAA];
        let px = convert(&bytes, 1, 1, 4, DRM_FORMAT_XRGB8888).unwrap();
        assert_eq!(px, vec![0xFF30_2010]);
    }

    #[test]
    fn convert_abgr_swaps_red_and_blue() {
        let bytes = [0x10, 0x20, 0x30, 0x40];
        let px = convert(&bytes, 1, 1, 4, DRM_FORMAT_ABGR8888).unwrap();
        assert_eq!(px, vec![0x4010_2030]);
    }

    #[test]
    fn convert_clamps_colour_channels_to_alpha() {
        // Alpha 0x10 with 0xFF channels is not premultiplied: each clamps to 0x10.
        let bytes = [0xFF, 0x08, 0xFF, 0x10];
        let px = convert(&bytes, 1, 1, 4, DRM_FORMAT_ARGB8888).unwrap();
        assert_eq!(px, vec![0x1010_0810]);
        // Composited over opaque white, no channel carries into the next.
        let mut c = Canvas::new(1, 1);
        c.pixels[0] = 0xFFFF_FFFF;
        let src = Canvas::from_pixels(1, 1, px);
        c.blit_canvas(0, 0, &src);
        assert_eq!(c.pixels[0], 0xFFFF_F7FF);
    }

    #[test]
    fn convert_rejects_unknown_format_and_short_buffer() {
        assert!(convert(&[0; 4], 1, 1, 4, 0xDEAD_BEEF).is_none());
        assert!(convert(&[0; 2], 1, 1, 4, DRM_FORMAT_ARGB8888).is_none());
    }

    #[test]
    fn premultiply_scales_by_alpha() {
        assert_eq!(premultiply(0x80FF_0000), 0x8080_0000);
        assert_eq!(premultiply(0xFF10_2030), 0xFF10_2030);
        assert_eq!(premultiply(0x0012_3456), 0x0000_0000);
    }

    #[test]
    fn fallback_glyph_pixels_at_16() {
        let theme = Theme::default();
        let c = fallback(&theme, 16);
        assert_eq!((c.width, c.height), (16, 16));
        assert_eq!(c.get(0, 0), theme.colors.button_dk_shadow.argb());
        assert_eq!(c.get(8, 3), theme.colors.active_title.argb());
        assert_eq!(c.get(1, 1), theme.colors.button_face.argb());
        assert_eq!(c.get(8, 8), theme.colors.window.argb());
    }

    #[test]
    fn fallback_glyph_scales_by_nearest() {
        let theme = Theme::default();
        let c = fallback(&theme, 32);
        assert_eq!((c.width, c.height), (32, 32));
        // 1 px outline becomes 2 px, the caption band (rows 2..5) becomes rows 4..10
        assert_eq!(c.get(1, 1), theme.colors.button_dk_shadow.argb());
        assert_eq!(c.get(2, 2), theme.colors.button_face.argb());
        assert_eq!(c.get(16, 4), theme.colors.active_title.argb());
        assert_eq!(c.get(16, 9), theme.colors.active_title.argb());
        assert_eq!(c.get(16, 10), theme.colors.window.argb());
    }

    #[test]
    fn a_hot_icon_draws_its_colours_18_percent_brighter() {
        let px = [
            0xFF8A_5202, // dark orange: every channel lifts
            0xFFE4_B36B, // light orange: red stops at full
            0x8040_2000, // half-transparent: full is the alpha
            0x0000_0000, // transparent stays so
            0xFFFF_FFFF, // white stays white
        ];
        let hot = hot(&Canvas::from_pixels(5, 1, px.to_vec()));
        assert_eq!(hot.pixels[0], 0xFFA3_6102, "138,82,2 lifts to 163,97,2");
        assert_eq!(
            hot.pixels[1], 0xFFFF_C878,
            "228,179,107 lifts to 255,200,120"
        );
        assert_eq!(
            hot.pixels[2], 0x804C_2600,
            "64,32,0 at half alpha to 76,38,0"
        );
        assert_eq!(hot.pixels[3], 0);
        assert_eq!(hot.pixels[4], 0xFFFF_FFFF);
    }

    #[test]
    fn closest_size_prefers_32() {
        assert!(closer_to_target(32, None));
        assert!(!closer_to_target(16, Some(48))); // |16-32| == |48-32|: keep first
        assert!(closer_to_target(24, Some(48)));
        assert!(!closer_to_target(64, Some(16)));
    }
}
