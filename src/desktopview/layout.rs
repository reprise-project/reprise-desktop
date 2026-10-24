use std::borrow::Cow;
use std::path::PathBuf;

use reprisescene::BufferId;

use crate::{
    compositor::State,
    desktop::{self, DesktopEntry, ICON_PX},
    output::upload_scaled,
    raster::Rect,
    textedit::TextEdit,
};

/// The cell of `grid` an answer landing `name` at `wanted` takes: `wanted` unless another icon is
/// laid out there (`cells` runs parallel to `entries`) or an earlier answer landed there since the
/// last read, else the free cell nearest it. A late answer never evicts an icon. `wanted` and
/// `landed` are 100% cells ([`desktop::to_saved`]): the grid may have grown since they were picked.
pub(super) fn landing_cell(
    grid: desktop::Cell,
    area: Rect,
    entries: &[DesktopEntry],
    cells: &[Rect],
    landed: &[(i32, i32)],
    name: &str,
    wanted: (i32, i32),
) -> (i32, i32) {
    let taken: std::collections::HashSet<(i32, i32)> = entries
        .iter()
        .zip(cells)
        .filter(|(e, _)| e.name != name)
        .map(|(_, r)| desktop::cell_at(grid, area, r.x, r.y))
        .chain(landed.iter().map(|&c| desktop::from_saved(grid, c)))
        .collect();
    desktop::free_cell_near(grid, area, &taken, desktop::from_saved(grid, wanted))
}

/// Everything a cell's pixels depend on besides the theme (`drawn_theme`). Equal
/// keys mean equal pixels. Built borrowed and stored owned, so an unchanged cell
/// costs no allocation.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct CellKey<'a> {
    pub(super) label: Cow<'a, str>,
    pub(super) icon: Cow<'a, str>,
    pub(super) shortcut: bool,
    pub(super) cell: Rect,
    pub(super) selected: bool,
    pub(super) ring: bool,
    pub(super) cut: bool,
    pub(super) editing: Option<(Cow<'a, str>, std::ops::Range<usize>, usize, bool)>,
    pub(super) preedit: Option<reprisekit::textedit::Preedit>,
}

impl CellKey<'_> {
    pub(super) fn into_owned(self) -> CellKey<'static> {
        CellKey {
            label: Cow::Owned(self.label.into_owned()),
            icon: Cow::Owned(self.icon.into_owned()),
            editing: self
                .editing
                .map(|(t, sel, caret, on)| (Cow::Owned(t.into_owned()), sel, caret, on)),
            ..self
        }
    }
}

struct Redraw {
    index: usize,
    node: BufferId,
    cell: Rect,
    label: String,
    icon: String,
    shortcut: bool,
    cut: bool,
    selection: Option<desktop::Selection>,
    editing: Option<(TextEdit, bool)>,
}

impl State {
    pub fn relayout_desktop(&mut self) {
        let out = self.first_output();
        let (area, scale) = match &out {
            Some(o) => (self.usable_area(Some(o)), self.draw_scale_at(Some(o))),
            None => (Rect::default(), 1),
        };
        let caret_on = self.caret.on && self.editor() == Some(crate::caret::Editor::Rename);
        let preedit = self.ime_field_preedit(crate::caret::Editor::Rename);
        let grid = self.desktop_grid();
        let mut spot = None;
        // A Cut file shows its icon at half strength until it is pasted.
        let cut: Vec<PathBuf> = self
            .own_file_clip()
            .filter(|c| c.cut)
            .map(|c| c.paths)
            .unwrap_or_default();
        let Some(mut view) = self.desktop.take() else {
            return;
        };
        view.cells = if out.is_none() {
            Vec::new()
        } else if view.auto_arrange {
            desktop::layout(grid, area, view.entries.len())
        } else {
            let names: Vec<&str> = view.entries.iter().map(|e| e.name.as_str()).collect();
            desktop::layout_free(grid, area, &names, &view.positions)
        };
        let count = view.cells.len();
        let mut changed = view.nodes.len() != count;
        while view.nodes.len() > count {
            if let Some(n) = view.nodes.pop() {
                self.scene.destroy(n);
            }
        }
        while view.nodes.len() < count {
            view.nodes.push(self.scene.buffer_create(view.tree));
        }
        let base = &self.config.theme;
        if view
            .drawn_theme
            .as_ref()
            .is_none_or(|(b, t)| b != base || t.scale != scale.max(1))
        {
            view.drawn_theme = Some((base.clone(), base.scaled(scale)));
            view.drawn.clear();
        }
        view.drawn.resize(count, None);
        let mut redraw = Vec::new();
        let dropping = view
            .rename
            .as_ref()
            .zip(view.rename_drop)
            .map(|((_, e), at)| {
                let mut e = e.clone();
                e.place(at, false);
                e
            });
        for (i, (e, cell)) in view.entries.iter().zip(&view.cells).enumerate() {
            let is_cut = cut.contains(&e.path);
            let editing = view
                .rename
                .as_ref()
                .filter(|(ri, _)| *ri == i)
                .map(|(_, edit)| match &dropping {
                    Some(d) => (d, true),
                    None => (edit, caret_on),
                });
            let selected = view.selection.is(i) || view.drop_hilite == Some(i);
            let ring = view.focus_ring && view.selection.focus == Some(i);
            let key = CellKey {
                label: Cow::Borrowed(&e.label),
                icon: Cow::Borrowed(&e.icon),
                shortcut: e.shortcut,
                cell: *cell,
                selected,
                ring,
                cut: is_cut,
                editing: editing.map(|(ed, on)| {
                    (
                        Cow::Borrowed(ed.text.as_str()),
                        ed.selection(),
                        ed.caret(),
                        on,
                    )
                }),
                preedit: editing.and(preedit.clone()),
            };
            if view.drawn[i].as_ref() == Some(&key) {
                continue;
            }
            view.drawn[i] = Some(key.into_owned());
            redraw.push(Redraw {
                index: i,
                node: view.nodes[i],
                cell: *cell,
                label: e.label.clone(),
                icon: e.icon.clone(),
                shortcut: e.shortcut,
                cut: is_cut,
                selection: selected.then_some(desktop::Selection {
                    ring,
                    origin: (cell.x, cell.y),
                }),
                editing: editing.map(|(ed, on)| (ed.clone(), on)),
            });
        }
        let px = ICON_PX * scale;
        let mut undecoded = false;
        if let Some((_, theme)) = view.drawn_theme.as_ref() {
            for r in &redraw {
                // Icon files are decoded by the desktop's read, never here: one not decoded at
                // this size (the scale changed, or the cache dropped it) shows no picture until
                // a read decodes it.
                let mut icon = if r.icon.starts_with('/') {
                    let put = self.icons.file_put(&r.icon, px);
                    if put.is_none() {
                        view.drawn[r.index] = None;
                        undecoded = true;
                    }
                    put.flatten()
                } else {
                    self.icons.get_shared(&r.icon, px)
                };
                if r.cut
                    && let Some(ic) = icon.as_mut()
                {
                    std::sync::Arc::make_mut(ic).fade(1, 2);
                }
                let editing = r
                    .editing
                    .as_ref()
                    .map(|(ed, on)| (ed, reprisekit::textedit::Caret::new(true, *on)));
                let mut canvas = desktop::draw_cell(
                    theme,
                    self.text.for_output(theme.scale),
                    &r.label,
                    icon.as_deref(),
                    r.shortcut,
                    r.selection,
                    editing,
                    editing.and(preedit.as_ref()),
                );
                // A rename box wider than the cell comes back on a wider
                // canvas centred on the cell; it goes on top of its
                // neighbours.
                let extra = (canvas.width as i32 - grid.w * scale) / (2 * scale.max(1));
                self.scene.set_position(r.node, r.cell.x - extra, r.cell.y);
                if editing.is_some() {
                    self.scene.raise_to_top(r.node);
                    let origin = (r.cell.x - extra, r.cell.y);
                    spot = Some((canvas.take_spot(), origin));
                }
                upload_scaled(&mut self.scene, r.node, &canvas, scale);
            }
        }
        changed |= !redraw.is_empty();
        let reread = undecoded && view.icons_read != (px, self.icons.file_drops());
        self.desktop = Some(view);
        if reread {
            self.rescan_desktop();
        }
        if let Some((spot, origin)) = spot {
            self.ime_field_drawn(crate::caret::Editor::Rename, spot, origin, scale);
        }
        if changed {
            self.redraw_all();
        }
        self.water_obstacles();
    }

    /// Forget what the cells were drawn from, so the next relayout redraws them all
    /// (a new icon cache or text renderer, which `CellKey` cannot see).
    pub fn invalidate_desktop(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.drawn.clear();
        }
    }

    pub(crate) fn desktop_area(&self) -> Rect {
        let out = self.first_output();
        self.usable_area(out.as_ref())
    }

    /// The desktop's grid cell, grown with the text size like its labels.
    pub(crate) fn desktop_grid(&self) -> desktop::Cell {
        desktop::Cell::grown(self.config.theme.text)
    }

    /// The drop target of the desktop cell under (`x`, `y`).
    pub(crate) fn desktop_drop_at(&self, x: i32, y: i32) -> crate::dragview::DropTarget {
        let grid = self.desktop_grid();
        let cell = desktop::cell_at(grid, self.desktop_area(), x, y);
        crate::dragview::DropTarget::Desktop(desktop::to_saved(grid, cell))
    }

    pub(crate) fn desktop_auto_arrange(&self) -> bool {
        self.desktop.as_ref().is_some_and(|d| d.auto_arrange)
    }

    pub(crate) fn place_icon(&mut self, name: &str, cell: (i32, i32)) {
        self.pin_icon(name, cell);
        self.relayout_desktop();
    }

    /// Remember grid cell `cell` for the icon called `name` and save it, in 100% cells
    /// ([`desktop::to_saved`]), without redrawing.
    pub(crate) fn pin_icon(&mut self, name: &str, cell: (i32, i32)) {
        let grid = self.desktop_grid();
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        // One icon per cell: whoever held it before falls back into the flow.
        view.positions
            .retain(|_, c| desktop::from_saved(grid, *c) != cell);
        view.positions
            .insert(name.to_string(), desktop::to_saved(grid, cell));
        crate::state::save_desktop(view.auto_arrange, &view.positions);
    }

    pub(super) fn arrange_by_name(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.positions.clear();
            crate::state::save_desktop(view.auto_arrange, &view.positions);
        }
        self.relayout_desktop();
    }

    /// Toggle Auto Arrange. Turning it on also drops the positions.
    pub(super) fn toggle_auto_arrange(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.auto_arrange = !view.auto_arrange;
            if view.auto_arrange {
                view.positions.clear();
            }
            crate::state::save_desktop(view.auto_arrange, &view.positions);
        }
        self.relayout_desktop();
    }

    /// Pin `name`, which an answer from the file jobs just put on the desktop, to `saved`, a 100%
    /// cell ([`desktop::to_saved`]) picked when the job was asked, or to the free cell nearest it
    /// when an icon took it while the answer was pending.
    pub(crate) fn land_icon(&mut self, name: &str, saved: (i32, i32)) {
        let (area, grid) = (self.desktop_area(), self.desktop_grid());
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let cell = landing_cell(
            grid,
            area,
            &view.entries,
            &view.cells,
            &view.landed,
            name,
            saved,
        );
        view.landed.push(desktop::to_saved(grid, cell));
        self.pin_icon(name, cell);
    }
}
