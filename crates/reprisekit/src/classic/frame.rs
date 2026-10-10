//! The frame drawn in the Classic look, over the shape and hit test of `crate::frame`.

use crate::art::glyph::Glyph as MenuGlyph;
use crate::classic::widgets::{
    CAPTION_RAMP_START, caption_colors, draw_caption, draw_caption_button, window_caption_ramp,
};
use crate::icon;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

pub use crate::frame::{
    ButtonSet, CaptionButton, Chrome, Edges, FrameGeometry, FrameHit, FrameState, Glow, ICON_AREA,
    caption_buttons, frame_canvas_size,
};

fn caption_glyph(kind: CaptionButton, restore: bool) -> MenuGlyph {
    match (kind, restore) {
        (CaptionButton::Minimize, _) => MenuGlyph::Minimize,
        (CaptionButton::Maximize, false) => MenuGlyph::Maximize,
        (CaptionButton::Maximize, true) => MenuGlyph::Restore,
        (CaptionButton::Close, _) => MenuGlyph::Close,
    }
}

/// Rasterize the frame at `geom.origin`. The client rectangle is left
/// transparent so the client surface shows through when stacked above in
/// the scene; a `translated` geometry paints one strip of the frame.
pub fn draw_frame(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    geom: &FrameGeometry,
    state: &FrameState,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let icon_px = theme.icon_size();
    let (w, h) = geom.total;
    let all = Rect::new(geom.origin.0, geom.origin.1, w, h);
    canvas.fill_rect(all, cs.button_face);
    if geom.frame > 0 {
        canvas.edge_t(all, EdgeStyle::WindowRaised, cs, s);
    }
    // A border strip is done here; the caption, its text and buttons would
    // only be rasterised to clip away.
    if geom.caption.intersect(&canvas.bounds()).is_empty() {
        punch_client_hole(canvas, geom);
        return;
    }
    if !geom.chrome.icon {
        draw_dialog_caption(canvas, theme, text, geom, state);
        punch_client_hole(canvas, geom);
        return;
    }
    let (left, right, ink) = caption_colors(cs, state.active);
    canvas.fill_rect(geom.caption, right);
    canvas.hgradient_stepped(
        window_caption_ramp(geom.caption, geom.buttons[0].x, s),
        left,
        right,
    );
    canvas.fill_rect(
        Rect::new(
            geom.caption.x,
            geom.caption.y,
            CAPTION_RAMP_START * s,
            geom.caption.height,
        ),
        left,
    );

    if state.tabbed {
        draw_buttons(canvas, cs, geom, state, s);
        punch_client_hole(canvas, geom);
        return;
    }

    match state.icon {
        Some(icon) => canvas.blit_canvas(geom.caption.x + 2 * s, geom.caption.y + s, icon),
        None => canvas.blit_canvas(
            geom.caption.x + 2 * s,
            geom.caption.y + s,
            &icon::fallback(theme, icon_px),
        ),
    }

    let first_button_x = geom.buttons[0].x;
    let title_x = geom.caption.x + (4 + ICON_AREA) * s;
    let title_rect = Rect::new(
        title_x,
        geom.caption.y,
        (first_button_x - 2 * s - title_x).max(0),
        geom.caption.height,
    );
    text.draw(
        canvas,
        title_rect,
        state.title,
        true,
        ink,
        Align::Left,
        size,
    );

    draw_buttons(canvas, cs, geom, state, s);
    punch_client_hole(canvas, geom);
}

fn draw_buttons(
    canvas: &mut Canvas,
    cs: &crate::theme::ColorScheme,
    geom: &FrameGeometry,
    state: &FrameState,
    s: i32,
) {
    for (kind, b) in CaptionButton::ALL.into_iter().zip(geom.buttons) {
        if !geom.chrome.buttons.has(kind) {
            continue;
        }
        let pressed = state.pressed == Some(kind);
        let glyph = caption_glyph(kind, state.maximized);
        draw_caption_button(canvas, cs, b, glyph, pressed, s);
    }
}

/// A caption without an icon, as the compositor's own dialogs draw it (`draw_caption`).
fn draw_dialog_caption(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    geom: &FrameGeometry,
    state: &FrameState,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let close = geom.chrome.buttons.has(CaptionButton::Close);
    let buttons: Vec<(Rect, MenuGlyph)> = close
        .then_some((geom.buttons[2], MenuGlyph::Close))
        .into_iter()
        .chain(
            geom.chrome
                .help
                .then_some((geom.help_box(), MenuGlyph::Help)),
        )
        .collect();
    draw_caption(
        canvas,
        text,
        cs,
        geom.caption,
        state.title,
        state.active,
        &buttons,
        theme.fonts.size_px,
        s,
    );
    if close && state.pressed == Some(CaptionButton::Close) {
        draw_caption_button(canvas, cs, geom.buttons[2], MenuGlyph::Close, true, s);
    }
}

/// Transparent hole for the client. `clear_rect` cuts it to the canvas,
/// which can be smaller than the frame geometry.
fn punch_client_hole(canvas: &mut Canvas, geom: &FrameGeometry) {
    canvas.clear_rect(geom.client);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Canvas;
    use crate::text::TextRenderer;
    use crate::theme::Color;
    use crate::theme::{FontConfig, Theme};

    #[test]
    fn frame_pixels_follow_the_edge_colours() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let g = FrameGeometry::layout(&theme.metrics, 200, 100, false, Chrome::WINDOW);
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_frame(
            &mut c,
            &theme,
            &text,
            &g,
            &FrameState {
                title: "Test",
                active: true,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        let left: Vec<u32> = (0..4).map(|x| c.get(x, 60)).collect();
        assert_eq!(
            left,
            vec![
                cs.button_face.argb(),
                cs.button_hilight.argb(),
                cs.button_face.argb(),
                cs.button_face.argb()
            ]
        );
        let top: Vec<u32> = (0..4).map(|y| c.get(100, y)).collect();
        assert_eq!(top, left);
        assert_eq!(c.get(4, 4), cs.active_title.argb());
        assert_eq!(c.get(203, 4), cs.gradient_active_title.argb());
        assert_eq!(c.get(100, 60), 0);
        let b = g.button_rect(CaptionButton::Close);
        assert_eq!(c.get(b.x, b.y + 7), cs.button_hilight.argb());
        assert_eq!(c.get(b.right() - 2, b.y + 7), cs.button_shadow.argb());
        assert_eq!(c.get(b.right() - 1, b.y + 7), cs.button_dk_shadow.argb());
        let black = (b.x..b.right())
            .flat_map(|x| (b.y..b.bottom()).map(move |y| (x, y)))
            .any(|(x, y)| c.get(x, y) == cs.button_text.argb());
        assert!(black);
        assert_eq!(c.get(207, 125), cs.button_dk_shadow.argb());
    }

    #[test]
    fn a_help_dialog_frame_is_the_dialog_frame() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let g = FrameGeometry::layout(&theme.metrics, 390, 440, false, Chrome::HELP_DIALOG);
        for active in [true, false] {
            let state = FrameState {
                title: "Open With",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            };
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            draw_frame(&mut c, &theme, &text, &g, &state);
            let mut dlg = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            crate::look::draw_dialog_frame(
                &mut dlg,
                &theme,
                &text,
                &FrameGeometry::layout(&theme.metrics, 390, 440, false, Chrome::DIALOG),
                &state,
                Some(crate::pointer::Interaction::Idle),
            );
            for y in 0..g.total.1 {
                for x in 0..g.total.0 {
                    let want = if g.client.contains(x, y) {
                        0
                    } else {
                        dlg.get(x, y)
                    };
                    assert_eq!(c.get(x, y), want, "({x}, {y}), active {active}");
                }
            }
        }
    }

    #[test]
    fn a_prompt_frame_is_the_dialog_frame_with_the_close_box_alone() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let g = FrameGeometry::layout(&theme.metrics, 413, 117, false, Chrome::PROMPT);
        for active in [true, false] {
            let state = FrameState {
                title: "Confirm File Delete",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            };
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            draw_frame(&mut c, &theme, &text, &g, &state);
            let mut dlg = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            crate::look::draw_dialog_frame(&mut dlg, &theme, &text, &g, &state, None);
            for y in 0..g.total.1 {
                for x in 0..g.total.0 {
                    let want = if g.client.contains(x, y) {
                        0
                    } else {
                        dlg.get(x, y)
                    };
                    assert_eq!(c.get(x, y), want, "({x}, {y}), active {active}");
                }
            }
        }
    }

    #[test]
    fn strips_are_crops_of_the_whole_frame() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let state = FrameState {
            title: "Strips",
            active: true,
            maximized: false,
            pressed: Some(CaptionButton::Close),
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        };
        for (maximized, chrome) in [
            (false, Chrome::WINDOW),
            (false, Chrome::DIALOG),
            (false, Chrome::HELP_DIALOG),
            (false, Chrome::PROMPT),
            (true, Chrome::WINDOW),
        ] {
            let g = FrameGeometry::layout(&theme.metrics, 200, 100, maximized, chrome);
            let mut whole = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            draw_frame(&mut whole, &theme, &text, &g, &state);
            let strips = g.strips();
            let mut covered = 0;
            for r in strips {
                if r.is_empty() {
                    continue;
                }
                covered += r.width * r.height;
                let mut c = Canvas::new(r.width as u32, r.height as u32);
                draw_frame(&mut c, &theme, &text, &g.translated(r.x, r.y), &state);
                for y in 0..r.height {
                    for x in 0..r.width {
                        assert_eq!(
                            c.get(x, y),
                            whole.get(r.x + x, r.y + y),
                            "strip {r:?} at ({x}, {y}), maximized {maximized}"
                        );
                    }
                }
            }
            assert_eq!(
                covered,
                g.total.0 * g.total.1 - g.client.width * g.client.height
            );
        }
    }

    #[test]
    fn caption_gradient_holds_ramps_and_holds() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let band = |client_w: i32, maximized: bool| {
            let g = FrameGeometry::layout(&theme.metrics, client_w, 100, maximized, Chrome::WINDOW);
            let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            draw_frame(
                &mut c,
                &theme,
                &text,
                &g,
                &FrameState {
                    title: "",
                    active: true,
                    maximized,
                    pressed: None,
                    glow: Glow::lit(None),
                    icon: None,
                    tabbed: false,
                },
            );
            let (x0, y) = (g.caption.x, g.caption.y + 16);
            move |i: i32| c.get(x0 + i, y)
        };
        let rgb = |r: u8, g: u8, b: u8| Color::rgb(r, g, b).argb();
        let w = band(1432, false);
        assert_eq!(w(18), rgb(10, 36, 106));
        assert_eq!(w(26), rgb(10, 36, 106));
        assert_eq!(w(27), rgb(11, 37, 106));
        assert_eq!(w(164), rgb(26, 53, 120));
        assert_eq!(w(664), rgb(84, 114, 169));
        assert_eq!(w(1064), rgb(129, 163, 209));
        assert_eq!(w(1377), rgb(165, 201, 239));
        assert_eq!(w(1378), rgb(166, 202, 240));
        assert_eq!(w(1431), rgb(166, 202, 240));
        let m = band(1920, true);
        assert_eq!(m(29), rgb(10, 36, 106));
        assert_eq!(m(30), rgb(11, 37, 106));
        assert_eq!(m(1865), rgb(165, 201, 239));
        assert_eq!(m(1866), rgb(166, 202, 240));
        assert_eq!(m(1919), rgb(166, 202, 240));
    }

    #[test]
    fn inactive_frame_uses_gray_gradient() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let g = FrameGeometry::layout(&theme.metrics, 100, 50, false, Chrome::WINDOW);
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_frame(
            &mut c,
            &theme,
            &text,
            &g,
            &FrameState {
                title: "",
                active: false,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        assert_eq!(c.get(4, 4), theme.colors.inactive_title.argb());
    }

    #[test]
    fn pressed_button_is_sunken() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let g = FrameGeometry::layout(&theme.metrics, 100, 50, false, Chrome::WINDOW);
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_frame(
            &mut c,
            &theme,
            &text,
            &g,
            &FrameState {
                title: "",
                active: true,
                maximized: false,
                pressed: Some(CaptionButton::Close),
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        let b = g.button_rect(CaptionButton::Close);
        assert_eq!(c.get(b.x, b.y + 7), theme.colors.button_shadow.argb());
    }

    #[test]
    fn draw_frame_does_not_panic_on_undersized_canvas() {
        // Canvas smaller than geom.total must not panic when punching the
        // transparent client hole.
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let g = FrameGeometry::layout(&theme.metrics, 200, 100, false, Chrome::WINDOW);
        let mut c = Canvas::new(50, 50);
        draw_frame(
            &mut c,
            &theme,
            &text,
            &g,
            &FrameState {
                title: "Test",
                active: true,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
    }

    #[test]
    fn draw_frame_at_scale_two() {
        let theme = Theme::default().scaled(2);
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let g = FrameGeometry::layout(&Theme::default().metrics, 200, 100, false, Chrome::WINDOW)
            .scaled(2);
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_frame(
            &mut c,
            &theme,
            &text,
            &g,
            &FrameState {
                title: "Test",
                active: true,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        let row: Vec<u32> = (0..4).map(|x| c.get(x, 100)).collect();
        assert_eq!(
            row,
            vec![
                cs.button_face.argb(),
                cs.button_face.argb(),
                cs.button_hilight.argb(),
                cs.button_hilight.argb()
            ]
        );
        assert_eq!(c.get(g.caption.x, g.caption.y + 30), cs.active_title.argb());
        assert_eq!(c.get(g.client.x + 10, g.client.y + 10), 0);
        let b = g.buttons[2];
        assert_eq!(c.get(b.x + 8, b.y + 6), cs.button_text.argb());
        assert_eq!(c.get(b.x + 9, b.y + 7), cs.button_text.argb());
        let b = g.buttons[0];
        assert_eq!(c.get(b.x + 8, b.y + 18), cs.button_text.argb());
        assert_eq!(c.get(b.x + 19, b.y + 21), cs.button_text.argb());
        assert_eq!(c.get(b.x + 20, b.y + 18), cs.button_face.argb());
    }
}
