use super::details::details_in;
use super::layout::label_rect;
use super::scroll::{icon_vparts, list_hparts};
use super::{Hit, Layout, Mode, ShellView};
use crate::classic::widgets::bar_hit;
use crate::controls::listview;
use crate::text::TextRenderer;

/// What's under `(x, y)`. Details delegates to the transient `ListView`'s
/// `hit`, translating `Hit::Row` to `Hit::Entry`. The icon modes test the
/// vertical scrollbar, then the cells, and report a group header as `Background`.
pub fn hit(sv: &ShellView, lay: &Layout, x: i32, y: i32) -> Hit {
    if matches!(sv.mode, Mode::Details) {
        let (lv, lv_lay) = details_in(sv, lay);
        return match lv.hit(&lv_lay, x, y) {
            listview::Hit::Row(i) => Hit::Entry(i),
            listview::Hit::Header(i) => Hit::Header(i),
            listview::Hit::Divider(i) => Hit::Divider(i),
            listview::Hit::Background => Hit::Background,
            listview::Hit::VBar(b) => Hit::VBar(b),
            listview::Hit::HBar(b) => Hit::HBar(b),
            listview::Hit::Check(_) | listview::Hit::Nothing => Hit::Nothing,
        };
    }
    if let Some(vb) = lay.vscroll.filter(|vb| vb.contains(x, y)) {
        return Hit::VBar(bar_hit(&icon_vparts(sv, lay, vb), x, y, false));
    }
    if let Some(hb) = lay.hscroll
        && hb.contains(x, y)
    {
        return Hit::HBar(bar_hit(&list_hparts(sv, lay, hb), x, y, true));
    }
    if lay.body.contains(x, y) {
        return lay
            .cells
            .iter()
            .position(|c| c.contains(x, y))
            .map_or(Hit::Background, Hit::Entry);
    }
    Hit::Nothing
}

/// The entry pointed at by its icon or name, what an infotip hangs from: any
/// point of the cell in the icon modes, only the icon and label in List and
/// Details, where the space after a short name and other columns point at nothing.
pub fn name_hit(
    sv: &ShellView,
    lay: &Layout,
    x: i32,
    y: i32,
    text: &TextRenderer,
    px: u32,
) -> Option<usize> {
    let Hit::Entry(i) = hit(sv, lay, x, y) else {
        return None;
    };
    let named = match sv.mode {
        // The name as its column cuts it short, not the rename box's rect, which runs on for the whole name.
        Mode::Details => {
            let (lv, lv_lay) = details_in(sv, lay);
            x < lv.name_rect(&lv_lay, i, text, px).right()
        }
        Mode::List => x < label_rect(sv, lay, i, text, px).right(),
        _ => true,
    };
    named.then_some(i)
}
