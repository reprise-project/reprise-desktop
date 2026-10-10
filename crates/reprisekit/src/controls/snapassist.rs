use crate::look::widgets;
use crate::overview::{Cell, ICON_GAP, ICON_PX, OverviewLayout, STAND_IN_PX, assist_panel};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

pub fn draw(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &OverviewLayout,
    cells: &[Cell],
    lit: Option<usize>,
    hot: Option<usize>,
) {
    let p = theme.paint();
    let cs = &theme.colors;
    let s = layout.scale.max(1);
    let size = theme.fonts.size_px * s as u32;
    let panel = assist_panel((layout.area.width, layout.area.height), s);
    widgets::draw_well(canvas, &p, panel, EdgeStyle::ButtonSunken);
    let mut fallback: Option<Canvas> = None;
    for (i, c) in cells.iter().enumerate().take(layout.cells.len()) {
        let (row, pic) = (layout.titles[i], layout.pictures[i]);
        let on = hot == Some(i) || lit == Some(i);
        let (bg, fg) = if on {
            (cs.hilight, cs.hilight_text)
        } else {
            (cs.button_face, cs.button_text)
        };
        canvas.fill_rect(row, bg);
        if c.picture.is_none() {
            canvas.fill_rect(pic, cs.button_shadow);
            let icon = match c.stand_in {
                Some(icon) => icon,
                None => {
                    &*fallback.get_or_insert_with(|| crate::icon::fallback(theme, STAND_IN_PX * s))
                }
            };
            let (w, h) = (icon.width as i32, icon.height as i32);
            canvas.blit_over(
                pic.x + (pic.width - w) / 2,
                pic.y + (pic.height - h) / 2,
                icon,
            );
        }
        let close = layout.assist_close(i).filter(|_| hot == Some(i));
        let mut x = row.x + 8 * s;
        if let Some(icon) = c.icon {
            canvas.blit_over(x, row.y + (row.height - icon.height as i32) / 2, icon);
            x += (ICON_PX + ICON_GAP) * s;
        }
        let end = close.map_or(row.right() - 8 * s, |b| b.x - 4 * s);
        let title = text.ellipsize(c.title, end - x, false, size);
        let at = Rect::new(x, row.y, end - x, row.height);
        text.draw(canvas, at, &title, false, fg, Align::Left, size);
        if let Some(b) = close {
            widgets::draw_close_box(canvas, &p, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::style::{Part, State};
    use crate::theme::FontConfig;

    #[test]
    fn blue_draws_its_own_close_box_on_the_hot_card() {
        let t = crate::interfaces::theme_for(Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let l = OverviewLayout::compute_assist(2, Rect::new(0, 0, 480, 400), (960, 800));
        let cells: Vec<Cell> = ["Notepad", "Terminal"]
            .iter()
            .map(|&title| Cell {
                title,
                icon: None,
                picture: None,
                stand_in: None,
            })
            .collect();
        let mut c = Canvas::new(480, 400);
        draw(&mut c, &t, &text, &l, &cells, None, Some(1));
        let b = l.assist_close(1).unwrap();
        let mut want = Canvas::new(b.width as u32, b.height as u32);
        let st = t.look.style().unwrap();
        let whole = want.bounds();
        st.draw(&mut want, whole, Part::CloseButton, State::Normal, 1);
        let (x, y) = (b.width / 2, 3);
        assert_eq!(
            c.get(b.x + x, b.y + y),
            want.get(x, y),
            "Blue's close button"
        );
        assert_ne!(
            c.get(b.x + x, b.y + y),
            t.colors.button_face.argb(),
            "not a grey box"
        );
    }

    #[test]
    fn classic_draws_a_sunken_well_and_lights_the_hot_card() {
        let t = Theme::default();
        let cs = &t.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let zone = Rect::new(0, 0, 480, 400);
        let l = OverviewLayout::compute_assist(2, zone, (960, 800));
        let cells: Vec<Cell> = ["Notepad", "Terminal"]
            .iter()
            .map(|&title| Cell {
                title,
                icon: None,
                picture: None,
                stand_in: None,
            })
            .collect();
        let mut c = Canvas::new(480, 400);
        draw(&mut c, &t, &text, &l, &cells, None, Some(1));
        assert_eq!(c.get(5, 5) >> 24, 0, "the 13 px margin is clear");
        assert_eq!(
            c.get(13, 13),
            cs.button_shadow.argb(),
            "the well's sunken edge"
        );
        let h = l.titles[1];
        assert_eq!(c.get(h.x + 2, h.y + 2), cs.hilight.argb(), "the hot header");
        let h0 = l.titles[0];
        assert_eq!(c.get(h0.x + 2, h0.y + 2), cs.button_face.argb());
        let x = l.assist_close(1).unwrap();
        assert_ne!(
            c.get(x.x + x.width / 2, x.y + x.height / 2),
            cs.hilight.argb(),
            "the close box"
        );
    }
}
