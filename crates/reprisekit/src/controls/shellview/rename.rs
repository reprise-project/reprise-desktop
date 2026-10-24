use super::layout::label_rect;
use super::{Layout, Mode, ShellView};
use crate::controls::iconview;
use crate::controls::listview;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::TextEdit;
use crate::theme::TextUnits;

/// The byte of the rename text under `(x, y)`, for a press in the rename box; `None` while no
/// entry is being renamed.
pub fn edit_index_at(
    sv: &ShellView,
    lay: &Layout,
    x: i32,
    y: i32,
    text: &TextRenderer,
    px: u32,
) -> Option<usize> {
    let (i, ed) = sv.edit.as_ref()?;
    let text_x = match sv.mode {
        Mode::Icons => {
            let cell = *lay.cells.get(*i)?;
            let el = iconview::edit_layout_at(1, px, text, ed, cell.width);
            return Some(el.index_at(text, px, x - cell.x, y - cell.y));
        }
        Mode::Details => label_rect(sv, lay, *i, text, px).x + listview::EDIT_PAD_L,
        _ => label_rect(sv, lay, *i, text, px).x - listview::EDIT_BACK + listview::EDIT_PAD_L,
    };
    Some(crate::textedit::index_at(text, &ed.text, x - text_x, px))
}

/// The rename box over entry `i`'s label, for every mode but Icons (which has
/// `iconview`'s wrapped box) and Details (where the transient `ListView` places
/// its own): `listview`'s single-line box, padded as it pads a name cell.
pub(crate) fn draw_rename(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sv: &ShellView,
    i: usize,
    label: Rect,
    px: u32,
) {
    let Some((_, ed)) = sv.edit.as_ref().filter(|(j, _)| *j == i) else {
        return;
    };
    let box_r = line_box(label, ed, text, px, p.text, p.s);
    listview::draw_rename_box(c, text, p, ed, box_r, px);
}

/// The single-line rename box over `label` (both at scale `s`), sized to `ed`'s text.
fn line_box(
    label: Rect,
    ed: &TextEdit,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
    s: i32,
) -> Rect {
    let w = text.measure(&ed.text, false, px) + listview::EDIT_PAD_L + listview::EDIT_PAD_R;
    Rect::new(
        label.x - listview::EDIT_BACK * s,
        label.y - s,
        w * s,
        u.len(listview::EDIT_H) * s,
    )
}

/// The rename box over entry `i`, border included, while `i` is being renamed: the field its
/// edit menu and its presses own.
pub fn rename_rect(
    sv: &ShellView,
    lay: &Layout,
    i: usize,
    text: &TextRenderer,
    px: u32,
) -> Option<Rect> {
    let (_, ed) = sv.edit.as_ref().filter(|(j, _)| *j == i)?;
    match sv.mode {
        Mode::Details => Some(label_rect(sv, lay, i, text, px)),
        Mode::Icons => {
            let cell = *lay.cells.get(i)?;
            let r = iconview::edit_layout_at(1, px, text, ed, cell.width).rect;
            Some(Rect::new(cell.x + r.x, cell.y + r.y, r.width, r.height))
        }
        _ => Some(line_box(
            label_rect(sv, lay, i, text, px),
            ed,
            text,
            px,
            lay.controls.text,
            1,
        )),
    }
}
