//! The Alt+Tab switcher: a centred panel listing windows and open dialogs in focus order (most recent first). The icon style shows one 32 px icon per entry, seven to a row, the selected one in a 2 px highlight frame and the hovered one in a 1 px one, with the hovered or else selected entry's title in a sunken field below; with the preview on, a sunken well above holds a live picture of the selected window, which the host puts there.
//! The thumbnail style gives every entry a [`CLASSIC`] cell: the window's picture with its icon at the bottom-right, under a title band. The model and both layouts are the root [`crate::switcher`], re-exported here; this module draws Classic's panels. The host owns the scene and the keys.

use crate::access::{self, Tree, accesskit};
use crate::raster::{Canvas, EdgeStyle, Insets, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;
use accesskit::{Node, NodeId, Role};

pub use crate::switcher::*;

const ICON_INSET: i32 = 5;

/// Classic's thumbnail panel: a cell of 144x85 abutting across, 8 px
/// between rows, 8 px of edge round the cells and a 45 px title band on
/// top with the baseline 30 px down. A picture fits a 122x69 box 11 px in
/// and 8 px down, and its icon's bottom-right sits 3 px past the
/// picture's. Up to 7 to a row. No shadow, square corners.
pub const CLASSIC: Grid = Grid {
    cell: (144, 85),
    row_gap: 8,
    pad: Insets::new(8, 45, 8, 8),
    title_baseline: 30,
    title_px: 0,
    picture: Rect::new(11, 8, 122, 69),
    badge: (3, 3),
    max_cols: 7,
    shadow: Insets::ZERO,
    corner: &[],
};

/// Draw the panel into `canvas` (sized to `layout.rect`): a raised window edge, one 32 px icon per shown cell from `icons` (indexed by cell; `None` draws the fallback glyph), a 2 px `hilight` frame round `selected`'s cell and 1 px round `hot`'s, and `title`, bold, in the sunken field. A preview well is sunken and empty, or holds `well_icon` when the host has no live picture.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &SwitcherLayout,
    title: &str,
    icons: &[Option<Canvas>],
    selected: usize,
    hot: Option<usize>,
    well_icon: Option<&Canvas>,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let icon_px = ICON_PX * s;
    let all = Rect::new(0, 0, layout.rect.width, layout.rect.height);
    canvas.fill_rect(all, cs.button_face);
    canvas.edge_t(all, EdgeStyle::WindowRaised, cs, s);
    let mut fallback: Option<Canvas> = None;
    for (k, cell) in layout.cells.iter().enumerate() {
        let i = layout.first + k;
        let at = (cell.x + ICON_INSET * s, cell.y + ICON_INSET * s);
        match icons.get(k).and_then(|o| o.as_ref()) {
            Some(icon) => canvas.blit_canvas(at.0, at.1, icon),
            None => {
                let fb = fallback.get_or_insert_with(|| crate::icon::fallback(theme, icon_px));
                canvas.blit_canvas(at.0, at.1, fb)
            }
        }
        if i == selected {
            frame(canvas, theme, *cell, 2);
        } else if hot == Some(i) {
            frame(canvas, theme, *cell, 1);
        }
    }
    if let Some(well) = layout.preview {
        crate::controls::taskpreview::draw_well(canvas, theme, well, well_icon);
    }
    draw_field(canvas, theme, text, layout.field, title);
}

/// Draw a thumbnail layout's panel into `canvas` (sized to `layout.canvas()`): a raised window edge, `title` (the `hot` entry's, else `selected`'s) bold and centred in the band, a sunken well per shown cell, a 2 px `hilight` frame round the selected cell and 1 px round `hot`. `cells` is indexed by shown cell. A still picture is blitted at its rect; an unpictured cell gets its icon (or the fallback glyph). The host lays live pictures over the rest, then `draw_badges`.
#[expect(clippy::too_many_arguments)]
pub fn draw_thumbnails(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &SwitcherLayout,
    title: &str,
    cells: &[ThumbCell],
    selected: usize,
    hot: Option<usize>,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let all = Rect::new(0, 0, layout.rect.width, layout.rect.height);
    canvas.fill_rect(all, cs.button_face);
    canvas.edge_t(all, EdgeStyle::WindowRaised, cs, s);
    let grid = layout.grid.unwrap_or(CLASSIC);
    let size = if grid.title_px == 0 {
        theme.fonts.size_px
    } else {
        grid.title_px * s as u32
    };
    let band = layout.field;
    let title = text.ellipsize(title, band.width, true, size);
    let w = text.measure(&title, true, size);
    text.draw_run(
        canvas,
        band.x + (band.width - w) / 2,
        grid.title_baseline * s,
        band,
        &title,
        true,
        cs.button_text,
        size,
    );
    let mut fallback: Option<Canvas> = None;
    for (k, (cell, well)) in layout.cells.iter().zip(&layout.wells).enumerate() {
        let c = cells.get(k);
        let stand_in = match c.and_then(|c| c.picture) {
            Some(_) => None,
            None => Some(match c.and_then(|c| c.icon) {
                Some(icon) => icon,
                None => &*fallback.get_or_insert_with(|| crate::icon::fallback(theme, ICON_PX * s)),
            }),
        };
        crate::controls::taskpreview::draw_well(canvas, theme, *well, stand_in);
        if let Some(c) = c
            && let (Some(r), Some(still)) = (c.picture, c.still)
        {
            canvas.blit_canvas(r.x, r.y, still);
        }
        let i = layout.first + k;
        if i == selected {
            frame(canvas, theme, *cell, 2);
        } else if hot == Some(i) {
            frame(canvas, theme, *cell, 1);
        }
    }
}

/// Draw the icons of a thumbnail layout's pictured cells into `canvas`
/// (sized like the panel's and otherwise clear, since it goes over the
/// live pictures): each at `badge` from its picture, the fallback glyph
/// for a cell without an icon.
pub fn draw_badges(
    canvas: &mut Canvas,
    theme: &Theme,
    layout: &SwitcherLayout,
    cells: &[ThumbCell],
) {
    let s = theme.scale.max(1);
    let grid = layout.grid.unwrap_or(CLASSIC);
    let mut fallback: Option<Canvas> = None;
    for c in cells {
        let Some(r) = c.picture else { continue };
        let icon = match c.icon {
            Some(icon) => icon,
            None => &*fallback.get_or_insert_with(|| crate::icon::fallback(theme, ICON_PX * s)),
        };
        let (x, y) = badge(&grid, r, ICON_PX * s, s);
        canvas.blit_over(x, y, icon);
    }
}

fn frame(canvas: &mut Canvas, theme: &Theme, cell: Rect, t: i32) {
    let c = theme.colors.hilight;
    let w = t * theme.scale.max(1);
    canvas.fill_rect(Rect::new(cell.x, cell.y, cell.width, w), c);
    canvas.fill_rect(Rect::new(cell.x, cell.bottom() - w, cell.width, w), c);
    canvas.fill_rect(Rect::new(cell.x, cell.y, w, cell.height), c);
    canvas.fill_rect(Rect::new(cell.right() - w, cell.y, w, cell.height), c);
}

fn draw_field(canvas: &mut Canvas, theme: &Theme, text: &TextRenderer, f: Rect, title: &str) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    canvas.edge_t(f, EdgeStyle::ButtonSunken, cs, s);
    text.draw(
        canvas,
        Rect::new(f.x + 3 * s, f.y + 2 * s, f.width - 5 * s, f.height - 4 * s),
        title,
        true,
        cs.button_text,
        Align::Left,
        theme.fonts.size_px,
    );
}

/// The panel's `access` part, in panel coordinates: a list of every
/// entry by its title, the `selected` one selected. Entries on another
/// page have no bounds. Returns the selected entry's node, which has the
/// focus while the panel is up.
pub fn access<S: AsRef<str>>(
    titles: &[S],
    selected: usize,
    layout: &SwitcherLayout,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let all = Rect::new(0, 0, layout.rect.width, layout.rect.height);
    let list = tree.push(parent, "entries", access::node(Role::ListBox, all));
    let mut focus = list;
    for (i, title) in titles.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(title.as_ref());
        if let Some(cell) = i
            .checked_sub(layout.first)
            .and_then(|k| layout.cells.get(k))
        {
            o.set_bounds(access::bounds(*cell));
        }
        o.set_selected(i == selected);
        let id = tree.push(list, i, o);
        if i == selected {
            focus = id;
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    #[test]
    fn the_selected_entry_has_the_focus_and_other_pages_no_bounds() {
        let titles: Vec<String> = (0..23).map(|i| format!("Window {i}")).collect();
        let layout = SwitcherLayout::compute(23, 22, Rect::new(0, 0, 800, 600), false);
        let mut t = Tree::window("", (layout.rect.width, layout.rect.height));
        let focus = access(&titles, 22, &layout, &mut t, access::ROOT);
        let n = t.get(focus).unwrap();
        assert_eq!(n.label(), Some("Window 22"));
        assert_eq!(n.is_selected(), Some(true));
        assert_eq!(
            n.bounds(),
            Some(access::bounds(layout.cells[1])),
            "entry 22 is the second cell of the second page"
        );
        let first = t.find(Role::ListBoxOption, "Window 0").unwrap();
        assert_eq!(
            t.get(first).unwrap().bounds(),
            None,
            "page one is not shown"
        );
    }

    #[test]
    fn three_windows_lay_out_one_row() {
        let area = Rect::new(0, 0, 1920, 1050);
        let l = SwitcherLayout::compute(3, 1, area, false);
        assert_eq!(l.rect, Rect::new(795, 471, 330, 107));
        assert_eq!(l.cells.len(), 3);
        assert_eq!(l.cells[0], Rect::new(101, 19, 43, 43));
        assert_eq!(l.cells[1].x, 144);
        assert_eq!(l.field, Rect::new(14, 72, 302, 25));
        assert_eq!(l.first, 0);
    }

    #[test]
    fn nine_windows_wrap_after_seven_and_grow_a_row() {
        let l = SwitcherLayout::compute(9, 1, Rect::new(0, 0, 1920, 1050), false);
        assert_eq!(l.rect.height, 150);
        assert_eq!(l.cells[0], Rect::new(15, 19, 43, 43));
        assert_eq!(l.cells[6].x, 15 + 6 * 43);
        assert_eq!(l.cells[7], Rect::new(15, 62, 43, 43));
        assert_eq!(l.field.y, 115);
    }

    #[test]
    fn past_twenty_one_windows_the_selection_pages() {
        let l = SwitcherLayout::compute(25, 22, Rect::new(0, 0, 1920, 1050), false);
        assert_eq!(l.first, 21);
        assert_eq!(l.cells.len(), 4);
        assert_eq!(l.rect.height, 107);
        let full = SwitcherLayout::compute(25, 20, Rect::new(0, 0, 1920, 1050), false);
        assert_eq!(full.first, 0);
        assert_eq!(full.cells.len(), 21);
        assert_eq!(full.rect.height, 19 + 3 * 43 + 10 + 25 + 10);
    }

    #[test]
    fn draw_frames_selected_cell_and_names_it() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = SwitcherLayout::compute(2, 1, Rect::new(0, 0, 800, 600), false);
        let mut c = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        draw(
            &mut c,
            &theme,
            &text,
            &l,
            "Two",
            &[None, None],
            1,
            None,
            None,
        );
        let cs = &theme.colors;
        let sel = l.cells[1];
        assert_eq!(c.get(sel.x, sel.y), cs.hilight.argb());
        assert_eq!(c.get(sel.x + 1, sel.y + 20), cs.hilight.argb());
        assert_eq!(c.get(sel.right() - 1, sel.bottom() - 1), cs.hilight.argb());
        assert_eq!(c.get(sel.x + 2, sel.y + 2), cs.button_face.argb());
        let other = l.cells[0];
        assert_eq!(c.get(other.x, other.y), cs.button_face.argb());
        assert_eq!(c.get(other.x + 5, other.y + 5), cs.button_dk_shadow.argb());
        let f = l.field;
        assert_eq!(c.get(f.x, f.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(f.right() - 1, f.y + 10), cs.button_hilight.argb());
        let ink = (f.x..f.right())
            .flat_map(|x| (f.y..f.bottom()).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
            .count();
        assert!(ink > 10, "title ink {ink}");
    }

    #[test]
    fn draw_rings_the_hot_cell_but_not_the_selected_one() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let l = SwitcherLayout::compute(3, 1, Rect::new(0, 0, 800, 600), false);
        let mut c = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        draw(
            &mut c,
            &theme,
            &text,
            &l,
            "Three",
            &[None, None, None],
            1,
            Some(2),
            None,
        );
        let (sel, hot) = (l.cells[1], l.cells[2]);
        assert_eq!(c.get(hot.x, hot.y + 20), cs.hilight.argb(), "hot ring");
        assert_eq!(c.get(hot.x + 1, hot.y + 20), cs.button_face.argb(), "1 px");
        assert_eq!(
            c.get(sel.x + 1, sel.y + 20),
            cs.hilight.argb(),
            "selection 2 px"
        );
        let mut d = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        draw(
            &mut d,
            &theme,
            &text,
            &l,
            "Two",
            &[None, None, None],
            1,
            Some(1),
            None,
        );
        assert_eq!(d.get(sel.x + 1, sel.y + 20), cs.hilight.argb());
    }

    #[test]
    fn a_later_page_draws_one_icon_per_shown_cell() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = SwitcherLayout::compute(25, 22, Rect::new(0, 0, 800, 600), false);
        let red = crate::theme::Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let icons = vec![Some(icon); l.cells.len()];
        let mut c = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        draw(
            &mut c,
            &theme,
            &text,
            &l,
            "Window 23",
            &icons,
            22,
            None,
            None,
        );
        let first = l.cells[0];
        assert_eq!(c.get(first.x + 5, first.y + 5), red.argb());
        let sel = l.cells[1];
        assert_eq!(c.get(sel.x, sel.y), theme.colors.hilight.argb());
    }

    #[test]
    fn scaled_layout_doubles_everything() {
        let l = SwitcherLayout::compute(3, 1, Rect::new(0, 0, 800, 600), true);
        let s = l.scaled(2);
        assert_eq!(s.rect, l.rect.scaled(2));
        assert_eq!(s.cells[1], l.cells[1].scaled(2));
        assert_eq!(s.field, l.field.scaled(2));
        assert_eq!(s.preview, l.preview.map(|r| r.scaled(2)));
    }

    #[test]
    fn the_preview_well_sits_above_the_cells_and_widens_the_panel() {
        let l = SwitcherLayout::compute(3, 1, Rect::new(0, 0, 1920, 1050), true);
        assert_eq!(l.preview, Some(Rect::new(14, 14, 400, 250)));
        assert_eq!(l.cells[0], Rect::new(150, 269, 43, 43));
        assert_eq!(l.field, Rect::new(14, 322, 400, 25));
        assert_eq!(l.rect, Rect::new(746, 346, 428, 357));
    }

    #[test]
    fn classic_thumbnails_frame_the_selection_and_ring_the_hot_cell() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let l = SwitcherLayout::thumbnails(3, 1, Rect::new(0, 0, 1920, 1040), CLASSIC);
        let red = crate::theme::Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let pic = crate::taskpreview::fit(400, 300, l.wells[0], 1);
        let cells = [
            ThumbCell {
                icon: Some(&icon),
                picture: Some(pic),
                still: None,
            },
            ThumbCell {
                icon: Some(&icon),
                picture: None,
                still: None,
            },
            ThumbCell {
                icon: None,
                picture: None,
                still: None,
            },
        ];
        let mut c = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        draw_thumbnails(&mut c, &theme, &text, &l, "Two", &cells, 1, Some(2));
        let (c1, c2) = (l.cells[1], l.cells[2]);
        assert_eq!(c.get(c1.x, c1.y), cs.hilight.argb(), "selection frame");
        assert_eq!(c.get(c1.x + 1, c1.y + 1), cs.hilight.argb(), "2 px");
        assert_eq!(c.get(c2.x, c2.y + 10), cs.hilight.argb(), "hot ring");
        assert_ne!(c.get(c2.x + 1, c2.y + 10), cs.hilight.argb(), "1 px");
        let w1 = l.wells[1];
        assert_eq!(c.get(w1.x + w1.width / 2, w1.y + w1.height / 2), red.argb());
        let ink: Vec<i32> = (l.field.x..l.field.right())
            .filter(|&x| (2..l.field.bottom()).any(|y| c.get(x, y) == cs.button_text.argb()))
            .collect();
        let mid = (ink[0] + ink[ink.len() - 1]) / 2;
        assert!(
            (mid - l.rect.width / 2).abs() <= 2,
            "title centred, mid {mid}"
        );
        let mut b = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        draw_badges(&mut b, &theme, &l, &cells);
        let (bx, by) = badge(&CLASSIC, pic, 32, 1);
        assert_eq!(b.get(bx + 1, by + 1), red.argb());
        assert_eq!(b.get(w1.x + w1.width / 2, w1.y + w1.height / 2), 0);
    }

    #[test]
    fn draw_sinks_the_well_and_centres_the_stand_in_icon() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = SwitcherLayout::compute(2, 1, Rect::new(0, 0, 800, 600), true);
        let red = crate::theme::Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let mut c = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        draw(
            &mut c,
            &theme,
            &text,
            &l,
            "Two",
            &[None, None],
            1,
            None,
            Some(&icon),
        );
        let cs = &theme.colors;
        let w = l.preview.unwrap();
        assert_eq!(c.get(w.x, w.y + 50), cs.button_shadow.argb());
        assert_eq!(c.get(w.x + 1, w.y + 50), cs.button_dk_shadow.argb());
        assert_eq!(c.get(w.x + 20, w.y + 20), cs.button_shadow.argb());
        assert_eq!(c.get(w.x + 200, w.y + 125), red.argb());
        assert_eq!(c.get(w.x + 200 - 17, w.y + 125), cs.button_shadow.argb());
    }
}
