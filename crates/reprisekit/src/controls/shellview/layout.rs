use super::details::{details_in, details_listview};
use super::{Group, ICON_PX, Layout, Mode, ShellView, list_cell};
use crate::classic::widgets::FIELD_PAD;
use crate::controls::iconview;
use crate::controls::listview;
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar;
use crate::controls::shellgrid;
use crate::raster::Rect;
use crate::text::TextRenderer;
use crate::theme::TextUnits;
use std::cell::RefCell;
use std::rc::Rc;

/// Rows per column and the columns that takes for `sv.entries`, given an
/// `area` as tall as the body. The column count can't reuse
/// `iconview::columns`, which answers "how many fit", not "how many are needed".
fn rows_and_cols(sv: &ShellView, area: Rect, u: TextUnits) -> (usize, usize) {
    let rows = iconview::rows_per_column(list_cell(u), area);
    let cols = sv.entries.len().div_ceil(rows);
    (rows, cols)
}

/// Entry `i`'s label rect, for the rename box and click tests against the
/// label rather than the cell. Each mode's is what `draw` fills with the
/// highlight: the name's ink padded as that mode pads it. Measured on demand:
/// measuring every label per layout made a pointer move O(folder).
pub fn label_rect(sv: &ShellView, lay: &Layout, i: usize, text: &TextRenderer, px: u32) -> Rect {
    if matches!(sv.mode, Mode::Details) {
        let (lv, lv_lay) = details_in(sv, lay);
        return lv.edit_rect(&lv_lay, i, text, px);
    }
    let (Some(&cell), Some(e)) = (lay.cells.get(i), sv.entries.get(i)) else {
        return Rect::new(0, 0, 0, 0);
    };
    match sv.mode {
        Mode::List => {
            let slot = lay.controls.text.len(ICON_PX);
            let w = (cell.width - slot).max(0);
            let shown = text.ellipsize(&e.name, w, false, px);
            Rect::new(
                cell.x + slot,
                cell.y,
                text.measure(&shown, false, px).min(w),
                cell.height,
            )
        }
        mode => shellgrid::label_of(mode, cell, e, text, px, lay.controls.text),
    }
}

/// `sv`'s layout in `r`. Measures nothing: every mode's cells are a fixed
/// pitch at the text size, and label rects come from `label_rect` on demand.
/// List: a sunken field (`FIELD_PAD` inset, matching the frame `draw` puts
/// round it) holding the fixed-pitch grid, with a horizontal scrollbar along
/// the bottom once the columns overflow, never a vertical one. The three icon
/// modes flow `grid` in the same field and take a vertical scrollbar once the
/// content overruns it, never a horizontal one. Details delegates to
/// `listview::layout` and reports its rows as `cells`; `body` spans the header
/// and rows together, so a cell's `y` sits the header height below `body.y`.
pub fn layout(sv: &ShellView, r: Rect, cm: &ControlMetrics) -> Layout {
    let frame = r;
    match sv.mode {
        Mode::List => {
            let sunken = frame.inset(FIELD_PAD);
            let cell = list_cell(cm.text);
            let (_, cols) = rows_and_cols(sv, sunken, cm.text);
            let overflows = cols as i32 * cell.w > sunken.width;
            let (body, hscroll) = if overflows {
                let h = (sunken.height - cm.scroll).max(0);
                let bar = Rect::new(sunken.x, sunken.y + h, sunken.width, cm.scroll);
                (Rect::new(sunken.x, sunken.y, sunken.width, h), Some(bar))
            } else {
                (sunken, None)
            };
            // The bar's height comes off the rows, which can push entries into another column: the range is the columns `body` holds.
            let content_w = rows_and_cols(sv, body, cm.text).1 as i32 * cell.w;
            let left = scrollbar::held(sv.left, body.width, content_w);
            let cells: Vec<Rect> = iconview::layout(cell, body, sv.entries.len())
                .into_iter()
                .map(|c| Rect::new(c.x - left, c.y, c.width, c.height))
                .collect();
            Layout {
                frame,
                body,
                cells,
                headers: Vec::new(),
                hscroll,
                vscroll: None,
                content: (content_w, body.height),
                controls: *cm,
            }
        }
        Mode::Details => {
            let lv = details_listview(sv);
            let lv_lay = listview::layout(&lv, frame, cm);
            // One rect per entry: a row scrolled out of view keeps its place above or below the body.
            let row_h = lv_lay.row_h();
            let cells: Vec<Rect> = (0..sv.entries.len())
                .map(|i| {
                    Rect::new(
                        lv_lay.body.x,
                        lv_lay.body.y + (i as i64 - sv.top as i64) as i32 * row_h,
                        lv_lay.body.width,
                        row_h,
                    )
                })
                .collect();
            let body = Rect::new(
                lv_lay.header.x,
                lv_lay.header.y,
                lv_lay.header.width,
                lv_lay.header.height + lv_lay.body.height,
            );
            Layout {
                frame,
                body,
                cells,
                headers: Vec::new(),
                hscroll: lv_lay.hscroll,
                vscroll: lv_lay.scrollbar,
                content: (lv_lay.total_w, sv.entries.len() as i32 * row_h),
                controls: *cm,
            }
        }
        Mode::Thumbnails | Mode::Tiles | Mode::Icons => {
            let sunken = frame.inset(FIELD_PAD);
            let grid = shellgrid::Grid::of(sv.mode, cm.text);
            let cell = grid.cell;
            // Reserving the bar narrows the body, which can need one more row: two
            // rounds reach the fixed point, as `listview::layout` does.
            let mut vbar = false;
            let mut body = sunken;
            let mut content_h = 0;
            for _ in 0..2 {
                body = Rect::new(
                    sunken.x,
                    sunken.y,
                    sunken.width - if vbar { cm.scroll } else { 0 },
                    sunken.height,
                );
                let h = shellgrid::grid_height(&grid, body, sv.entries.len(), &sv.groups);
                content_h = h;
                vbar |= h > body.height;
            }
            let (cells, headers) =
                shellgrid::grid(&grid, body, sv.entries.len(), &sv.groups, sv.top as i32);
            Layout {
                frame,
                body,
                cells,
                headers,
                hscroll: None,
                vscroll: vbar.then(|| Rect::new(body.right(), body.y, cm.scroll, body.height)),
                content: ((body.width / cell.0).max(1) * cell.0, content_h),
                controls: *cm,
            }
        }
    }
}

/// The last `layout` a host asked for, handed out again while nothing it
/// reads has changed, so layout on every pointer motion and paint builds the cells once per change.
#[derive(Debug, Default)]
pub struct LayoutCache(RefCell<Option<(LayoutKey, Rc<Layout>)>>);

/// Everything `layout` reads: rect, the look's sizes, mode, entry count, both scroll offsets, groups and Details columns.
#[derive(Debug)]
struct LayoutKey {
    r: Rect,
    cm: ControlMetrics,
    mode: Mode,
    len: usize,
    top: usize,
    left: i32,
    groups: Vec<Group>,
    columns: Vec<listview::Column>,
}

impl LayoutKey {
    fn of(sv: &ShellView, r: Rect, cm: &ControlMetrics) -> Self {
        LayoutKey {
            r,
            cm: *cm,
            mode: sv.mode,
            len: sv.entries.len(),
            top: sv.top,
            left: sv.left,
            groups: sv.groups.clone(),
            columns: sv.columns.clone(),
        }
    }

    fn matches(&self, sv: &ShellView, r: Rect, cm: &ControlMetrics) -> bool {
        self.r == r
            && self.cm == *cm
            && self.mode == sv.mode
            && self.len == sv.entries.len()
            && self.top == sv.top
            && self.left == sv.left
            && self.groups == sv.groups
            && self.columns == sv.columns
    }
}

impl LayoutCache {
    pub fn get(&self, sv: &ShellView, r: Rect, cm: &ControlMetrics) -> Rc<Layout> {
        let mut slot = self.0.borrow_mut();
        if let Some((key, lay)) = slot.as_ref()
            && key.matches(sv, r, cm)
        {
            return Rc::clone(lay);
        }
        let lay = Rc::new(layout(sv, r, cm));
        *slot = Some((LayoutKey::of(sv, r, cm), Rc::clone(&lay)));
        lay
    }
}
