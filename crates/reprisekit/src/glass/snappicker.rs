//! The snap layouts panel in Reprise Glass: a glass sheet with round tiles.

use super::Settings;
use super::panel::{self, Corners, Panel};
use crate::raster::{Canvas, Insets, Rect, rounded_border, rounded_fill};
use crate::snaplayouts::{PANEL_CORNER, PickerLayout, TILE_RADIUS, Zone};
use crate::style::{Part, PartImage, State, Style};
use crate::theme::Color;

/// The sheet: the focused window's plain glass with the frame's outline and light line, every
/// corner cut by `PANEL_CORNER`, no sheen and no shadow.
pub const PANEL: Panel = Panel {
    corners: Corners::Cut {
        rows: PANEL_CORNER,
        bottom: true,
    },
    lines: (1.0, 1.0),
    sheen_rows: 0,
    ends: None,
    deepen: 0.0,
    density: 0.0,
    shadow: None,
};

const FIXED: i32 = 16;

/// [`PANEL`] as a nine-grid part, painted at `d`.
pub fn frame_part(s: &Settings, d: i32) -> PartImage {
    let n = 2 * FIXED + 1;
    panel::part(&PANEL, s, n, n, Insets::new(FIXED, FIXED, FIXED, FIXED), d)
}

const TILE: (Color, u32) = (Color::rgb(0xFF, 0xFF, 0xFF), 48);
const RIM: (Color, u32) = (Color::rgb(0xFF, 0xFF, 0xFF), 140);
const FILLED: (Color, u32) = (Color::rgb(0x00, 0x00, 0x00), 110);

/// Draw the panel into `canvas` (sized to `layout`, device pixels) in `style`, the lit tile in
/// `hilight`.
pub fn draw(
    canvas: &mut Canvas,
    style: &Style,
    hilight: Color,
    layout: &PickerLayout,
    icons: &[(Zone, &Canvas)],
    hot: Option<Zone>,
) {
    let s = layout.scale.max(1);
    let whole = Rect::new(0, 0, layout.width, layout.height);
    style.draw(canvas, whole, Part::SnapFrame, State::Normal, s);
    // A small tile, as in Settings' preview, rounds by at most a quarter of its shorter side.
    let radius = |r: Rect| (TILE_RADIUS * s).min(r.width.min(r.height) / 4);
    let faded = |canvas: &mut Canvas, r: Rect, (color, alpha): (Color, u32), rim: bool| {
        let mut t = Canvas::new(r.width.max(1) as u32, r.height.max(1) as u32);
        let local = Rect::new(0, 0, r.width, r.height);
        if rim {
            rounded_border(&mut t, local, radius(r), color, s);
        } else {
            rounded_fill(&mut t, local, radius(r), color);
        }
        canvas.blit_over_faded(r.x, r.y, &t, alpha, 255);
    };
    for (l, tiles) in layout.tiles.iter().enumerate() {
        let lit_cell = layout.cell_lit(hot, l);
        for (z, r) in tiles.iter().enumerate() {
            let filled = layout.filled[l][z];
            if hot == Some((l, z)) {
                rounded_fill(canvas, *r, radius(*r), hilight);
                continue;
            }
            faded(canvas, *r, if filled { FILLED } else { TILE }, false);
            if filled && lit_cell {
                rounded_border(canvas, *r, radius(*r), hilight, s);
            } else {
                faded(canvas, *r, RIM, true);
            }
            if let Some((_, icon)) = icons.iter().find(|(k, _)| *k == (l, z)) {
                let (w, h) = (icon.width as i32, icon.height as i32);
                canvas.blit_over(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, icon);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snaplayouts::{PickerLayout, Shape, sample_cells};
    use std::sync::Arc;

    fn style() -> Arc<Style> {
        super::super::style(&Settings::default())
    }

    /// Settings' strip of the six layouts: each tile's sides and top run straight from a quarter
    /// of the way along to the middle, so no tile, however thin, draws as an oval.
    #[test]
    fn a_previews_small_tiles_keep_straight_sides_at_both_scales() {
        use crate::snaplayouts::{PICKER_SPECS, ZoneLayout};
        let layouts: Vec<_> = (PICKER_SPECS.iter())
            .map(|s| ZoneLayout::parse(s).unwrap())
            .collect();
        for sc in [1, 2] {
            let l = PickerLayout::preview(&layouts).scaled(sc);
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw(
                &mut c,
                &style(),
                Color::rgb(0x33, 0x99, 0xFF),
                &l,
                &[],
                None,
            );
            for t in l.tiles.iter().flatten() {
                let side = |y: i32| c.get(t.x, t.y + y);
                let top = |x: i32| c.get(t.x + x, t.y);
                assert_eq!(
                    side(t.height / 4),
                    side(t.height / 2),
                    "x{sc} {t:?} left side"
                );
                assert_eq!(top(t.width / 4), top(t.width / 2), "x{sc} {t:?} top");
            }
        }
    }

    #[test]
    fn glass_draws_a_sheet_translucent_tiles_and_a_lit_one() {
        let l = PickerLayout::compute(Shape::Grid, (1920, 1050), &sample_cells());
        let hilight = Color::rgb(0x33, 0x99, 0xFF);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw(&mut c, &style(), hilight, &l, &[], Some((0, 1)));
        assert_eq!(c.get(0, 0) >> 24, 0, "the cut corner is clear");
        let sheet = c.get(l.width / 2, 6);
        assert_ne!(sheet >> 24, 0, "the sheet has its tint");
        let lit = l.tiles[0][1];
        assert_eq!(c.get(lit.x + 6, lit.y + 6), hilight.argb());
        let free = l.tiles[1][0];
        let light = |p: u32| (0..4).map(|i| (p >> (8 * i)) & 0xFF).sum::<u32>();
        assert!(
            light(c.get(free.x + 6, free.y + 6)) > light(sheet),
            "a tile lighter than the sheet"
        );
        let filled = l.tiles[0][0];
        assert_eq!(
            c.get(filled.x + filled.width / 2, filled.y),
            hilight.argb(),
            "lit beside the hot tile"
        );
        assert_ne!(
            c.get(filled.x + 6, filled.y + 6),
            c.get(free.x + 6, free.y + 6),
            "darker than a free tile"
        );
    }
}
