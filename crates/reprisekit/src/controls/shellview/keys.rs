use super::details::with_details;
use super::scroll::ensure_visible;
use super::{Layout, Mode, ShellView};
use crate::controls::iconview::{self, IconMove};
use crate::controls::listview;
use crate::selection::Mods;

/// Move the focus by `dir` (`iconview::move_selection`'s rule: the nearest
/// cell in a 90-degree cone), let `mods` decide the selection, then scroll it
/// into view; `true` when the focus moved. Details uses `details_step`.
pub fn step(sv: &mut ShellView, dir: IconMove, lay: &Layout, mods: Mods) -> bool {
    if matches!(sv.mode, Mode::Details) {
        return details_step(sv, dir, lay, mods);
    }
    let next = iconview::move_selection(sv.selection.focus, &lay.cells, dir);
    if next == sv.selection.focus {
        return false;
    }
    let Some(to) = next else { return false };
    sv.selection.move_focus(to, mods);
    sv.focus_ring = true;
    ensure_visible(sv, to, lay);
    true
}

/// Details' arm of `step`: Up/Down move a row, Home/End jump to the ends, Left/Right do nothing.
fn details_step(sv: &mut ShellView, dir: IconMove, lay: &Layout, mods: Mods) -> bool {
    let row_step = match dir {
        IconMove::Up => -1,
        IconMove::Down => 1,
        IconMove::Home => i64::MIN / 2,
        IconMove::End => i64::MAX / 2,
        IconMove::Left | IconMove::Right => return false,
    };
    with_details(sv, |lv| {
        let lv_lay = listview::layout(lv, lay.frame, &lay.controls);
        lv.step(row_step, &lv_lay, mods)
    })
}

/// Jump to the next entry whose name starts with `ch`, case-insensitively,
/// searching from just after the focus and wrapping; `true` when something matched.
pub fn type_ahead(sv: &mut ShellView, ch: char) -> bool {
    let names = sv.entries.iter().map(|e| e.name.as_str());
    match crate::selection::type_ahead(names, sv.selection.focus, ch) {
        Some(i) => {
            sv.selection.set_only(Some(i));
            true
        }
        None => false,
    }
}
