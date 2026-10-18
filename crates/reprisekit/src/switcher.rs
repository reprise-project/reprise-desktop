//! The Alt+Tab switcher as a model: what the panel lists and in what order, where it opens, how the
//! selection steps, and where its cells go. Entries are the windows and open dialogs in focus order
//! (most recent first), with the desktop entry last when the host lists one.
//!
//! Two layouts share [`SwitcherLayout`]. The icon layout ([`SwitcherLayout::compute`]) is a
//! fixed-width panel of 43 px icon cells, seven to a row, with the selected entry's title in a field
//! below and an optional preview well above. The thumbnail layout ([`SwitcherLayout::thumbnails`])
//! puts every entry in a cell of a look's [`Grid`], rows of cells under a title band, each holding a
//! picture box. [`hit`] finds the entry under the pointer in either. `controls::switcher` draws Classic's panels.

use crate::raster::{Canvas, Insets, Rect};

/// The icon panel: 330 px wide whatever the count, 43 px icon cells from 19 px below the top, at most
/// 7 per row and 3 rows (more windows page), a 25 px title field 14 px in from the sides, 10 px under
/// the icons and 10 px above the bottom.
pub const WIDTH: i32 = 330;
pub const CELL: i32 = 43;
pub const COLS: usize = 7;
pub const MAX_ROWS: usize = 3;
const TOP: i32 = 19;
const SIDE: i32 = 14;
const FIELD_H: i32 = 25;
const FIELD_GAP: i32 = 10;
const BOTTOM: i32 = 10;
pub const ICON_PX: i32 = 32;
/// The preview well: 400x250, 14 px in from the panel's top-left, widening the panel to 428; the icon cells start 5 px under it.
const PREVIEW_W: i32 = 400;
const PREVIEW_H: i32 = 250;
const PREVIEW_GAP: i32 = 5;
const THUMB_MAX_ROWS: usize = 3;

/// A look's thumbnail panel geometry, at scale 1: the cell (the selection box), the row gap (columns
/// abut), the panel's edge round the cells (its top holds the title band), the title's baseline and size
/// (0: the theme's font size), the picture box (cell-relative), how far the icon's bottom-right sits
/// past the picture's, the most columns, and the shadow and corners as in [`crate::taskpreview::Grid`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub cell: (i32, i32),
    pub row_gap: i32,
    pub pad: Insets,
    pub title_baseline: i32,
    pub title_px: u32,
    pub picture: Rect,
    pub badge: (i32, i32),
    pub max_cols: usize,
    pub shadow: Insets,
    pub corner: &'static [i32],
}

/// How large a thumbnail layout's pictures are (`[switcher] size`): Small as the look's grid draws
/// them, Medium 1.5 times, Large twice and Huge three times as wide and tall.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    #[default]
    Small,
    Medium,
    Large,
    Huge,
}

impl Grid {
    /// The grid with its pictures grown for `size` and its cells with them, margins kept.
    pub fn sized(self, size: Size) -> Grid {
        let (num, den) = match size {
            Size::Small => return self,
            Size::Medium => (3, 2),
            Size::Large => (2, 1),
            Size::Huge => (3, 1),
        };
        let p = self.picture;
        let grow = |v: i32| (v * num + den / 2) / den;
        let (w, h) = (grow(p.width), grow(p.height));
        Grid {
            cell: (self.cell.0 + w - p.width, self.cell.1 + h - p.height),
            picture: Rect::new(p.x, p.y, w, h),
            ..self
        }
    }
}

/// One shown entry of a thumbnail layout, for the painters.
///
/// `picture: Some(r)`: a picture fills `r`, panel-relative (painters shift it by
/// `-layout.canvas().x/y`) and fitted by the host. It is live unless `still` is set, in which case the
/// painter blits `still` at `r`. `picture: None`: the icon (or fallback glyph) stands in, centred in the cell's box.
#[derive(Clone, Copy)]
pub struct ThumbCell<'a> {
    pub icon: Option<&'a Canvas>,
    pub picture: Option<Rect>,
    pub still: Option<&'a Canvas>,
}

/// The switcher's entries, most recent first, from the windows (most recently focused first) and the
/// open dialogs (top first). Dialogs sit above every window, so they follow the entry with the
/// keyboard: first when a dialog has it (`dialog_first`), else after the focused window. `desktop` goes last.
pub fn order<T: Clone>(
    windows: &[T],
    dialogs: &[T],
    dialog_first: bool,
    desktop: Option<T>,
) -> Vec<T> {
    let split = usize::from(!dialog_first).min(windows.len());
    let (head, tail) = windows.split_at(split);
    let mut entries = [head, dialogs, tail].concat();
    entries.extend(desktop);
    entries
}

/// The entry the panel opens on, or None when there is nothing to switch to. `len` counts the desktop
/// entry when `desktop` is on, which alone is nothing to switch to. A window or dialog holding the
/// keyboard (`held`) is entry 0, so the panel starts on entry 1.
pub fn start(len: usize, held: bool, desktop: bool) -> Option<usize> {
    let real = len - usize::from(desktop && len > 0);
    if real == 0 {
        return None;
    }
    let first = usize::from(held);
    (len > first).then_some(first)
}

/// Advance `selected` by one, wrapping; `backwards` reverses. `len` zero returns 0.
pub fn step(selected: usize, len: usize, backwards: bool) -> usize {
    if len == 0 {
        return 0;
    }
    if backwards {
        (selected + len - 1) % len
    } else {
        (selected + 1) % len
    }
}

/// The panel's rect (in `area`'s coordinates), each shown entry's cell relative to the panel's
/// top-left, the title field (the title band of a thumbnail layout), the preview well if any, and the
/// first entry shown (paging). A thumbnail layout also has each shown cell's well round its picture
/// box, and the grid it was laid out on; an icon layout has neither. `scale` is device pixels per
/// logical pixel: 1 from a constructor, `n` from `scaled(n)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitcherLayout {
    pub rect: Rect,
    pub cells: Vec<Rect>,
    pub field: Rect,
    pub preview: Option<Rect>,
    pub first: usize,
    pub wells: Vec<Rect>,
    pub scale: i32,
    pub grid: Option<Grid>,
}

impl SwitcherLayout {
    /// Lay out `count` entries as icons with `selected` on the shown page: the row of cells is centred
    /// (left-aligned rows below), the panel grows 43 px per row and is centred in `area`. `preview` adds the well above.
    pub fn compute(count: usize, selected: usize, area: Rect, preview: bool) -> SwitcherLayout {
        let page = COLS * MAX_ROWS;
        let first = (selected / page) * page;
        let shown = count.saturating_sub(first).min(page);
        let cols = shown.min(COLS) as i32;
        let rows = shown.div_ceil(COLS).max(1) as i32;
        let (width, well, top) = if preview {
            let well = Rect::new(SIDE, SIDE, PREVIEW_W, PREVIEW_H);
            (
                PREVIEW_W + 2 * SIDE,
                Some(well),
                well.bottom() + PREVIEW_GAP,
            )
        } else {
            (WIDTH, None, TOP)
        };
        let x0 = (width - cols * CELL + 1) / 2;
        let cells = (0..shown)
            .map(|i| {
                let (c, r) = ((i % COLS) as i32, (i / COLS) as i32);
                Rect::new(x0 + c * CELL, top + r * CELL, CELL, CELL)
            })
            .collect();
        let field_y = top + rows * CELL + FIELD_GAP;
        let field = Rect::new(SIDE, field_y, width - 2 * SIDE, FIELD_H);
        let height = field.bottom() + BOTTOM;
        let x = area.x + (area.width - width) / 2;
        let y = area.y + (area.height - height) / 2;
        SwitcherLayout {
            rect: Rect::new(x, y, width, height),
            cells,
            field,
            preview: well,
            first,
            wells: Vec::new(),
            scale: 1,
            grid: None,
        }
    }

    /// Lay out `count` entries as thumbnails with `selected` on the shown page: `grid.cell` cells
    /// abutting across, `grid.row_gap` between rows, as many columns as fit `area` (1 to
    /// `grid.max_cols`) and rows (1 to 3), the panel as wide as the first row and centred, later rows
    /// left-aligned. `field` is the title band; `wells` are the picture boxes widened by the 2 px edge
    /// `taskpreview::fit` takes off.
    pub fn thumbnails(count: usize, selected: usize, area: Rect, grid: Grid) -> SwitcherLayout {
        let (cw, ch) = grid.cell;
        let p = grid.pad;
        let fit_cols =
            ((area.width - p.left - p.right) / cw).clamp(1, grid.max_cols as i32) as usize;
        let fit_rows = ((area.height - p.top - p.bottom + grid.row_gap) / (ch + grid.row_gap))
            .clamp(1, THUMB_MAX_ROWS as i32) as usize;
        let page = fit_cols * fit_rows;
        let first = (selected / page) * page;
        let shown = count.saturating_sub(first).min(page);
        let cols = shown.clamp(1, fit_cols);
        let rows = shown.div_ceil(cols).max(1) as i32;
        let width = p.left + cols as i32 * cw + p.right;
        let height = p.top + rows * ch + (rows - 1) * grid.row_gap + p.bottom;
        let cells: Vec<Rect> = (0..shown)
            .map(|i| {
                let (c, r) = ((i % cols) as i32, (i / cols) as i32);
                Rect::new(p.left + c * cw, p.top + r * (ch + grid.row_gap), cw, ch)
            })
            .collect();
        let wells = cells
            .iter()
            .map(|c| {
                let b = grid.picture;
                Rect::new(c.x + b.x - 2, c.y + b.y - 2, b.width + 4, b.height + 4)
            })
            .collect();
        let x = area.x + (area.width - width) / 2;
        let y = area.y + (area.height - height) / 2;
        SwitcherLayout {
            rect: Rect::new(x, y, width, height),
            cells,
            field: Rect::new(p.left, 0, width - p.left - p.right, p.top),
            preview: None,
            first,
            wells,
            scale: 1,
            grid: Some(grid),
        }
    }

    /// The same layout in device pixels for drawing at scale `n`; the grid stays at scale 1.
    pub fn scaled(&self, n: i32) -> SwitcherLayout {
        let n = n.max(1);
        SwitcherLayout {
            rect: self.rect.scaled(n),
            cells: self.cells.iter().map(|r| r.scaled(n)).collect(),
            field: self.field.scaled(n),
            preview: self.preview.map(|r| r.scaled(n)),
            first: self.first,
            wells: self.wells.iter().map(|r| r.scaled(n)).collect(),
            scale: self.scale * n,
            grid: self.grid,
        }
    }

    /// The panel plus its shadow, relative to the panel's top-left, in this layout's pixels. No grid, no shadow.
    pub fn canvas(&self) -> Rect {
        let s = self.scale.max(1);
        let m = self.grid.map_or(Insets::ZERO, |g| g.shadow);
        Rect::new(
            -m.left * s,
            -m.top * s,
            self.rect.width + (m.left + m.right) * s,
            self.rect.height + (m.top + m.bottom) * s,
        )
    }

    /// Where the panel's buffers take the pointer: the panel itself, relative to `canvas()`'s top-left, so the shadow takes none.
    pub fn input_area(&self) -> Rect {
        let c = self.canvas();
        Rect::new(-c.x, -c.y, self.rect.width, self.rect.height)
    }
}

/// The entry under `(x, y)`, relative to the panel's top-left in the layout's pixels: a shown cell's,
/// counted from `layout.first`. Gaps, the title band and the edge hit nothing.
pub fn hit(layout: &SwitcherLayout, x: i32, y: i32) -> Option<usize> {
    layout
        .cells
        .iter()
        .position(|c| c.contains(x, y))
        .map(|k| layout.first + k)
}

/// Where an icon `icon_px` square goes over `picture` (device pixels at `scale`): bottom-right `grid.badge` past the picture's.
pub fn badge(grid: &Grid, picture: Rect, icon_px: i32, scale: i32) -> (i32, i32) {
    let s = scale.max(1);
    (
        picture.right() + grid.badge.0 * s - icon_px,
        picture.bottom() + grid.badge.1 * s - icon_px,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::switcher::CLASSIC as G;

    #[test]
    fn order_puts_the_desktop_last() {
        let w = ["foot", "notepad"];
        let d = ["Run"];
        assert_eq!(
            order(&w, &d, false, Some("Desktop")),
            ["foot", "Run", "notepad", "Desktop"]
        );
        assert_eq!(order(&w, &d, true, None), ["Run", "foot", "notepad"]);
    }

    #[test]
    fn start_ignores_a_lone_desktop_entry() {
        assert_eq!(start(1, false, true), None);
        // One window and the desktop: the desktop holds the keyboard, so the window is entry 0 and the panel opens on it.
        assert_eq!(start(2, false, true), Some(0));
        assert_eq!(start(2, true, true), Some(1));
        assert_eq!(start(3, true, false), Some(1));
        assert_eq!(start(1, true, false), None);
    }

    #[test]
    fn a_desktop_and_one_window_still_switch() {
        assert!(start(2, true, true).is_some());
        assert!(start(1, false, true).is_none());
    }

    #[test]
    fn three_rows_of_seven_on_a_wide_screen() {
        let l = SwitcherLayout::thumbnails(16, 1, Rect::new(0, 0, 1920, 1040), G);
        assert_eq!(l.rect, Rect::new(448, 358, 1024, 324));
        assert_eq!(l.cells[0], Rect::new(8, 45, 144, 85));
        assert_eq!(l.cells[1].x, 152);
        assert_eq!(l.cells[7], Rect::new(8, 138, 144, 85));
        assert_eq!(l.cells[14].y, 231);
        assert_eq!(l.cells[15].x, 152);
        assert_eq!(l.wells[0], Rect::new(8 + 9, 45 + 6, 126, 73));
        assert_eq!(l.field, Rect::new(8, 0, 1008, 45), "the title band");
    }

    #[test]
    fn every_well_has_the_size_of_a_lone_entry_s_well() {
        // The compositor composes the desktop still for a one-entry layout ahead of the first open.
        let area = Rect::new(0, 0, 1920, 1040);
        let lone = SwitcherLayout::thumbnails(1, 0, area, G).wells[0];
        for n in [2, 7, 16, 30] {
            let l = SwitcherLayout::thumbnails(n, n - 1, area, G);
            assert!(
                l.wells
                    .iter()
                    .all(|w| (w.width, w.height) == (lone.width, lone.height)),
                "{n} entries: every well {}x{}",
                lone.width,
                lone.height
            );
        }
    }

    #[test]
    fn one_row_is_as_wide_as_its_cells() {
        let l = SwitcherLayout::thumbnails(6, 1, Rect::new(0, 0, 1920, 1040), G);
        assert_eq!((l.rect.width, l.rect.height), (16 + 6 * 144, 45 + 85 + 8));
    }

    #[test]
    fn thumbnails_fit_a_narrow_area_and_page() {
        let l = SwitcherLayout::thumbnails(20, 16, Rect::new(0, 0, 800, 600), G);
        assert_eq!(l.first, 15);
        assert_eq!(l.cells.len(), 5);
        assert_eq!(l.rect.width, 16 + 5 * 144);
        assert!(l.rect.x >= 0 && l.rect.right() <= 800);
    }

    #[test]
    fn sized_grids_grow_the_picture_and_keep_its_margins() {
        assert_eq!(G.sized(Size::Small), G);
        let m = G.sized(Size::Medium);
        assert_eq!(m.picture, Rect::new(11, 8, 183, 104));
        assert_eq!(m.cell, (205, 120));
        let l = G.sized(Size::Large);
        assert_eq!(l.picture, Rect::new(11, 8, 244, 138));
        assert_eq!(l.cell, (266, 154));
        assert_eq!(
            (l.row_gap, l.pad, l.max_cols),
            (G.row_gap, G.pad, G.max_cols)
        );
        let p = SwitcherLayout::thumbnails(21, 1, Rect::new(0, 0, 3440, 1400), l);
        assert_eq!(p.rect.width, 16 + 7 * 266);
        assert_eq!(p.rect.height, 45 + 3 * 154 + 2 * 8 + 8);
        let h = G.sized(Size::Huge);
        assert_eq!(h.picture, Rect::new(11, 8, 366, 207));
        assert_eq!(h.cell, (388, 223));
        let p = SwitcherLayout::thumbnails(21, 1, Rect::new(0, 0, 3440, 1400), h);
        assert_eq!((p.cells.len(), p.rect.width), (21, 16 + 7 * 388));
        let p = SwitcherLayout::thumbnails(21, 1, Rect::new(0, 0, 1920, 1040), h);
        assert_eq!((p.cells.len(), p.rect.width), (12, 16 + 4 * 388));
    }

    #[test]
    fn scaled_thumbnails_double() {
        let l = SwitcherLayout::thumbnails(3, 1, Rect::new(0, 0, 1920, 1040), G);
        let s = l.scaled(2);
        assert_eq!(s.rect, l.rect.scaled(2));
        assert_eq!(s.cells[2], l.cells[2].scaled(2));
        assert_eq!(s.wells[2], l.wells[2].scaled(2));
        assert_eq!(s.scale, 2);
        assert_eq!(
            s.canvas(),
            Rect::new(0, 0, s.rect.width, s.rect.height),
            "Classic has no shadow"
        );
    }

    #[test]
    fn hit_finds_cells_not_gaps_or_the_title() {
        let l = SwitcherLayout::thumbnails(9, 1, Rect::new(0, 0, 1920, 1040), G);
        assert_eq!(hit(&l, 10, 50), Some(0));
        assert_eq!(hit(&l, 152 + 3, 50), Some(1));
        assert_eq!(hit(&l, 10, 20), None, "title band");
        assert_eq!(hit(&l, 10, 45 + 85 + 2), None, "row gap");
        assert_eq!(hit(&l, 8 + 144 + 1, 138 + 10), Some(8));
        assert_eq!(
            hit(&l, 8 + 2 * 144 + 1, 138 + 10),
            None,
            "past the last cell"
        );
    }

    #[test]
    fn the_shadow_hits_nothing() {
        let l = SwitcherLayout::thumbnails(
            3,
            0,
            Rect::new(0, 0, 1920, 1040),
            crate::glass::switcher::GRID,
        );
        assert_eq!(hit(&l, -5, 50), None);
        assert_eq!(hit(&l, 10, 50), Some(0));
        assert_eq!((l.canvas().x, l.canvas().y), (-13, -12));
        assert_eq!(
            l.input_area(),
            Rect::new(13, 12, l.rect.width, l.rect.height)
        );
        let c = SwitcherLayout::compute(3, 0, Rect::new(0, 0, 1920, 1040), false);
        assert_eq!(c.input_area(), Rect::new(0, 0, c.rect.width, c.rect.height));
    }

    #[test]
    fn hit_counts_from_the_shown_page() {
        let l = SwitcherLayout::thumbnails(30, 25, Rect::new(0, 0, 1920, 1040), G);
        assert_eq!(l.first, 21);
        assert_eq!(hit(&l, 10, 50), Some(21));
    }

    #[test]
    fn badge_sits_on_the_fitted_picture() {
        let pic = Rect::new(19, 53, 122, 70);
        assert_eq!(badge(&G, pic, 32, 1), (19 + 122 + 3 - 32, 53 + 70 + 3 - 32));
        assert_eq!(
            badge(&G, pic.scaled(2), 64, 2),
            ((19 + 122 + 3) * 2 - 64, (53 + 70 + 3) * 2 - 64)
        );
    }

    #[test]
    fn order_puts_the_active_entry_first_then_dialogs_then_windows() {
        let w = ["foot", "notepad", "explorer"];
        let d = ["Reprise Settings", "Run"];
        assert_eq!(
            order(&w, &d, true, None),
            ["Reprise Settings", "Run", "foot", "notepad", "explorer"]
        );
        assert_eq!(
            order(&w, &d, false, None),
            ["foot", "Reprise Settings", "Run", "notepad", "explorer"]
        );
        assert_eq!(order(&[], &d, false, None), ["Reprise Settings", "Run"]);
    }

    #[test]
    fn start_skips_the_entry_with_the_keyboard_and_not_the_desktop() {
        assert_eq!(start(3, true, false), Some(1));
        assert_eq!(start(1, true, false), None);
        assert_eq!(start(3, false, false), Some(0));
        assert_eq!(start(1, false, false), Some(0));
        assert_eq!(start(0, false, false), None);
    }

    #[test]
    fn step_wraps_forward_and_back() {
        assert_eq!(step(1, 3, false), 2);
        assert_eq!(step(2, 3, false), 0);
        assert_eq!(step(0, 3, true), 2);
    }

    #[test]
    fn step_with_empty_list_stays_zero() {
        assert_eq!(step(0, 0, false), 0);
        assert_eq!(step(0, 0, true), 0);
    }
}
