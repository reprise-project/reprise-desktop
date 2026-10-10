//! The Appearance and Themes tabs' preview: sample windows drawn in the pending colour scheme.
//! A skinned look hands in its own frame painter ([`Frames`]); sample coordinates count from the
//! sunken box's inner top-left.

use crate::art::glyph::Glyph;
use crate::classic::frame::{Chrome, caption_buttons};
use crate::classic::widgets::{Arrow, DROP_W, draw_caption, draw_disabled_label};
use crate::look::widgets::{draw_arrow_button, draw_button, draw_field_frame, draw_scrollbar};
use crate::paint::Paint;
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

pub type Frames<'a> = &'a dyn Fn(&mut Canvas, Rect, &str, bool, Chrome) -> Rect;

#[allow(clippy::too_many_arguments)]
pub fn draw_window(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    frame: Rect,
    title: &str,
    active: bool,
    menubar: bool,
    client: bool,
    frames: Option<Frames>,
) {
    let m = &theme.metrics;
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    let f = frame.scaled(s);
    let inner = match frames {
        Some(paint) => paint(c, f, title, active, Chrome::WINDOW),
        None => {
            c.fill_rect(f, cs.button_face);
            c.edge_t(f, EdgeStyle::WindowRaised, cs, s);
            let b = m.frame_width;
            let caption = Rect::new(f.x + b, f.y + b, f.width - 2 * b, m.caption_height);
            let [min, max, close] = caption_buttons(m, caption, 2 * s);
            let buttons = [
                (min, Glyph::Minimize),
                (max, Glyph::Maximize),
                (close, Glyph::Close),
            ];
            draw_caption(c, text, cs, caption, title, active, &buttons, px, s);
            let top = caption.bottom();
            Rect::new(caption.x, top, caption.width, f.bottom() - b - top)
        }
    };
    let mut y = inner.y;
    if menubar {
        let bar = Rect::new(inner.x, y, inner.width, 19 * s);
        c.fill_rect(bar, cs.menu);
        let row = |x: i32| Rect::new(x, bar.y + 2 * s, 80 * s, 13 * s);
        let mut x = f.x + 10 * s;
        text.draw(c, row(x), "Normal", false, cs.menu_text, Align::Left, px);
        x += text.measure("Normal", false, px) + 12 * s;
        draw_disabled_label(c, text, p, row(x), "Disabled", Align::Left, px);
        x += text.measure("Disabled", false, px) + 12 * s;
        text.draw(c, row(x), "Selected", false, cs.menu_text, Align::Left, px);
        y = bar.bottom();
    }
    if client {
        let cl = Rect::new(inner.x, y, inner.width, inner.bottom() - y);
        c.fill_rect(cl, cs.window);
        draw_field_frame(c, p, cl, cs.window);
        let inner = cl.inset(2 * s);
        if p.look.style().is_some() {
            draw_sample_scrollbar(c, p, inner);
        } else {
            let sb = Rect::new(
                inner.right() - DROP_W * s,
                inner.y,
                DROP_W * s,
                inner.height,
            );
            c.fill_rect(sb, cs.button_face);
            let up = Rect::new(sb.x, sb.y, sb.width, DROP_W * s);
            let down = Rect::new(sb.x, sb.bottom() - DROP_W * s, sb.width, DROP_W * s);
            draw_arrow_button(c, p, up, Arrow::Up, true);
            draw_arrow_button(c, p, down, Arrow::Down, true);
        }
        let tr = Rect::new(inner.x + 2 * s, inner.y + 2 * s, inner.width, 13 * s);
        text.draw(c, tr, "Window Text", true, cs.window_text, Align::Left, px);
    }
}

fn draw_sample_scrollbar(c: &mut Canvas, p: &Paint, inner: Rect) {
    let w = p.look.controls().scroll * p.s;
    let sb = Rect::new(inner.right() - w, inner.y, w, inner.height);
    let track = (sb.height - 2 * w).max(0) as usize;
    draw_scrollbar(c, p, sb, 0, w as usize, track, None);
}

/// The sample's message box: a 3 px frame with a caption and X,
/// the text and a plain OK. With `frames` the frame is the look's, and
/// the box grows by however much taller its frame's top is than Classic's
/// 21 px, so the text and OK keep their place under the caption.
pub fn draw_message_box(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    frame: Rect,
    frames: Option<Frames>,
) {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    let m = &theme.metrics;
    // Where the text starts and OK's top: OK sits 12 px under the look's
    // frame top, as it sits 33 px down the Classic box.
    let (f, top, ok_y) = match frames {
        Some(paint) => {
            let grow = (m.dialog_frame_width + m.caption_height - 21 * s).max(0);
            let f = frame.scaled(s);
            let f = Rect::new(f.x, f.y, f.width, f.height + grow);
            let top = paint(c, f, "Message Box", true, Chrome::DIALOG).y;
            (f, top, top + 12 * s)
        }
        None => {
            let f = frame.scaled(s);
            c.fill_rect(f, cs.button_face);
            c.edge_t(f, EdgeStyle::WindowRaised, cs, s);
            let b = m.dialog_frame_width;
            let caption = Rect::new(f.x + b, f.y + b, f.width - 2 * b, m.caption_height);
            let close = caption_buttons(m, caption, 2 * s)[2];
            let buttons = [(close, Glyph::Close)];
            draw_caption(c, text, cs, caption, "Message Box", true, &buttons, px, s);
            (f, caption.bottom(), f.y + 33 * s)
        }
    };
    let body = Rect::new(f.x + 6 * s, top + 3 * s, f.width, 13 * s);
    text.draw(
        c,
        body,
        "Message Text",
        false,
        cs.button_text,
        Align::Left,
        px,
    );
    let ok = Rect::new(f.x + 67 * s, ok_y, 72 * s, 24 * s);
    draw_button(
        c,
        text,
        p,
        ok,
        "OK",
        false,
        Interaction::Idle,
        true,
        false,
        px,
    );
}

pub const APPEARANCE_SIZE: (i32, i32) = (346, 184);
pub const THEMES_SIZE: (i32, i32) = (346, 218);

pub fn draw_appearance(
    theme: &Theme,
    text: &TextRenderer,
    frames: Option<Frames>,
    size: (i32, i32),
) -> Canvas {
    let s = theme.scale.max(1);
    let (w, h) = (size.0.max(APPEARANCE_SIZE.0), size.1.max(APPEARANCE_SIZE.1));
    let (dw, dh) = (w - APPEARANCE_SIZE.0, h - APPEARANCE_SIZE.1);
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), theme.colors.background);
    draw_window(
        &mut c,
        theme,
        text,
        Rect::new(16, 8, 305 + dw, 139 + dh),
        "Inactive Window",
        false,
        false,
        false,
        frames,
    );
    draw_window(
        &mut c,
        theme,
        text,
        Rect::new(20, 31, 323 + dw, 121 + dh),
        "Active Window",
        true,
        true,
        true,
        frames,
    );
    let message = Rect::new(28, 110 + dh, 207, 63);
    draw_message_box(&mut c, theme, text, message, frames);
    c
}

pub fn draw_themes(
    theme: &Theme,
    text: &TextRenderer,
    icon: Option<&Canvas>,
    frames: Option<Frames>,
) -> Canvas {
    let s = theme.scale.max(1);
    let (w, h) = THEMES_SIZE;
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), theme.colors.background);
    draw_window(
        &mut c,
        theme,
        text,
        Rect::new(22, 31, 231, 113),
        "Active Window",
        true,
        true,
        true,
        frames,
    );
    if let Some(ic) = icon {
        c.blit_over(294 * s, 178 * s, ic);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::widgets::caption_ramp;
    use crate::theme::FontConfig;

    #[test]
    fn appearance_sample_puts_the_windows_on_their_pixels() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let c = draw_appearance(&theme, &text, None, APPEARANCE_SIZE);
        assert_eq!(c.get(5, 5), cs.background.argb());
        assert_eq!(c.get(17, 9), cs.button_hilight.argb());
        assert_eq!(c.get(20, 16), cs.inactive_title.argb());
        assert_eq!(c.get(24, 38), cs.active_title.argb());
        assert_eq!(c.get(321, 37), cs.button_hilight.argb());
        assert_eq!(c.get(342, 36), cs.button_dk_shadow.argb());
        assert_eq!(c.get(100, 56), cs.menu.argb());
        assert_eq!(c.get(26, 74), cs.window.argb());
        assert_eq!(c.get(328, 80), cs.button_text.argb());
        assert_eq!(c.get(328, 106), cs.button_face.argb());
        assert_eq!(c.get(31, 116), cs.active_title.argb());
        assert_eq!(c.get(95, 143), cs.button_hilight.argb());
        assert_eq!(c.get(166, 166), cs.button_dk_shadow.argb());
    }

    #[test]
    fn a_blue_sample_draws_the_looks_scrollbar() {
        use crate::interface::Interface;
        use crate::interfaces::theme_for;
        use crate::look::draw_sample;
        let theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let p = theme.paint();
        for (themes, sb) in [
            (true, Rect::new(230, 82, 17, 56)),
            (false, Rect::new(320, 82, 17, 64)),
        ] {
            let c = draw_sample(themes, &theme, 1, &text, None);
            let (w, h) = c.device_size();
            let mut want = Canvas::new(w, h);
            let track = (sb.height - 34) as usize;
            draw_scrollbar(&mut want, &p, sb, 0, 17, track, None);
            for y in sb.y..sb.bottom() {
                for x in sb.x..sb.right() {
                    assert_eq!(c.get(x, y), want.get(x, y), "themes {themes}: ({x}, {y})");
                }
            }
            let mid = sb.x + 8;
            let art = |part, x, y| {
                crate::look::styled::fixture::blue_pixel(part, crate::style::State::Normal, x, y)
            };
            use crate::style::Part::{ScrollThumbV, ScrollTrackV};
            assert_eq!(
                c.get(mid, sb.y + 33),
                art(ScrollThumbV, 8, 21),
                "the thumb's base line"
            );
            assert_eq!(
                c.get(mid, sb.y + 34),
                art(ScrollTrackV, 8, 0),
                "the track's middle"
            );
            let edge = art(ScrollTrackV, 0, 0);
            assert!(
                (sb.y..sb.bottom()).all(|y| c.get(sb.x, y) == edge),
                "the bar's left edge is the track's edge all the way down"
            );
        }
    }

    #[test]
    fn long_sample_title_stops_where_the_caption_ramp_ends() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut c = Canvas::new(240, 60);
        let frame = Rect::new(0, 0, 231, 60);
        draw_window(
            &mut c,
            &theme,
            &text,
            frame,
            &"W".repeat(40),
            true,
            false,
            false,
            None,
        );
        let caption = Rect::new(4, 4, 223, 18);
        let buttons = caption_buttons(&theme.metrics, caption, 2);
        let ramp_end = caption.x + caption_ramp(caption.width, 3, 1);
        assert_eq!(ramp_end, 175);
        for x in ramp_end..caption.right() {
            for y in caption.y..caption.bottom() {
                if buttons.iter().any(|b| b.contains(x, y)) {
                    continue;
                }
                assert_ne!(
                    c.get(x, y),
                    cs.title_text.argb(),
                    "no title ink at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn scaled_sample_caption_is_the_scaled_caption_height() {
        let theme = Theme::default().scaled(2);
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let c = draw_appearance(&theme, &text, None, APPEARANCE_SIZE);
        assert_eq!(c.get(48, 70), cs.active_title.argb());
        assert_eq!(c.get(48, 105), cs.active_title.argb());
        assert_eq!(c.get(48, 106), cs.menu.argb());
    }
}
