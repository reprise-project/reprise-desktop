use reprisekit::classic::frame::{Chrome, Edges, FrameGeometry};
use smithay::utils::Serial;

use crate::compositor::State;
use crate::raster::Rect;
use crate::theme::Metrics;
use crate::windowrules::{Want, WindowState};

use super::{SizeLimits, Window, WindowId};

/// Edges a drag-resize pins; an xdg client picks its own size, so a LEFT/TOP drag moves the frame
/// once the new size arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeAnchor {
    pub edges: Edges,
    pub right: i32,
    pub bottom: i32,
}

pub fn anchored_position(a: &ResizeAnchor, total: (i32, i32), current: (i32, i32)) -> (i32, i32) {
    let (mut x, mut y) = current;
    if a.edges.contains(Edges::LEFT) {
        x = a.right - total.0;
    }
    if a.edges.contains(Edges::TOP) {
        y = a.bottom - total.1;
    }
    (x, y)
}

/// Whether the anchor is done after the drag: the client committed the last configure sent (`acked`
/// vs `sent`); with none sent (X11), any resize commit is the last.
pub fn anchor_done(sent: Option<Serial>, acked: Option<Serial>, resized: bool) -> bool {
    match (sent, acked) {
        (Some(s), Some(a)) => a >= s,
        (Some(_), None) => false,
        (None, _) => resized,
    }
}

pub fn cascade_position(index: usize, area: Rect, m: &Metrics) -> (i32, i32) {
    let step = m.cascade_step * (index % 10) as i32;
    let x = area.x + m.cascade_origin + step;
    let y = area.y + m.cascade_origin + step;
    let max_y = area.bottom() - m.caption_height - 2 * m.frame_width;
    (x.min(area.right() - 64), y.min(max_y.max(area.y)))
}

pub fn cascade_from(prev: (i32, i32), size: (i32, i32), area: Rect, m: &Metrics) -> (i32, i32) {
    let (x, y) = (prev.0 + m.cascade_step, prev.1 + m.cascade_step);
    if x + size.0 > area.right() || y + size.1 > area.bottom() {
        (area.x + m.cascade_origin, area.y + m.cascade_origin)
    } else {
        (x, y)
    }
}

pub fn keep_inside(origin: (i32, i32), size: (i32, i32), area: Rect) -> (i32, i32) {
    let max_x = (area.right() - size.0).max(area.x);
    let max_y = (area.bottom() - size.1).max(area.y);
    (origin.0.clamp(area.x, max_x), origin.1.clamp(area.y, max_y))
}

pub fn below_top_bar(y: i32, p: &crate::barplace::BarPlacement) -> i32 {
    match p.edge {
        reprisekit::taskbar::Edge::Top => y.max(p.reserved.bottom()),
        _ => y,
    }
}

pub fn rect_below_top_bar(r: Rect, p: &crate::barplace::BarPlacement) -> Rect {
    Rect::new(r.x, below_top_bar(r.y, p), r.width, r.height)
}

pub fn fit_size(size: (i32, i32), area: Rect) -> (i32, i32) {
    (
        size.0.min(area.width).max(1),
        size.1.min(area.height).max(1),
    )
}

/// The largest client a normal window of `chrome` has in `area`: the xdg-toplevel bounds.
pub fn area_client(m: &Metrics, area: Rect, chrome: Chrome) -> (i32, i32) {
    FrameGeometry::client_size_for_total(m, area.width, area.height, false, chrome)
}

/// Client size for a first configure; a CSD client is asked for the whole frame. A normal size
/// fits `area` and goes under the client's `limits` only where they would not fit it.
pub fn first_configure_size(
    m: &Metrics,
    want: &Want,
    limits: SizeLimits,
    screen: Rect,
    area: Rect,
    chrome: Chrome,
) -> Option<(i32, i32)> {
    let resizable = limits.resizable();
    match want.state {
        Some(WindowState::Fullscreen) => Some((screen.width, screen.height)),
        Some(WindowState::Maximized) if resizable => Some(FrameGeometry::client_size_for_total(
            m,
            area.width,
            area.height,
            true,
            chrome,
        )),
        _ => want.size.map(|s| {
            let (w, h) = fit_size(s, area);
            let client = FrameGeometry::client_size_for_total(m, w, h, false, chrome);
            let (cw, ch) = crate::snap::commit_size(client, limits);
            let most = area_client(m, area, chrome);
            (cw.min(most.0), ch.min(most.1))
        }),
    }
}

/// Where a frame that was `was` and is now `now` goes to stay inside `area`: moved as little as
/// keeps it there, when it was inside and grew past an edge. `None` leaves it where it is.
pub fn grown_back_inside(was: Rect, now: Rect, area: Rect) -> Option<(i32, i32)> {
    let inside = |r: Rect| {
        r.x >= area.x && r.y >= area.y && r.right() <= area.right() && r.bottom() <= area.bottom()
    };
    let grew = now.width > was.width || now.height > was.height;
    if area.width <= 0 || !grew || !inside(was) || inside(now) {
        return None;
    }
    let to = keep_inside((now.x, now.y), (now.width, now.height), area);
    (to != (now.x, now.y)).then_some(to)
}

/// Where a frame that was `was` in work area `was_area` and is `now` in `area` after a look or
/// text size change goes: inside `area` as `keep_inside` puts it, when it was wholly on `screen`,
/// its frame or the taskbar grew, and it now leaves the work area. `None` leaves it where it is.
pub fn retheme_back_inside(
    was: Rect,
    now: Rect,
    screen: Rect,
    was_area: Rect,
    area: Rect,
) -> Option<(i32, i32)> {
    let within = |r: Rect, b: Rect| {
        r.x >= b.x && r.y >= b.y && r.right() <= b.right() && r.bottom() <= b.bottom()
    };
    let grew = now.width > was.width || now.height > was.height;
    let shrank = !within(was_area, area);
    if area.width <= 0 || !(grew || shrank) || !within(was, screen) || within(now, area) {
        return None;
    }
    let to = keep_inside((now.x, now.y), (now.width, now.height), area);
    (to != (now.x, now.y)).then_some(to)
}

/// A dialog's frame origin: centred on its owner (or on `area` if ownerless), pushed inside `area`.
pub fn centre_on(parent: Rect, size: (i32, i32), area: Rect) -> (i32, i32) {
    let x = parent.x + (parent.width - size.0) / 2;
    let y = parent.y + (parent.height - size.1) / 2;
    keep_inside((x, y), size, area)
}

impl State {
    pub fn output_box_for_window(&self, w: WindowId) -> Rect {
        let Some(r) = self.win(w).map(Window::frame_rect) else {
            return Rect::default();
        };
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        let b = self.layout_box(out.as_ref());
        if b.width <= 0 {
            return r;
        }
        b
    }

    pub fn draw_scale_for_window(&self, w: WindowId) -> i32 {
        let Some(r) = self.win(w).map(Window::frame_rect) else {
            return 1;
        };
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        self.draw_scale_at(out.as_ref())
    }

    pub fn centre_window(&mut self, w: WindowId, output: Option<&smithay::output::Output>) {
        let Some(win) = self.win(w) else {
            return;
        };
        if win.maximized || win.fullscreen || self.layout_box(output).width <= 0 {
            return;
        }
        self.cancel_interaction_for(w);
        let area = self.usable_area(output);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.snapped = None;
        let r = win.frame_rect();
        let (x, y) = centre_on(area, (r.width, r.height), area);
        self.move_to(w, x, y);
    }

    /// A window that was inside the work area and grew past it on its own (a program growing
    /// with the text size) moves back in as little as it takes; one the user is sizing, or
    /// one already partly outside, stays put.
    pub(super) fn keep_grown_inside(&mut self, w: WindowId, was: Rect) {
        let Some(win) = self.win(w) else {
            return;
        };
        let now = win.frame_rect();
        let user = win.resize_anchor.is_some()
            || win.maximized
            || win.fullscreen
            || self.interaction.is_resize_of(w)
            || self.wm.snap.follows_drag(w);
        let area = self.usable_area_for_window(w);
        if let Some((x, y)) = grown_back_inside(was, now, area).filter(|_| !user) {
            self.move_to(w, x, y);
        }
    }

    /// The screen and work area of the output under `r`'s centre; `None` off every output.
    pub(crate) fn screen_and_area(&self, r: Rect) -> Option<(Rect, Rect)> {
        let out = self.output_at(f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
        let screen = self.layout_box(out.as_ref());
        (screen.width > 0).then(|| (screen, self.usable_area(out.as_ref())))
    }

    /// After a look or text size change re-lays out `w`'s frame (it was `was` in work area
    /// `was_area`), a normal window that was on screen moves back inside the work area if its
    /// frame or the taskbar grew. Maximized and snapped ones refit on their own.
    pub(crate) fn keep_on_screen_inside(&mut self, w: WindowId, was: Rect, was_area: Rect) {
        let Some(win) = self.win(w) else {
            return;
        };
        if win.maximized || win.fullscreen || win.snapped.is_some() {
            return;
        }
        let now = win.frame_rect();
        let Some((screen, area)) = self.screen_and_area(was) else {
            return;
        };
        if let Some((x, y)) = retheme_back_inside(was, now, screen, was_area, area) {
            self.move_to(w, x, y);
        }
    }

    /// Send xdg toplevel `w` the bounds of the work area its frame's centre is on, when they
    /// differ from the ones it last heard.
    pub fn refresh_bounds(&mut self, w: WindowId) {
        if self.set_bounds(w) {
            self.send_configure(w);
        }
    }

    /// Put the bounds `refresh_bounds` sends into `w`'s pending state; true when they changed.
    pub(super) fn set_bounds(&self, w: WindowId) -> bool {
        let Some(win) = self.win(w).filter(|w| w.mapped) else {
            return false;
        };
        let Some(t) = win.xdg() else {
            return false;
        };
        let r = win.frame_rect();
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        let area = self.usable_area(out.as_ref());
        if out.is_none() || area.width <= 0 || area.height <= 0 {
            return false;
        }
        let most = area_client(&self.config.theme.metrics, area, win.chrome());
        t.with_pending_state(|s| {
            let was = s.bounds.replace(most.into());
            was != s.bounds
        })
    }

    pub fn usable_area_for_window(&self, w: WindowId) -> Rect {
        let Some(r) = self.win(w).map(Window::frame_rect) else {
            return Rect::default();
        };
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        if self.layout_box(out.as_ref()).width <= 0 {
            return r;
        }
        self.usable_area(out.as_ref())
    }
}
