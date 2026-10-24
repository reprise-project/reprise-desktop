use reprisekit::classic::frame::FrameGeometry;
use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::input::Interaction;
use crate::raster::Rect;

use super::{WindowId, rect_below_top_bar};

impl State {
    pub fn caption_clear(&self, r: Rect) -> Rect {
        self.output_at(f64::from(r.x + r.width / 2), f64::from(r.y))
            .map_or(r, |o| rect_below_top_bar(r, &self.bar_placement(&o)))
    }

    /// Put the frame at a saved rect's origin and return the client size that rect holds under the
    /// current chrome.
    pub(super) fn restore_to(&mut self, w: WindowId, r: Rect) -> (i32, i32) {
        let m = self.config.theme.metrics.clone();
        let r = self.caption_clear(r);
        let Some(win) = self.win_mut(w) else {
            return (r.width, r.height);
        };
        win.x = r.x;
        win.y = r.y;
        FrameGeometry::client_size_for_total(&m, r.width, r.height, false, win.chrome())
    }

    pub fn relayout(&mut self, w: WindowId) {
        let m = self.config.theme.metrics.clone();
        let opens = reprisekit::look::client_glass(&self.config.theme, true).is_some();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let (cw, ch) = win.client_size();
        win.client_w = cw;
        win.client_h = ch;
        let glass_top = match opens {
            true => crate::clientglass::glass_top(&win.blur, cw),
            false => 0,
        };
        win.geom = FrameGeometry::layout(&m, cw, ch, win.maximized, win.chrome())
            .with_glass_top(glass_top);
        let (tree, content, surface, frame, on) = (
            win.tree,
            win.content,
            win.surface,
            win.frame,
            win.has_frame(),
        );
        let (x, y, client) = (win.x, win.y, win.geom.client);
        let (gx, gy) = win.geometry_origin();
        self.scene.set_position(tree, x, y);
        for sb in frame {
            self.scene.set_enabled(sb, on);
        }
        self.scene.set_position(content, client.x, client.y);
        self.scene.set_position(surface, -gx, -gy);
        self.sync_popups(w);
        self.redraw_frame(w);
        // X11 clients track their own absolute position, so every placement
        // change has to be sent back to them.
        self.place_x11(w);
        self.flip_window_changed(w);
    }

    /// Tell an X11 client where its client rect starts, keeping the size its X window was last
    /// given (a pending `request_size` must not be undone).
    pub(crate) fn place_x11(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let Some(x) = win.x11().filter(|_| !win.override_redirect) else {
            return;
        };
        let size = x.last_configure().size;
        self.configure_x11(w, size.w, size.h);
    }

    fn configure_x11(&self, w: WindowId, cw: i32, ch: i32) {
        let Some(win) = self.win(w) else {
            return;
        };
        let Some(x) = win.x11().filter(|_| !win.override_redirect) else {
            return;
        };
        let (cx, cy) = win.client_origin();
        let (gx, gy, gw, gh) = x11_geometry(cx, cy, cw, ch);
        let rect = Rectangle::new(
            (i32::from(gx), i32::from(gy)).into(),
            (i32::from(gw), i32::from(gh)).into(),
        );
        if rect != x.last_configure()
            && let Err(e) = x.configure(rect)
        {
            log::debug!("x11 configure: {e}");
        }
    }

    pub fn move_to(&mut self, w: WindowId, x: i32, y: i32) {
        self.drag_to(w, x, y);
        self.settle_move(w);
    }

    /// Move the tree only; `settle_move` does the rest before the next render. Called per pointer
    /// motion.
    pub fn drag_to(&mut self, w: WindowId, x: i32, y: i32) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.x = x;
        win.y = y;
        win.move_pending = true;
        let tree = win.tree;
        let outputs = self.scene.move_on_outputs(tree, x, y);
        crate::backend::schedule_on(self, &outputs, crate::backend::Ask::Scene);
    }

    pub fn settle_move(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if !std::mem::take(&mut win.move_pending) {
            return;
        }
        self.refit_scale(w);
        self.refresh_bounds(w);
        self.place_ime_popup();
        self.place_x11(w);
        self.follow_tabs(w, false);
    }

    /// Finish moves `drag_to` left pending; for a dragged window, track caption and snap targets
    /// and untile clients that left a zone.
    pub fn settle_moves(&mut self) {
        let dragged = match self.interaction {
            Interaction::Move { window, .. }
                if self.win(window).is_some_and(|w| w.move_pending) =>
            {
                Some(window)
            }
            _ => None,
        };
        let pending: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.move_pending)
            .map(|w| w.id)
            .collect();
        for w in pending {
            self.settle_move(w);
        }
        let frame = dragged.and_then(|w| self.win(w)).map(|w| w.frame_rect());
        self.water_edge_settle(frame);
        if let Some(w) = dragged {
            self.track_move(w);
        }
        self.sync_tiled();
    }

    pub fn track_move(&mut self, w: WindowId) {
        if crate::shakeview::shaking(self, w) {
            crate::shakeview::hold_drops(self);
            return;
        }
        // Held over a caption, the window joins its group on release and snaps nowhere.
        if self.track_tab_drop(w) {
            log::debug!(target: "reprise::snapbarview", "motion {w:?}: over a caption, no bar");
            self.hide_snap_outline();
            crate::snapbarview::end(self);
            if let Interaction::Move { snap, .. } = &mut self.interaction {
                *snap = None;
            }
        } else {
            self.track_snap(w);
        }
    }

    /// Ask `w` for a client size; a normal window is never asked for less than its minimum or more
    /// than its maximum.
    pub fn request_size(&mut self, w: WindowId, cw: i32, ch: i32) {
        let (cw, ch) = match self.win(w) {
            Some(x) if !x.maximized && !x.fullscreen => {
                crate::snap::commit_size((cw, ch), x.size_limits())
            }
            _ => (cw, ch),
        };
        if self.win(w).is_some_and(|x| x.x11().is_some()) {
            self.configure_x11(w, cw, ch);
            return;
        }
        if let Some(win) = self.win(w)
            && let Some(t) = win.xdg()
        {
            t.with_pending_state(|s| s.size = Some((cw.max(1), ch.max(1)).into()));
            let sent = (t.is_initial_configure_sent())
                .then(|| t.send_pending_configure())
                .flatten();
            if let Some(serial) = sent
                && let Some(win) = self.win_mut(w)
            {
                win.size_serial = Some(serial);
            }
        }
    }
}

/// A layout rect as X carries it: 16-bit origin, size >= 1, saturated.
pub(crate) fn x11_geometry(x: i32, y: i32, w: i32, h: i32) -> (i16, i16, u16, u16) {
    let pos = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let len = |v: i32| v.clamp(1, u16::MAX as i32) as u16;
    (pos(x), pos(y), len(w), len(h))
}

/// Clamp a client size to 1..=32767 per side so sums cannot overflow `i32`; wp_viewport lets a
/// client claim any size.
pub(super) fn client_extent(w: i32, h: i32) -> (i32, i32) {
    let len = |v: i32| v.clamp(1, i16::MAX as i32);
    (len(w), len(h))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeLimits {
    pub min: (i32, i32),
    pub max: (Option<i32>, Option<i32>),
}

impl SizeLimits {
    pub fn resizable(self) -> bool {
        self.max != (Some(self.min.0), Some(self.min.1))
    }
}

/// From (min_w, min_h, max_w, max_h); a max <= 0 is unbounded as in xdg-shell.
pub(crate) fn client_limits(limits: (i32, i32, i32, i32)) -> SizeLimits {
    let (min_w, min_h, max_w, max_h) = limits;
    let max = |v: i32| (v > 0).then(|| v.min(i16::MAX as i32));
    SizeLimits {
        min: client_extent(min_w, min_h),
        max: (max(max_w), max(max_h)),
    }
}
