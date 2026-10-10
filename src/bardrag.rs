//! Dragging an unlocked taskbar: from empty bar space to another edge of
//! the output, or a side bar by its inner edge to a new width. The edge
//! drag outlines the bar where it would land (`State::outline_tree`) and
//! leaves the bars where they are until the release, which moves every
//! output's bar. The width drag lays the bars out at the dragged width
//! while they reserve the old one (`barplace::place_resizing`), so windows
//! refit once, on the release. A release applies `[taskbar]` as a reload
//! of the file would (`configwatch::reload`) and writes the key on the
//! config writer thread, so the reload the write brings finds nothing new.
//! Escape, or anything else that takes the pointer, drops a drag
//! unapplied.

use reprisescene::TreeId;
use toml_edit::Value;

use crate::compositor::State;
use crate::config::TaskbarConfig;
use crate::input::Interaction;
use crate::raster::Rect;

impl State {
    /// A left press on empty space of unlocked bar `tree`: held, until the
    /// pointer moves `desktop::DRAG_PX` away, then an edge drag.
    pub(crate) fn begin_bar_move(&mut self, tree: TreeId) {
        let p = self.pointer_location();
        self.interaction = Interaction::BarMove {
            bar: tree,
            p0: (p.x, p.y),
            target: None,
        };
    }

    /// The pointer moved during a drag of the bar: an edge drag aims at
    /// an edge (`bar_move_moved`), a width drag relays the bars out
    /// (`bar_width_moved`).
    pub(crate) fn bar_drag_moved(&mut self) {
        match self.interaction {
            Interaction::BarMove { bar, p0, target } => self.bar_move_moved(bar, p0, target),
            Interaction::BarWidth { bar, grab } => self.bar_width_moved(bar, grab),
            _ => {}
        }
    }

    /// An edge drag's motion: past the slop it aims at the edge whose
    /// triangle holds the pointer on the output under it, with the `move`
    /// cursor and the outline there.
    fn bar_move_moved(
        &mut self,
        bar: TreeId,
        p0: (f64, f64),
        target: Option<reprisekit::taskbar::Edge>,
    ) {
        let p = self.pointer_location();
        let slop = f64::from(crate::desktop::DRAG_PX);
        if target.is_none() && (p.x - p0.0).abs() < slop && (p.y - p0.1).abs() < slop {
            return;
        }
        let Some(out) = self.output_at(p.x, p.y) else {
            return;
        };
        let b = self.layout_box(Some(&out));
        let edge = crate::barplace::edge_for(b, p.x, p.y);
        self.interaction = Interaction::BarMove {
            bar,
            p0,
            target: Some(edge),
        };
        if target.is_none() {
            self.set_cursor("move");
        }
        let (theme, tb) = (&self.config.theme, &self.config.taskbar);
        let t = crate::barplace::thickness(
            edge,
            &theme.metrics,
            &theme.taskbar_shape(),
            tb.side_width,
            b,
        );
        self.show_bar_outline(crate::barplace::place(b, edge, t).shown);
    }

    /// A width drag's motion: the bars relay out at the width the
    /// pointer gives the dragged bar's output, when that changed.
    fn bar_width_moved(&mut self, bar: TreeId, grab: i32) {
        let Some(output) = self.bar(bar).map(|b| b.output.clone()) else {
            return;
        };
        let p = self.pointer_location();
        let b = self.layout_box(Some(&output));
        let theme = &self.config.theme;
        let w = crate::barplace::dragged_width(
            self.config.taskbar.position,
            &theme.metrics,
            &theme.taskbar_shape(),
            b,
            (p.x as i32, p.y as i32),
            grab,
        );
        if self.bar_shared.side_width_drag != Some(w) {
            self.bar_shared.side_width_drag = Some(w);
            self.redraw_taskbars();
        }
    }

    /// A left press on the inner edge of unlocked side bar `tree`: the
    /// bars take the width the pointer gives them until the release.
    pub(crate) fn begin_bar_width(&mut self, tree: TreeId) {
        let Some(output) = self.bar(tree).map(|b| b.output.clone()) else {
            return;
        };
        let p = self.pointer_location();
        let b = self.layout_box(Some(&output));
        let placed = self.bar_placement(&output);
        let depth = crate::barplace::depth(self.config.taskbar.position, b, p.x as i32, p.y as i32);
        self.interaction = Interaction::BarWidth {
            bar: tree,
            grab: placed.thickness - depth,
        };
        self.set_cursor("ew-resize");
    }

    /// The last button came up during a drag of the bar (`was`, already
    /// taken off `interaction`): an edge drag aimed at another edge moves
    /// every bar there; one released on its own edge, or before it
    /// started, does nothing. A width drag keeps the width it gave,
    /// written as `side_width` when it differs from the configured one.
    pub(crate) fn end_bar_drag(&mut self, was: Interaction) {
        self.hide_bar_outline();
        match was {
            Interaction::BarMove {
                target: Some(edge), ..
            } if edge != self.config.taskbar.position => {
                self.set_taskbar("position", Value::from(edge.name()), |t| t.position = edge);
            }
            Interaction::BarWidth { bar, .. } => {
                let Some(w) = self.bar_shared.side_width_drag.take() else {
                    return;
                };
                // The dragged bar's output is gone: drop the drag.
                let Some(output) = self.bar(bar).map(|b| b.output.clone()) else {
                    self.redraw_taskbars();
                    return;
                };
                // The configured width on the dragged bar's output.
                if self.bar_placement(&output).thickness == w {
                    self.redraw_taskbars();
                } else {
                    self.set_taskbar("side_width", Value::from(i64::from(w)), |t| {
                        t.side_width = w
                    });
                }
            }
            _ => {}
        }
    }

    /// A drag of the bar cut short (`cancel_interaction`): nothing is
    /// applied; the outline goes and the bars take their configured
    /// width back.
    pub(crate) fn cancel_bar_drag(&mut self) {
        self.hide_bar_outline();
        if self.bar_shared.side_width_drag.take().is_some() {
            self.redraw_taskbars();
        }
    }

    /// Apply `change` to `[taskbar]` live, every bar, popup and window
    /// following as on a reload of the file, then write `name = value`
    /// into `config.toml` on the writer thread.
    pub(crate) fn set_taskbar(
        &mut self,
        name: &'static str,
        value: Value,
        change: impl FnOnce(&mut TaskbarConfig),
    ) {
        let mut cfg = self.config.clone();
        change(&mut cfg.taskbar);
        crate::configwatch::reload(self, cfg);
        let key = repriseconf::key("taskbar", name);
        crate::configwrite::save_value(&self.config_path, key, value);
    }

    /// Outline `r`, the bar's landing place; an unchanged one costs
    /// nothing.
    fn show_bar_outline(&mut self, r: Rect) {
        if self.bar_shared.outline.is_some_and(|(_, s)| s == r) {
            return;
        }
        self.hide_bar_outline();
        self.bar_shared.outline = self
            .outline_tree(r, reprisekit::classic::snapframe::BAR_BAND)
            .map(|t| (t, r));
        self.redraw_all();
    }

    fn hide_bar_outline(&mut self) {
        if let Some((tree, _)) = self.bar_shared.outline.take() {
            self.scene.destroy(tree);
            self.redraw_all();
        }
    }
}
