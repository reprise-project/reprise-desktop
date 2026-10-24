use super::details::details_in;
use super::draw::{ListRow, draw_list_row};
use super::{Entry, ICON_PX, Layout, Mode, ShellView};
use crate::controls::iconview;
use crate::controls::listview;
use crate::controls::shellgrid;
use crate::look::widgets::item::Mark;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;

/// One dragged entry: its index and the icon its mode draws (a thumbnail's picture) at the image's
/// scale.
pub struct DragRow<'a> {
    pub index: usize,
    pub icon: Option<&'a Canvas>,
}

/// The image a List, Details, Tiles or Thumbnails drag of `rows` carries, faded 1:2. A List or
/// Details row is painted as an unselected List row where it sits in the view, keeping only the
/// rows the body shows; Details rows take the Name column's width, from its icon. A tile or
/// thumbnail is painted unselected in its cell, cut as [`grid_drag_cut`] cuts. Returns the
/// image at `theme.scale` and the point of it under `pointer`, in view coordinates; None in Icons,
/// which draws through [`iconview::drag_image`].
pub fn drag_image(
    theme: &crate::theme::Theme,
    text: &TextRenderer,
    sv: &ShellView,
    lay: &Layout,
    rows: &[DragRow],
    pointer: (i32, i32),
) -> Option<(Canvas, (i32, i32))> {
    let (body, row_of): (Rect, Box<dyn Fn(Rect) -> Rect>) = match sv.mode {
        Mode::List => (lay.body, Box::new(|cell| cell)),
        Mode::Details => {
            let (lv, lv_lay) = details_in(sv, lay);
            let x = lv_lay.body.x - lv.left_in(&lv_lay) + listview::ICON_X;
            let w = sv
                .columns
                .first()
                .map_or(0, |col| lv_lay.col_w(col) - listview::ICON_X);
            let body = lv_lay.body;
            // A Name column scrolled out of view is drawn where it sits scrolled home.
            let x = if x + w <= body.x {
                body.x + listview::ICON_X
            } else {
                x
            };
            (
                body,
                Box::new(move |cell: Rect| Rect::new(x, cell.y, w, cell.height)),
            )
        }
        Mode::Thumbnails | Mode::Tiles => {
            return grid_drag_image(theme, text, sv, lay, rows, pointer);
        }
        Mode::Icons => return None,
    };
    let placed: Vec<(&DragRow, &Entry, Rect)> = rows
        .iter()
        .filter_map(|r| {
            let cell = row_of(*lay.cells.get(r.index)?);
            let e = sv.entries.get(r.index)?;
            (!cell.intersect(&body).is_empty()).then_some((r, e, cell))
        })
        .collect();
    let bounds = placed
        .iter()
        .map(|&(_, _, cell)| cell.intersect(&body))
        .reduce(|a, b| a.union(&b))?;
    let p = theme.paint();
    let s = p.s;
    let mut c = Canvas::new((bounds.width * s) as u32, (bounds.height * s) as u32);
    let at = |r: Rect| Rect::new(r.x - bounds.x, r.y - bounds.y, r.width, r.height).scaled(s);
    for (r, e, cell) in placed {
        let row = ListRow {
            name: &e.name,
            icon: r.icon,
            slot: lay.controls.text.len(ICON_PX),
            mark: Mark::default(),
            selected: false,
            label: true,
        };
        draw_list_row(
            &mut c,
            text,
            &p,
            &row,
            at(cell),
            at(cell.intersect(&body)),
            theme.fonts.size_px,
        );
    }
    c.fade(1, 2);
    Some((c, (pointer.0 - bounds.x, pointer.1 - bounds.y)))
}

/// The tiles or thumbnails a drag of `cells` shows: those held whole by a square
/// [`iconview::DRAG_IMAGE_MAX`] 100% px across, grown by the text size, centred on the cell under
/// `pointer`, so the pressed cell always shows whole. Cuts as [`iconview::drag_cut`] when the
/// pointer is on none of them.
fn grid_drag_cut(
    u: crate::theme::TextUnits,
    cells: &[Rect],
    pointer: (i32, i32),
) -> Option<(Vec<usize>, Rect)> {
    let Some(&pressed) = cells.iter().find(|c| c.contains(pointer.0, pointer.1)) else {
        return iconview::drag_cut(u, cells, pointer);
    };
    let side = u.len(iconview::DRAG_IMAGE_MAX);
    let (cx, cy) = (
        pressed.x + pressed.width / 2,
        pressed.y + pressed.height / 2,
    );
    let window = Rect::new(cx - side / 2, cy - side / 2, side, side).union(&pressed);
    let shown: Vec<usize> = (0..cells.len())
        .filter(|&i| window.intersect(&cells[i]) == cells[i])
        .collect();
    let r = shown.iter().map(|&i| cells[i]).reduce(|a, b| a.union(&b))?;
    Some((shown, r))
}

fn grid_drag_image(
    theme: &crate::theme::Theme,
    text: &TextRenderer,
    sv: &ShellView,
    lay: &Layout,
    rows: &[DragRow],
    pointer: (i32, i32),
) -> Option<(Canvas, (i32, i32))> {
    let placed: Vec<(&DragRow, &Entry, Rect)> = rows
        .iter()
        .filter_map(|r| Some((r, sv.entries.get(r.index)?, *lay.cells.get(r.index)?)))
        .collect();
    let cells: Vec<Rect> = placed.iter().map(|&(_, _, cell)| cell).collect();
    let (shown, r) = grid_drag_cut(theme.text, &cells, pointer)?;
    let (cs, s, u, px) = (
        &theme.colors,
        theme.scale.max(1),
        theme.text,
        theme.fonts.size_px,
    );
    let mut c = Canvas::new((r.width * s) as u32, (r.height * s) as u32);
    let at = |x: Rect| Rect::new(x.x - r.x, x.y - r.y, x.width, x.height).scaled(s);
    for (row, e, cell) in shown.into_iter().map(|i| placed[i]) {
        let label = at(shellgrid::label_of(sv.mode, cell, e, text, px, u));
        let draw = match sv.mode {
            Mode::Tiles => shellgrid::draw_tile,
            _ => shellgrid::draw_thumb,
        };
        let unselected = (false, false);
        draw(
            &mut c,
            text,
            cs,
            e,
            row.icon,
            at(cell),
            label,
            unselected,
            px,
            s,
            u,
        );
    }
    c.fade(1, 2);
    Some((c, (pointer.0 - r.x, pointer.1 - r.y)))
}
