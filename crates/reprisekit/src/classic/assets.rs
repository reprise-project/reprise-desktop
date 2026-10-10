//! The display preview bitmaps: the monitor, drawn once (its screen transparent so a preview shows
//! through), the desktop painted in it, and the colour spectrum bar at any scale.

use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::theme::{Color, ColorScheme};
use std::sync::OnceLock;

pub const MONITOR_SIZE: (i32, i32) = (191, 168);
pub const MONITOR_SCREEN: (i32, i32, i32, i32) = (25, 24, 152, 112);
pub const SPECTRUM_SIZE: (i32, i32) = (153, 13);

pub fn monitor() -> &'static Canvas {
    static C: OnceLock<Canvas> = OnceLock::new();
    C.get_or_init(draw_monitor)
}

/// The spectrum bar at scale `s`: a hue sweep across its `153 * s` columns, red at the left edge.
pub fn spectrum(s: i32) -> Canvas {
    let (w, h) = (SPECTRUM_SIZE.0 * s, SPECTRUM_SIZE.1 * s);
    let mut c = Canvas::new(w as u32, h as u32);
    for x in 0..w {
        c.fill_rect(Rect::new(x, 0, 1, h), hue(x * 360 / w));
    }
    c
}

/// The desktop the Settings tab paints in the monitor: the scheme's background with a taskbar strip along the bottom.
pub fn screen_settings(cs: &ColorScheme) -> Canvas {
    let (_, _, w, h) = MONITOR_SCREEN;
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(Rect::new(0, 0, w, h), cs.background);
    let bar = Rect::new(0, h - 8, w, 8);
    c.fill_rect(bar, cs.button_face);
    c.fill_rect(Rect::new(0, bar.y, w, 1), cs.button_hilight);
    c.edge_t(
        Rect::new(2, bar.y + 2, 18, 5),
        EdgeStyle::ButtonRaised,
        cs,
        1,
    );
    c
}

fn draw_monitor() -> Canvas {
    let cs = ColorScheme::default();
    let (w, h) = MONITOR_SIZE;
    let (sx, sy, sw, sh) = MONITOR_SCREEN;
    let mut c = Canvas::new(w as u32, h as u32);
    let bezel = Rect::new(sx - 13, sy - 14, sw + 26, sh + 28);
    c.fill_rect(bezel, cs.button_face);
    c.edge_t(bezel, EdgeStyle::ButtonRaised, &cs, 1);
    c.edge_t(
        Rect::new(sx - 2, sy - 2, sw + 4, sh + 4),
        EdgeStyle::ButtonSunken,
        &cs,
        1,
    );
    let mid = sx + sw / 2;
    let neck = Rect::new(mid - 15, bezel.bottom(), 31, 8);
    c.fill_rect(neck, cs.button_face);
    c.edge_t(neck, EdgeStyle::ButtonRaised, &cs, 1);
    let foot = Rect::new(mid - 40, neck.bottom(), 81, 8);
    c.fill_rect(foot, cs.button_face);
    c.edge_t(foot, EdgeStyle::ButtonRaised, &cs, 1);
    c.clear_rect(Rect::new(sx, sy, sw, sh));
    c
}

fn hue(deg: i32) -> Color {
    let sector = deg / 60;
    let t = ((deg % 60) * 255 / 60) as u8;
    let (r, g, b) = match sector {
        0 => (255, t, 0),
        1 => (255 - t, 255, 0),
        2 => (0, 255, t),
        3 => (0, 255 - t, 255),
        4 => (t, 0, 255),
        _ => (255, 0, 255 - t),
    };
    Color::rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_has_a_transparent_screen_inside_an_opaque_bezel() {
        let m = monitor();
        assert_eq!((m.width as i32, m.height as i32), MONITOR_SIZE);
        let (sx, sy, sw, sh) = MONITOR_SCREEN;
        assert_eq!(m.get(sx + sw / 2, sy + sh / 2) >> 24, 0, "screen is a hole");
        assert_eq!(m.get(sx - 3, sy + sh / 2) >> 24, 255, "bezel is opaque");
        assert_eq!(m.get(2, 2) >> 24, 0, "outside the bezel is clear");
    }

    #[test]
    fn spectrum_sweeps_hue_from_red() {
        let s = spectrum(1);
        assert_eq!((s.width as i32, s.height as i32), SPECTRUM_SIZE);
        assert_eq!(s.get(0, 6) & 0xff_ffff, 0xff_0000, "red at the left edge");
        let mid = s.get(76, 6) & 0xff_ffff;
        assert_ne!(mid, 0xff_0000, "the middle is another hue");
        assert_eq!(s.get(76, 6) >> 24, 255, "opaque");
    }

    #[test]
    fn spectrum_at_scale_two_is_306x26_and_sweeps_every_column() {
        let s = spectrum(2);
        assert_eq!((s.width, s.height), (306, 26));
        assert_eq!(s.get(0, 12) & 0xff_ffff, 0xff_0000, "red at the left edge");
        assert_ne!(s.get(1, 12), s.get(0, 12), "each column has its own hue");
        assert_eq!(s.get(305, 12) & 0xff_ffff, 0xff_0009, "the right edge");
        assert_eq!(s.get(305, 25), s.get(305, 0), "a column is one colour");
    }

    #[test]
    fn screen_preview_uses_the_scheme() {
        let cs = ColorScheme::default();
        let p = screen_settings(&cs);
        let (_, _, w, h) = MONITOR_SCREEN;
        assert_eq!((p.width as i32, p.height as i32), (w, h));
        assert_eq!(
            p.get(w / 2, h / 3) & 0xff_ffff,
            cs.background.argb() & 0xff_ffff
        );
        assert_eq!(
            p.get(w / 2, h - 3) & 0xff_ffff,
            cs.button_face.argb() & 0xff_ffff
        );
    }
}
