//! The window overview drawn flat in Classic: no bevels and no shading.

use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

pub use crate::overview::*;

const FRAME: i32 = 2;

/// Draw the cells of `layout` (in device pixels) into `canvas`, which covers `layout.area` and is
/// clear: `cells[i]` is window `i`, the `selected` one framed and the `hot` one ringed.
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
        if i == selected {
            frame(canvas, cell, s, cs.hilight_text);
            let inner = cell.inset(s);
            frame(canvas, inner, FRAME * s, cs.hilight);
            canvas.fill_rect(
                Rect::new(inner.x, row.y, inner.width, inner.bottom() - row.y),
                cs.hilight,
            );
        } else if hot == Some(i) {
            frame(canvas, cell, s, cs.button_shadow);
        }
        if c.picture.is_none() {
            canvas.fill_rect(pic, cs.button_face);
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
            |t| text.measure(t, false, size),
            |t, w| text.ellipsize(t, w, false, size),
        );
        if let Some(icon) = c.icon {
            canvas.blit_over(ix, row.y + (row.height - icon.height as i32) / 2, icon);
        }
        if let Some(base) = text.baseline(row, false, size) {
            text.draw_run_over(canvas, tx, base, row, &title, false, cs.hilight_text, size);
        }
    }
}

fn frame(canvas: &mut Canvas, r: Rect, t: i32, color: crate::theme::Color) {
    canvas.fill_rect(Rect::new(r.x, r.y, r.width, t), color);
    canvas.fill_rect(Rect::new(r.x, r.bottom() - t, r.width, t), color);
    canvas.fill_rect(Rect::new(r.x, r.y, t, r.height), color);
    canvas.fill_rect(Rect::new(r.right() - t, r.y, t, r.height), color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Color, FontConfig};

    fn sample(selected: usize, hot: Option<usize>, scale: i32) -> (Canvas, OverviewLayout) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = OverviewLayout::compute(3, Rect::new(0, 0, 1280, 720)).scaled(scale);
        let red = Color::rgb(255, 0, 0);
        let mut icon = Canvas::new((ICON_PX * scale) as u32, (ICON_PX * scale) as u32);
        icon.fill_rect(icon.bounds(), red);
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
                icon: Some(&icon),
                picture: Some(l.pictures[2]),
                stand_in: None,
            },
        ];
        let mut c = Canvas::new(l.area.width as u32, l.area.height as u32);
        draw(&mut c, &theme, &text, &l, &cells, selected, hot);
        (c, l)
    }

    #[test]
    fn the_selection_is_a_flat_frame_over_a_title_band() {
        let cs = Theme::default().colors;
        let (c, l) = sample(0, Some(2), 1);
        let (cell, row) = (l.cells[0], l.titles[0]);
        let (hl, ring) = (cs.hilight.argb(), cs.hilight_text.argb());
        assert_eq!(c.get(cell.x, cell.y), ring, "a 1 px light ring");
        assert_eq!(c.get(cell.right() - 1, cell.y + 40), ring);
        assert_eq!(c.get(cell.x + 1, cell.y + 1), hl);
        assert_eq!(c.get(cell.x + 2, cell.y + 2), hl, "2 px inside it");
        assert_eq!(c.get(cell.x + 3, cell.y + 3), 0, "clear inside the frame");
        assert_eq!(
            c.get(cell.x, row.y + 1),
            ring,
            "the ring goes round the band"
        );
        assert_eq!(c.get(cell.x + 1, row.y + 1), hl, "the band");
        assert_eq!(c.get(cell.right() - 4, row.bottom() - 2), hl);
        assert_eq!(c.get(cell.x + 20, cell.bottom() - 1), ring);
        let p = l.pictures[0];
        assert_eq!(c.get(p.x + p.width / 2, p.y + p.height / 2), 0);
        let h = l.cells[2];
        assert_eq!(c.get(h.x, h.y + 20), cs.button_shadow.argb());
        assert_eq!(c.get(h.x + 1, h.y + 20), 0);
        assert_eq!(c.get(l.cells[1].x, l.cells[1].y + 20), 0);
    }

    #[test]
    fn a_window_without_a_picture_gets_a_face_box_and_its_icon() {
        let cs = Theme::default().colors;
        let (c, l) = sample(0, None, 1);
        let p = l.pictures[1];
        assert_eq!(c.get(p.x + 2, p.y + 2), cs.button_face.argb());
        let (mx, my) = (p.x + p.width / 2, p.y + p.height / 2);
        let left = mx - STAND_IN_PX / 2;
        assert_eq!(c.get(left, my), cs.button_dk_shadow.argb());
    }

    #[test]
    fn the_title_is_centred_under_the_picture_after_its_icon() {
        let cs = Theme::default().colors;
        let (c, l) = sample(1, None, 1);
        let row = l.titles[0];
        let white = cs.hilight_text.argb();
        let ink: Vec<i32> = (row.x..row.right())
            .filter(|&x| (row.y..row.bottom()).any(|y| c.get(x, y) == white))
            .collect();
        assert!(!ink.is_empty());
        let red = Color::rgb(255, 0, 0).argb();
        let icon: Vec<i32> = (row.x..row.right())
            .filter(|&x| c.get(x, row.y + row.height / 2) == red)
            .collect();
        assert_eq!(icon.len(), ICON_PX as usize);
        let (from, to) = (icon[0], *ink.last().unwrap());
        let mid = (from + to) / 2;
        assert!((mid - (row.x + row.width / 2)).abs() <= 2, "{from}..{to}");
        assert!(
            ink[0] >= icon[ICON_PX as usize - 1] + ICON_GAP,
            "text after the icon"
        );
    }

    #[test]
    fn scale_two_doubles_the_frame() {
        let cs = Theme::default().colors;
        let (c, l) = sample(0, None, 2);
        let cell = l.cells[0];
        assert_eq!(c.get(cell.x + 1, cell.y + 1), cs.hilight_text.argb());
        assert_eq!(c.get(cell.x + 2, cell.y + 2), cs.hilight.argb());
        assert_eq!(c.get(cell.x + 5, cell.y + 5), cs.hilight.argb());
        assert_eq!(c.get(cell.x + 6, cell.y + 6), 0);
    }
}
