use super::switcher::draw_title;
use crate::overview::{Cell, OverviewLayout, title_run};
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::text::TextRenderer;

#[expect(clippy::too_many_arguments)]
pub fn draw(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    size_px: u32,
    layout: &OverviewLayout,
    cells: &[Cell],
    selected: usize,
    hot: Option<usize>,
) {
    let s = layout.scale.max(1);
    let bold = style.caption_text.bold;
    let whole = canvas.bounds();
    for (i, c) in cells.iter().enumerate().take(layout.cells.len()) {
        let (cell, pic, row) = (layout.cells[i], layout.pictures[i], layout.titles[i]);
        if i == selected {
            style.draw(canvas, cell, Part::SwitcherSelect, State::Normal, s);
        } else if hot == Some(i) {
            style.draw(canvas, cell, Part::SwitcherHot, State::Normal, s);
        }
        match c.picture {
            Some(r) => style.draw(canvas, r, Part::SwitcherShadow, State::Normal, s),
            None => draw_stand_in(canvas, pic, c),
        }
        let (ix, tx, title) = title_run(
            row,
            c.title,
            s,
            c.icon.is_some(),
            |t| text.measure(t, bold, size_px),
            |t, w| text.ellipsize(t, w, bold, size_px),
        );
        if let Some(icon) = c.icon {
            canvas.blit_over(ix, row.y + (row.height - icon.height as i32) / 2, icon);
        }
        let Some(base) = text.baseline(row, bold, size_px) else {
            continue;
        };
        // A band as wide as the text, so `draw_title` centres it where
        // the title row put it.
        let w = text.measure(&title, bold, size_px);
        let band = Rect::new(tx, row.y, w, row.height);
        draw_title(canvas, style, text, size_px, band, base, whole, &title, s);
    }
}

pub(super) fn draw_stand_in(canvas: &mut Canvas, pic: Rect, c: &Cell) {
    if let Some(icon) = c.stand_in.or(c.icon) {
        let (w, h) = (icon.width as i32, icon.height as i32);
        canvas.blit_over(
            pic.x + (pic.width - w) / 2,
            pic.y + (pic.height - h) / 2,
            icon,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::overview::ICON_PX;
    use crate::theme::Color;

    fn sample(selected: usize, hot: Option<usize>) -> (Canvas, OverviewLayout) {
        let theme = crate::interfaces::theme_for(Interface::Glass);
        let style = theme.look.style().unwrap();
        let text = crate::look::caption_text(style);
        let l = OverviewLayout::compute(3, Rect::new(0, 0, 1280, 720));
        let mut icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        icon.fill_rect(icon.bounds(), Color::rgb(255, 0, 0));
        let cells = [
            Cell {
                title: "Terminal",
                icon: Some(&icon),
                picture: Some(l.pictures[0]),
                stand_in: None,
            },
            Cell {
                title: "Notepad",
                icon: Some(&icon),
                picture: None,
                stand_in: None,
            },
            Cell {
                title: "Explorer",
                icon: None,
                picture: Some(l.pictures[2]),
                stand_in: None,
            },
        ];
        let mut c = Canvas::new(l.area.width as u32, l.area.height as u32);
        let size = style.caption_text.size_px;
        draw(&mut c, style, &text, size, &l, &cells, selected, hot);
        (c, l)
    }

    #[test]
    fn the_selected_box_is_stronger_than_the_hot_one() {
        let (c, l) = sample(0, Some(2));
        let alpha = |px: u32| px >> 24;
        let (sel, hot) = (l.cells[0], l.cells[2]);
        let a = alpha(c.get(sel.x + 4, sel.y + sel.height / 2));
        let b = alpha(c.get(hot.x + 4, hot.y + hot.height / 2));
        assert!(a > b && b > 0, "selected {a}, hot {b}");
        assert_eq!(c.get(l.cells[1].x + 4, l.cells[1].y + 40), 0, "no box");
    }

    #[test]
    fn a_picture_casts_a_shadow_and_a_bare_cell_shows_its_icon() {
        let (c, l) = sample(1, None);
        let p = l.pictures[2];
        assert!(c.get(p.right() + 1, p.y + p.height / 2) >> 24 > 0, "shadow");
        let q = l.pictures[1];
        assert_eq!(
            c.get(q.x + q.width / 2, q.y + q.height / 2),
            Color::rgb(255, 0, 0).argb()
        );
    }

    #[test]
    fn the_title_is_inked_in_its_row() {
        let (c, l) = sample(1, None);
        let row = l.titles[0];
        let inked = (row.x..row.right())
            .flat_map(|x| (row.y..row.bottom()).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) >> 24 == 0xFF)
            .count();
        assert!(inked > 20, "{inked}");
    }
}
