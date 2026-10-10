//! The window overview in Reprise Blue: the selected cell in a rounded box of
//! the scheme's `hilight` at half strength with a light rim, the hot one in a
//! fainter one, titles in bold white over a dark shadow. A window with no picture
//! gets a rounded `button_face` box with its icon centred. Layout is the shared `crate::overview`.

use crate::overview::{Cell, OverviewLayout, STAND_IN_PX, title_run};
use crate::raster::{Canvas, Rect, rounded_border, rounded_fill};
use crate::text::TextRenderer;
use crate::theme::{Color, Theme};

const RADIUS: i32 = 6;
const SELECT: (u32, u32) = (128, 180);
const HOT: (u32, u32) = (56, 100);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const SHADOW: Color = Color::rgb(0, 0, 0);

/// Draw the cells of `layout` (device pixels) into the clear `canvas` covering `layout.area`:
/// `cells[i]` is window `i`, with the `selected` and `hot` ones boxed. The host lays live pictures over each `picture` rect.
pub fn draw(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &OverviewLayout,
    cells: &[Cell],
    selected: usize,
    hot: Option<usize>,
) {
    let cs = &theme.colors;
    let s = layout.scale.max(1);
    let size = theme.fonts.size_px * s as u32;
    let mut fallback: Option<Canvas> = None;
    for (i, c) in cells.iter().enumerate().take(layout.cells.len()) {
        let (cell, pic, row) = (layout.cells[i], layout.pictures[i], layout.titles[i]);
        let strength = if i == selected {
            Some(SELECT)
        } else if hot == Some(i) {
            Some(HOT)
        } else {
            None
        };
        if let Some((fill, rim)) = strength {
            boxed(canvas, cell, RADIUS * s, s, cs.hilight, fill, rim);
        }
        if c.picture.is_none() {
            let mut face = Canvas::new(pic.width.max(1) as u32, pic.height.max(1) as u32);
            let b = face.bounds();
            rounded_fill(&mut face, b, RADIUS * s, cs.button_face);
            canvas.blit_over(pic.x, pic.y, &face);
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
        let (ix, tx, title) = title_run(
            row,
            c.title,
            s,
            c.icon.is_some(),
            |t| text.measure(t, true, size),
            |t, w| text.ellipsize(t, w, true, size),
        );
        if let Some(icon) = c.icon {
            canvas.blit_over(ix, row.y + (row.height - icon.height as i32) / 2, icon);
        }
        if let Some(base) = text.baseline(row, true, size) {
            let clip = Rect::new(row.x, row.y, row.width + s, row.height + s);
            text.draw_run_over(canvas, tx + s, base + s, clip, &title, true, SHADOW, size);
            text.draw_run_over(canvas, tx, base, clip, &title, true, WHITE, size);
        }
    }
}

/// A rounded box over `r`: `color` at `fill`/255 inside a `s` px white rim at `rim`/255.
fn boxed(canvas: &mut Canvas, r: Rect, radius: i32, s: i32, color: Color, fill: u32, rim: u32) {
    let (w, h) = (r.width.max(1) as u32, r.height.max(1) as u32);
    let mut inside = Canvas::new(w, h);
    let b = inside.bounds();
    rounded_fill(&mut inside, b, radius, color);
    canvas.blit_over_faded(r.x, r.y, &inside, fill, 255);
    let mut edge = Canvas::new(w, h);
    let b = edge.bounds();
    rounded_border(&mut edge, b, radius, WHITE, s);
    canvas.blit_over_faded(r.x, r.y, &edge, rim, 255);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview::ICON_PX;
    use crate::theme::FontConfig;

    fn sample(selected: usize, hot: Option<usize>) -> (Canvas, OverviewLayout, Theme) {
        let theme = crate::blue::style_for(crate::blue::Variant::Blue);
        let theme = Theme {
            colors: theme.colors.clone(),
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
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
                icon: None,
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
        draw(&mut c, &theme, &text, &l, &cells, selected, hot);
        (c, l, theme)
    }

    fn split(px: u32) -> (u32, (u32, u32, u32)) {
        let a = px >> 24;
        let un = |sh: u32| ((px >> sh) & 0xFF) * 255 / a.max(1);
        (a, (un(16), un(8), un(0)))
    }

    #[test]
    fn the_selected_cell_is_a_half_strength_hilight_box_with_a_rim() {
        let (c, l, theme) = sample(0, Some(2));
        let cell = l.cells[0];
        let hl = theme.colors.hilight;
        let (a, rgb) = split(c.get(cell.x + 4, cell.y + cell.height / 2));
        assert_eq!(a, 128);
        let near = |v: u32, w: u8| (v as i32 - i32::from(w)).abs() <= 2;
        assert!(
            near(rgb.0, hl.r) && near(rgb.1, hl.g) && near(rgb.2, hl.b),
            "{rgb:?}"
        );
        let (_, rim) = split(c.get(cell.x, cell.y + cell.height / 2));
        assert!(rim.0 > rgb.0 && rim.2 > rgb.2, "{rim:?}");
        assert_eq!(c.get(cell.x, cell.y), 0, "the corner is rounded off");
        let h = l.cells[2];
        let (ha, _) = split(c.get(h.x + 4, h.y + h.height / 2));
        assert_eq!(ha, 56);
        assert_eq!(c.get(l.cells[1].x + 4, l.cells[1].y + 40), 0, "no box");
    }

    #[test]
    fn titles_are_white_over_a_shadow() {
        let (c, l, _) = sample(1, None);
        let row = l.titles[2];
        let mut white = 0;
        let mut dark = 0;
        for y in row.y..row.bottom() + 1 {
            for x in row.x..row.right() + 1 {
                match c.get(x, y) {
                    0xFFFF_FFFF => white += 1,
                    0xFF00_0000 => dark += 1,
                    _ => {}
                }
            }
        }
        assert!(white > 20 && dark > 5, "white {white}, shadow {dark}");
    }

    #[test]
    fn a_window_without_a_picture_gets_a_rounded_face() {
        let (c, l, theme) = sample(0, None);
        let p = l.pictures[1];
        assert_eq!(c.get(p.x + 8, p.y + 8), theme.colors.button_face.argb());
        assert_eq!(c.get(p.x, p.y), 0, "rounded");
    }
}
