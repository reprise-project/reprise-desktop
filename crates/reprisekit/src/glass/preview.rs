use super::Settings;
use super::panel::{self, Corners, Fade, LightBox, Panel, Shadow};
use crate::raster::{Canvas, Insets, Rect};
use crate::style::{Layout, Part, PartImage, State, Style};
use crate::taskpreview::{Cell, Grid, TaskPreviewLayout};
use crate::text::TextRenderer;
use crate::theme::Color;

pub const GRID: Grid = Grid {
    cell: (215, 146),
    pad: 10,
    gap: 2,
    well: Rect::new(6, 27, 203, 112),
    close: Rect::new(193, 9, 14, 14),
    shadow: Insets::new(12, 11, 11, 8),
    corner: &[4, 2, 1, 1, 0, 0],
};

pub const PANEL: Panel = Panel {
    corners: Corners::Round(7.0),
    lines: (0.75, 0.5),
    sheen_rows: 0,
    ends: None,
    deepen: 66.0,
    density: 0.0,
    shadow: Some(Shadow {
        margin: GRID.shadow,
        alpha: 255.0,
        sigma: 5.0,
        offset: (1.0, 1.5),
        grow: 1.5,
        radius: 0.0,
    }),
};

/// How many px of the popup at each side its part keeps fixed: past the
/// corners and the shadow's rounding of them.
const EDGE: i32 = 12;

pub const THUMB_HOT: LightBox = LightBox {
    outline: (Color::rgb(232, 247, 255), 101),
    line: None,
    fill: (
        (Color::rgb(232, 247, 255), 48),
        (Color::rgb(232, 247, 255), 48),
    ),
    radius: 3.0,
    fade: Fade::Box,
};

/// Drawn a row of cells tall and nine-gridded with `EDGE` px fixed at each
/// side, so the glass keeps its fade on a panel of another height.
pub fn frame_part(s: &Settings, d: i32) -> PartImage {
    let h = 2 * GRID.pad + GRID.cell.1;
    panel::part(
        &PANEL,
        s,
        2 * EDGE + 1,
        h,
        Insets::new(EDGE, EDGE, EDGE, EDGE),
        d,
    )
}

pub fn thumb_part(d: i32) -> PartImage {
    PartImage::new(
        panel::light_box(&THUMB_HOT, 9, 9, d),
        &[State::Hot],
        Layout::Vertical,
    )
    .sizing(Insets::new(4, 4, 4, 4))
    .at_scale(d)
}

const ICON_AT: (i32, i32) = (8, 8);
const TEXT_X: i32 = 29;
const BASELINE: i32 = 20;
const TEXT_END: i32 = 189;
const INK: Color = Color::rgb(0xFF, 0xFF, 0xFF);

pub fn draw(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    size_px: u32,
    layout: &TaskPreviewLayout,
    cells: &[Cell],
    hot: Option<usize>,
) {
    let s = layout.scale.max(1);
    let area = layout.canvas();
    let at = |r: Rect| Rect::new(r.x - area.x, r.y - area.y, r.width, r.height);
    let whole = Rect::new(0, 0, area.width, area.height);
    style.draw(canvas, whole, Part::PreviewFrame, State::Normal, s);
    for (i, cell) in cells.iter().enumerate() {
        let (Some(r), Some(well)) = (layout.cells.get(i).copied(), layout.well(i)) else {
            break;
        };
        let r = at(r);
        if hot == Some(i) {
            style.draw(canvas, r, Part::PreviewThumb, State::Hot, s);
        }
        if let Some(icon) = cell.icon {
            canvas.blit_over(r.x + ICON_AT.0 * s, r.y + ICON_AT.1 * s, icon);
        }
        let clip = Rect::new(
            r.x + TEXT_X * s,
            r.y,
            (TEXT_END - TEXT_X) * s,
            layout.grid.cell.1 * s,
        );
        let title = text.ellipsize(cell.title, clip.width, false, size_px);
        let baseline = r.y + BASELINE * s;
        text.draw_run_over(canvas, clip.x, baseline, clip, &title, false, INK, size_px);
        if let Some(p) = cell.picture {
            let b = at(well).inset(2 * s);
            let (w, h) = (p.width as i32, p.height as i32);
            canvas.blit_over(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, p);
        }
        if hot == Some(i)
            && let Some(x) = layout.close(i)
        {
            style.draw(canvas, at(x), Part::PreviewClose, State::Normal, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{self, Settings};
    use crate::taskbar::Edge;
    use crate::taskpreview::close_hit;
    use crate::theme::FontConfig;

    const BOUNDS: Rect = Rect::new(0, 0, 1920, 1040);
    const BUTTON: Rect = Rect::new(300, 1040, 60, 40);

    #[test]
    fn one_window_is_a_235x166_popup_8_px_above_the_bar() {
        let l = TaskPreviewLayout::compute(GRID, 1, BUTTON, BOUNDS, Edge::Bottom);
        assert_eq!(
            l.rect,
            Rect::new(213, 866, 235, 166),
            "centred on the button, bottom 8 px above the bar"
        );
        assert_eq!(l.cells, [Rect::new(10, 10, 215, 146)]);
        assert_eq!(l.well(0), Some(Rect::new(16, 37, 203, 112)));
        assert_eq!(
            l.canvas(),
            Rect::new(-12, -11, 258, 185),
            "the canvas reaches over the shadow"
        );
    }

    #[test]
    fn three_windows_sit_side_by_side_2_px_apart() {
        let l = TaskPreviewLayout::compute(GRID, 3, BUTTON, BOUNDS, Edge::Bottom);
        assert_eq!(l.rect.width, 20 + 3 * 215 + 2 * 2, "669 px wide");
        assert_eq!(l.rect.height, 166);
        assert_eq!(l.cells[1], Rect::new(10 + 217, 10, 215, 146));
        assert_eq!(l.cells[2], Rect::new(10 + 2 * 217, 10, 215, 146));
        assert_eq!(l.close(2), Some(Rect::new(10 + 2 * 217 + 193, 19, 14, 14)));
        let e =
            TaskPreviewLayout::compute(GRID, 3, Rect::new(0, 1040, 60, 40), BOUNDS, Edge::Bottom);
        assert_eq!(e.rect.x, 12, "pushed in by the shadow's width");
    }

    #[test]
    fn close_hit_takes_the_close_button_not_the_picture() {
        let l = TaskPreviewLayout::compute(GRID, 3, BUTTON, BOUNDS, Edge::Bottom);
        let c = l.cells[1];
        assert_eq!(
            close_hit(&l, Some(1), c.x + 193, c.y + 9),
            Some(1),
            "its top-left"
        );
        assert_eq!(
            close_hit(&l, Some(1), c.x + 206, c.y + 22),
            Some(1),
            "its bottom-right"
        );
        assert_eq!(
            close_hit(&l, Some(1), c.x + 207, c.y + 9),
            None,
            "right of it"
        );
        assert_eq!(
            close_hit(&l, Some(1), c.x + 100, c.y + 80),
            None,
            "the picture"
        );
    }

    #[test]
    fn the_shadow_takes_no_pointer_but_the_strip_down_to_the_bar_does() {
        let l = TaskPreviewLayout::compute(GRID, 1, BUTTON, BOUNDS, Edge::Bottom);
        assert_eq!(l.input_area(), Rect::new(12, 11, 235, 174));
        assert_eq!(
            l.scaled(2).input_area(),
            Rect::new(24, 22, 470, 348),
            "at scale 2, in device pixels"
        );
    }

    #[test]
    fn under_a_top_bar_the_popup_hangs_8_px_below_the_bounds_top_and_takes_the_strip_up_to_it() {
        let bounds = Rect::new(0, 40, 1920, 1040);
        let button = Rect::new(300, 0, 60, 40);
        let l = TaskPreviewLayout::compute(GRID, 1, button, bounds, Edge::Top);
        assert_eq!(
            l.rect,
            Rect::new(213, 48, 235, 166),
            "8 px under the bar's foot, as far as it sits over a Bottom bar"
        );
        assert_eq!(
            l.input_area(),
            Rect::new(12, 3, 235, 174),
            "the popup and the 8 rows up to the bar"
        );
        assert_eq!(
            l.scaled(2).input_area(),
            Rect::new(24, 6, 470, 348),
            "at scale 2, in device pixels"
        );
        let bottom = TaskPreviewLayout::compute(GRID, 1, BUTTON, BOUNDS, Edge::Bottom);
        assert_eq!(
            bottom.rect,
            Rect::new(213, 866, 235, 166),
            "over a Bottom bar it stays put"
        );
    }

    fn over_grey(n: usize, hot: Option<usize>) -> (TaskPreviewLayout, Canvas) {
        let style = glass::style(&Settings::default());
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskPreviewLayout::compute(GRID, n, BUTTON, BOUNDS, Edge::Bottom);
        let a = l.canvas();
        let mut c = Canvas::new(a.width as u32, a.height as u32);
        c.fill_rect(c.bounds(), Color::rgb(128, 128, 128));
        let cells: Vec<Cell> = (0..n)
            .map(|_| Cell {
                title: "",
                icon: None,
                active: false,
                picture: None,
            })
            .collect();
        draw(&mut c, &style, &text, 12, &l, &cells, hot);
        (l, c)
    }

    fn rgb(px: u32) -> [i32; 3] {
        [16, 8, 0].map(|s| ((px >> s) & 0xFF) as i32)
    }

    #[test]
    fn the_hot_cell_lights_up_and_shows_a_red_close_button() {
        let (l, lit) = over_grey(3, Some(1));
        let (_, cold) = over_grey(3, None);
        let a = l.canvas();
        let c = l.cells[1];
        let px = |cv: &Canvas, x: i32, y: i32| rgb(cv.get(c.x + x - a.x, c.y + y - a.y));
        let (on, off) = (px(&lit, 100, 140), px(&cold, 100, 140));
        assert!(
            (0..3).all(|i| on[i] > off[i] + 15),
            "the lit cell's glass is lighter: {on:?} against {off:?}"
        );
        let red = px(&lit, 195, 16);
        assert!(
            red[0] > 150 && red[1] < 100 && red[2] < 90,
            "the close button is red: {red:?}"
        );
        assert_eq!(
            px(&cold, 195, 16),
            px(&cold, 208, 16),
            "a cold cell has no close button"
        );
        let other = l.cells[0];
        assert_eq!(
            lit.get(other.x + 100 - a.x, other.y + 140 - a.y),
            cold.get(other.x + 100 - a.x, other.y + 140 - a.y),
            "the other cells stay as they are"
        );
    }

    #[test]
    fn with_transparency_off_the_popup_is_opaque_and_its_shadow_is_not() {
        let style = glass::style(&Settings {
            transparency: false,
            ..Settings::default()
        });
        let p = style.part(Part::PreviewFrame).expect("the popup frame");
        let (ox, oy) = (12, 11);
        assert_eq!(p.atlas.get(ox + 10, oy + 50) >> 24, 0xFF, "the glass");
        assert_eq!(p.atlas.get(ox, oy + 50) >> 24, 0xFF, "the outline");
        assert!(p.atlas.get(4, oy + 50) >> 24 < 0xFF, "the shadow");
    }

    #[test]
    fn a_hot_popup_at_scale_2_is_its_scale_1_design_twice_over() {
        let style = glass::style(&Settings::default());
        let style2 = style.at_scale(2);
        let text = TextRenderer::load(&FontConfig::default());
        let cells: Vec<Cell> = (0..2)
            .map(|_| Cell {
                title: "",
                icon: None,
                active: false,
                picture: None,
            })
            .collect();
        let draw_at = |st: &Style, l: &TaskPreviewLayout| {
            let a = l.canvas();
            let mut c = Canvas::new(a.width as u32, a.height as u32);
            c.fill_rect(c.bounds(), Color::rgb(128, 128, 128));
            draw(&mut c, st, &text, 12, l, &cells, Some(1));
            c
        };
        let l = TaskPreviewLayout::compute(GRID, 2, BUTTON, BOUNDS, Edge::Bottom);
        let l2 = l.scaled(2);
        let a = l2.canvas();
        let at = |r: Rect| Rect::new(r.x - a.x, r.y - a.y, r.width, r.height);
        let popup = at(Rect::new(0, 0, l2.rect.width, l2.rect.height));
        let lit = at(l2.cells[1]);
        let close = at(l2.close(1).expect("the lit cell's close button"));
        panel::assert_twice_over(
            "the popup",
            &draw_at(&style, &l),
            &draw_at(&style2, &l2),
            |x, y| {
                panel::in_corner(popup, 7.0, x, y)
                    || panel::beside(popup, x, y)
                    || panel::in_corner(lit, 3.0, x, y)
                    || close.contains(x, y)
            },
        );
    }
}
