//! A marquee drag over the view, armed by a press on its background.

use super::{Layout, Mode, ShellView, hstep, scroll};
use crate::selection;
use std::collections::BTreeSet;

/// How far the pointer moves from the press before the band shows.
pub const MARQUEE_SLOP: i32 = 4;

/// Once the pointer has moved [`MARQUEE_SLOP`] px from the press, the band from the press to the
/// pointer selects the entries it touches; with Ctrl it toggles them against the selection the
/// press found. The press's corner stays on the content while the view scrolls.
#[derive(Clone, Debug, PartialEq)]
pub struct MarqueeDrag {
    /// The press, from the first cell's corner as laid out then (the body's for an empty view).
    from: (i32, i32),
    /// The pointer, in the caller's coordinates.
    to: (i32, i32),
    pub base: BTreeSet<usize>,
    ctrl: bool,
    shown: bool,
}

/// The point the press's corner is kept from: the first cell's top-left, which moves with the
/// content.
fn origin(lay: &Layout) -> (i32, i32) {
    let r = lay.cells.first().copied().unwrap_or(lay.body);
    (r.x, r.y)
}

impl MarqueeDrag {
    /// A press at `at` on the background of `sv` laid out as `lay`, Ctrl held with `ctrl`.
    pub fn press(sv: &ShellView, lay: &Layout, at: (i32, i32), ctrl: bool) -> MarqueeDrag {
        let o = origin(lay);
        MarqueeDrag {
            from: (at.0 - o.0, at.1 - o.1),
            to: at,
            base: sv.selection.set.clone(),
            ctrl,
            shown: false,
        }
    }

    /// The pointer moved to `at`: the band follows it once past the slop. True when the view
    /// draws differently.
    pub fn motion(&mut self, sv: &mut ShellView, lay: &Layout, at: (i32, i32)) -> bool {
        self.to = at;
        let o = origin(lay);
        let from = (self.from.0 + o.0, self.from.1 + o.1);
        if !self.shown
            && (at.0 - from.0).abs() < MARQUEE_SLOP
            && (at.1 - from.1).abs() < MARQUEE_SLOP
        {
            return false;
        }
        self.shown = true;
        let band = selection::band(from, at);
        let before = (sv.selection.set.clone(), sv.marquee);
        sv.selection
            .marquee(&lay.cells, band, &self.base, self.ctrl);
        sv.marquee = Some(band);
        before != (sv.selection.set.clone(), sv.marquee)
    }

    /// The way the view scrolls toward a pointer past the body's edge: sideways in List, else
    /// up or down. None while the band is not up or the pointer is inside.
    pub fn edge(&self, sv: &ShellView, lay: &Layout) -> Option<i32> {
        if !self.shown {
            return None;
        }
        let (at, low, high) = match sv.mode {
            Mode::List => (self.to.0, lay.body.x, lay.body.right()),
            _ => (self.to.1, lay.body.y, lay.body.bottom()),
        };
        match at {
            _ if at < low => Some(-1),
            _ if at >= high => Some(1),
            _ => None,
        }
    }

    /// One step of scroll toward the pointer past the edge, for a host's tick. True when the
    /// view scrolled; the host lays it out again and hands the new layout to [`Self::follow`].
    pub fn scroll_step(&self, sv: &mut ShellView, lay: &Layout) -> bool {
        match (self.edge(sv, lay), sv.mode) {
            (Some(dir), Mode::List) => hstep(sv, dir, lay),
            (Some(dir), _) => scroll(sv, dir, lay),
            (None, _) => false,
        }
    }

    /// The band again over `lay`, the view's layout after a scroll.
    pub fn follow(&mut self, sv: &mut ShellView, lay: &Layout) -> bool {
        self.motion(sv, lay, self.to)
    }

    /// The release: the band goes, the selection stays.
    pub fn release(self, sv: &mut ShellView) {
        sv.marquee = None;
    }

    /// Escape: the band goes and the selection is the one the press found.
    pub fn cancel(self, sv: &mut ShellView) {
        sv.marquee = None;
        sv.selection.set = self.base;
        sv.selection.focus = sv.selection.set.iter().next().copied();
        sv.selection.anchor = sv.selection.focus;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::metrics::ControlMetrics;
    use crate::controls::shellview::{Entry, layout};
    use crate::raster::Rect;

    fn view(n: usize, mode: Mode) -> ShellView {
        let mut sv = ShellView {
            mode,
            ..ShellView::default()
        };
        sv.entries = (0..n)
            .map(|i| Entry {
                name: format!("{i:02}.txt"),
                ..Entry::default()
            })
            .collect();
        sv
    }

    const BODY: Rect = Rect::new(0, 0, 200, 120);

    fn lay(sv: &ShellView) -> Layout {
        layout(sv, BODY, &ControlMetrics::default())
    }

    #[test]
    fn the_band_shows_past_the_slop_and_selects_the_cells_it_touches() {
        let mut sv = view(30, Mode::Icons);
        let l = lay(&sv);
        let mut m = MarqueeDrag::press(&sv, &l, (1, 1), false);
        assert!(!m.motion(&mut sv, &l, (2, 2)), "within the slop");
        assert!(sv.marquee.is_none());
        let past = (l.cells[1].x + 2, l.cells[0].y + 5);
        assert!(m.motion(&mut sv, &l, past));
        assert_eq!(sv.selection.set, BTreeSet::from([0, 1]));
        m.release(&mut sv);
        assert!(sv.marquee.is_none());
        assert_eq!(
            sv.selection.set,
            BTreeSet::from([0, 1]),
            "the selection stays"
        );
    }

    #[test]
    fn ctrl_toggles_against_the_selection_the_press_found_and_escape_puts_it_back() {
        let mut sv = view(30, Mode::Icons);
        sv.selection.set = BTreeSet::from([0, 5]);
        let l = lay(&sv);
        let mut m = MarqueeDrag::press(&sv, &l, (1, 1), true);
        let c1 = l.cells[1];
        m.motion(&mut sv, &l, (c1.x + 2, c1.y + 2));
        assert_eq!(
            sv.selection.set,
            BTreeSet::from([1, 5]),
            "0 toggled off, 1 on"
        );
        m.cancel(&mut sv);
        assert_eq!(sv.selection.set, BTreeSet::from([0, 5]));
        assert!(sv.marquee.is_none());
    }

    #[test]
    fn past_the_edge_the_view_scrolls_and_the_press_corner_stays_on_its_entry() {
        let mut sv = view(60, Mode::Icons);
        let l = lay(&sv);
        let first = l.cells[0];
        let mut m = MarqueeDrag::press(&sv, &l, (first.x + 1, first.y + 1), false);
        assert_eq!(m.edge(&sv, &l), None, "no band yet");
        m.motion(&mut sv, &l, (BODY.right() - 1, BODY.bottom() + 10));
        assert_eq!(m.edge(&sv, &l), Some(1));
        let before = sv.selection.set.len();
        assert!(m.scroll_step(&mut sv, &l));
        let l = lay(&sv);
        m.follow(&mut sv, &l);
        assert!(
            sv.selection.set.contains(&0),
            "the press's entry stays in the band"
        );
        assert!(
            sv.selection.set.len() > before,
            "the band takes the rows scrolled in"
        );
        let band = sv.marquee.unwrap();
        assert_eq!(
            band.y,
            l.cells[0].y + 1,
            "the corner moved with the content"
        );
    }

    #[test]
    fn list_scrolls_sideways() {
        let mut sv = view(60, Mode::List);
        let l = lay(&sv);
        let mut m = MarqueeDrag::press(&sv, &l, (1, 1), false);
        m.motion(&mut sv, &l, (BODY.right() + 10, 40));
        assert_eq!(m.edge(&sv, &l), Some(1));
        assert!(m.scroll_step(&mut sv, &l));
        assert!(sv.left > 0);
    }
}
