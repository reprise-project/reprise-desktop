//! A one-line tip and a two-line tip over the desktop colour.

use reprisekit::controls::tooltip::draw_tip;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub const HEIGHT: i32 = 60;

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    c.fill_rect(r, theme.colors.background);
    let one = draw_tip(theme, text, "Tuesday, September 29, 2026");
    c.blit_canvas(r.x + 10, r.y + 10, &one);
    let two = draw_tip(theme, text, "Volume\nMuted");
    c.blit_canvas(r.x + 20 + one.width as i32, r.y + 10, &two);
}
