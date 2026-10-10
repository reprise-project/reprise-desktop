//! Classic's task button preview panel: a cell per window with its title over a well. The host owns the pictures,
//! timers and scene; layout, hit tests and [`fit`] are the root [`crate::taskpreview`], re-exported here.

use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::art::glyph::Glyph;
use crate::classic::widgets::{CLOSE_H, CLOSE_W, draw_caption_button};
use crate::raster::{Canvas, EdgeStyle, Insets, Rect};
use crate::taskpreview::{TITLE_GAP, TITLE_H, WELL_H};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

pub use crate::taskpreview::{CELL_H, CELL_W, Cell, Grid, TaskPreviewLayout, close_hit, fit, hit};

const PAD: i32 = 6;
const GAP: i32 = 6;
/// The title row's icon, 1 px in from the row's top-left, and the text
/// 4 px after it.
const ICON_PX: i32 = 16;
const TEXT_GAP: i32 = 4;

/// Classic's panel: 176x132 cells 6 px apart and 6 px in from the edge,
/// the well under the title row, the close button a caption close box at
/// the title row's right end, 2 px in. No shadow, square corners.
pub const CLASSIC: Grid = Grid {
    cell: (CELL_W, CELL_H),
    pad: PAD,
    gap: GAP,
    well: Rect::new(0, TITLE_H + TITLE_GAP, CELL_W, WELL_H),
    close: Rect::new(CELL_W - 2 - CLOSE_W, 2, CLOSE_W, CLOSE_H),
    shadow: Insets::ZERO,
    corner: &[],
};

/// Draw a sunken well with `picture` (an icon standing in for the live
/// one) centred in it.
pub fn draw_well(canvas: &mut Canvas, theme: &Theme, well: Rect, picture: Option<&Canvas>) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    canvas.fill_rect(well, cs.button_shadow);
    canvas.edge_t(well, EdgeStyle::ButtonSunken, cs, s);
    if let Some(p) = picture {
        let (w, h) = (p.width as i32, p.height as i32);
        canvas.blit_canvas(
            well.x + (well.width - w) / 2,
            well.y + (well.height - h) / 2,
            p,
        );
    }
}

/// Draw Classic's panel into `canvas` (sized to `layout.rect`, which is
/// in device pixels at `theme.scale`): a raised window edge, and per cell
/// the icon and title over its well; the `hot` cell's title row in the
/// highlight colours, a close box at its right end.
pub fn draw(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &TaskPreviewLayout,
    cells: &[Cell],
    hot: Option<usize>,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let all = Rect::new(0, 0, layout.rect.width, layout.rect.height);
    canvas.fill_rect(all, cs.button_face);
    canvas.edge_t(all, EdgeStyle::WindowRaised, cs, s);
    for (i, cell) in cells.iter().enumerate() {
        let (Some(row), Some(well)) = (layout.title_row(i), layout.well(i)) else {
            break;
        };
        let close = layout.close(i).filter(|_| hot == Some(i));
        let fg = if close.is_some() {
            canvas.fill_rect(row, cs.hilight);
            cs.hilight_text
        } else {
            cs.button_text
        };
        if let Some(icon) = cell.icon {
            canvas.blit_canvas(row.x + s, row.y + s, icon);
        }
        let tx = row.x + s + (ICON_PX + TEXT_GAP) * s;
        let end = close.map_or(row.right(), |c| c.x);
        let tw = end - 2 * s - tx;
        let title = text.ellipsize(cell.title, tw, cell.active, size);
        text.draw(
            canvas,
            Rect::new(tx, row.y, tw, row.height),
            &title,
            cell.active,
            fg,
            Align::Left,
            size,
        );
        draw_well(canvas, theme, well, cell.picture);
        if let Some(c) = close {
            draw_caption_button(canvas, cs, c, Glyph::Close, false, s);
        }
    }
}

/// The panel's screen reader nodes under `parent`, laid out from the
/// panel's top-left: a button per cell named for its window's title, and
/// in the `hot` cell its close button, named "Close".
pub fn access(
    layout: &TaskPreviewLayout,
    titles: &[&str],
    hot: Option<usize>,
    tree: &mut Tree,
    parent: NodeId,
) {
    for (i, (cell, title)) in layout.cells.iter().zip(titles).enumerate() {
        let id = tree.push(parent, i, access::named(Role::Button, *cell, title));
        if let Some(close) = layout.close(i).filter(|_| hot == Some(i)) {
            tree.push(id, "close", access::named(Role::Button, close, "Close"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskbar::Edge;
    use crate::theme::FontConfig;

    #[test]
    fn access_names_each_cell_and_the_hot_cells_close_button() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        let mut tree = Tree::window("Previews", (l.rect.width, l.rect.height));
        access(
            &l,
            &["Notes", "Mail"],
            Some(1),
            &mut tree,
            crate::access::ROOT,
        );
        let bounds = |label| {
            let id = tree.find(Role::Button, label).expect(label);
            tree.get(id).unwrap().bounds().unwrap()
        };
        assert_eq!(
            bounds("Notes"),
            access::bounds(l.cells[0]),
            "cell 0 is a button"
        );
        assert_eq!(
            bounds("Mail"),
            access::bounds(l.cells[1]),
            "cell 1 is a button"
        );
        assert_eq!(
            bounds("Close"),
            access::bounds(l.close(1).unwrap()),
            "the hot cell's close button"
        );
        let closes = tree
            .nodes()
            .filter(|(_, n)| n.label() == Some("Close"))
            .count();
        assert_eq!(closes, 1, "only the hot cell has a close button");
    }

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1052);

    #[test]
    fn one_window_centres_a_single_cell_over_its_button() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            1,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect, Rect::new(286, 908, 188, 144));
        assert_eq!(l.cells, [Rect::new(6, 6, 176, 132)]);
        assert_eq!(l.well(0), Some(Rect::new(6, 28, 176, 110)));
    }

    #[test]
    fn a_group_lays_cells_side_by_side() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            3,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect.width, 12 + 3 * 176 + 2 * 6);
        assert_eq!(l.cells[1].x, 6 + 182);
        assert_eq!(l.cells[2], Rect::new(6 + 2 * 182, 6, 176, 132));
    }

    #[test]
    fn a_panel_near_the_edge_is_pushed_inside() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            3,
            Rect::new(4, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect.x, 0);
        let r = TaskPreviewLayout::compute(
            CLASSIC,
            1,
            Rect::new(1850, 1054, 60, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(r.rect.right(), 1920);
    }

    #[test]
    fn more_cells_than_fit_wrap_into_rows() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            11,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.rect.width, 12 + 10 * 176 + 9 * 6);
        assert_eq!(l.rect.height, 12 + 2 * 132 + 6);
        assert_eq!(l.cells[10], Rect::new(6, 6 + 138, 176, 132));
    }

    #[test]
    fn hit_finds_the_cell_and_misses_the_gaps() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(hit(&l, 10, 10), Some(0));
        assert_eq!(hit(&l, 6 + 182 + 100, 100), Some(1));
        assert_eq!(hit(&l, 6 + 177, 50), None);
        assert_eq!(hit(&l, 2, 2), None);
    }

    #[test]
    fn draw_highlights_the_hot_title_and_sinks_the_wells() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        let mut c = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let red = crate::theme::Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let cells = [
            Cell {
                title: "one",
                icon: None,
                active: true,
                picture: None,
            },
            Cell {
                title: "two",
                icon: None,
                active: false,
                picture: Some(&icon),
            },
        ];
        draw(&mut c, &theme, &text, &l, &cells, Some(1));
        let cs = &theme.colors;
        assert_eq!(c.get(0, 10), cs.button_face.argb());
        assert_eq!(c.get(1, 10), cs.button_hilight.argb());
        assert_eq!(c.get(l.rect.width - 1, 10), cs.button_dk_shadow.argb());
        let (a, b) = (l.cells[0], l.cells[1]);
        assert_eq!(c.get(b.right() - 2, b.y + 2), cs.hilight.argb());
        assert_eq!(c.get(a.right() - 2, a.y + 2), cs.button_face.argb());
        let w = l.well(0).unwrap();
        assert_eq!(c.get(w.x, w.y + 20), cs.button_shadow.argb());
        assert_eq!(c.get(w.x + 1, w.y + 20), cs.button_dk_shadow.argb());
        assert_eq!(c.get(w.right() - 1, w.y + 20), cs.button_hilight.argb());
        assert_eq!(c.get(w.x + 10, w.y + 10), cs.button_shadow.argb());
        let w1 = l.well(1).unwrap();
        assert_eq!(c.get(w1.x + 88, w1.y + 55), red.argb());
        assert_eq!(c.get(w1.x + 88 - 17, w1.y + 55), cs.button_shadow.argb());
        let x = l.close(1).unwrap();
        assert_eq!(
            c.get(x.right() - 1, x.y + 5),
            cs.button_dk_shadow.argb(),
            "the close box's right edge is dark shadow"
        );
        let x0 = l.close(0).unwrap();
        assert_eq!(
            c.get(x0.right() - 1, x0.y + 5),
            cs.button_face.argb(),
            "a cold cell's title row stays face where a close box would be"
        );
    }

    #[test]
    fn the_close_box_sits_at_the_title_rows_right_end() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.close(0), Some(Rect::new(6 + 176 - 18, 8, 16, 14)));
        assert_eq!(l.close(1), Some(Rect::new(6 + 182 + 176 - 18, 8, 16, 14)));
        assert_eq!(l.scaled(2).close(0), Some(Rect::new(2 * 164, 16, 32, 28)));
    }

    #[test]
    fn a_classic_panel_takes_the_pointer_everywhere_on_it() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        assert_eq!(l.input_area(), Rect::new(0, 0, l.rect.width, l.rect.height));
    }

    #[test]
    fn close_hit_finds_the_close_box_only() {
        let l = TaskPreviewLayout::compute(
            CLASSIC,
            2,
            Rect::new(300, 1054, 160, 22),
            SCREEN,
            Edge::Bottom,
        );
        let x = l.close(1).unwrap();
        let hot = Some(1);
        assert_eq!(close_hit(&l, hot, x.x, x.y), Some(1), "its top-left pixel");
        assert_eq!(close_hit(&l, hot, x.right() - 1, x.bottom() - 1), Some(1));
        assert_eq!(
            close_hit(&l, None, x.x, x.y),
            None,
            "no cell hot: no close box drawn, none hit"
        );
        assert_eq!(
            close_hit(&l, Some(0), x.x, x.y),
            None,
            "another cell hot: this cell's close box is not drawn"
        );
        assert_eq!(
            close_hit(&l, hot, x.x - 1, x.y + 5),
            None,
            "the title left of it"
        );
        assert_eq!(
            close_hit(&l, hot, x.x + 5, x.bottom()),
            None,
            "the well under it"
        );
    }
}
