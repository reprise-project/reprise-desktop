//! Where the taskbar sits on its output (`[taskbar] position`): the strip it draws
//! in (`shown`), the strip it keeps windows off (`reserved`), what is left
//! (`usable`), and where popups from the bar go (`bar_popup`). With
//! `[taskbar] auto_hide` on (`place_hiding`), `reserved` is the reveal line and
//! `shown` slides toward the edge.

use reprisekit::taskbar::{Edge, REVEAL_LINE, StartMenuDock, TaskbarShape};

use crate::raster::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarPlacement {
    pub edge: Edge,
    pub thickness: i32,
    pub shown: Rect,
    pub reserved: Rect,
}

impl BarPlacement {
    pub fn screen_side(&self) -> i32 {
        match self.edge {
            Edge::Bottom => self.shown.y,
            Edge::Top => self.shown.bottom(),
            Edge::Left => self.shown.right(),
            Edge::Right => self.shown.x,
        }
    }

    pub fn anchor_point(&self, r: Rect) -> (i32, i32) {
        if self.edge.is_side() {
            (self.screen_side(), r.y)
        } else {
            (r.x, self.screen_side())
        }
    }

    pub fn click_point(&self, cell: Rect) -> (i32, i32) {
        if self.edge.is_side() {
            (self.screen_side(), cell.y + cell.height / 2)
        } else {
            (cell.x + cell.width / 2, self.screen_side())
        }
    }

    /// Whether the bar is out of sight but for its reveal line (an auto-hidden bar
    /// headed out); false with auto-hide off.
    pub fn is_hidden(&self) -> bool {
        match self.edge {
            Edge::Bottom => self.shown.bottom() > self.reserved.bottom(),
            Edge::Top => self.shown.y < self.reserved.y,
            Edge::Left => self.shown.x < self.reserved.x,
            Edge::Right => self.shown.right() > self.reserved.right(),
        }
    }

    pub fn start_dock(&self, dock: StartMenuDock) -> (StartMenuDock, Rect) {
        if self.is_hidden() {
            (StartMenuDock::OnTop, self.reserved)
        } else {
            (dock, self.shown)
        }
    }
}

/// The bar `thickness` px across along `edge` of `output`, never larger than the
/// output. An empty `output` gives empty strips.
pub fn place(output: Rect, edge: Edge, thickness: i32) -> BarPlacement {
    let across = if edge.is_side() {
        output.width
    } else {
        output.height
    };
    let t = if output.is_empty() {
        0
    } else {
        thickness.clamp(0, across)
    };
    let strip = match edge {
        Edge::Bottom => Rect::new(output.x, output.bottom() - t, output.width.max(0), t),
        Edge::Top => Rect::new(output.x, output.y, output.width.max(0), t),
        Edge::Left => Rect::new(output.x, output.y, t, output.height.max(0)),
        Edge::Right => Rect::new(output.right() - t, output.y, t, output.height.max(0)),
    };
    BarPlacement {
        edge,
        thickness,
        shown: strip,
        reserved: strip,
    }
}

pub fn thickness(
    edge: Edge,
    m: &crate::theme::Metrics,
    shape: &TaskbarShape,
    side_width: i32,
    output: Rect,
) -> i32 {
    if edge.is_side() {
        shape.side_width(m, side_width, output.width)
    } else {
        m.taskbar_height
    }
}

/// The edge of `output` whose diagonal triangle holds (`x`, `y`); on a diagonal
/// the row edge wins (Bottom, then Top).
pub fn edge_for(output: Rect, x: f64, y: f64) -> Edge {
    let (w, h) = (f64::from(output.width), f64::from(output.height));
    let (dx, dy) = (x - f64::from(output.x), y - f64::from(output.y));
    [
        (Edge::Bottom, (h - dy) * w),
        (Edge::Top, dy * w),
        (Edge::Left, dx * h),
        (Edge::Right, (w - dx) * h),
    ]
    .into_iter()
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map_or(Edge::Bottom, |(e, _)| e)
}

pub fn depth(edge: Edge, output: Rect, x: i32, y: i32) -> i32 {
    match edge {
        Edge::Bottom => output.bottom() - y,
        Edge::Top => y - output.y,
        Edge::Left => x - output.x,
        Edge::Right => output.right() - x,
    }
}

/// The width a side bar takes with its inner edge dragged to `at`, held `grab` px
/// from the pointer: one slot to a third of the output ([`TaskbarShape::side_width`]).
/// Never 0 or less, which would read as the look's default.
pub fn dragged_width(
    edge: Edge,
    m: &crate::theme::Metrics,
    shape: &TaskbarShape,
    output: Rect,
    at: (i32, i32),
    grab: i32,
) -> i32 {
    let wanted = (depth(edge, output, at.0, at.1) + grab).max(1);
    shape.side_width(m, wanted, output.width)
}

/// `place` with `[taskbar] auto_hide`: `hide` is None with it off, else the slide, 0.0 shown to 1.0 hidden.
pub fn place_hiding(output: Rect, edge: Edge, thickness: i32, hide: Option<f32>) -> BarPlacement {
    let p = place(output, edge, thickness);
    let Some(hide) = hide else {
        return p;
    };
    let s = p.shown;
    let t = if edge.is_side() { s.width } else { s.height };
    let line = t.min(REVEAL_LINE);
    let slid = (hide.clamp(0.0, 1.0) * (t - line) as f32).round() as i32;
    let (shown, reserved) = match edge {
        Edge::Bottom => (
            Rect::new(s.x, s.y + slid, s.width, t),
            Rect::new(s.x, s.bottom() - line, s.width, line),
        ),
        Edge::Top => (
            Rect::new(s.x, s.y - slid, s.width, t),
            Rect::new(s.x, s.y, s.width, line),
        ),
        Edge::Left => (
            Rect::new(s.x - slid, s.y, t, s.height),
            Rect::new(s.x, s.y, line, s.height),
        ),
        Edge::Right => (
            Rect::new(s.x + slid, s.y, t, s.height),
            Rect::new(s.right() - line, s.y, line, s.height),
        ),
    };
    BarPlacement {
        shown,
        reserved,
        ..p
    }
}

pub fn place_resizing(
    output: Rect,
    edge: Edge,
    kept: i32,
    live: i32,
    hide: Option<f32>,
) -> BarPlacement {
    BarPlacement {
        reserved: place_hiding(output, edge, kept, hide).reserved,
        ..place_hiding(output, edge, live, hide)
    }
}

pub fn on_output(p: &BarPlacement, output: Rect) -> Rect {
    let r = p.shown.intersect(&output);
    if r.is_empty() {
        return Rect::default();
    }
    Rect::new(r.x - p.shown.x, r.y - p.shown.y, r.width, r.height)
}

pub fn clear_of(area: Rect, p: &BarPlacement) -> Rect {
    match p.edge {
        Edge::Bottom => {
            let bottom = area.bottom().min(p.shown.y).max(area.y);
            Rect::new(area.x, area.y, area.width, bottom - area.y)
        }
        Edge::Top => {
            let top = area.y.max(p.shown.bottom()).min(area.bottom());
            Rect::new(area.x, top, area.width, area.bottom() - top)
        }
        Edge::Left => {
            let left = area.x.max(p.shown.right()).min(area.right());
            Rect::new(left, area.y, area.right() - left, area.height)
        }
        Edge::Right => {
            let right = area.right().min(p.shown.x).max(area.x);
            Rect::new(area.x, area.y, right - area.x, area.height)
        }
    }
}

pub fn orb_opacity(hidden: f32) -> Option<f32> {
    (hidden < 1.0).then(|| 1.0 - hidden.clamp(0.0, 1.0))
}

pub fn usable(full: Rect, p: &BarPlacement) -> Rect {
    let (across, reserved) = if p.edge.is_side() {
        (full.width, p.reserved.width)
    } else {
        (full.height, p.reserved.height)
    };
    let t = reserved.clamp(0, across.max(0));
    let rest = (across - t).max(0);
    match p.edge {
        Edge::Bottom => Rect::new(full.x, full.y, full.width, rest),
        Edge::Top => Rect::new(full.x, full.y + t, full.width, rest),
        Edge::Left => Rect::new(full.x + t, full.y, rest, full.height),
        Edge::Right => Rect::new(full.x, full.y, rest, full.height),
    }
}

/// The left edge `x` of a Start menu or panel `w` wide beside side bar strip
/// `bar`, kept inside `area` and `bar` together; overflow moves back over the bar.
pub fn beside_on_output(x: i32, w: i32, bar: Rect, area: Rect) -> i32 {
    let (left, right) = (area.x.min(bar.x), area.right().max(bar.right()));
    x.min(right - w).max(left)
}

/// Top-left of a popup `size` big opening from `anchor` on the screen side of
/// bar `p`, pushed back inside `bounds`; one taller than `bounds` keeps its top.
pub fn bar_popup(p: &BarPlacement, anchor: Rect, size: (i32, i32), bounds: Rect) -> (i32, i32) {
    let (w, h) = size;
    let side = p.screen_side();
    let (x, y) = match p.edge {
        Edge::Bottom => (anchor.x, side - h),
        Edge::Top => (anchor.x, side),
        Edge::Left => (side, anchor.y),
        Edge::Right => (side - w, anchor.y),
    };
    (
        x.min(bounds.right() - w).max(bounds.x),
        y.min(bounds.bottom() - h).max(bounds.y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 800, 600);

    #[test]
    fn a_point_picks_the_edge_whose_triangle_holds_it() {
        let out = Rect::new(100, 50, 800, 600);
        assert_eq!(edge_for(out, 500.0, 640.0), Edge::Bottom);
        assert_eq!(edge_for(out, 500.0, 60.0), Edge::Top);
        assert_eq!(edge_for(out, 110.0, 350.0), Edge::Left);
        assert_eq!(edge_for(out, 890.0, 350.0), Edge::Right);
        assert_eq!(
            edge_for(out, 500.0, 400.0),
            Edge::Bottom,
            "deep in its triangle"
        );
        assert_eq!(edge_for(out, 181.0, 110.0), Edge::Top);
        assert_eq!(edge_for(out, 179.0, 110.0), Edge::Left);
        assert_eq!(
            edge_for(out, 180.0, 110.0),
            Edge::Top,
            "on a diagonal the row edge"
        );
        assert_eq!(
            edge_for(out, 820.0, 590.0),
            Edge::Bottom,
            "on the bottom-right one too"
        );
        assert_eq!(
            edge_for(out, 500.0, 350.0),
            Edge::Bottom,
            "the centre: Bottom"
        );
    }

    #[test]
    fn a_dragged_inner_edge_sets_the_width_from_a_slot_to_a_third() {
        use reprisekit::taskbar::TaskbarShape;
        let (m, c) = (crate::theme::Metrics::default(), TaskbarShape::CLASSIC);
        let out = Rect::new(100, 0, 1920, 1080);
        assert_eq!(depth(Edge::Left, out, 160, 500), 60);
        assert_eq!(depth(Edge::Right, out, 1960, 500), 60);
        assert_eq!(depth(Edge::Bottom, out, 500, 1052), 28);
        assert_eq!(depth(Edge::Top, out, 500, 28), 28);
        let grab = 60 - depth(Edge::Left, out, 159, 500);
        assert_eq!(
            dragged_width(Edge::Left, &m, &c, out, (249, 500), grab),
            150
        );
        assert_eq!(dragged_width(Edge::Right, &m, &c, out, (1870, 500), 0), 150);
        assert_eq!(
            dragged_width(Edge::Left, &m, &c, out, (101, 500), 0),
            28,
            "never under a slot"
        );
        assert_eq!(
            dragged_width(Edge::Left, &m, &c, out, (100, 500), 0),
            28,
            "at the outer edge a slot, not the look's default"
        );
        assert_eq!(dragged_width(Edge::Left, &m, &c, out, (60, 500), -5), 28);
        assert_eq!(
            dragged_width(Edge::Left, &m, &c, out, (1500, 500), 0),
            640,
            "never past a third"
        );
    }

    #[test]
    fn a_width_drag_draws_the_bar_live_and_keeps_its_reserved_strip() {
        let out = Rect::new(0, 0, 1920, 1080);
        let r = place_resizing(out, Edge::Right, 60, 150, None);
        assert_eq!(r.shown, Rect::new(1770, 0, 150, 1080));
        assert_eq!(r.reserved, Rect::new(1860, 0, 60, 1080));
        assert_eq!(r.thickness, 150, "laid out at the live width");
        assert_eq!(usable(out, &r), usable(out, &place(out, Edge::Right, 60)));
        let l = place_resizing(out, Edge::Left, 60, 150, None);
        assert_eq!(
            (l.shown, l.reserved),
            (Rect::new(0, 0, 150, 1080), Rect::new(0, 0, 60, 1080))
        );
        let h = place_resizing(out, Edge::Left, 60, 150, Some(0.0));
        assert_eq!(
            (h.shown, h.reserved),
            (
                Rect::new(0, 0, 150, 1080),
                Rect::new(0, 0, REVEAL_LINE, 1080)
            ),
            "auto-hidden: the reveal line whatever the width"
        );
        assert_eq!(
            place_resizing(out, Edge::Left, 60, 60, None),
            place(out, Edge::Left, 60)
        );
    }

    #[test]
    fn a_side_bar_takes_its_side_width_clamped_and_a_top_or_bottom_bar_its_height() {
        use reprisekit::taskbar::TaskbarShape;
        let m = crate::theme::Metrics::default();
        let c = TaskbarShape::CLASSIC;
        let out = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            thickness(Edge::Bottom, &m, &c, 300, out),
            28,
            "the bar's height"
        );
        assert_eq!(thickness(Edge::Top, &m, &c, 0, out), 28);
        assert_eq!(
            thickness(Edge::Left, &m, &c, 0, out),
            60,
            "Classic's own width"
        );
        assert_eq!(thickness(Edge::Right, &m, &c, 120, out), 120);
        assert_eq!(
            thickness(Edge::Left, &m, &c, 5, out),
            28,
            "never under a slot"
        );
        assert_eq!(
            thickness(Edge::Left, &m, &c, 900, out),
            640,
            "never past a third"
        );
        let p = place(out, Edge::Left, thickness(Edge::Left, &m, &c, 0, out));
        assert_eq!(p.shown, Rect::new(0, 0, 60, 1080));
    }

    #[test]
    fn with_auto_hide_off_the_bar_is_placed_as_before() {
        for edge in Edge::ALL {
            assert_eq!(
                place_hiding(SCREEN, edge, 30, None),
                place(SCREEN, edge, 30),
                "{edge:?}"
            );
        }
    }

    #[test]
    fn an_auto_hidden_bar_reserves_its_reveal_line_wherever_it_slides() {
        for hide in [0.0, 0.25, 0.5, 1.0] {
            let b = place_hiding(SCREEN, Edge::Bottom, 30, Some(hide));
            assert_eq!(b.reserved, Rect::new(0, 598, 800, 2), "bottom at {hide}");
            assert_eq!(
                usable(SCREEN, &b),
                Rect::new(0, 0, 800, 598),
                "bottom at {hide}"
            );
            let t = place_hiding(SCREEN, Edge::Top, 30, Some(hide));
            assert_eq!(t.reserved, Rect::new(0, 0, 800, 2), "top at {hide}");
            assert_eq!(
                usable(SCREEN, &t),
                Rect::new(0, 2, 800, 598),
                "top at {hide}"
            );
        }
    }

    #[test]
    fn an_auto_hidden_bar_slides_toward_its_edge_until_its_reveal_line_is_left() {
        let at = |edge, hide| place_hiding(SCREEN, edge, 30, Some(hide)).shown;
        assert_eq!(
            at(Edge::Bottom, 0.0),
            Rect::new(0, 570, 800, 30),
            "shown, where it always sits"
        );
        assert_eq!(at(Edge::Bottom, 0.5), Rect::new(0, 584, 800, 30));
        assert_eq!(
            at(Edge::Bottom, 1.0),
            Rect::new(0, 598, 800, 30),
            "hidden, 2 rows stay on"
        );
        assert_eq!(at(Edge::Top, 0.0), Rect::new(0, 0, 800, 30));
        assert_eq!(at(Edge::Top, 0.5), Rect::new(0, -14, 800, 30));
        assert_eq!(at(Edge::Top, 1.0), Rect::new(0, -28, 800, 30));
    }

    #[test]
    fn only_the_bars_rows_on_its_output_show() {
        use reprisekit::taskbar::{TaskbarCounts, TaskbarLayout, TaskbarShape};
        let m = crate::theme::Metrics {
            taskbar_height: 30,
            ..Default::default()
        };
        for edge in [Edge::Bottom, Edge::Top] {
            let whole = Rect::new(0, 0, 800, 30);
            assert_eq!(
                on_output(&place(SCREEN, edge, 30), SCREEN),
                whole,
                "{edge:?}: auto-hide off"
            );
            let shown = place_hiding(SCREEN, edge, 30, Some(0.0));
            assert_eq!(
                on_output(&shown, SCREEN),
                whole,
                "{edge:?}: shown, all of it"
            );
            let l = TaskbarLayout::compute(
                &m,
                &TaskbarShape::CLASSIC,
                800,
                true,
                TaskbarCounts::default(),
                40,
            )
            .on_edge(edge);
            let hidden = place_hiding(SCREEN, edge, 30, Some(1.0));
            assert_eq!(
                on_output(&hidden, SCREEN),
                l.reveal_line(),
                "{edge:?}: hidden, its reveal line"
            );
        }
        let half = place_hiding(SCREEN, Edge::Bottom, 30, Some(0.5));
        assert_eq!(on_output(&half, SCREEN), Rect::new(0, 0, 800, 16));
        let half = place_hiding(SCREEN, Edge::Top, 30, Some(0.5));
        assert_eq!(on_output(&half, SCREEN), Rect::new(0, 14, 800, 16));
        assert_eq!(
            on_output(&half, Rect::new(800, 0, 800, 600)),
            Rect::default(),
            "off its output"
        );
    }

    #[test]
    fn a_balloon_keeps_clear_of_a_shown_auto_hidden_bar() {
        for edge in [Edge::Bottom, Edge::Top] {
            let p = place(SCREEN, edge, 30);
            let area = usable(SCREEN, &p);
            assert_eq!(
                clear_of(area, &p),
                area,
                "{edge:?}: a bar reserving its strip changes nothing"
            );
            let panel = Rect::new(0, 40, 800, 500);
            assert_eq!(
                clear_of(panel, &p),
                panel,
                "{edge:?}: nor with a panel between"
            );
        }
        let shown = place_hiding(SCREEN, Edge::Bottom, 30, Some(0.0));
        assert_eq!(
            clear_of(usable(SCREEN, &shown), &shown),
            Rect::new(0, 0, 800, 570)
        );
        let hidden = place_hiding(SCREEN, Edge::Bottom, 30, Some(1.0));
        assert_eq!(
            clear_of(usable(SCREEN, &hidden), &hidden),
            Rect::new(0, 0, 800, 598)
        );
        let shown = place_hiding(SCREEN, Edge::Top, 30, Some(0.0));
        assert_eq!(
            clear_of(usable(SCREEN, &shown), &shown),
            Rect::new(0, 30, 800, 570)
        );
    }

    #[test]
    fn the_orb_fades_with_the_slide_and_is_gone_once_the_bar_is_hidden() {
        assert_eq!(orb_opacity(0.0), Some(1.0), "shown, the orb is whole");
        assert_eq!(orb_opacity(0.25), Some(0.75));
        assert_eq!(orb_opacity(1.0), None, "hidden, no orb hangs past the line");
    }

    #[test]
    fn a_bar_is_a_strip_along_its_edge() {
        let p = place(SCREEN, Edge::Bottom, 30);
        assert_eq!(p.shown, Rect::new(0, 570, 800, 30));
        assert_eq!(p.reserved, p.shown);
        assert_eq!(p.thickness, 30);
        assert_eq!(p.screen_side(), 570, "a bottom bar's top edge");
        let p = place(SCREEN, Edge::Top, 30);
        assert_eq!(p.shown, Rect::new(0, 0, 800, 30));
        assert_eq!(p.reserved, p.shown);
        assert_eq!(p.screen_side(), 30, "a top bar's bottom edge");
        let b = Rect::new(800, 40, 1280, 720);
        assert_eq!(
            place(b, Edge::Bottom, 30).shown,
            Rect::new(800, 730, 1280, 30)
        );
        assert_eq!(place(b, Edge::Top, 30).shown, Rect::new(800, 40, 1280, 30));
    }

    #[test]
    fn an_output_out_of_the_layout_has_an_empty_bar_at_its_edge() {
        for edge in Edge::ALL {
            let p = place(Rect::default(), edge, 30);
            assert_eq!(p.shown, Rect::new(0, 0, 0, 0), "{edge:?}");
            assert!(p.reserved.is_empty(), "{edge:?}");
            assert_eq!(usable(Rect::default(), &p), Rect::default(), "{edge:?}");
        }
    }

    #[test]
    fn the_usable_area_is_what_the_bar_leaves() {
        let b = Rect::new(1280, 40, 800, 600);
        assert_eq!(
            usable(b, &place(b, Edge::Bottom, 30)),
            Rect::new(1280, 40, 800, 570)
        );
        assert_eq!(
            usable(b, &place(b, Edge::Top, 30)),
            Rect::new(1280, 70, 800, 570)
        );
        let short = Rect::new(0, 0, 640, 20);
        assert_eq!(
            usable(short, &place(short, Edge::Bottom, 30)),
            Rect::new(0, 0, 640, 0)
        );
        assert_eq!(
            usable(short, &place(short, Edge::Top, 30)),
            Rect::new(0, 20, 640, 0)
        );
    }

    #[test]
    fn a_popup_opens_above_a_bottom_bar() {
        let p = place(SCREEN, Edge::Bottom, 30);
        let usable = usable(SCREEN, &p);
        let icon = Rect::new(100, 577, 16, 16);
        assert_eq!(bar_popup(&p, icon, (180, 100), usable), (100, 470));
        let icon = Rect::new(750, 577, 16, 16);
        assert_eq!(bar_popup(&p, icon, (180, 100), usable), (620, 470));
        assert_eq!(bar_popup(&p, icon, (180, 900), usable), (620, 0));
    }

    #[test]
    fn a_popup_opens_below_a_top_bar() {
        let p = place(SCREEN, Edge::Top, 30);
        let usable = usable(SCREEN, &p);
        let icon = Rect::new(100, 7, 16, 16);
        assert_eq!(bar_popup(&p, icon, (180, 100), usable), (100, 30));
        let icon = Rect::new(750, 7, 16, 16);
        assert_eq!(bar_popup(&p, icon, (180, 100), usable), (620, 30));
        assert_eq!(bar_popup(&p, icon, (180, 900), usable), (620, 30));
    }

    #[test]
    fn a_popup_stays_on_its_own_output_at_the_left_corner() {
        let b = Rect::new(800, 0, 800, 600);
        for (edge, y) in [(Edge::Bottom, 470), (Edge::Top, 30)] {
            let p = place(b, edge, 30);
            let anchor = Rect::new(790, p.shown.y + 7, 16, 16);
            assert_eq!(
                bar_popup(&p, anchor, (180, 100), usable(b, &p)),
                (800, y),
                "{edge:?}"
            );
        }
    }

    #[test]
    fn a_bottom_bar_popup_matches_the_old_tray_menu_position() {
        fn old(icon: Rect, top: i32, size: (i32, i32), usable: Rect) -> (i32, i32) {
            let x = icon.x.min(usable.right() - size.0).max(usable.x);
            (x, (top - size.1).max(usable.y))
        }
        let p = place(SCREEN, Edge::Bottom, 30);
        let usable = usable(SCREEN, &p);
        for icon_x in [0, 100, 620, 750, 799] {
            for size in [(180, 100), (180, 570), (180, 900), (40, 20)] {
                let icon = Rect::new(icon_x, 577, 16, 16);
                assert_eq!(
                    bar_popup(&p, icon, size, usable),
                    old(icon, p.shown.y, size, usable),
                    "icon at {icon_x}, popup {size:?}"
                );
            }
        }
    }

    #[test]
    fn a_popup_rises_above_a_panel_between_the_windows_and_a_bottom_bar() {
        let p = place(SCREEN, Edge::Bottom, 30);
        let bounds = Rect::new(0, 0, 800, 546);
        let icon = Rect::new(100, 577, 16, 16);
        assert_eq!(bar_popup(&p, icon, (180, 100), bounds), (100, 446));
    }

    #[test]
    fn a_bar_hears_clicks_on_its_screen_side_edge_level_with_the_cell() {
        let b = place(SCREEN, Edge::Bottom, 30);
        let cell = Rect::new(700, 577, 16, 16);
        assert_eq!(
            (b.click_point(cell), b.anchor_point(cell)),
            ((708, 570), (700, 570))
        );
        let t = place(SCREEN, Edge::Top, 30);
        let cell = Rect::new(700, 7, 16, 16);
        assert_eq!(
            (t.click_point(cell), t.anchor_point(cell)),
            ((708, 30), (700, 30))
        );
    }

    #[test]
    fn a_side_bar_is_a_column_along_its_edge() {
        let l = place(SCREEN, Edge::Left, 60);
        assert_eq!(l.shown, Rect::new(0, 0, 60, 600));
        assert_eq!(l.reserved, l.shown);
        assert_eq!(l.screen_side(), 60, "a left bar's right edge");
        assert_eq!(usable(SCREEN, &l), Rect::new(60, 0, 740, 600));
        let r = place(SCREEN, Edge::Right, 60);
        assert_eq!(r.shown, Rect::new(740, 0, 60, 600));
        assert_eq!(r.screen_side(), 740, "a right bar's left edge");
        assert_eq!(usable(SCREEN, &r), Rect::new(0, 0, 740, 600));
        let b = Rect::new(800, 40, 1280, 720);
        assert_eq!(place(b, Edge::Left, 60).shown, Rect::new(800, 40, 60, 720));
        assert_eq!(
            place(b, Edge::Right, 60).shown,
            Rect::new(2020, 40, 60, 720)
        );
        let thin = Rect::new(0, 0, 40, 600);
        assert_eq!(
            usable(thin, &place(thin, Edge::Left, 60)),
            Rect::new(40, 0, 0, 600)
        );
    }

    #[test]
    fn an_auto_hidden_side_bar_slides_sideways_and_reserves_its_reveal_line() {
        for hide in [0.0, 0.5, 1.0] {
            let l = place_hiding(SCREEN, Edge::Left, 60, Some(hide));
            assert_eq!(l.reserved, Rect::new(0, 0, 2, 600), "left at {hide}");
            assert_eq!(
                usable(SCREEN, &l),
                Rect::new(2, 0, 798, 600),
                "left at {hide}"
            );
            let r = place_hiding(SCREEN, Edge::Right, 60, Some(hide));
            assert_eq!(r.reserved, Rect::new(798, 0, 2, 600), "right at {hide}");
            assert_eq!(
                usable(SCREEN, &r),
                Rect::new(0, 0, 798, 600),
                "right at {hide}"
            );
        }
        let at = |edge, hide| place_hiding(SCREEN, edge, 60, Some(hide));
        assert_eq!(at(Edge::Left, 0.5).shown, Rect::new(-29, 0, 60, 600));
        assert_eq!(at(Edge::Left, 1.0).shown, Rect::new(-58, 0, 60, 600));
        assert_eq!(at(Edge::Right, 1.0).shown, Rect::new(798, 0, 60, 600));
        assert!(at(Edge::Left, 1.0).is_hidden() && !at(Edge::Left, 0.0).is_hidden());
        assert!(at(Edge::Right, 1.0).is_hidden() && !at(Edge::Right, 0.0).is_hidden());
        assert_eq!(
            on_output(&at(Edge::Left, 1.0), SCREEN),
            Rect::new(58, 0, 2, 600),
            "hidden, its two inner columns"
        );
        assert_eq!(
            on_output(&at(Edge::Right, 1.0), SCREEN),
            Rect::new(0, 0, 2, 600)
        );
        let hidden = at(Edge::Left, 1.0);
        assert_eq!(
            hidden.start_dock(StartMenuDock::Into(4)),
            (StartMenuDock::OnTop, hidden.reserved),
            "a hidden side bar's menu stands on its line"
        );
    }

    #[test]
    fn a_popup_opens_beside_a_side_bar() {
        let l = place(SCREEN, Edge::Left, 60);
        let area = usable(SCREEN, &l);
        let icon = Rect::new(22, 500, 16, 16);
        assert_eq!(
            bar_popup(&l, icon, (180, 60), area),
            (60, 500),
            "right of the bar, its top at the icon's"
        );
        assert_eq!(
            bar_popup(&l, icon, (180, 300), area),
            (60, 300),
            "kept above the output's foot"
        );
        assert_eq!(bar_popup(&l, icon, (180, 900), area), (60, 0));
        let r = place(SCREEN, Edge::Right, 60);
        let area = usable(SCREEN, &r);
        let icon = Rect::new(762, 500, 16, 16);
        assert_eq!(
            bar_popup(&r, icon, (180, 60), area),
            (560, 500),
            "left of the bar"
        );
    }

    #[test]
    fn a_balloon_keeps_clear_of_a_shown_auto_hidden_side_bar() {
        let l = place_hiding(SCREEN, Edge::Left, 60, Some(0.0));
        assert_eq!(clear_of(usable(SCREEN, &l), &l), Rect::new(60, 0, 740, 600));
        let r = place_hiding(SCREEN, Edge::Right, 60, Some(0.0));
        assert_eq!(clear_of(usable(SCREEN, &r), &r), Rect::new(0, 0, 740, 600));
        let hidden = place_hiding(SCREEN, Edge::Left, 60, Some(1.0));
        assert_eq!(
            clear_of(usable(SCREEN, &hidden), &hidden),
            Rect::new(2, 0, 798, 600)
        );
    }

    #[test]
    fn a_side_bar_hears_clicks_on_its_screen_side_edge_level_with_the_cell() {
        let l = place(SCREEN, Edge::Left, 60);
        let cell = Rect::new(22, 530, 16, 16);
        assert_eq!(
            (l.click_point(cell), l.anchor_point(cell)),
            ((60, 538), (60, 530))
        );
        let r = place(SCREEN, Edge::Right, 60);
        let cell = Rect::new(762, 530, 16, 16);
        assert_eq!(
            (r.click_point(cell), r.anchor_point(cell)),
            ((740, 538), (740, 530))
        );
    }
}
