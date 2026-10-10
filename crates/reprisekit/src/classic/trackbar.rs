//! Classic's trackbar paint, over the model and layout in [`crate::trackbar`].

use crate::paint::Paint;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::theme::ColorScheme;
use crate::trackbar::{Layout, THUMB_H, THUMB_W, vertical_ticks};

const TICK_Y: i32 = 22;

pub fn draw(c: &mut Canvas, p: &Paint, lay: &Layout, enabled: bool) {
    draw_held(c, p, lay, enabled, false)
}

pub fn draw_held(c: &mut Canvas, p: &Paint, lay: &Layout, enabled: bool, held: bool) {
    let speckled = !enabled || held;
    if lay.vertical {
        return draw_vertical(c, p, lay, !speckled);
    }
    let Paint { cs, s, .. } = *p;
    let ch = lay.channel.scaled(s);
    c.fill_rect(ch, cs.button_face);
    c.edge_t(ch, EdgeStyle::ButtonSunken, cs, s);
    let base = lay.bounds.y + TICK_Y;
    for (i, &tx) in lay.ticks.iter().enumerate() {
        let last = i + 1 == lay.ticks.len();
        if !lay.ticked || (i % lay.tick_every != 0 && !last) {
            continue;
        }
        let tall = if i == 0 || last { 4 } else { 3 };
        c.fill_rect(Rect::new(tx * s, base * s, s, tall * s), cs.button_text);
    }
    draw_thumb(c, cs, lay.thumb, !speckled, s);
}

fn draw_vertical(c: &mut Canvas, p: &Paint, lay: &Layout, enabled: bool) {
    let Paint { cs, s, .. } = *p;
    let ch = lay.channel.scaled(s);
    c.fill_rect(ch, cs.button_face);
    c.edge_t(ch, EdgeStyle::ButtonSunken, cs, s);
    for tick in vertical_ticks(lay, lay.tick_every, 3, 4) {
        c.fill_rect(tick.scaled(s), cs.button_text);
    }
    let t = lay.thumb;
    c.fill_rect(t.scaled(s), cs.button_face);
    c.edge_t(t.scaled(s), EdgeStyle::ButtonRaised, cs, s);
    if !enabled {
        for y in 2..t.height - 2 {
            for x in 2..t.width - 2 {
                if (x + y) % 2 == 1 {
                    c.fill_rect(
                        Rect::new((t.x + x) * s, (t.y + y) * s, s, s),
                        cs.button_hilight,
                    );
                }
            }
        }
    }
}

fn draw_thumb(c: &mut Canvas, cs: &ColorScheme, t: Rect, enabled: bool, s: i32) {
    let body = 16;
    let px = |c: &mut Canvas, x: i32, y: i32, col| {
        c.fill_rect(Rect::new((t.x + x) * s, (t.y + y) * s, s, s), col)
    };
    for y in 0..THUMB_H {
        let inset = (y - body + 1).max(0);
        let (l, r) = (inset, THUMB_W - 1 - inset);
        if l > r {
            break;
        }
        for x in l..=r {
            let colour = if y == 0 {
                if x == r {
                    cs.button_dk_shadow
                } else {
                    cs.button_hilight
                }
            } else if x == l && l < r {
                cs.button_hilight
            } else if x == r {
                cs.button_dk_shadow
            } else if x == r - 1 && (y < body || l < r - 1) {
                cs.button_shadow
            } else if enabled {
                cs.button_face
            } else if y == 1 || x == 1 || (x + y) % 2 == 0 {
                // The top row and left column stay face, so the dither never touches the hilight edge.
                cs.button_face
            } else {
                cs.button_hilight
            };
            px(c, x, y, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::Theme;
    use crate::trackbar::{TrackBar, layout};

    #[test]
    fn draws_the_channel_and_thumb_pixels() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let tb = TrackBar::new(13, 18);
        let l = layout(&tb, 69, 315, 71);
        let mut c = Canvas::new(200, 400);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &classic_paint(cs, 1), &l, true);
        assert_eq!(c.get(80, 322), cs.button_shadow.argb());
        assert_eq!(c.get(80, 323), cs.button_dk_shadow.argb());
        assert_eq!(c.get(80, 324), cs.button_face.argb());
        assert_eq!(c.get(80, 325), cs.button_hilight.argb());
        assert_eq!(c.get(69, 325), cs.button_hilight.argb());
        assert_eq!(c.get(138, 323), cs.button_face.argb());
        assert_eq!(c.get(139, 322), cs.button_hilight.argb());
        assert_eq!(c.get(115, 315), cs.button_hilight.argb());
        assert_eq!(c.get(124, 315), cs.button_hilight.argb());
        assert_eq!(c.get(125, 315), cs.button_dk_shadow.argb());
        assert_eq!(c.get(115, 320), cs.button_hilight.argb());
        assert_eq!(c.get(124, 320), cs.button_shadow.argb());
        assert_eq!(c.get(125, 320), cs.button_dk_shadow.argb());
        assert_eq!(c.get(116, 331), cs.button_hilight.argb());
        assert_eq!(c.get(123, 331), cs.button_shadow.argb());
        assert_eq!(c.get(124, 331), cs.button_dk_shadow.argb());
        assert_eq!(c.get(119, 334), cs.button_hilight.argb());
        assert_eq!(c.get(120, 334), cs.button_shadow.argb());
        assert_eq!(c.get(121, 334), cs.button_dk_shadow.argb());
        assert_eq!(c.get(120, 335), cs.button_dk_shadow.argb());
        assert_eq!(c.get(119, 335), cs.button_face.argb());
        assert_eq!(c.get(78, 337), cs.button_text.argb());
        assert_eq!(c.get(78, 339), cs.button_text.argb());
        assert_eq!(c.get(78, 340), cs.button_face.argb());
        assert_eq!(c.get(74, 340), cs.button_text.argb());
        assert_eq!(c.get(134, 340), cs.button_text.argb());
    }

    #[test]
    fn a_disabled_trackbar_greys_its_thumb_and_ticks() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let mut tb = TrackBar::new(13, 18);
        tb.enabled = false;
        let l = layout(&tb, 69, 315, 71);
        let mut c = Canvas::new(200, 400);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &classic_paint(cs, 1), &l, false);
        assert_eq!(c.get(80, 322), cs.button_shadow.argb());
        assert_eq!(c.get(80, 323), cs.button_dk_shadow.argb());
        assert_eq!(c.get(74, 340), cs.button_text.argb());
        assert_eq!(c.get(134, 340), cs.button_text.argb());
        assert_eq!(c.get(125, 320), cs.button_dk_shadow.argb());
        assert_eq!(c.get(121, 320), cs.button_hilight.argb());
        assert_eq!(c.get(120, 320), cs.button_face.argb());
    }

    #[test]
    fn an_enabled_trackbar_is_unchanged() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let l = layout(&TrackBar::new(13, 18), 69, 315, 71);
        let mut c = Canvas::new(200, 400);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &classic_paint(cs, 1), &l, true);
        assert_eq!(c.get(125, 320), cs.button_dk_shadow.argb());
        assert_eq!(c.get(74, 340), cs.button_text.argb());
    }
}
