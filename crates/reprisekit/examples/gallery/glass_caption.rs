use super::blurred_stripes;
use crate::interfaces;
use reprisekit::classic::frame::{Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::glass::Mode;
use reprisekit::interface::Interface;
use reprisekit::look;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, FontConfig};

/// Bare Glass captions over the stripes at scale `n`: a row per mode, light then dark, each a
/// focused and an inactive restored window, then a focused maximized one; 950x230 at scale 1.
pub fn render_glass_bare_captions(n: i32) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(950 * n, 230 * n);
    for (row, mode) in [Mode::Light, Mode::Dark].into_iter().enumerate() {
        let g = reprisekit::glass::Settings {
            mode,
            ..Default::default()
        };
        let theme = interfaces::theme_with(Interface::Glass, Default::default(), false, &g);
        let caption_text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
        let at = theme.scaled(n);
        let y = 10 + 110 * row as i32;
        for (x, w, active, maximized) in [
            (10, 290, true, false),
            (320, 290, false, false),
            (630, 310, true, true),
        ] {
            let geom = FrameGeometry::layout(&theme.metrics, w, 60, maximized, Chrome::WINDOW);
            let st = FrameState {
                title: "Documents",
                active,
                maximized,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: true,
            };
            let geom = geom.scaled(n);
            let mut f = Canvas::new(geom.total.0 as u32, geom.total.1 as u32);
            look::draw_frame(&mut f, &at, &text, &caption_text, &geom, &st);
            f.fill_rect(geom.client, Color::rgb(0xFF, 0xFF, 0xFF));
            c.blit_canvas(x * n, y * n, &f);
        }
    }
    c
}
