use smithay::backend::input::InputTime;

use reprisekit::classic::frame::{CaptionButton, Edges, FrameGeometry};
use reprisekit::fluid::Source;

use crate::{
    compositor::State,
    raster::Rect,
    window::{ResizeAnchor, SizeLimits, WindowId},
};

use super::{Interaction, MoveStart};

impl State {
    /// Move `w` with the pointer; `title` when a title bar press (the
    /// frame's, a client's own caption or a torn-off tab) started it, which
    /// lets the snap bar show.
    pub fn begin_move(&mut self, w: WindowId, title: bool) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.mapped {
            return;
        }
        let (x, y, snapped) = (win.x, win.y, win.snapped);
        let start = MoveStart {
            rect: win.frame_rect(),
            maximized: win.maximized,
            restore: win.saved,
        };
        let free = !win.maximized && !win.fullscreen;
        self.tabs.forget_torn();
        if self.interaction != Interaction::Idle {
            self.end_interaction();
        }
        let pos = self.pointer_location();
        let (cx, cy) = (pos.x, pos.y);
        let press = if self.buttons_down > 0 {
            self.press_at
        } else {
            (cx, cy)
        };
        let bar = title && free && self.buttons_down > 0;
        log::debug!(
            target: "reprise::snapbarview",
            "begin_move {w:?}: bar={bar} (title={title} free={free} buttons_down={})",
            self.buttons_down
        );
        // A snapped, maximized or fullscreen window keeps its rect until
        // `repoint` sees the cursor leave the press, so a click leaves it.
        self.interaction = Interaction::Move {
            window: w,
            dx: cx - x as f64,
            dy: cy - y as f64,
            origin: None,
            was_snapped: snapped,
            start,
            press,
            snap: None,
            armed: false,
            bar,
        };
        self.wm.snap.begin_move(w);
        crate::shakeview::begin(self, w, title && self.buttons_down > 0);
        self.clear_pointer_focus();
        self.set_cursor("move");
    }

    /// The window a move drags once the pointer has left the press (and,
    /// for a fitted window, the slop around it): a bare click on a
    /// caption has no drop to track.
    pub(crate) fn moving_window(&self) -> Option<WindowId> {
        let Interaction::Move { window, press, .. } = self.interaction else {
            return None;
        };
        let pos = self.pointer_location();
        let slop = f64::from(crate::desktop::DRAG_PX);
        let held = (pos.x - press.0).abs() < slop && (pos.y - press.1).abs() < slop;
        let still = (pos.x, pos.y) == press || (held && self.fitted(window));
        (!still).then_some(window)
    }

    /// Whether `w` fills a rect it leaves when a drag of it starts: a
    /// snap zone, the usable area or the output.
    pub(super) fn fitted(&self, w: WindowId) -> bool {
        self.win(w)
            .is_some_and(|w| w.snapped.is_some() || w.maximized || w.fullscreen)
    }

    /// Give a snapped, maximized or fullscreen `w` its size from before back
    /// once a move has really started, placed so the pointer at (`cx`, `cy`)
    /// holds it where the press at `press` did (`snap::drag_out_grab`).
    /// Returns the move's new grab offset, or None when `w` was in none of
    /// those states.
    pub(super) fn unfit_for_move(
        &mut self,
        w: WindowId,
        (cx, cy): (f64, f64),
        press: (f64, f64),
    ) -> Option<(f64, f64)> {
        let win = self.win(w)?;
        let cur = win.frame_rect();
        let r = if win.maximized || win.fullscreen {
            let saved = if win.maximized {
                win.saved
            } else {
                win.saved_fullscreen
            };
            // Leaving either state cancels the move; it goes on from
            // the new offset returned below.
            let mv = self.interaction;
            self.set_fullscreen(w, false);
            self.set_maximized(w, false);
            self.interaction = mv;
            saved.unwrap_or(cur)
        } else {
            self.win_mut(w)?.snapped.take()?
        };
        let (gx, gy) = crate::snap::drag_out_grab(cur, r, press);
        let m = &self.config.theme.metrics;
        let chrome = self.win(w)?.chrome();
        let (cw, ch) = FrameGeometry::client_size_for_total(m, r.width, r.height, false, chrome);
        self.move_to(w, (cx - gx) as i32, (cy - gy) as i32);
        // A stale anchor from an earlier drag-resize would re-derive the
        // position from the old edges on the client's commit.
        self.win_mut(w)?.resize_anchor = None;
        self.request_size(w, cw, ch);
        let win = self.win(w)?;
        Some((cx - win.x as f64, cy - win.y as f64))
    }

    pub fn begin_resize(&mut self, w: WindowId, edges: Edges) {
        self.start_resize(w, edges, false);
    }

    pub(crate) fn start_resize(&mut self, w: WindowId, edges: Edges, keyboard: bool) {
        if self.interaction != Interaction::Idle {
            self.end_interaction();
        }
        let pos = self.pointer_location();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let start = win.frame_rect();
        win.resize_anchor = Some(ResizeAnchor {
            edges,
            right: start.right(),
            bottom: start.bottom(),
        });
        self.interaction = Interaction::Resize {
            window: w,
            edges,
            start,
            cx: pos.x,
            cy: pos.y,
            keyboard: keyboard.then_some(start),
        };
        self.set_resizing(w, true);
        // A drag of an edge shared within a snap group sizes the neighbours
        // along it; a keyboard size brings them in as its arrows pick sides.
        if !keyboard {
            self.begin_group_resize(w, edges, start);
        }
        self.clear_pointer_focus();
        // A keyboard size has no side yet; show the move cursor until
        // an arrow picks one.
        self.set_cursor(if keyboard {
            "move"
        } else {
            edges.cursor_name()
        });
    }

    pub fn end_interaction(&mut self) {
        self.settle_moves();
        if let Interaction::Move { window, .. } = self.interaction
            && self.finish_tab_drop(window)
        {
            self.wm.snap.end_move();
            self.hide_snap_outline();
            crate::snapbarview::end(self);
            self.interaction = Interaction::Idle;
            self.set_cursor("default");
            return;
        }
        match self.interaction {
            Interaction::Move {
                window,
                snap,
                start,
                was_snapped,
                ..
            } => {
                self.end_move(window, snap, start.restore_rect(was_snapped));
                self.remember_file_dialog(window);
            }
            Interaction::Resize { window, .. } => {
                // `resize_anchor` deliberately survives the release: the
                // client's commit for the last requested size still has to
                // be anchored. `handle_commit` drops it once that lands.
                self.set_resizing(window, false);
                self.end_group_resize();
            }
            Interaction::TabPress { .. } => self.tab_press_released(),
            Interaction::PressButton { window, button } => {
                if let Some(w) = self.win_mut(window) {
                    w.pressed = None;
                }
                self.redraw_frame(window);
                self.redraw_window(window);
                let pos = self.pointer_location();
                let hit = self
                    .win(window)
                    .map(|w| w.hit_at(pos.x, pos.y, &self.config.theme.metrics));
                if hit.is_some_and(|h| reprisekit::frame::held_button(h, button).is_some()) {
                    self.caption_action(window, button);
                }
            }
            Interaction::TaskPress { .. }
            | Interaction::QuickPress { .. }
            | Interaction::ArrowPress { .. }
            | Interaction::Band { .. } => {
                self.end_taskbar_press();
            }
            Interaction::BarMove { .. } | Interaction::BarWidth { .. } => {
                let was = std::mem::take(&mut self.interaction);
                self.end_bar_drag(was);
            }
            Interaction::Drag { .. } => self.drag_finish(),
            Interaction::Marquee => self.end_marquee(),
            Interaction::Stir => {
                self.end_marquee();
                self.stir_up(Source::Pointer);
            }
            Interaction::FieldDrag {
                editor: crate::caret::Editor::Dialog,
            } => self.dialog_release(),
            Interaction::FieldDrag {
                editor:
                    crate::caret::Editor::Launcher
                    | crate::caret::Editor::StartPanel
                    | crate::caret::Editor::Rename
                    | crate::caret::Editor::ClipView
                    | crate::caret::Editor::EmojiView
                    | crate::caret::Editor::NotifyReply,
            } => {}
            Interaction::DialogPress => self.dialog_release_button(),
            Interaction::NotifyPress => {
                let pos = self.pointer_location();
                crate::notifyview::release(self, pos.x, pos.y);
                crate::notifycenter::release(self, pos.x, pos.y);
            }
            Interaction::DisplaysSlide => crate::displaysview::release(self),
            Interaction::DialogMove { index, .. } | Interaction::DialogResize { index, .. } => {
                self.remember_browse(index)
            }
            Interaction::Idle => {}
        }
        self.interaction = Interaction::Idle;
        self.set_cursor("default");
    }

    /// Something took the pointer away mid-gesture, or Escape ended a
    /// keyboard move or size. What the gesture already changed on screen
    /// stays (a moved window is not snapped, a held button draws released
    /// and does not act); a keyboard move or size goes back where it
    /// started, snap included. Pointer focus and cursor are worked out
    /// again at once.
    pub fn cancel_interaction(&mut self) {
        self.settle_moves();
        let was = std::mem::take(&mut self.interaction);
        match was {
            Interaction::Idle => return,
            Interaction::Move { .. } => {
                self.end_tab_drop();
                self.hide_snap_outline();
                crate::snapbarview::end(self);
            }
            Interaction::TabPress { window, .. } => self.tab_press_cancelled(window),
            Interaction::Resize {
                window, keyboard, ..
            } => {
                self.set_resizing(window, false);
                if keyboard.is_some() {
                    self.cancel_group_resize();
                } else {
                    self.end_group_resize();
                }
            }
            Interaction::PressButton { window, .. } => {
                if let Some(w) = self.win_mut(window) {
                    w.pressed = None;
                }
                self.redraw_frame(window);
            }
            Interaction::TaskPress { .. }
            | Interaction::QuickPress { .. }
            | Interaction::ArrowPress { .. }
            | Interaction::Band { .. } => self.cancel_taskbar_press(was),
            Interaction::BarMove { .. } | Interaction::BarWidth { .. } => self.cancel_bar_drag(),
            Interaction::Drag { .. } => self.drag_view_cancel(),
            Interaction::Marquee => self.end_marquee(),
            Interaction::Stir => {
                self.end_marquee();
                self.stir_cancel(Source::Pointer);
            }
            // A dragged or sized dialog stays as it is.
            Interaction::DialogMove { index, .. } | Interaction::DialogResize { index, .. } => {
                self.remember_browse(index)
            }
            Interaction::FieldDrag {
                editor: crate::caret::Editor::Dialog,
            }
            | Interaction::DialogPress => self.dialog_cancel_press(),
            Interaction::NotifyPress => {
                crate::notifyview::cancel_press(self);
                crate::notifycenter::cancel_press(self);
            }
            Interaction::DisplaysSlide => crate::displaysview::release(self),
            Interaction::FieldDrag { .. } => {}
        }
        self.restore_keyboard(was);
        self.set_cursor("default");
        let pos = self.pointer_location();
        self.repoint(pos, InputTime::now());
    }

    /// Escape during a pointer move undoes it: the window goes back to its
    /// rect, maximized state, pre-snap size and snap group and zone from the
    /// start of the drag; the outline and snap bar go and nothing drops. A
    /// torn-off tab goes back to its tab.
    pub fn cancel_move(&mut self) {
        let Interaction::Move {
            window,
            start,
            origin: None,
            ..
        } = self.interaction
        else {
            return;
        };
        // Put back as a keyboard move's Escape puts back its origin and
        // its snap group (`restore_keyboard`).
        if let Interaction::Move { origin, .. } = &mut self.interaction {
            *origin = Some(start.rect);
        }
        self.cancel_interaction();
        self.untear(window);
        if start.maximized && self.win(window).is_some_and(|w| !w.maximized) {
            let kept = self.win(window).and_then(|w| w.snapped);
            self.set_maximized(window, true);
            if let Some(win) = self.win_mut(window)
                && win.maximized
            {
                win.saved = start.restore.or(win.saved);
                win.snapped = kept;
            }
        }
    }

    pub fn caption_action(&mut self, w: WindowId, button: CaptionButton) {
        match button {
            CaptionButton::Minimize => self.set_minimized(w, true),
            CaptionButton::Maximize => {
                let maximized = self.win(w).is_some_and(|w| w.maximized);
                self.set_maximized(w, !maximized);
            }
            CaptionButton::Close => self.close(w),
        }
    }

    pub(super) fn drag_resize(&mut self, w: WindowId, edges: Edges, start: Rect, cx: f64, cy: f64) {
        let pos = self.pointer_location();
        let dx = (pos.x - cx) as i32;
        let dy = (pos.y - cy) as i32;
        let Some(win) = self.win(w) else {
            return;
        };
        let limits = win.size_limits();
        let pad = (
            win.geom.total.0 - win.geom.client.width,
            win.geom.total.1 - win.geom.client.height,
        );
        let mut r = resize_rect(start, edges, dx, dy, limits, pad);
        if let Some(o) = self.output_at(pos.x, pos.y) {
            r = top_below_top_bar(r, edges, &self.bar_placement(&o));
        }
        let r = self.hold_for_partners(r);
        let Some(win) = self.win(w) else {
            return;
        };
        let m = &self.config.theme.metrics;
        let (cw, ch) =
            FrameGeometry::client_size_for_total(m, r.width, r.height, false, win.chrome());
        // The client answers with a size of its own choosing, so a
        // LEFT/TOP drag cannot move the frame yet: `handle_commit` places it
        // from `resize_anchor` once the new size lands.
        let anchored = edges.contains(Edges::LEFT) || edges.contains(Edges::TOP);
        if !anchored {
            self.move_to(w, r.x, r.y);
        }
        self.request_size(w, cw, ch);
        self.drag_partners(r);
    }
}

/// Resized frame `r` with a dragged top edge held at or below a top
/// bar's foot, its bottom where it was; any other drag or edge leaves it
/// alone.
pub(super) fn top_below_top_bar(r: Rect, edges: Edges, p: &crate::barplace::BarPlacement) -> Rect {
    if !edges.contains(Edges::TOP) {
        return r;
    }
    let y = crate::window::below_top_bar(r.y, p);
    Rect::new(r.x, y, r.width, r.bottom() - y)
}

/// Pure resize math: the new frame rect for a frame `start`, dragged on
/// `edges` by (`dx`, `dy`), under the client's size limits and `pad`, what
/// the frame adds to the client's size. The edges not dragged stay anchored,
/// including after a min/max clamp. Windows and sizable dialogs both use it.
pub(crate) fn resize_rect(
    start: Rect,
    edges: Edges,
    dx: i32,
    dy: i32,
    limits: SizeLimits,
    pad: (i32, i32),
) -> Rect {
    let min_total_w = limits.min.0 + pad.0;
    let min_total_h = limits.min.1 + pad.1;
    let mut r = start;
    if edges.contains(Edges::LEFT) {
        r.x += dx;
        r.width -= dx;
    } else if edges.contains(Edges::RIGHT) {
        r.width += dx;
    }
    if edges.contains(Edges::TOP) {
        r.y += dy;
        r.height -= dy;
    } else if edges.contains(Edges::BOTTOM) {
        r.height += dy;
    }
    if r.width < min_total_w {
        if edges.contains(Edges::LEFT) {
            r.x = start.right() - min_total_w;
        }
        r.width = min_total_w;
    }
    if r.height < min_total_h {
        if edges.contains(Edges::TOP) {
            r.y = start.bottom() - min_total_h;
        }
        r.height = min_total_h;
    }
    if let Some(max_w) = limits.max.0 {
        r.width = r.width.min(max_w + pad.0);
    }
    if let Some(max_h) = limits.max.1 {
        r.height = r.height.min(max_h + pad.1);
    }
    // Re-anchor the fixed edge from the final clamped size.
    if edges.contains(Edges::LEFT) {
        r.x = start.right() - r.width;
    }
    if edges.contains(Edges::TOP) {
        r.y = start.bottom() - r.height;
    }
    r
}
