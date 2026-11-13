//! The display switcher in Reprise Glass: the snap layouts sheet with the Alt+Tab switcher's
//! selection and hot boxes.

use super::switcher::draw_title;
use crate::projection::Projection;
use crate::projector::{Ink, ProjectorLayout, draw_picture};
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::text::TextRenderer;
use crate::theme::Color;

const MARK: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// A greyed tile's opacity, of 255.
const GREYED: u32 = 96;

/// Draws the switcher into `canvas` (sized to `layout`, device px) in `style`: the lit tile in
/// the switcher's selection box, the hot one in its hot box, each picture's lit screens in
/// `hilight`, captions in `text` at `size_px` over the caption glow, a greyed tile faded.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    size_px: u32,
    hilight: Color,
    layout: &ProjectorLayout,
    lit: usize,
    hot: Option<usize>,
    enabled: [bool; 4],
) {
    let s = layout.scale.max(1);
    let whole = Rect::new(0, 0, layout.width, layout.height);
    style.draw(canvas, whole, Part::SnapFrame, State::Normal, s);
    let ink = Ink {
        frame: style.caption_text.color,
        screen: Some(hilight),
        mark: MARK,
    };
    for (i, choice) in Projection::ALL.into_iter().enumerate() {
        let tile = layout.tiles[i];
        if i == lit {
            style.draw(canvas, tile, Part::SwitcherSelect, State::Normal, s);
        } else if hot == Some(i) {
            style.draw(canvas, tile, Part::SwitcherHot, State::Normal, s);
        }
        let mut t = Canvas::new(tile.width.max(1) as u32, tile.height.max(1) as u32);
        let local = |r: Rect| Rect::new(r.x - tile.x, r.y - tile.y, r.width, r.height);
        draw_picture(
            &mut t,
            text,
            size_px,
            local(layout.picture(i)),
            choice,
            &ink,
            s,
        );
        let band = local(layout.caption(i));
        let bold = style.caption_text.bold;
        let baseline = text.baseline(band, bold, size_px).unwrap_or(band.bottom());
        let clip = t.bounds();
        let label = choice.label();
        draw_title(
            &mut t, style, text, size_px, band, baseline, clip, &label, s,
        );
        let alpha = if enabled[i] { 255 } else { GREYED };
        canvas.blit_over_faded(tile.x, tile.y, &t, alpha, 255);
    }
}
