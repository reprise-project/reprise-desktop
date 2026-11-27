use super::layout::label_rect;
use super::{Layout, Mode, ShellView};
use crate::controls::iconview;
use crate::controls::listview;
use crate::controls::shellgrid;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::{Caret, TextEdit};
use crate::theme::{TextUnits, Theme};

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
            let el = iconview::edit_layout_at(
                1,
                px,
                text,
                ed,
                shellgrid::icons_cell(cell, lay.controls.text),
            );
            return Some(el.index_at(text, px, x - cell.x, y - cell.y));
        }
        Mode::Details => label_rect(sv, lay, *i, text, px).x + listview::EDIT_PAD_L,
        Mode::Thumbnails | Mode::Tiles => {
            let el = grid_box(sv, lay, *i, ed, text, px)?;
            return Some(el.index_at(text, px, x, y));
        }
        _ => label_rect(sv, lay, *i, text, px).x - listview::EDIT_BACK + listview::EDIT_PAD_L,
    };
    Some(crate::textedit::index_at(text, &ed.text, x - text_x, px))
}

/// List's rename box over entry `i`'s label: `listview`'s single-line box, padded as it pads a
/// name cell. Details' `ListView` places its own; the grid modes wrap ([`draw_wrapped_rename`]).
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

/// The rename box over entry `i`'s label in Thumbnails and Tiles (`label` in `c`'s device px):
/// `iconview`'s wrapped box, wrapping in the label's `room` ([`wrapped_box`]).
pub(crate) fn draw_wrapped_rename(
    c: &mut Canvas,
    text: &TextRenderer,
    theme: &Theme,
    sv: &ShellView,
    i: usize,
    label: Rect,
    room: i32,
) {
    let Some((_, ed)) = sv.edit.as_ref().filter(|(j, _)| *j == i) else {
        return;
    };
    let (s, px) = (theme.scale, theme.fonts.size_px);
    let caret = Caret::new(theme.active, true);
    iconview::draw_edit(c, theme, text, ed, caret, |ed| {
        wrapped_box(sv.mode, label, room, ed, text, px, s)
    });
}

/// `iconview`'s rename box at scale `s`, first line on `label`'s: centred under a thumbnail and
/// wrapping as its label does, left-aligned in a tile and ending where the tile's `room` does.
fn wrapped_box(
    mode: Mode,
    label: Rect,
    room: i32,
    ed: &TextEdit,
    text: &TextRenderer,
    px: u32,
    s: i32,
) -> iconview::EditLayout {
    let wide = match mode {
        Mode::Tiles => {
            room + listview::EDIT_BACK - iconview::EDIT_BOX_PAD + iconview::EDIT_WRAP_INSET
        }
        _ => room + iconview::EDIT_WRAP_INSET,
    };
    let mut el = iconview::edit_layout_at(s, px, text, ed, iconview::Cell::square(wide));
    let dx = match mode {
        Mode::Tiles => label.x - listview::EDIT_BACK * s - el.rect.x,
        _ => label.x + label.width / 2 - (el.rect.x + el.rect.width / 2),
    };
    el.moved(dx, label.y - 2 * s - el.rect.y);
    if mode == Mode::Tiles {
        for l in &mut el.lines {
            l.x = el.rect.x + iconview::EDIT_BOX_PAD / 2 * s;
        }
    }
    el
}

/// [`wrapped_box`] for entry `i` in `lay`'s logical px.
pub(super) fn grid_box(
    sv: &ShellView,
    lay: &Layout,
    i: usize,
    ed: &TextEdit,
    text: &TextRenderer,
    px: u32,
) -> Option<iconview::EditLayout> {
    let cell = *lay.cells.get(i)?;
    let room = shellgrid::label_room(sv.mode, cell.width, lay.controls.text);
    let label = label_rect(sv, lay, i, text, px);
    Some(wrapped_box(sv.mode, label, room, ed, text, px, 1))
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
            let r = iconview::edit_layout_at(
                1,
                px,
                text,
                ed,
                shellgrid::icons_cell(cell, lay.controls.text),
            )
            .rect;
            Some(Rect::new(cell.x + r.x, cell.y + r.y, r.width, r.height))
        }
        Mode::Thumbnails | Mode::Tiles => Some(grid_box(sv, lay, i, ed, text, px)?.rect),
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
