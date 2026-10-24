use super::details::{details_in, with_details};
use super::{Layout, Mode, ShellView, list_cell};
use crate::classic::widgets::{ScrollParts, scrollbar_parts, scrollbar_pos_at};
use crate::controls::listview;
use crate::controls::scrollbar;
use crate::controls::shellgrid;
use crate::pointer::WHEEL_ROWS;
use crate::raster::Rect;
use crate::selection::scroll_into_view;

/// Keep the focused cell inside `lay.body`, sliding by the minimum distance
/// (`listview::ListView::follow`'s rule) on each mode's axis. `step` calls this
/// on every move; `type_ahead` has no `Layout`, so a host calls `follow` after it.
pub fn follow(sv: &mut ShellView, lay: &Layout) {
    if let Some(i) = sv.selection.focus {
        ensure_visible(sv, i, lay);
    }
}

/// Scroll cell `i` into view: `sv.left` in List, `sv.top` in the icon modes
/// (px) and Details (rows). Does nothing when `i` is already inside `lay.body`.
pub fn ensure_visible(sv: &mut ShellView, i: usize, lay: &Layout) {
    let Some(&cell) = lay.cells.get(i) else {
        return;
    };
    match sv.mode {
        Mode::List => {
            let left = list_left(sv, lay);
            if cell.x < lay.body.x {
                sv.left = (left - (lay.body.x - cell.x)).max(0);
            } else if cell.right() > lay.body.right() {
                sv.left = left + cell.right() - lay.body.right();
            }
        }
        Mode::Details => {
            let row_h = lay.controls.text.len(listview::ROW_H);
            let rows = (lay.body.height - lay.controls.list_header).max(row_h) / row_h;
            scroll_into_view(&mut sv.top, i, rows as usize);
        }
        _ => {
            if cell.y < lay.body.y {
                sv.top = sv.top.saturating_sub((lay.body.y - cell.y) as usize);
            } else if cell.bottom() > lay.body.bottom() {
                sv.top += (cell.bottom() - lay.body.bottom()) as usize;
            }
        }
    }
}

/// Scroll cell `i` into view without picking it, for a reader's ScrollIntoView. True when it moved.
pub fn reveal(sv: &mut ShellView, i: usize, lay: &Layout) -> bool {
    let (top, left) = (sv.top, sv.left);
    ensure_visible(sv, i, lay);
    (sv.top, sv.left) != (top, left)
}

/// Scroll `by` steps: rows in Details, a cell's height per step in the icon
/// modes, and in List a column sideways per wheel notch (`WHEEL_ROWS` steps),
/// as the bar's arrows step. `true` when the offset moved.
pub fn scroll(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => {
            let notches = by.signum() * (by.abs() / WHEEL_ROWS).max(1);
            hscroll(sv, notches * list_cell(lay.controls.text).w, lay)
        }
        Mode::Details => with_details(sv, lay, |lv, lv_lay| lv.scroll(by as i64, lv_lay)),
        _ => {
            let pitch = shellgrid::Grid::of(sv.mode, lay.controls.text).cell.1;
            let max = (lay.content.1 - lay.body.height).max(0) as i64;
            let top = (sv.top as i64 + by as i64 * pitch as i64).clamp(0, max) as usize;
            let changed = top != sv.top;
            sv.top = top;
            changed
        }
    }
}

/// Scroll by a page of rows, as a click on the vertical track does; `true` when the offset moved.
pub fn page(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::Details => {
            let rows = details_in(sv, lay).1.rows.max(1) as i32;
            scroll(sv, by * rows, lay)
        }
        Mode::List => false,
        _ => {
            let pitch = shellgrid::Grid::of(sv.mode, lay.controls.text)
                .cell
                .1
                .max(1);
            scroll(sv, by * (lay.body.height / pitch).max(1), lay)
        }
    }
}

/// List's horizontal bar parts: `left` and the columns' width in px, as `draw` hands the bar.
pub(super) fn list_hparts(sv: &ShellView, lay: &Layout, hb: Rect) -> ScrollParts {
    scrollbar_parts(
        hb,
        1,
        list_left(sv, lay) as usize,
        lay.body.width.max(0) as usize,
        lay.content.0.max(0) as usize,
        true,
        lay.controls.thumb_min,
    )
}

/// How far List's columns are scrolled under `lay`: `sv.left` held to what
/// overflows the body. A window widening after a scroll leaves `sv.left` past
/// the end; this keeps the last column's end at the body's.
pub(super) fn list_left(sv: &ShellView, lay: &Layout) -> i32 {
    scrollbar::held(sv.left, lay.body.width, lay.content.0)
}

/// Scroll sideways by `by` px, clamped to what overflows: List's columns, or
/// Details' through its `listview`. The icon modes never scroll sideways.
pub fn hscroll(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => {
            let from = list_left(sv, lay);
            sv.left = scrollbar::held(from + by, lay.body.width, lay.content.0);
            sv.left != from
        }
        Mode::Details => with_details(sv, lay, |lv, lv_lay| lv.hscroll(by, lv_lay)),
        _ => false,
    }
}

/// How far one arrow press scrolls Details sideways, in every look.
const DETAILS_NUDGE: i32 = 16;

pub fn hstep(sv: &mut ShellView, dir: i32, lay: &Layout) -> bool {
    let step = match sv.mode {
        Mode::List => list_cell(lay.controls.text).w,
        _ => DETAILS_NUDGE,
    };
    hscroll(sv, dir.signum() * step, lay)
}

pub fn hpage(sv: &mut ShellView, by: i32, lay: &Layout) -> bool {
    hscroll(sv, by * lay.body.width, lay)
}

/// The horizontal thumb, grabbed `grab` px right of its left edge, dragged to pointer `x`.
pub fn drag_hthumb(sv: &mut ShellView, x: i32, grab: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => {
            let Some(hb) = lay.hscroll else {
                return false;
            };
            let p = list_hparts(sv, lay, hb);
            let visible = lay.body.width.max(0) as usize;
            let total = lay.content.0.max(0) as usize;
            let left = scrollbar_pos_at(&p, x - grab, visible, total, true) as i32;
            let changed = left != sv.left;
            sv.left = left;
            changed
        }
        Mode::Details => with_details(sv, lay, |lv, lv_lay| {
            let left = lv.left_at(lv_lay, x, grab);
            let changed = left != lv.left;
            lv.left = left;
            changed
        }),
        _ => false,
    }
}

/// The icon modes' vertical bar parts: `top` and content height are px, as `draw` hands the bar.
pub(super) fn icon_vparts(sv: &ShellView, lay: &Layout, vb: Rect) -> ScrollParts {
    scrollbar_parts(
        vb,
        1,
        sv.top,
        lay.body.height.max(0) as usize,
        lay.content.1.max(0) as usize,
        false,
        lay.controls.thumb_min,
    )
}

/// The vertical thumb, grabbed `grab` px below its top, dragged to pointer
/// `y`. In Details the thumb is `listview`'s; List never scrolls vertically.
pub fn drag_thumb(sv: &mut ShellView, y: i32, grab: i32, lay: &Layout) -> bool {
    match sv.mode {
        Mode::List => false,
        Mode::Details => with_details(sv, lay, |lv, lv_lay| {
            let top = lv.top_at(lv_lay, y, grab);
            let changed = top != lv.top;
            lv.top = top;
            changed
        }),
        _ => {
            let Some(vb) = lay.vscroll else {
                return false;
            };
            let p = icon_vparts(sv, lay, vb);
            let visible = lay.body.height.max(0) as usize;
            let top = scrollbar_pos_at(&p, y - grab, visible, lay.content.1.max(0) as usize, false);
            let changed = top != sv.top;
            sv.top = top;
            changed
        }
    }
}
