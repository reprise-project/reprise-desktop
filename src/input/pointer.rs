use std::time::{Duration, Instant};

use smithay::{
    backend::input::InputTime,
    input::pointer::{CursorImageStatus, MotionEvent},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

use reprisekit::classic::frame::{Edges, FrameHit};
use reprisekit::fluid::Source;

use crate::{
    compositor::{SceneOwner, State},
    taskbar::TaskbarHit,
};

use super::{Hit, Interaction};

/// Minimum spacing of the hover work (`State::repoint`) for a moving
/// pointer, about 120 runs a second; clients still get every motion.
const HOVER_EVERY: Duration = Duration::from_millis(8);

impl State {
    /// The bounding box of every output; a 1x1 box at 0,0 with none.
    pub(crate) fn output_area(&self) -> Rectangle<i32, Logical> {
        self.scene
            .outputs()
            .iter()
            .filter_map(reprisescene::output_rect)
            .reduce(|a, b| a.merge(b))
            .unwrap_or_else(|| Rectangle::from_size((1, 1).into()))
    }

    pub fn pointer_to(&mut self, pos: Point<f64, Logical>, time: InputTime) {
        let from = self.pointer_location();
        let pos = self.clamp_to_layout(pos);
        if !self.motion_only(pos, time) {
            self.repoint(pos, time);
        }
        self.water_pointer(from, pos);
        crate::capture::pointer_moved(self);
        self.hot_corner_motion(pos.x, pos.y);
        self.redraw_pointer();
    }

    /// Move the pointer to `pos` for the client under it and leave the hover
    /// work owed, when it may wait: the pointer is free, keeps its focus and
    /// `repoint` ran under `HOVER_EVERY` ago. False when `repoint` must run
    /// now.
    fn motion_only(&mut self, pos: Point<f64, Logical>, time: InputTime) -> bool {
        let Some(left) = self
            .pointers
            .hovered_at
            .and_then(|t| HOVER_EVERY.checked_sub(t.elapsed()))
        else {
            return false;
        };
        if !self.pointer_free() {
            return false;
        }
        let focus = match self.scene_hit(pos) {
            Hit::Surface {
                surface, origin, ..
            } => Some((surface, origin)),
            _ => None,
        };
        if !self.focus_holds(focus.as_ref().map(|f| &f.0)) {
            return false;
        }
        self.pointer_motion(focus, pos, time);
        self.owe_repoint(time, left);
        true
    }

    /// Whether the seat's pointer focus is `surface` (None: no client).
    /// The seat's own, not `pointers.focus`: a tablet tool or a drag moves
    /// the pointer without `pointer_motion`.
    fn focus_holds(&self, surface: Option<&WlSurface>) -> bool {
        let seat = self.seat.get_pointer().and_then(|p| p.current_focus());
        seat.as_ref().and_then(|f| f.surface()) == surface
    }

    /// Whether the pointer moves nothing but itself: no interaction,
    /// grab, drag, lock, region overlay or Alt+Tab panel holds it.
    pub(crate) fn pointer_free(&self) -> bool {
        matches!(self.interaction, Interaction::Idle)
            && !self.is_locked()
            && !self.drag_running()
            && self.panels.region.is_none()
            && !self.switcher.is_open()
            && self.overview.view.is_none()
            && self.seat.get_pointer().is_some_and(|p| !p.is_grabbed())
    }

    pub(crate) fn owe_repoint(&mut self, time: InputTime, left: Duration) {
        self.pointers.owed = Some(time);
        if self.pointers.settle_timer.is_some() {
            return;
        }
        let timer = Timer::from_duration(left);
        match self.handle.insert_source(timer, |_, _, state| {
            state.pointers.settle_timer = None;
            state.settle_pointer();
            TimeoutAction::Drop
        }) {
            Ok(t) => self.pointers.settle_timer = Some(t),
            Err(e) => {
                log::warn!("pointer settle timer: {}", e.error);
                self.settle_pointer();
            }
        }
    }

    /// Run the hover work a motion left owed (`motion_only`) where the
    /// pointer is now: from its timer, and before a button, wheel, key or
    /// any other input acts on what is hovered.
    pub fn settle_pointer(&mut self) {
        if let Some(t) = self.pointers.settle_timer.take() {
            self.handle.remove(t);
        }
        if let Some(time) = self.pointers.owed.take() {
            self.repoint(self.pointer_location(), time);
        }
    }

    /// What lies under layout point `pos`: a client surface, a window's
    /// frame, the shell's own trees, or the desktop.
    pub fn scene_hit(&self, pos: Point<f64, Logical>) -> Hit {
        let Some(h) = self.scene.node_at(self.scene.root(), pos) else {
            return Hit::Desktop { index: None };
        };
        let window = match self.scene.find_data(h.node).map(|(_, owner)| *owner) {
            Some(SceneOwner::Window(w)) => Some(w),
            Some(SceneOwner::Desktop) => {
                return Hit::Desktop {
                    index: self.desktop_hit(pos.x, pos.y),
                };
            }
            // A node of no one's: the wallpaper, part of the empty desktop.
            None if h.surface.is_none() => return Hit::Desktop { index: None },
            Some(SceneOwner::Taskbar(bar)) => return self.taskbar_scene_hit(bar, pos.x, pos.y),
            Some(SceneOwner::Menu(depth)) => {
                let row = self.menu_row_at(depth, pos.x, pos.y);
                return Hit::Menu { depth, row };
            }
            // A layer surface or one of its popups: no window of ours.
            Some(SceneOwner::Layer) => None,
            // Its buffer lets the pointer through; never reached.
            Some(SceneOwner::Debug) => None,
            Some(SceneOwner::Dialog(index)) => {
                return Hit::Dialog {
                    index,
                    button: self.dialog_hit(index, pos.x, pos.y),
                };
            }
            Some(SceneOwner::Launcher) => {
                return Hit::Launcher {
                    hit: self.launcher_hit(pos.x, pos.y),
                };
            }
            Some(SceneOwner::StartPanel) => {
                return Hit::StartPanel {
                    spot: self.start_panel_hit(pos.x, pos.y),
                };
            }
            Some(SceneOwner::Osd) => return Hit::Osd,
            Some(SceneOwner::Switcher) => {
                return Hit::Switcher {
                    cell: self.switcher_hit(pos.x, pos.y),
                };
            }
            Some(SceneOwner::Overview) => {
                return Hit::Overview {
                    cell: self.overview_hit(pos.x, pos.y),
                };
            }
            Some(SceneOwner::TrayFlyout) => {
                let cell = crate::trayflyout::hit(self, pos.x, pos.y);
                return Hit::TrayFlyout { cell };
            }
            Some(SceneOwner::TaskPreview) => {
                let cell = crate::taskpreview::hit(self, pos.x, pos.y);
                return Hit::TaskPreview { cell };
            }
            Some(SceneOwner::SnapPicker) => {
                let zone = crate::snappickerview::hit(self, pos.x, pos.y);
                return Hit::SnapPicker { zone };
            }
            Some(SceneOwner::Notification) => return Hit::Notification,
            Some(SceneOwner::NotifyCenter) => return Hit::NotifyCenter,
            Some(SceneOwner::ClipView) => {
                return Hit::ClipView {
                    hit: self.clipview_hit(pos.x, pos.y),
                };
            }
            Some(SceneOwner::EmojiView) => {
                return Hit::EmojiView {
                    hit: self.emoji_hit(pos.x, pos.y),
                };
            }
            Some(SceneOwner::Calendar) => {
                let hit = crate::calendarview::hit(self, pos.x, pos.y);
                return Hit::Calendar { hit };
            }
            Some(SceneOwner::Displays) => {
                let hit = crate::displaysview::hit(self, pos.x, pos.y);
                return Hit::Displays { hit };
            }
            None => None,
        };
        if let Some(surface) = h.surface {
            return Hit::Surface {
                surface,
                origin: pos - h.local,
                window,
            };
        }
        match window.and_then(|w| self.win(w)) {
            Some(w) => {
                let mut hit = w.hit_at(pos.x, pos.y, &self.config.theme.metrics);
                if hit == FrameHit::Icon && self.tabs.groups.group_of(w.id).is_some() {
                    hit = FrameHit::Title;
                }
                Hit::Frame { window: w.id, hit }
            }
            None => Hit::Nothing,
        }
    }

    pub(crate) fn pointer_motion(
        &mut self,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        pos: Point<f64, Logical>,
        time: InputTime,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // Nothing new for the client: the same surface and origin at the
        // same spot, as `settle_pointer` finds after `motion_only` sent it.
        if focus == self.pointers.focus
            && pos == pointer.current_location()
            && !pointer.is_grabbed()
            && self.focus_holds(focus.as_ref().map(|f| &f.0))
        {
            return;
        }
        self.pointers.focus.clone_from(&focus);
        // Under a grab Reprise's own drawing is a target too: the grab an
        // X11 program's drag runs in offers it the drag there (`xdnd`).
        // Every other grab keeps its own focus.
        let focus = match focus {
            Some((s, o)) => Some((s.into(), o)),
            None if pointer.is_grabbed() => Some((crate::xdnd::PointerFocus::Reprise, pos)),
            None => None,
        };
        pointer.motion(
            self,
            focus,
            &MotionEvent {
                location: pos,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    /// Take the pointer from whatever client has it, where it is.
    pub(crate) fn clear_pointer_focus(&mut self) {
        let pos = self.pointer_location();
        self.pointer_motion(None, pos, InputTime::now());
    }

    /// Show the named cursor (`default`, `move`, an edge's resize arrow).
    pub fn set_cursor(&mut self, name: &str) {
        let icon = name.parse().unwrap_or_default();
        let named = CursorImageStatus::Named(icon);
        // A capture keeps the pointer hidden.
        if self.cursor != named && !self.inputproto.captured() {
            self.cursor = named;
            self.redraw_cursor();
        }
    }

    /// The pointer moved to `pos`: carry on the interaction it is in, or
    /// hand it to what lies under it.
    pub fn repoint(&mut self, pos: Point<f64, Logical>, time: InputTime) {
        self.pointers.hovered_at = Some(Instant::now());
        self.pointers.owed = None;
        if self.is_locked() {
            crate::lockview::motion(self, pos, time);
            return;
        }
        crate::autohideview::poke(self);
        if self.drag_running() {
            self.drag_motion(pos, time);
            return;
        }
        // The region overlay takes the pointer from everything.
        if self.panels.region.is_some() {
            self.pointer_motion(None, pos, time);
            self.region_motion(pos.x, pos.y);
            return;
        }
        match self.interaction {
            Interaction::Move {
                window,
                mut dx,
                mut dy,
                press,
                ..
            } => {
                self.pointer_motion(None, pos, time);
                let (cx, cy) = (pos.x, pos.y);
                let slop = f64::from(crate::desktop::DRAG_PX);
                let held = (cx - press.0).abs() < slop && (cy - press.1).abs() < slop;
                let grouped = self.wm.snap.group_of(window).is_some();
                if held && (self.fitted(window) || grouped) {
                    return;
                }
                // Moved off its zone for real, not jittered: out of its
                // group.
                if !held {
                    self.wm.snap.leave_group(window);
                }
                if let Some((ndx, ndy)) = self.unfit_for_move(window, (cx, cy), press) {
                    dx = ndx;
                    dy = ndy;
                    if let Interaction::Move {
                        dx: idx, dy: idy, ..
                    } = &mut self.interaction
                    {
                        *idx = ndx;
                        *idy = ndy;
                    }
                }
                let mut y = (cy - dy) as i32;
                if let Some(o) = self.output_at(cx, cy) {
                    y = crate::window::below_top_bar(y, &self.bar_placement(&o));
                }
                if let Some(old) = self.win(window).map(|w| w.frame_rect()) {
                    self.water_edge_moved(old);
                }
                self.drag_to(window, (cx - dx) as i32, y);
                crate::shakeview::motion(self, window, pos);
                return;
            }
            Interaction::Resize {
                window,
                mut edges,
                start,
                cx,
                cy,
                keyboard,
            } => {
                self.pointer_motion(None, pos, time);
                if keyboard.is_some() && edges == Edges::NONE {
                    // A keyboard size with no side yet: the first pointer
                    // move picks the side it heads for.
                    let Some(e) = crate::keyboard::edge_for_motion(pos.x - cx, pos.y - cy) else {
                        return;
                    };
                    edges = e;
                    self.set_resize_edges(e);
                }
                self.drag_resize(window, edges, start, cx, cy);
                return;
            }
            Interaction::PressButton { window, button } => {
                self.pointer_motion(None, pos, time);
                // The held button pops up while the pointer is off it,
                // and no other caption button lights up until the
                // release.
                let Some(hit) = self
                    .win(window)
                    .map(|w| w.hit_at(pos.x, pos.y, &self.config.theme.metrics))
                else {
                    return;
                };
                let held = reprisekit::frame::held_button(hit, button);
                self.frame_hover(
                    Some(window),
                    held.map_or(FrameHit::Outside, FrameHit::Button),
                );
                if self.win(window).is_some_and(|w| w.pressed != held) {
                    if let Some(w) = self.win_mut(window) {
                        w.pressed = held;
                    }
                    self.redraw_frame(window);
                }
                return;
            }
            Interaction::TabPress { .. } => {
                self.pointer_motion(None, pos, time);
                self.tab_press_moved(pos.x, pos.y);
                return;
            }
            Interaction::TaskPress { .. }
            | Interaction::QuickPress { .. }
            | Interaction::ArrowPress { .. }
            | Interaction::Band { .. } => {
                self.pointer_motion(None, pos, time);
                self.taskbar_press_moved();
                return;
            }
            Interaction::BarMove { .. } | Interaction::BarWidth { .. } => {
                self.pointer_motion(None, pos, time);
                self.bar_drag_moved();
                return;
            }
            Interaction::Drag { .. } => {
                self.pointer_motion(None, pos, time);
                self.drag_view_motion(pos);
                return;
            }
            Interaction::Marquee => {
                self.pointer_motion(None, pos, time);
                self.drag_marquee(pos.x, pos.y);
                return;
            }
            Interaction::Stir => {
                self.pointer_motion(None, pos, time);
                self.stir_move(Source::Pointer, pos);
                return;
            }
            Interaction::FieldDrag { editor } => {
                self.pointer_motion(None, pos, time);
                match editor {
                    crate::caret::Editor::Rename => self.drag_rename(pos.x, pos.y),
                    crate::caret::Editor::Dialog => self.dialog_drag_field(pos.x, pos.y),
                    // The launcher's and the panels' fields take no drag.
                    crate::caret::Editor::Launcher
                    | crate::caret::Editor::StartPanel
                    | crate::caret::Editor::ClipView
                    | crate::caret::Editor::EmojiView
                    | crate::caret::Editor::NotifyReply => {}
                }
                return;
            }
            Interaction::DialogMove { index, dx, dy } => {
                self.pointer_motion(None, pos, time);
                self.move_dialog(index, (pos.x - dx) as i32, (pos.y - dy) as i32);
                return;
            }
            Interaction::DialogResize {
                index,
                edges,
                start,
                cx,
                cy,
            } => {
                self.pointer_motion(None, pos, time);
                let (dx, dy) = ((pos.x - cx) as i32, (pos.y - cy) as i32);
                self.size_dialog(index, edges, start, dx, dy);
                return;
            }
            Interaction::DialogPress => {
                self.pointer_motion(None, pos, time);
                self.dialog_drag_button();
                return;
            }
            Interaction::NotifyPress => {
                self.pointer_motion(None, pos, time);
                crate::notifyview::drag(self, pos.x, pos.y);
                crate::notifycenter::drag(self, pos.x, pos.y);
                return;
            }
            Interaction::DisplaysSlide => {
                self.pointer_motion(None, pos, time);
                crate::displaysview::drag(self, pos.x);
                return;
            }
            Interaction::Idle => {}
        }
        let hit = self.scene_hit(pos);
        // Off the taskbar, the dialogs, the desktop's icons, the caption
        // buttons and the Glass Start menu's spots the tip goes, and none is owed.
        if !matches!(
            hit,
            Hit::Taskbar { .. }
                | Hit::Dialog { .. }
                | Hit::Desktop { index: Some(_) }
                | Hit::Frame {
                    hit: FrameHit::Button(_),
                    ..
                }
                | Hit::StartPanel { spot: Some(_) }
        ) {
            crate::tooltip::hide(self);
        }
        if !matches!(hit, Hit::Frame { .. }) {
            self.frame_hover(None, FrameHit::Outside);
        }
        // The snap layout picker stays while the pointer crosses from the
        // maximize button onto it.
        if !matches!(hit, Hit::Frame { .. } | Hit::SnapPicker { .. }) {
            crate::snappickerview::leave(self);
        }
        let over_caption = match hit {
            Hit::Frame {
                window,
                hit: FrameHit::Title,
            } => Some(window),
            _ => None,
        };
        self.tab_hover(over_caption, pos.x, pos.y);
        // The preview panel stays while the pointer crosses from its
        // button onto it.
        if !matches!(hit, Hit::Taskbar { .. } | Hit::TaskPreview { .. }) {
            crate::taskpreview::hide(self);
        }
        if !matches!(hit, Hit::Taskbar { .. }) {
            self.taskbar_hover(None, TaskbarHit::Nothing);
        }
        if !matches!(hit, Hit::Menu { .. }) {
            self.clear_menu_hover();
        }
        if !matches!(hit, Hit::Dialog { .. }) {
            self.dialog_hover_caption(None);
            self.dialog_hover(None);
        }
        if !matches!(hit, Hit::Switcher { .. }) {
            self.switcher_hover(None);
        }
        if !matches!(hit, Hit::Overview { .. }) {
            self.overview_hover(None);
        }
        if !matches!(hit, Hit::StartPanel { .. }) {
            self.start_panel_pointer(None);
        }
        crate::notifyview::hover(self, pos.x, pos.y, matches!(hit, Hit::Notification));
        let flyout_cell = match hit {
            Hit::TrayFlyout { cell } => cell,
            _ => None,
        };
        crate::trayflyout::hover(self, flyout_cell);
        let calendar_hit = match hit {
            Hit::Calendar { hit } => Some(hit),
            _ => None,
        };
        crate::calendarview::hover(self, calendar_hit);
        let displays_hit = match hit {
            Hit::Displays { hit } => Some(hit),
            _ => None,
        };
        crate::displaysview::hover(self, displays_hit);
        // The Alt+Tab panel owns the pointer while it is up: what lies
        // around it gets neither focus nor hover.
        let owned = (self.switcher.is_open() && !matches!(hit, Hit::Switcher { .. }))
            || (self.overview.view.is_some() && !matches!(hit, Hit::Overview { .. }));
        if owned {
            self.pointer_motion(None, pos, time);
            self.set_cursor("default");
            return;
        }
        match hit {
            Hit::Surface {
                surface,
                origin,
                window,
            } => {
                if let Some(w) = window {
                    self.input_reached(w);
                }
                // Entering a different surface drops the cursor we set over
                // a frame; the client sets its own on enter.
                let changed = self
                    .seat
                    .get_pointer()
                    .and_then(|p| p.current_focus())
                    .is_none_or(|f| f != surface);
                if changed {
                    self.set_cursor("default");
                }
                self.pointer_motion(Some((surface, origin)), pos, time);
            }
            Hit::Frame { window, hit } => {
                // On the way to its close button, say.
                self.input_reached(window);
                self.pointer_motion(None, pos, time);
                let name = match hit {
                    FrameHit::Resize(e) => e.cursor_name(),
                    _ => "default",
                };
                self.set_cursor(name);
                self.frame_hover(Some(window), hit);
                crate::snappickerview::hover_frame(self, window, hit);
                crate::tooltip::hover_caption(self, window, hit);
            }
            Hit::SnapPicker { zone } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
                crate::snappickerview::hover_panel(self, zone);
            }
            Hit::Taskbar { bar, hit } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor(if hit == TaskbarHit::Resize {
                    "ew-resize"
                } else {
                    "default"
                });
                crate::tooltip::hover(self, bar, hit);
                crate::taskpreview::hover(self, bar, hit);
                self.taskbar_hover(Some(bar), hit);
            }
            Hit::TaskPreview { cell } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
                crate::taskpreview::hover_panel(self, cell);
            }
            Hit::Menu { depth, row } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
                self.menu_hover(depth, row);
            }
            Hit::Dialog { index, button } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor(crate::dialogview::cursor_over(button));
                let b = match button {
                    Some(crate::dialog::DialogHit::Button(i)) => Some(i),
                    _ => None,
                };
                crate::tooltip::hover_dialog(self, index, b);
                let caption = crate::dialog::caption_button(button);
                self.dialog_hover_caption(Some((index, caption)));
                self.dialog_hover(Some((index, button)));
            }
            Hit::Switcher { cell } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
                self.switcher_hover(cell);
            }
            Hit::Overview { cell } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
                self.overview_hover(cell);
            }
            Hit::StartPanel { spot } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
                self.start_panel_pointer(spot);
            }
            Hit::Desktop { index } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
                crate::tooltip::hover_desktop(self, index);
            }
            Hit::Nothing
            | Hit::Launcher { .. }
            | Hit::ClipView { .. }
            | Hit::EmojiView { .. }
            | Hit::Osd
            | Hit::TrayFlyout { .. }
            | Hit::Notification
            | Hit::NotifyCenter
            | Hit::Calendar { .. }
            | Hit::Displays { .. } => {
                self.pointer_motion(None, pos, time);
                self.set_cursor("default");
            }
        }
    }
}
