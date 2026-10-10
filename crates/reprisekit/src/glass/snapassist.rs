//! Snap assist in Reprise Glass: the zone's glass panel over the blurred backdrop, with a card per
//! window. The layout is the shared `crate::overview`.

use super::overview::draw_stand_in;
use crate::overview::{Cell, ICON_GAP, ICON_PX, OverviewLayout, assist_panel};
use crate::raster::{Canvas, Rect, rounded_fill};
use crate::style::{Part, State, Style};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

const HEADER: (Color, u32) = (Color::rgb(0, 0, 0), 120);
const LIT: (Color, u32) = (Color::rgb(0xFF, 0xFF, 0xFF), 40);
const RADIUS: i32 = 8;

#[expect(clippy::too_many_arguments)]
pub fn draw(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    size_px: u32,
    layout: &OverviewLayout,
    cells: &[Cell],
    lit: Option<usize>,
    hot: Option<usize>,
) {
    let s = layout.scale.max(1);
    let panel = assist_panel((layout.area.width, layout.area.height), s);
    style.draw(canvas, panel, Part::SnapFrame, State::Normal, s);
    let faded = |canvas: &mut Canvas, r: Rect, (color, alpha): (Color, u32)| {
        let mut t = Canvas::new(r.width.max(1) as u32, r.height.max(1) as u32);
        rounded_fill(
            &mut t,
            Rect::new(0, 0, r.width, r.height),
            RADIUS * s,
            color,
        );
        canvas.blit_over_faded(r.x, r.y, &t, alpha, 255);
    };
    let bold = style.caption_text.bold;
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    for (i, c) in cells.iter().enumerate().take(layout.cells.len()) {
        let (card, row, pic) = (layout.cells[i], layout.titles[i], layout.pictures[i]);
        if hot == Some(i) || lit == Some(i) {
            faded(canvas, card, LIT);
        }
        if c.picture.is_none() {
            draw_stand_in(canvas, pic, c);
        }
        faded(canvas, row, HEADER);
        let mut x = row.x + 8 * s;
        if let Some(icon) = c.icon {
            canvas.blit_over(x, row.y + (row.height - icon.height as i32) / 2, icon);
            x += (ICON_PX + ICON_GAP) * s;
        }
        let close = layout.assist_close(i).filter(|_| hot == Some(i));
        let end = close.map_or(row.right() - 8 * s, |b| b.x - 4 * s);
        let title = text.ellipsize(c.title, end - x, bold, size_px);
        let at = Rect::new(x, row.y, end - x, row.height);
        text.draw(canvas, at, &title, bold, white, Align::Left, size_px);
        if let Some(b) = close {
            style.draw(canvas, b, Part::PreviewClose, State::Normal, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::Settings;
    use crate::overview::Cell;

    #[test]
    fn glass_centres_the_stand_in_in_an_empty_picture_box() {
        let s = super::super::style(&Settings::default());
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let l = OverviewLayout::compute_assist(1, Rect::new(0, 0, 480, 400), (960, 800));
        let red = Color::rgb(0xFF, 0, 0);
        let mut icon = Canvas::new(48, 48);
        icon.fill_rect(icon.bounds(), red);
        let cells = [Cell {
            title: "Notepad",
            icon: None,
            picture: None,
            stand_in: Some(&icon),
        }];
        let mut c = Canvas::new(480, 400);
        draw(&mut c, &s, &text, 12, &l, &cells, None, None);
        let p = l.pictures[0];
        assert_eq!(
            c.get(p.x + p.width / 2, p.y + p.height / 2),
            red.argb(),
            "the stand-in"
        );
    }

    #[test]
    fn glass_draws_its_sheet_and_a_lit_card() {
        let s = super::super::style(&Settings::default());
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let l = OverviewLayout::compute_assist(1, Rect::new(0, 0, 480, 400), (960, 800));
        let cells = [Cell {
            title: "Notepad",
            icon: None,
            picture: None,
            stand_in: None,
        }];
        let mut c = Canvas::new(480, 400);
        draw(&mut c, &s, &text, 12, &l, &cells, None, Some(0));
        assert_eq!(c.get(5, 5) >> 24, 0, "the margin is clear");
        assert_ne!(c.get(240, 20) >> 24, 0, "the sheet");
        // Under the picture, in the frame round it: (x + 2, bottom - 3)
        // would sit outside the card's 8 px rounded corner.
        let card = l.cells[0];
        assert_ne!(
            c.get(card.x + card.width / 2, card.bottom() - 3),
            c.get(240, 20),
            "the lit card stands out"
        );
    }
}
