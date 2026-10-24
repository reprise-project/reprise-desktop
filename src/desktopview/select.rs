use std::path::PathBuf;

use reprisekit::selection::{self, Mods};
use smithay::utils::Rectangle;

use crate::{
    compositor::State,
    desktop::{self, DesktopEntry},
    raster::Rect,
};

use super::{Marquee, Press};

/// The indices of `set` a command aimed at `index` takes: `index` first, then
/// the rest of the set; with `files_only`, only its files (a place or the Recycle
/// Bin is no file to copy or send). Indices past `entries` are dropped.
pub fn aimed_in(
    entries: &[DesktopEntry],
    set: &[usize],
    index: usize,
    files_only: bool,
) -> Vec<usize> {
    std::iter::once(index)
        .chain(set.iter().copied().filter(|&i| i != index))
        .filter(|&i| {
            entries
                .get(i)
                .is_some_and(|e| i == index || !files_only || e.kind == desktop::Kind::File)
        })
        .collect()
}

pub(super) fn source_of(e: &DesktopEntry) -> crate::dragview::DragSource {
    crate::dragview::DragSource::Icon {
        id: e.id(),
        path: e.path.clone(),
        name: e.shortcut_base().to_string(),
    }
}

/// A 1 px dotted line `len` logical px long, across or `vertical`, with the
/// screen-wide pattern of a line starting at the origin: a dot on every odd px.
pub(super) fn band_strip(
    len: i32,
    vertical: bool,
    scale: i32,
    ink: reprisekit::theme::Color,
) -> reprisekit::raster::Canvas {
    let (w, h) = if vertical { (1, len) } else { (len, 1) };
    let mut c = reprisekit::raster::Canvas::new((w * scale) as u32, (h * scale) as u32);
    reprisekit::classic::widgets::draw_focus_ring_screen(
        &mut c,
        Rect::new(0, 0, w * scale, h * scale),
        scale,
        ink,
        (0, 0),
    );
    c
}

/// The part of a `band_strip`, in buffer px, that draws `edge`: it starts
/// `(x + y) % 2` px in, so the edge's dots fall where x + y is odd.
pub(super) fn band_crop(edge: Rect, vertical: bool, scale: i32) -> Rect {
    let phase = (edge.x + edge.y).rem_euclid(2) * scale;
    if vertical {
        Rect::new(0, phase, scale, edge.height * scale)
    } else {
        Rect::new(phase, 0, edge.width * scale, scale)
    }
}

impl State {
    pub fn desktop_hit(&self, lx: f64, ly: f64) -> Option<usize> {
        let view = self.desktop.as_ref()?;
        let (x, y) = (lx.floor() as i32, ly.floor() as i32);
        view.cells
            .iter()
            .position(|c| c.contains(x, y))
            .filter(|&i| i < view.entries.len())
    }

    /// A left press on the desktop at `at`: on a cell it clicks it (Ctrl toggles,
    /// Shift ranges, see `Selection::click`), and a second plain press on the same
    /// cell within `double_click_ms` opens it. A plain press on an icon of a
    /// multi-selection keeps the set for a drag (`Press::Icon`). On empty desktop it clears the
    /// selection (Ctrl keeps it), arms the marquee at `at` and is true on a double click.
    pub fn desktop_press(
        &mut self,
        index: Option<usize>,
        at: (i32, i32),
        time_msec: u32,
        mods: Mods,
    ) -> bool {
        let click = self.config.theme.metrics.click(at.0, at.1, time_msec);
        let Some(view) = self.desktop.as_mut() else {
            return false;
        };
        let mut empty_double = false;
        let before = view.selection.clone();
        let mut open = None;
        match index.filter(|&i| i < view.entries.len()) {
            Some(i) => {
                let double = view.last_click.press(Some(i), click);
                let in_set =
                    mods == Mods::default() && view.selection.len() > 1 && view.selection.is(i);
                view.press = in_set.then_some(Press::Icon(i));
                if !in_set {
                    view.selection.click(i, mods);
                }
                if double && mods == Mods::default() {
                    open = Some(i);
                }
            }
            None => {
                empty_double = view.last_click.press(None, click);
                if !mods.ctrl {
                    view.selection.clear();
                }
                view.press = Some(Press::Marquee(Marquee {
                    origin: at,
                    base: view.selection.set.clone(),
                    ctrl: mods.ctrl,
                    band: None,
                }));
            }
        }
        if view.selection != before {
            self.relayout_desktop();
        }
        if let Some(i) = open {
            self.open_icon(i);
        }
        empty_double
    }

    /// The button came up after a press that kept a multi-selection (`Press::Icon`):
    /// without a drag that icon is selected alone; after one the set stays.
    pub fn settle_click(&mut self, dragged: bool) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let pending = match view.press {
            Some(Press::Icon(i)) => {
                view.press = None;
                Some(i)
            }
            _ => None,
        };
        if let (Some(i), false) = (pending, dragged) {
            self.select_icon(Some(i));
        }
    }

    /// The pointer moved to (`lx`, `ly`) with a marquee armed: past
    /// `desktop::DRAG_PX` the band spans the press point and the pointer and every
    /// icon it touches is selected (toggled against the press-time set with Ctrl).
    pub fn drag_marquee(&mut self, lx: f64, ly: f64) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let Some(Press::Marquee(m)) = view.press.as_mut() else {
            return;
        };
        let to = (lx as i32, ly as i32);
        if m.band.is_none()
            && (to.0 - m.origin.0).abs() < desktop::DRAG_PX
            && (to.1 - m.origin.1).abs() < desktop::DRAG_PX
        {
            return;
        }
        let band = selection::band(m.origin, to);
        if m.band == Some(band) {
            return;
        }
        m.band = Some(band);
        let (base, ctrl) = (m.base.clone(), m.ctrl);
        let before = view.selection.clone();
        view.selection.marquee(&view.cells, band, &base, ctrl);
        if view.selection != before {
            self.relayout_desktop();
        }
        self.draw_band(Some(band));
    }

    pub fn end_marquee(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        if let Some(Press::Marquee(m)) = view.press.take()
            && m.band.is_some()
        {
            self.draw_band(None);
        }
    }

    /// Show `band`'s four dotted edges, or hide them on `None`. The dots are the
    /// desktop colour's inverse; each edge crops one of two cached strips, so a
    /// pointer step draws nothing. A look with a selection box (Glass) shows that
    /// box instead.
    fn draw_band(&mut self, band: Option<Rect>) {
        let out = self.first_output();
        let (scale, screen) = match &out {
            Some(o) => (self.draw_scale_at(Some(o)).max(1), self.layout_box(Some(o))),
            None => (1, Rect::default()),
        };
        let ink = self.config.theme.colors.background.inverted();
        let boxed = reprisekit::look::widgets::selection_box(&self.config.theme.look);
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let nodes = view.band_nodes;
        let band = band.filter(|b| !b.is_empty());
        if let Some(rects) = view.band_box
            && (band.is_none() || boxed.is_none())
        {
            for n in rects {
                self.scene.set_enabled(n, false);
            }
        }
        let Some(b) = band else {
            for n in nodes {
                self.scene.set_enabled(n, false);
            }
            self.redraw_all();
            return;
        };
        if let Some(sel) = boxed {
            for n in nodes {
                self.scene.set_enabled(n, false);
            }
            let tree = view.tree;
            let rects = *view
                .band_box
                .get_or_insert_with(|| crate::snapview::selection_box_nodes(&mut self.scene, tree));
            crate::snapview::place_selection_box(&mut self.scene, &rects, sel, b);
            self.redraw_all();
            return;
        }
        // One logical px of slack for the phase shift in `band_crop`.
        let (need_w, need_h) = (b.width + 1, b.height + 1);
        let fits = view
            .band_strip
            .is_some_and(|(s, c, w, h)| s == scale && c == ink && w >= need_w && h >= need_h);
        if !fits {
            let (w, h) = (need_w.max(screen.width + 1), need_h.max(screen.height + 1));
            view.band_strip = Some((scale, ink, w, h));
            let across = band_strip(w, false, scale, ink);
            let down = band_strip(h, true, scale, ink);
            for (n, strip) in nodes.into_iter().zip([&across, &across, &down, &down]) {
                let (w, h) = (strip.width as i32, strip.height as i32);
                self.scene.buffer_set_canvas(n, &strip.pixels, w, h);
            }
        }
        let edges = [
            (Rect::new(b.x, b.y, b.width, 1), false),
            (Rect::new(b.x, b.bottom() - 1, b.width, 1), false),
            (Rect::new(b.x, b.y, 1, b.height), true),
            (Rect::new(b.right() - 1, b.y, 1, b.height), true),
        ];
        for (n, (edge, vertical)) in nodes.into_iter().zip(edges) {
            let crop = band_crop(edge, vertical, scale);
            let source = Rectangle::new(
                (f64::from(crop.x), f64::from(crop.y)).into(),
                (f64::from(crop.width), f64::from(crop.height)).into(),
            );
            self.scene.buffer_set_source_box(n, Some(source));
            self.scene
                .buffer_set_dest_size(n, Some((edge.width, edge.height).into()));
            self.scene.set_position(n, edge.x, edge.y);
            self.scene.raise_to_top(n);
            self.scene.set_enabled(n, true);
        }
        self.redraw_all();
    }

    pub fn drag_sources(&self, index: usize) -> Vec<crate::dragview::DragSource> {
        self.aimed_sources(index, false)
    }

    pub(super) fn aimed_sources(
        &self,
        index: usize,
        files_only: bool,
    ) -> Vec<crate::dragview::DragSource> {
        let Some(view) = self.desktop.as_ref() else {
            return Vec::new();
        };
        aimed_in(
            &view.entries,
            &self.selection_around(index),
            index,
            files_only,
        )
        .into_iter()
        .map(|i| source_of(&view.entries[i]))
        .collect()
    }

    pub(crate) fn select_icon(&mut self, index: Option<usize>) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let index = index.filter(|&i| i < view.entries.len());
        let before = view.selection.clone();
        view.selection.set_only(index);
        if view.selection != before {
            self.relayout_desktop();
        }
    }

    /// A right press on icon `index`: one outside the selection becomes the only
    /// selected one, one inside keeps the set (its menu is for all of them).
    pub fn desktop_right_press(&mut self, index: usize) {
        let keep = self.desktop.as_ref().is_some_and(|v| v.selection.is(index));
        if !keep {
            self.select_icon(Some(index));
        }
    }

    pub(crate) fn selection_around(&self, index: usize) -> Vec<usize> {
        match self.desktop.as_ref() {
            Some(v) if v.selection.len() > 1 && v.selection.is(index) => {
                v.selection.set.iter().copied().collect()
            }
            _ => vec![index],
        }
    }

    pub(super) fn aimed_files(&self, index: usize) -> Vec<PathBuf> {
        let Some(view) = self.desktop.as_ref() else {
            return Vec::new();
        };
        aimed_in(&view.entries, &self.selection_around(index), index, true)
            .into_iter()
            .filter(|&i| !view.entries[i].is_dir)
            .map(|i| view.entries[i].path.clone())
            .collect()
    }
}
