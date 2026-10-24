use super::details::details_listview;
use super::layout::label_rect;
use super::rename::draw_rename;
use super::scroll::list_left;
use super::{ICON_PX, Layout, Mode, ShellView, list_icon};
use crate::classic::widgets::{Axis, ScrollHit, axis_lit};
use crate::controls::listview;
use crate::controls::shellgrid;
use crate::look::Paint;
use crate::look::widgets::item::{self, Mark};
use crate::look::widgets::{draw_field_frame, draw_hscrollbar, draw_scrollbar};
use crate::pointer::Lit;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};

/// One List row as `draw_list_row` paints it.
pub(super) struct ListRow<'a> {
    pub(super) name: &'a str,
    pub(super) icon: Option<&'a Canvas>,
    /// The icon column's width, [`ICON_PX`] grown with the text.
    pub(super) slot: i32,
    pub(super) mark: Mark,
    pub(super) selected: bool,
    /// False while the name is being renamed: the host draws the rename box over the label.
    pub(super) label: bool,
}

/// The label's part of an already-scaled List cell: all of it right of an icon column `slot`
/// px wide.
fn label_area(cell_s: Rect, s: i32, slot: i32) -> Rect {
    Rect::new(
        cell_s.x + slot * s,
        cell_s.y,
        (cell_s.width - slot * s).max(0),
        cell_s.height,
    )
}

/// `row` in the already-scaled List cell `cell_s`, clipped to `clip_s`.
pub(super) fn draw_list_row(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    row: &ListRow,
    cell_s: Rect,
    clip_s: Rect,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let slot = row.slot;
    let label_full = label_area(cell_s, s, slot);
    let shown = text.ellipsize(row.name, label_full.width, false, px);
    // A look that boxes its items boxes the icon and name, 3 px past its ink, under both; the text stays.
    let ink_w = text.measure(&shown, false, px);
    let item_box =
        Rect::new(cell_s.x, cell_s.y, slot * s + ink_w + 3 * s, cell_s.height).intersect(&clip_s);
    let boxed = item::draw_item(c, p, item_box, row.mark);
    if let Some(icon) = row.icon {
        let (mut dx, mut dy) = (0, (cell_s.height / s - slot) / 2 * s);
        if slot > ICON_PX {
            dx += (slot * s - icon.width as i32).max(0) / 2;
            dy += (slot * s - icon.height as i32).max(0) / 2;
        }
        c.blit_over_clipped(cell_s.x + dx, cell_s.y + dy, icon, clip_s);
    }
    let label = label_full.intersect(&clip_s);
    if label.is_empty() || !row.label {
        return;
    }
    let fg = if row.selected && !boxed {
        c.fill_rect(label, cs.hilight);
        cs.hilight_text
    } else {
        cs.window_text
    };
    text.draw_clipped(c, label_full, label, &shown, false, fg, Align::Left, px);
}

pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sv: &ShellView,
    lay: &Layout,
    bars: Lit<(Axis, ScrollHit)>,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    if matches!(sv.mode, Mode::Details) {
        let lv = details_listview(sv);
        let lv_lay = listview::layout(&lv, lay.frame, &lay.controls);
        listview::draw(c, text, p, &lv, &lv_lay, bars, px);
        return;
    }
    // A list: the style's list line (as Details' list view draws), not its edit line.
    let frame = lay.frame.scaled(s);
    c.fill_rect(frame, cs.window);
    draw_field_frame(c, p, frame, cs.window);
    if !matches!(sv.mode, Mode::List) {
        shellgrid::draw_grid(c, text, p, sv, lay, px);
        if let Some(vb) = lay.vscroll {
            draw_scrollbar(
                c,
                p,
                vb.scaled(s),
                sv.top,
                lay.body.height.max(0) as usize,
                lay.content.1.max(0) as usize,
                axis_lit(bars, Axis::Vertical),
            );
        }
        return;
    }
    let slot = lay.controls.text.len(ICON_PX);
    for (i, (cell, entry)) in lay.cells.iter().zip(&sv.entries).enumerate() {
        let visible = cell.intersect(&lay.body);
        if visible.is_empty() {
            continue;
        }
        let editing = matches!(&sv.edit, Some((j, _)) if *j == i);
        let icon = shellgrid::faded(entry, list_icon(entry, slot).map(|c| &**c));
        let row = ListRow {
            name: &entry.name,
            icon: icon.as_deref(),
            slot,
            mark: if editing { Mark::default() } else { sv.mark(i) },
            selected: sv.selection.is(i) || sv.drop_target == Some(i),
            label: !editing,
        };
        let (cell_s, visible_s) = (cell.scaled(s), visible.scaled(s));
        draw_list_row(c, text, p, &row, cell_s, visible_s, px);
        if editing && !label_area(cell_s, s, slot).intersect(&visible_s).is_empty() {
            let label = label_rect(sv, lay, i, text, px);
            draw_rename(c, text, p, sv, i, label.scaled(s), px);
        }
    }
    if let Some(hb) = lay.hscroll {
        draw_hscrollbar(
            c,
            p,
            hb.scaled(s),
            list_left(sv, lay) as usize,
            lay.body.width.max(0) as usize,
            lay.content.0.max(0) as usize,
            axis_lit(bars, Axis::Horizontal),
        );
    }
    if let Some(m) = sv.marquee {
        let m = m.intersect(&lay.body).scaled(s);
        if !m.is_empty() {
            crate::look::widgets::draw_marquee(c, p, m, cs.window_text);
        }
    }
}
