//! A caption tab through the style's [`crate::style::CaptionTabLook`]:
//! rounded boxes laid over the caption band, alpha and all, so they work
//! on opaque bands and on glass alike. Returns `false` when the style has
//! none, so the caller draws Classic's.
//!
//! Rects are device px, already scaled by `s`.

use crate::frame::CaptionTab;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::Style;
use crate::text::TextRenderer;
use crate::theme::Color;

pub fn draw_caption_tab(
    st: &Style,
    c: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    tab: &CaptionTab,
    window_active: bool,
    s: i32,
) -> bool {
    let Some(l) = st.caption_tabs else {
        return false;
    };
    let radius = l.radius * s;
    let fill = if tab.ghost {
        l.ghost
    } else if tab.selected {
        l.selected
    } else if tab.ptr != Interaction::Idle {
        l.hot
    } else {
        l.rest
    };
    lay(c, r, radius, 0, fill);
    if tab.selected || tab.ghost {
        lay(c, r, radius, s, l.outline);
    }
    let spec = &st.caption_text;
    let ink = match (tab.selected || tab.ghost, window_active) {
        (true, true) => spec.color,
        (true, false) => spec.inactive,
        (false, true) => l.other_ink.0,
        (false, false) => l.other_ink.1,
    };
    let mut x = r.x + 3 * s;
    if let Some(icon) = tab.icon {
        let (iw, ih) = icon.device_size();
        c.blit_over(x, r.y + (r.height - ih as i32) / 2, icon);
        x += iw as i32 + 4 * s;
    }
    let end = tab.close.map_or(r.right() - 4 * s, |(b, _)| b.x - 2 * s);
    let room = Rect::new(x, r.y, (end - x).max(0), r.height);
    let size = spec.size_px * s as u32;
    let bold = spec.bold && tab.selected;
    let title = text.ellipsize(tab.title, room.width, bold, size);
    if let Some(baseline) = text.baseline(room, bold, size) {
        if let Some(sh) = spec
            .shadow
            .as_ref()
            .filter(|_| tab.selected && window_active)
        {
            let clip = Rect::new(
                room.x + sh.dx * s,
                room.y + sh.dy * s,
                room.width,
                room.height,
            );
            text.draw_run_over(
                c,
                clip.x,
                baseline + sh.dy * s,
                clip.intersect(&room),
                &title,
                bold,
                sh.color,
                size,
            );
        }
        text.draw_run_over(c, room.x, baseline, room, &title, bold, ink, size);
    }
    if let Some((b, ptr)) = tab.close {
        let back = match ptr {
            Interaction::Idle => None,
            Interaction::Hot => Some(l.close_hot),
            Interaction::Pressed => Some(l.close_held),
        };
        if let Some(back) = back {
            lay(c, b, 2 * s, 0, back);
        }
        draw_cross(c, b, s, ink);
    }
    true
}

/// Lay `color` at `alpha` over `r` with its corners rounded by `radius`:
/// the whole box, or with `ring` > 0 only a line that thick round its
/// edge. An alpha of 0 lays nothing.
fn lay(c: &mut Canvas, r: Rect, radius: i32, ring: i32, (color, alpha): (Color, u8)) {
    if alpha == 0 || r.is_empty() {
        return;
    }
    let inner = Rect::new(
        r.x + ring,
        r.y + ring,
        r.width - 2 * ring,
        r.height - 2 * ring,
    );
    let inner_radius = (radius - ring).max(0);
    let (w, h) = (r.width as usize, r.height as usize);
    let mut mask = vec![0u8; w * h];
    for y in 0..r.height {
        for x in 0..r.width {
            let (px, py) = (r.x + x, r.y + y);
            let inside = rounded_has(r, radius, px, py)
                && (ring == 0 || !rounded_has(inner, inner_radius, px, py));
            if inside {
                mask[y as usize * w + x as usize] = alpha;
            }
        }
    }
    let all = c.bounds();
    c.blit_mask_over(r.x, r.y, w, h, &mask, color, all);
}

fn rounded_has(r: Rect, radius: i32, x: i32, y: i32) -> bool {
    if !r.contains(x, y) {
        return false;
    }
    let radius = radius.min(r.width / 2).min(r.height / 2);
    if radius <= 0 {
        return true;
    }
    // In half pixels: a pixel's centre against the corner circle's, so a
    // radius of 2 cuts the corner pixel alone.
    let centre = |p: i32, lo: i32, hi: i32| {
        if p < lo + radius {
            Some(2 * (lo + radius))
        } else if p >= hi - radius {
            Some(2 * (hi - radius))
        } else {
            None
        }
    };
    match (centre(x, r.x, r.right()), centre(y, r.y, r.bottom())) {
        (Some(cx), Some(cy)) => {
            let (dx, dy) = (2 * x + 1 - cx, 2 * y + 1 - cy);
            dx * dx + dy * dy <= 4 * radius * radius
        }
        _ => true,
    }
}

/// A cross in `ink`, 2 px strokes (at scale 1) corner to corner of a box
/// 4 px inside `b`'s smaller side, centred in `b`.
fn draw_cross(c: &mut Canvas, b: Rect, s: i32, ink: Color) {
    let n = (b.width.min(b.height) - 8 * s).max(2 * s);
    let (x0, y0) = (b.x + (b.width - n) / 2, b.y + (b.height - n) / 2);
    for i in 0..n {
        let w = (s + 1).min(n - i);
        c.fill_rect(Rect::new(x0 + i, y0 + i, w, 1), ink);
        c.fill_rect(Rect::new(x0 + n - i - w, y0 + i, w, 1), ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_radius_of_2_cuts_the_corner_pixel_alone() {
        let r = Rect::new(0, 0, 10, 10);
        assert!(!rounded_has(r, 2, 0, 0));
        assert!(rounded_has(r, 2, 1, 0));
        assert!(rounded_has(r, 2, 0, 1));
        assert!(rounded_has(r, 2, 1, 1));
        assert!(!rounded_has(r, 2, 9, 9));
        assert!(rounded_has(r, 0, 0, 0), "no radius, a square box");
    }
}
