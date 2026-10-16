//! Input events from any backend: the keyboard goes to the focused window
//! (Ctrl+Alt+Fn switches VT first; keys go through `keyboard`), the pointer
//! moves over the outputs, a press focuses and raises the window under it,
//! and a press on a frame moves, sizes or works its caption buttons.

use std::time::{Duration, Instant};

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        InputTime, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

use reprisekit::classic::frame::{CaptionButton, Edges, FrameGeometry, FrameHit};
use reprisekit::pointer::{Click, DoubleClick};

use reprisescene::TreeId;

use crate::{
    compositor::{SceneOwner, State},
    keys,
    menuview::MenuRoot,
    raster::Rect,
    taskbar::{TaskKey, TaskbarHit},
    window::{ResizeAnchor, SizeLimits, WindowId},
};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

/// Minimum spacing of the hover work (`State::repoint`) for a moving
/// pointer, about 120 runs a second; clients still get every motion.
const HOVER_EVERY: Duration = Duration::from_millis(8);

/// What is under the cursor.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    /// A window's tree whose window is gone.
    Nothing,
    /// The desktop: an icon's index, or None for empty desktop (the
    /// wallpaper or no node at all).
    Desktop {
        index: Option<usize>,
    },
    /// A client surface, its origin in layout coordinates, and the window
    /// it belongs to.
    Surface {
        surface: WlSurface,
        origin: Point<f64, Logical>,
        window: Option<WindowId>,
    },
    Frame {
        window: WindowId,
        hit: FrameHit,
    },
    /// A taskbar, named by its tree (`Bar::tree`), and what on it.
    Taskbar {
        bar: TreeId,
        hit: TaskbarHit,
    },
    /// The open menu at `depth` of `State::menus`, and the row under the
    /// pointer (None on a separator or the border).
    Menu {
        depth: usize,
        row: Option<usize>,
    },
    /// Dialog `index` of `State::dialogs` and what the point lands on.
    Dialog {
        index: usize,
        button: Option<crate::dialog::DialogHit>,
    },
    Launcher {
        hit: reprisekit::controls::launcher::Hit,
    },
    /// The Glass Start menu and the spot the point lands on.
    StartPanel {
        spot: Option<reprisekit::startpanel::Spot>,
    },
    Osd,
    /// The Alt+Tab panel and the entry whose cell the point lands on.
    Switcher {
        cell: Option<usize>,
    },
    Overview {
        cell: Option<usize>,
    },
    /// The tray's hidden icons flyout and the icon the point lands on.
    TrayFlyout {
        cell: Option<usize>,
    },
    TaskPreview {
        cell: Option<usize>,
    },
    /// The snap layout picker and the zone the point lands on.
    SnapPicker {
        zone: Option<reprisekit::controls::snappicker::Zone>,
    },
    /// A notification balloon; `notifyview::press` finds its part.
    Notification,
    /// The notification history popup; see `notifycenter::press`.
    NotifyCenter,
    ClipView {
        hit: reprisekit::controls::cliphistory::Hit,
    },
    EmojiView {
        hit: reprisekit::controls::emojipicker::Hit,
    },
    Calendar {
        hit: reprisekit::controls::calendar::PopupHit,
    },
    Displays {
        hit: reprisekit::controls::displaypanel::Hit,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveStart {
    pub rect: Rect,
    pub maximized: bool,
    /// Where a restore from maximized goes (`Window.saved`).
    pub restore: Option<Rect>,
}

impl MoveStart {
    /// Where a restore puts back a window this drag dropped into a zone or
    /// maximized: its size from before a snap, else the rect a restore from
    /// maximized went to, else the rect the drag found. Never the rect at
    /// the release, which an edge drop can leave mostly off screen.
    pub fn restore_rect(&self, was_snapped: Option<Rect>) -> Rect {
        let maximized = self.restore.filter(|_| self.maximized);
        was_snapped.or(maximized).unwrap_or(self.rect)
    }
}

/// A pointer-driven interaction in progress: dragging a title bar or a
/// resize edge, holding down a caption button, or a press or drag on the
/// taskbar, the desktop or a dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Interaction {
    #[default]
    Idle,
    Move {
        window: WindowId,
        dx: f64,
        dy: f64,
        /// Started from the window menu (no button held): the frame rect to
        /// put back on Escape. A button press or Enter commits instead of a
        /// release, and the arrow keys nudge the cursor.
        origin: Option<Rect>,
        /// What `Window.snapped` held when the drag started, so Escape on
        /// a keyboard move can put the snap back along with `origin`.
        was_snapped: Option<Rect>,
        /// The window as the drag found it: Escape on a pointer move puts
        /// it back (`cancel_move`).
        start: MoveStart,
        /// Cursor at the press. A snapped, maximized or fullscreen window
        /// stays as it is until the cursor is `desktop::DRAG_PX` away from
        /// this point, so a bare click on its title bar (or the first click
        /// of a double-click) changes nothing.
        press: (f64, f64),
        /// The drop the pointer is over (`snap::target_at`), made on release.
        snap: Option<crate::snap::Target>,
        /// The pointer has been outside every edge band since the move
        /// began (`snap::armed_target`); no drop until it has.
        armed: bool,
        /// A title-bar press on a window that was neither maximized nor
        /// fullscreen started it: the snap bar may show.
        bar: bool,
    },
    Resize {
        window: WindowId,
        edges: Edges,
        start: Rect,
        cx: f64,
        cy: f64,
        /// Started from the window menu: the frame rect Escape puts back,
        /// as `Move::origin`. `edges` stays NONE until the first arrow key
        /// or pointer move picks a side, and `start` follows each new side.
        keyboard: Option<Rect>,
    },
    PressButton {
        window: WindowId,
        button: CaptionButton,
    },
    /// A caption tab held down (`tabview`): `member`'s tab on `window`'s
    /// caption. The release over its close box closes the tab; a drag past
    /// `desktop::DRAG_PX` from `press` moves it along or off the caption.
    TabPress {
        window: WindowId,
        member: WindowId,
        close: bool,
        press: (f64, f64),
    },
    /// A task button held down with `[taskbar] pinned` on: clicks on the
    /// release over the same button unless dragged past `taskbar::press_drags`,
    /// which moves it along the row. Held by window or pin so a relayout does
    /// not hand the release another button.
    TaskPress {
        bar: TreeId,
        key: TaskKey<WindowId>,
        /// The pointer's layout position along the bar at the press
        /// (`TaskbarLayout::along_f`): a drag past `DRAG_PX` from it moves
        /// the button.
        along0: f64,
        /// How far past the button's start along the bar the press was, so
        /// the dragged button stays where it was grabbed.
        grab: i32,
        dragging: bool,
    },
    /// A Quick Launch button held down: it runs on the release over it,
    /// unless the pointer travelled far enough with the taskbar unlocked
    /// to become a drag, which moves it to another gap of the band.
    QuickPress {
        bar: TreeId,
        index: usize,
        p0: (f64, f64),
        /// Pointer minus the icon's top-left at the press, so the drag
        /// image keeps the icon where it was grabbed.
        grab: (i32, i32),
        dragging: bool,
    },
    /// A tray arrow held down (`trayreveal`): it opens or shuts its run on
    /// the release over it, and draws held while the pointer is on it.
    ArrowPress { bar: TreeId },
    /// A taskbar band's grip held down: the band follows the pointer, see
    /// `TaskbarLayout::drag_band`.
    Band {
        bar: TreeId,
        band: crate::taskbar::Band,
        dx: i32,
    },
    /// Empty space of an unlocked bar held down: past `desktop::DRAG_PX`
    /// from `p0` it drags the bar toward the edge whose triangle holds the
    /// pointer (`target`), and the release moves every bar there (`bardrag`).
    BarMove {
        bar: TreeId,
        p0: (f64, f64),
        target: Option<reprisekit::taskbar::Edge>,
    },
    /// An unlocked side bar's inner edge held down: the bars take the
    /// width the pointer gives them, `grab` px past its depth into the
    /// output, and the release keeps it (`bardrag`).
    BarWidth { bar: TreeId, grab: i32 },
    /// A button held on desktop icons or a Start menu program
    /// (`dragview`): past `desktop::DRAG_PX` from the press at (`x0`, `y0`)
    /// the drag has `started`; a right press that stays put opens the
    /// icon's menu on the release.
    Drag { x0: f64, y0: f64, started: bool },
    /// A left press on empty desktop: the marquee (`desktopview`).
    Marquee,
    /// The pointer drags a selection in an edit field (a dialog's, the
    /// desktop rename), or a form's thumb.
    FieldDrag { editor: crate::caret::Editor },
    /// A dialog held by its caption; `dx`/`dy` are the grab offset from its origin.
    DialogMove { index: usize, dx: f64, dy: f64 },
    /// A sizable dialog (Browse) held by its border or size grip: the
    /// pointer's travel from (`cx`, `cy`) moves `edges` of the frame that
    /// stood at `start`, in layout coordinates.
    DialogResize {
        index: usize,
        edges: Edges,
        start: Rect,
        cx: f64,
        cy: f64,
    },
    /// A dialog's push button held down: it draws pushed while the
    /// pointer is on it and acts on the release over it.
    DialogPress,
    /// A notification balloon's action button, or the history's check box
    /// or Clear All, held down: it acts on the release over it.
    NotifyPress,
    /// A slider on the displays panel held: it follows the pointer until the release.
    DisplaysSlide,
}

impl Interaction {
    pub fn is_resize_of(self, w: WindowId) -> bool {
        matches!(self, Interaction::Resize { window, .. } if window == w)
    }

    /// Whether this moves, sizes or holds a caption button of `w`: the
    /// gestures a change to the window's frame (maximize, snap, a new
    /// desktop) cuts short.
    pub fn acts_on(self, w: WindowId) -> bool {
        match self {
            Interaction::Move { window, .. }
            | Interaction::PressButton { window, .. }
            | Interaction::Resize { window, .. } => window == w,
            Interaction::TabPress { window, member, .. } => window == w || member == w,
            Interaction::TaskPress { .. }
            | Interaction::QuickPress { .. }
            | Interaction::ArrowPress { .. }
            | Interaction::Band { .. }
            | Interaction::BarMove { .. }
            | Interaction::BarWidth { .. }
            | Interaction::Drag { .. }
            | Interaction::Marquee
            | Interaction::FieldDrag { .. }
            | Interaction::DialogMove { .. }
            | Interaction::DialogResize { .. }
            | Interaction::DialogPress
            | Interaction::NotifyPress
            | Interaction::DisplaysSlide
            | Interaction::Idle => false,
        }
    }

    /// Whether this holds a taskbar button (a task, a Quick Launch button,
    /// the tray arrow) or drags a band: `end_taskbar_press` and
    /// `cancel_taskbar_press` take these.
    pub fn is_taskbar_press(self) -> bool {
        matches!(
            self,
            Interaction::TaskPress { .. }
                | Interaction::QuickPress { .. }
                | Interaction::ArrowPress { .. }
                | Interaction::Band { .. }
        )
    }

    /// Whether this move or size was started from the window menu and so
    /// owns the keyboard until it is committed or cancelled.
    pub fn is_keyboard(self) -> bool {
        matches!(
            self,
            Interaction::Move {
                origin: Some(_),
                ..
            } | Interaction::Resize {
                keyboard: Some(_),
                ..
            }
        )
    }
}

/// `pos` held inside `area` (the outputs' bounding box), the right and
/// bottom edges excluded.
pub fn clamp_to(pos: Point<f64, Logical>, area: Rectangle<i32, Logical>) -> Point<f64, Logical> {
    let max_x = f64::from(area.loc.x + area.size.w - 1).max(f64::from(area.loc.x));
    let max_y = f64::from(area.loc.y + area.size.h - 1).max(f64::from(area.loc.y));
    (
        pos.x.clamp(f64::from(area.loc.x), max_x),
        pos.y.clamp(f64::from(area.loc.y), max_y),
    )
        .into()
}

/// Whether a press on `hit` closes the Glass Start menu: anywhere but on
/// the panel, a popup over it (a menu) and Start, which toggles it itself.
fn press_closes_start_panel(hit: &Hit) -> bool {
    !matches!(
        hit,
        Hit::StartPanel { .. }
            | Hit::Menu { .. }
            | Hit::Taskbar {
                hit: TaskbarHit::Start,
                ..
            }
    )
}

/// Whether a press on `hit` first ends a gesture another button still
/// holds (`State::cancel_interaction`): the desktop starts its own.
fn press_ends_gesture(hit: &Hit) -> bool {
    matches!(hit, Hit::Desktop { .. })
}

impl State {
    pub fn process_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        if !matches!(
            event,
            InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. }
        ) {
            self.settle_pointer();
        }
        if crate::idle::is_activity(&event) {
            crate::idle::notify_activity(self);
        }
        match event {
            InputEvent::Keyboard { event } => self.key::<B>(event),
            InputEvent::PointerMotion { event } => {
                if self.capture_motion(event.delta()) {
                    return;
                }
                let delta = self.zoom_motion(event.delta());
                self.pointer_moved(delta, event.delta_unaccel(), Event::time(&event));
            }
            InputEvent::PointerMotionAbsolute { event } => {
                let area = self.output_area();
                let pos = event.position_transformed(area.size) + area.loc.to_f64();
                if self.capture_absolute(clamp_to(pos, area)) {
                    return;
                }
                // As a relative move, so a locked or confined pointer holds
                // for absolute devices (a VM's tablet) too.
                let delta = self.zoom_absolute(pos) - self.pointer_location();
                self.pointer_moved(delta, delta, Event::time(&event));
            }
            InputEvent::PointerButton { event } => {
                let time = Event::time(&event);
                self.handle_button(event.button_code(), event.state(), time);
                crate::access::touch(self);
            }
            InputEvent::PointerAxis { event } => {
                let source = event.source();
                let mut frame = AxisFrame::new(Event::time(&event)).source(source);
                for axis in [Axis::Horizontal, Axis::Vertical] {
                    let v120 = event.amount_v120(axis);
                    let amount = event
                        .amount(axis)
                        .unwrap_or_else(|| v120.unwrap_or(0.0) * 15.0 / 120.0);
                    if amount != 0.0 {
                        frame = frame.value(axis, amount);
                        if let Some(v120) = v120 {
                            frame = frame.v120(axis, v120 as i32);
                        }
                    } else if source == AxisSource::Finger {
                        frame = frame.stop(axis);
                    }
                }
                if self.capture_axis(&frame) {
                    return;
                }
                self.handle_axis(frame);
            }
            event @ (InputEvent::GestureSwipeBegin { .. }
            | InputEvent::GestureSwipeUpdate { .. }
            | InputEvent::GestureSwipeEnd { .. }
            | InputEvent::GesturePinchBegin { .. }
            | InputEvent::GesturePinchUpdate { .. }
            | InputEvent::GesturePinchEnd { .. }
            | InputEvent::GestureHoldBegin { .. }
            | InputEvent::GestureHoldEnd { .. }) => self.gesture(event),
            InputEvent::DeviceAdded { device } => self.device_added(&device),
            InputEvent::DeviceRemoved { device } => self.device_removed(&device),
            InputEvent::TouchDown { event } => self.touch_down::<B>(&event),
            InputEvent::TouchMotion { event } => self.touch_motion::<B>(&event),
            InputEvent::TouchUp { event } => self.touch_up::<B>(&event),
            InputEvent::TouchCancel { event } => self.touch_cancel::<B>(&event),
            InputEvent::TouchFrame { .. } => self.touch_frame(),
            InputEvent::TabletToolProximity { event } => self.tool_proximity::<B>(&event),
            InputEvent::TabletToolAxis { event } => self.tool_axis::<B>(&event),
            InputEvent::TabletToolTip { event } => self.tool_tip::<B>(&event),
            InputEvent::TabletToolButton { event } => self.tool_button::<B>(&event),
            InputEvent::SwitchToggle { event } => {
                use smithay::backend::input::SwitchToggleEvent;
                crate::power::switch_toggled(self, event.switch(), event.state())
            }
            _ => {}
        }
    }

    /// A wheel or scroll frame, from the backend or an EIS sender: a tray
    /// icon's, the notification history's (`tray_axis`), a dialog's, the
    /// emoji picker's, the clipboard history's or the displays panel's when
    /// the pointer is over one, else the client's under the pointer.
    pub fn handle_axis(&mut self, frame: AxisFrame) {
        self.settle_pointer();
        // The wheel rolls the 3D switcher; the Alt+Tab panel and region
        // overlay own the pointer and ignore it.
        if self.switcher.stack.is_some() {
            self.flip_wheel(frame.v120.map_or(0, |v| v.1), frame.axis.1);
            return;
        }
        if self.switcher.panel.is_some() || self.panels.region.is_some() {
            return;
        }
        if self.tray_axis(&frame) {
            return;
        }
        let hit = self.scene_hit(self.pointer_location());
        if matches!(hit, Hit::EmojiView { .. }) {
            let discrete = frame.v120.map_or(0, |v| v.1);
            self.emoji_wheel(discrete, frame.axis.1);
            return;
        }
        if matches!(hit, Hit::ClipView { .. }) {
            self.clipview_wheel(frame.v120.map_or(0, |v| v.1), frame.axis.1);
            return;
        }
        if matches!(hit, Hit::Displays { .. }) {
            let pos = self.pointer_location();
            let discrete = frame.v120.map_or(0, |v| v.1);
            crate::displaysview::wheel(self, pos.x, pos.y, discrete, frame.axis.1);
            return;
        }
        if let Hit::Dialog { index, button } = &hit
            && self.dialog_wheel(*index, *button, frame.v120.map_or(0, |v| v.1), frame.axis.1)
        {
            return;
        }
        if matches!(hit, Hit::StartPanel { .. })
            && self.start_panel_wheel(frame.v120.map_or(0, |v| v.1), frame.axis.1)
        {
            return;
        }
        self.panels.wheel = 0;
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        pointer.axis(self, frame);
        pointer.frame(self);
    }

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

    fn owe_repoint(&mut self, time: InputTime, left: Duration) {
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
            self.redraw_all();
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
        // Off the taskbar, the dialogs, the desktop's icons and the Glass
        // Start menu's spots the tip goes, and none is owed.
        if !matches!(
            hit,
            Hit::Taskbar { .. }
                | Hit::Dialog { .. }
                | Hit::Desktop { index: Some(_) }
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
            self.dialog_hover_close(None);
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
                let close = matches!(button, Some(crate::dialog::DialogHit::Close));
                self.dialog_hover_close(Some((index, close)));
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

    /// A button went down or up: a press focuses what it lands on and may
    /// start a move, size or caption button press; the last release ends it.
    pub(crate) fn handle_button(&mut self, button: u32, state: ButtonState, time: InputTime) {
        self.settle_pointer();
        if self.capture_button(button, state) {
            return;
        }
        let serial = SERIAL_COUNTER.next_serial();
        let pos = self.pointer_location();
        if state == ButtonState::Pressed && self.interaction.is_keyboard() {
            self.end_interaction();
            self.repoint(pos, time);
            self.redraw_all();
            return;
        }
        if self.panels.region.is_some() {
            self.region_button(button, state == ButtonState::Pressed);
            self.redraw_all();
            return;
        }
        if state == ButtonState::Pressed && self.switcher.is_open() {
            // The Alt+Tab panel owns the pointer: the press is its alone.
            self.buttons_down += 1;
            self.press_at = (pos.x, pos.y);
            self.keys_saw_button();
            self.switcher_press(button);
            self.redraw_all();
            return;
        }
        if state == ButtonState::Pressed && self.overview.view.is_some() {
            self.buttons_down += 1;
            self.press_at = (pos.x, pos.y);
            self.keys_saw_button();
            self.overview_press(button);
            self.redraw_all();
            return;
        }
        if state == ButtonState::Pressed {
            // A press takes the keyboard from the taskbar.
            self.end_bar_keys();
            self.buttons_down += 1;
            self.press_at = (pos.x, pos.y);
            self.press(button, pos, time);
        } else {
            self.buttons_down = self.buttons_down.saturating_sub(1);
            if self.buttons_down == 0 {
                self.start_panel_release();
            }
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        // A press the seat never saw (the Alt+Tab panel's) sends no
        // release to a client either.
        let theirs = state == ButtonState::Released
            && (self.switcher_release(button) | self.overview_release(button));
        if !theirs {
            pointer.button(
                self,
                &ButtonEvent {
                    button,
                    state,
                    serial,
                    time,
                },
            );
            pointer.frame(self);
        }
        if state == ButtonState::Pressed {
            self.keys_saw_button();
        }
        if state == ButtonState::Released
            && self.buttons_down == 0
            && self.interaction != Interaction::Idle
        {
            self.end_interaction();
            self.repoint(pos, time);
        }
        self.redraw_all();
    }

    pub(crate) fn press(&mut self, button: u32, pos: Point<f64, Logical>, time: InputTime) {
        if self.is_locked() {
            crate::lockview::button(self, button, pos);
            return;
        }
        if self.drag_running() {
            return;
        }
        // A second button lets go of a held taskbar button or band first,
        // so it neither acts nor stays drawn held.
        if self.interaction.is_taskbar_press() {
            let was = std::mem::take(&mut self.interaction);
            self.cancel_taskbar_press(was);
        }
        let mods = self
            .seat
            .get_keyboard()
            .map_or(0, |k| keys::mask(&k.modifier_state()));
        let hit = self.scene_hit(pos);
        let at = (pos.x as i32, pos.y as i32);
        // A press anywhere but on a menu closes the open menus and goes on
        // to what it landed on; Start toggles its menu itself. `closed`
        // lets a click on what opened a menu shut it without reopening it.
        let closed = self.menu_root();
        crate::tooltip::press(self);
        let flyout_was_on = crate::trayflyout::open_on(self);
        if !matches!(hit, Hit::TrayFlyout { .. }) {
            crate::trayflyout::close(self);
        }
        if !matches!(hit, Hit::TaskPreview { .. }) {
            crate::taskpreview::hide(self);
        }
        if !matches!(hit, Hit::SnapPicker { .. }) {
            crate::snappickerview::hide(self);
        }
        // Likewise the history popup, except on its own action menu and
        // on the tray, where the bell toggles it (`tray::click`).
        if !matches!(
            hit,
            Hit::NotifyCenter
                | Hit::Menu { .. }
                | Hit::Taskbar {
                    hit: TaskbarHit::Tray(_),
                    ..
                }
        ) {
            crate::notifycenter::close(self);
        }
        // A press off the balloon takes the keyboard from its reply field,
        // which stays open.
        if !matches!(hit, Hit::Notification) {
            crate::notifyview::blur(self);
        }
        let calendar_was_open = crate::calendarview::is_open(self);
        if !matches!(hit, Hit::Calendar { .. }) {
            crate::calendarview::close(self);
        }
        // The displays cell toggles its panel itself (`builtin::click`).
        let displays_cell_hit = match hit {
            Hit::Taskbar {
                bar,
                hit: TaskbarHit::Tray(i),
            } => crate::tray::is_displays_cell(self, bar, i),
            _ => false,
        };
        if !matches!(hit, Hit::Displays { .. }) && !displays_cell_hit {
            crate::displaysview::close(self);
        }
        // A tray cell decides itself whether an open menu closes or
        // toggles (`tray::click`).
        if !matches!(
            hit,
            Hit::Menu { .. }
                | Hit::Taskbar {
                    hit: TaskbarHit::Start | TaskbarHit::Tray(_),
                    ..
                }
        ) {
            self.close_menu();
        }
        if !matches!(hit, Hit::Launcher { .. } | Hit::Menu { .. }) {
            self.close_launcher();
        }
        if press_closes_start_panel(&hit) {
            self.close_start_panel();
        }
        if !matches!(hit, Hit::ClipView { .. }) {
            self.close_clipview();
        }
        if !matches!(hit, Hit::EmojiView { .. }) {
            self.close_emoji();
        }
        if press_ends_gesture(&hit) {
            self.cancel_interaction();
        }
        match hit {
            // A window under a modal dialog takes no press on its frame
            // either: the press brings the dialog up.
            Hit::Frame { window, .. } if self.modal_target(window) != window => {
                self.focus_window(window);
            }
            Hit::Surface {
                window, surface, ..
            } => {
                let Some(window) = window else {
                    crate::layer::press(self, &surface);
                    return;
                };
                // An X11 menu or tooltip takes the focus only if it asks.
                if self.win(window).is_some_and(|w| w.wants_focus_on_click()) {
                    self.focus_window(window);
                }
                if mods == keys::MOD_LOGO && button == BTN_LEFT {
                    self.begin_move(window, false);
                }
            }
            Hit::Frame { window, hit } => {
                self.focus_window(window);
                let click = (self.config.theme.metrics).click(at.0, at.1, time.millis());
                let double = caption_double(&mut self.caption_clicks, window, hit, button, click);
                if hit == FrameHit::Title
                    && let Some((tab, member)) = self.tab_at(window, pos.x, pos.y)
                {
                    self.tab_press(window, tab, member, button, at, double);
                    return;
                }
                if button == BTN_RIGHT && matches!(hit, FrameHit::Title | FrameHit::Icon) {
                    self.open_window_menu(window, at, false);
                    return;
                }
                if button != BTN_LEFT {
                    return;
                }
                match hit {
                    FrameHit::Icon => {
                        // A second click on the icon closed the menu above.
                        if double {
                            self.close(window);
                        } else if closed == Some(MenuRoot::Window(window)) {
                            self.caption_clicks.reset();
                        } else {
                            self.open_window_menu_below(window, false);
                        }
                    }
                    FrameHit::Title => {
                        if double {
                            let maximized = self.win(window).is_some_and(|w| w.maximized);
                            self.set_maximized(window, !maximized);
                        } else {
                            self.begin_move(window, true);
                        }
                    }
                    FrameHit::Button(b) => {
                        if let Some(w) = self.win_mut(window) {
                            w.pressed = Some(b);
                        }
                        self.redraw_frame(window);
                        self.interaction = Interaction::PressButton { window, button: b };
                    }
                    FrameHit::Resize(edges) => self.begin_resize(window, edges),
                    FrameHit::Help | FrameHit::Client | FrameHit::Outside => {}
                }
            }
            Hit::Taskbar {
                bar,
                hit: TaskbarHit::Tray(i),
            } => {
                // A client popup mapping soon after holds an auto-hidden bar.
                if let Some(o) = self.bar(bar).map(|b| b.output.clone()) {
                    crate::autohideview::tray_pressed(self, &o);
                }
                crate::tray::click(self, bar, i, button);
            }
            Hit::Taskbar {
                bar,
                hit: TaskbarHit::Chevron,
            } => {
                // A look with the tray arrow slides the hidden icons out in
                // place, on the release; the others open them in the flyout
                // on the press.
                if !crate::trayreveal::arrow_look(self) {
                    crate::trayflyout::chevron_press(self, bar, button, flyout_was_on);
                } else if button == BTN_LEFT {
                    crate::trayreveal::press(self, bar);
                }
            }
            Hit::Taskbar {
                bar,
                hit: TaskbarHit::Clock,
            } if button == BTN_LEFT => {
                crate::calendarview::clock_press(self, bar, at, time.millis(), calendar_was_open)
            }
            // An icon takes the press as on the bar; the panel keeps it.
            Hit::TrayFlyout { cell } => {
                if let Some(o) = &flyout_was_on {
                    crate::autohideview::tray_pressed(self, o);
                }
                crate::trayflyout::press(self, cell, button);
            }
            Hit::Calendar { hit } => crate::calendarview::clicked(self, hit),
            Hit::Displays { .. } => {
                if button == BTN_LEFT {
                    crate::displaysview::press(self, pos.x, pos.y);
                }
            }
            // A left click on a cell activates its window; any other
            // press on the panel just closes it.
            Hit::TaskPreview { cell } if button == BTN_LEFT => {
                crate::taskpreview::clicked(self, cell)
            }
            Hit::TaskPreview { .. } => crate::taskpreview::hide(self),
            Hit::SnapPicker { zone } => crate::snappickerview::press(self, zone, button),
            Hit::Notification => {
                if crate::notifyview::press(self, pos.x, pos.y, button) {
                    self.interaction = Interaction::NotifyPress;
                }
            }
            Hit::NotifyCenter => {
                if crate::notifycenter::press(self, pos.x, pos.y, button, time.millis()) {
                    self.interaction = Interaction::NotifyPress;
                }
            }
            // The bar leaves the focus where it is: a task button acts
            // on the focused window.
            Hit::Taskbar { bar, hit } => {
                // A bar not all the way out only comes back for it.
                if !crate::autohideview::line_pressed(self, bar) {
                    let shift = mods & keys::MOD_SHIFT != 0;
                    self.taskbar_press(bar, hit, button, at, shift, closed);
                }
            }
            // A menu row acts on the press; a program row may be dragged
            // off the menu and acts on the release.
            Hit::Menu { depth, row } => self.menu_row_press(depth, row, button),
            Hit::Dialog {
                index,
                button: dialog_button,
            } => {
                if button == BTN_LEFT {
                    self.dialog_press(index, dialog_button, time.millis());
                } else if let Some(nav) = reprisekit::pointer::nav_button(button) {
                    self.dialog_nav(index, nav);
                }
            }
            // The desktop takes the keyboard from the windows. Any button on
            // an icon may become a drag (middle drops as left does); a right
            // press that stays put opens the icon's menu on the release.
            Hit::Desktop { index } => {
                self.focus_desktop();
                // During a rename a press in its box moves the caret (and
                // may drag-select); anywhere else commits the name first.
                if self.rename_press(pos.x, pos.y, time.millis()) {
                    self.interaction = Interaction::FieldDrag {
                        editor: crate::caret::Editor::Rename,
                    };
                    return;
                }
                match (button, index) {
                    (BTN_LEFT, _) => {
                        let mods = reprisekit::selection::Mods {
                            ctrl: mods & keys::MOD_CTRL != 0,
                            shift: mods & keys::MOD_SHIFT != 0,
                        };
                        self.desktop_press(index, at, time.millis(), mods);
                        if index.is_none() {
                            self.interaction = Interaction::Marquee;
                            self.water_click(pos);
                        }
                    }
                    (BTN_RIGHT, None) => self.open_desktop_menu(at),
                    (BTN_RIGHT, Some(i)) => self.desktop_right_press(i),
                    _ => {}
                }
                if let (Some(i), BTN_LEFT | BTN_RIGHT | BTN_MIDDLE) = (index, button) {
                    let sources = self.drag_sources(i);
                    self.drag_arm(sources, button);
                }
            }
            // The panel is not a target: the click takes it away and
            // stops there rather than falling through to what it covers.
            Hit::Osd => self.hide_osd(),
            // Likewise: a row starts it, the rest keeps it open.
            Hit::Launcher { hit } => match button {
                BTN_LEFT => self.launcher_clicked(hit),
                BTN_RIGHT => self.launcher_right_clicked(hit),
                _ => {}
            },
            Hit::StartPanel { spot } => self.start_panel_press(spot, button),
            Hit::ClipView { hit } => {
                if button == BTN_LEFT {
                    self.clipview_clicked(hit);
                }
            }
            Hit::EmojiView { hit } => {
                if button == BTN_LEFT {
                    self.emoji_clicked(hit);
                }
            }
            // The panel's presses go to `switcher_press`.
            Hit::Switcher { .. } | Hit::Overview { .. } | Hit::Nothing => {}
        }
    }

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
    fn fitted(&self, w: WindowId) -> bool {
        self.win(w)
            .is_some_and(|w| w.snapped.is_some() || w.maximized || w.fullscreen)
    }

    /// Give a snapped, maximized or fullscreen `w` its size from before back
    /// once a move has really started, placed so the pointer at (`cx`, `cy`)
    /// holds it where the press at `press` did (`snap::drag_out_grab`).
    /// Returns the move's new grab offset, or None when `w` was in none of
    /// those states.
    fn unfit_for_move(
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
        win.snapped = None;
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
        // A pointer drag of an edge shared within a snap group sizes the
        // neighbours along it.
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
            } => self.end_move(window, snap, start.restore_rect(was_snapped)),
            Interaction::Resize {
                window,
                edges,
                keyboard,
                ..
            } => {
                // `resize_anchor` deliberately survives the release: the
                // client's commit for the last requested size still has to
                // be anchored. `handle_commit` drops it once that lands.
                self.set_resizing(window, false);
                if let Some(origin) = keyboard {
                    self.wm.snap.end_key_resize(window, origin, edges);
                }
                self.end_group_resize();
            }
            Interaction::TabPress { .. } => self.tab_press_released(),
            Interaction::PressButton { window, button } => {
                if let Some(w) = self.win_mut(window) {
                    w.pressed = None;
                }
                self.redraw_frame(window);
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
            Interaction::DialogMove { .. }
            | Interaction::DialogResize { .. }
            | Interaction::Idle => {}
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
            Interaction::Resize { window, .. } => {
                self.set_resizing(window, false);
                self.end_group_resize();
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
            // A dragged or sized dialog stays as it is.
            Interaction::DialogMove { .. } | Interaction::DialogResize { .. } => {}
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

    fn drag_resize(&mut self, w: WindowId, edges: Edges, start: Rect, cx: f64, cy: f64) {
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
fn top_below_top_bar(r: Rect, edges: Edges, p: &crate::barplace::BarPlacement) -> Rect {
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

/// Whether a press on frame part `hit` completes a double click on the caption or icon. Only left
/// presses there pair; another button forgets the last.
fn caption_double(
    clicks: &mut DoubleClick<(WindowId, FrameHit)>,
    window: WindowId,
    hit: FrameHit,
    button: u32,
    click: Click,
) -> bool {
    if button != BTN_LEFT {
        clicks.reset();
        return false;
    }
    matches!(hit, FrameHit::Title | FrameHit::Icon) && clicks.press((window, hit), click)
}

/// An interaction on `window` was cancelled: a caption or icon click on it no longer pairs, one on
/// another window still does.
pub(crate) fn caption_cancelled(clicks: &mut DoubleClick<(WindowId, FrameHit)>, window: WindowId) {
    clicks.forget_if(|(w, _)| *w == window);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_presses_pair_on_one_part_within_the_configured_time_and_slop() {
        let m = reprisekit::theme::Metrics {
            double_click_ms: 900,
            ..Default::default()
        };
        let w = WindowId::next();
        let mut clicks = DoubleClick::default();
        let mut press = |hit, button, x, time| {
            caption_double(&mut clicks, w, hit, button, m.click(x, 10, time))
        };
        assert!(!press(FrameHit::Title, BTN_LEFT, 100, 1000));
        assert!(press(FrameHit::Title, BTN_LEFT, 104, 1800), "800 ms, 4 px");
        assert!(!press(FrameHit::Title, BTN_LEFT, 100, 3000));
        assert!(!press(FrameHit::Icon, BTN_LEFT, 100, 3100), "another part");
        assert!(!press(FrameHit::Icon, BTN_RIGHT, 100, 3200));
        assert!(
            !press(FrameHit::Icon, BTN_LEFT, 100, 3300),
            "a right press between"
        );
        assert!(
            press(FrameHit::Icon, BTN_LEFT, 100, 3400),
            "the icon pairs too"
        );
        assert!(!press(FrameHit::Title, BTN_LEFT, 100, 5000));
        assert!(!press(FrameHit::Title, BTN_LEFT, 105, 5100), "5 px off");
    }

    #[test]
    fn a_cancel_on_another_window_keeps_the_caption_click_pairing() {
        let m = reprisekit::theme::Metrics::default();
        let (w, other) = (WindowId::next(), WindowId::next());
        let mut clicks = DoubleClick::default();
        caption_double(
            &mut clicks,
            w,
            FrameHit::Title,
            BTN_LEFT,
            m.click(100, 10, 1000),
        );
        caption_cancelled(&mut clicks, other);
        assert!(
            caption_double(
                &mut clicks,
                w,
                FrameHit::Title,
                BTN_LEFT,
                m.click(100, 10, 1100)
            ),
            "another window's cancel leaves the first click"
        );
        caption_double(
            &mut clicks,
            w,
            FrameHit::Title,
            BTN_LEFT,
            m.click(100, 10, 3000),
        );
        caption_cancelled(&mut clicks, w);
        assert!(
            !caption_double(
                &mut clicks,
                w,
                FrameHit::Title,
                BTN_LEFT,
                m.click(100, 10, 3100)
            ),
            "its own window's cancel forgets it"
        );
    }

    #[test]
    fn a_drop_restores_to_the_rect_from_before_the_drag() {
        // Grabbed far right and dropped at the left edge: the frame at
        // the release starts at -700.
        let found = Rect::new(400, 300, 800, 600);
        let free = MoveStart {
            rect: found,
            maximized: false,
            restore: None,
        };
        assert_eq!(free.restore_rect(None), found, "the rect the drag found");
        let pre_snap = Rect::new(100, 120, 640, 480);
        assert_eq!(
            free.restore_rect(Some(pre_snap)),
            pre_snap,
            "the size from before the snap"
        );
        let maximized = MoveStart {
            rect: Rect::new(0, 0, 1920, 1032),
            maximized: true,
            restore: Some(pre_snap),
        };
        assert_eq!(
            maximized.restore_rect(None),
            pre_snap,
            "the rect a restore from maximized went to"
        );
    }

    #[test]
    fn a_second_press_lets_go_of_the_held_taskbar_buttons_and_bands_only() {
        let scene = crate::compositor::DeskScene::new();
        let bar = scene.root();
        let held = [
            Interaction::ArrowPress { bar },
            Interaction::QuickPress {
                bar,
                index: 0,
                p0: (0.0, 0.0),
                grab: (0, 0),
                dragging: false,
            },
            Interaction::TaskPress {
                bar,
                key: crate::taskbar::TaskKey::Window(WindowId::next()),
                along0: 0.0,
                grab: 0,
                dragging: false,
            },
            Interaction::Band {
                bar,
                band: crate::taskbar::Band::Tasks,
                dx: 0,
            },
        ];
        for i in held {
            assert!(i.is_taskbar_press(), "{i:?} is let go of");
        }
        for i in [
            Interaction::Idle,
            Interaction::NotifyPress,
            Interaction::DialogPress,
            Interaction::Marquee,
            Interaction::BarWidth { bar, grab: 0 },
        ] {
            assert!(!i.is_taskbar_press(), "{i:?} is not a taskbar press");
        }
    }

    #[test]
    fn the_pointer_stays_on_the_outputs() {
        let area = Rectangle::new((0, 0).into(), (1920, 1080).into());
        assert_eq!(clamp_to((-5.0, 20.0).into(), area), (0.0, 20.0).into());
        assert_eq!(
            clamp_to((5000.0, 5000.0).into(), area),
            (1919.0, 1079.0).into()
        );
        assert_eq!(clamp_to((10.5, 7.25).into(), area), (10.5, 7.25).into());
    }

    #[test]
    fn a_press_on_a_popup_keeps_the_start_panel_and_one_on_neither_closes_it() {
        assert!(!press_closes_start_panel(&Hit::Menu {
            depth: 0,
            row: Some(1)
        }));
        assert!(!press_closes_start_panel(&Hit::StartPanel { spot: None }));
        assert!(press_closes_start_panel(&Hit::Nothing));
        assert!(press_closes_start_panel(&Hit::Desktop { index: None }));
    }

    #[test]
    fn only_a_desktop_press_ends_a_gesture_another_button_holds() {
        assert!(press_ends_gesture(&Hit::Desktop { index: None }));
        assert!(press_ends_gesture(&Hit::Desktop { index: Some(0) }));
        assert!(!press_ends_gesture(&Hit::Nothing));
        assert!(!press_ends_gesture(&Hit::StartPanel { spot: None }));
        assert!(!press_ends_gesture(&Hit::Menu {
            depth: 0,
            row: Some(1)
        }));
    }

    use crate::theme::Metrics;

    fn frame_pad(m: &Metrics) -> (i32, i32) {
        (2 * m.frame_width, 2 * m.frame_width + m.caption_height)
    }

    #[test]
    fn resize_rect_left_edge_reanchors_at_max_width() {
        let m = Metrics::default();
        let start = Rect::new(100, 50, 208, 126);
        let limits = SizeLimits {
            min: (1, 1),
            max: (Some(150), None),
        };
        let r = resize_rect(start, Edges::LEFT, -50, 0, limits, frame_pad(&m));
        assert_eq!(r.right(), start.right());
        assert_eq!(r.width, 150 + 2 * m.frame_width);
    }

    #[test]
    fn resize_rect_top_edge_reanchors_at_max_height() {
        let m = Metrics::default();
        let start = Rect::new(100, 50, 208, 126);
        let limits = SizeLimits {
            min: (1, 1),
            max: (None, Some(80)),
        };
        let r = resize_rect(start, Edges::TOP, 0, -80, limits, frame_pad(&m));
        assert_eq!(r.bottom(), start.bottom());
        assert_eq!(r.height, 80 + 2 * m.frame_width + m.caption_height);
    }

    #[test]
    fn a_top_edge_drag_stops_at_a_top_bars_foot() {
        use reprisekit::taskbar::Edge;
        let out = Rect::new(0, 0, 800, 600);
        let top = crate::barplace::place(out, Edge::Top, 30);
        let r = Rect::new(100, 10, 200, 190);
        assert_eq!(
            top_below_top_bar(r, Edges::TOP | Edges::LEFT, &top),
            Rect::new(100, 30, 200, 170)
        );
        let low = Rect::new(100, 40, 200, 160);
        assert_eq!(top_below_top_bar(low, Edges::TOP, &top), low);
        assert_eq!(top_below_top_bar(r, Edges::BOTTOM, &top), r);
        let bottom = crate::barplace::place(out, Edge::Bottom, 30);
        assert_eq!(top_below_top_bar(r, Edges::TOP, &bottom), r);
    }

    #[test]
    fn is_resize_of_only_matches_its_own_window() {
        let a = WindowId::next();
        let b = WindowId::next();
        let r = Interaction::Resize {
            window: a,
            edges: Edges::LEFT,
            start: Rect::new(0, 0, 10, 10),
            cx: 0.0,
            cy: 0.0,
            keyboard: None,
        };
        assert!(r.is_resize_of(a));
        assert!(!r.is_resize_of(b));
        assert!(!Interaction::Idle.is_resize_of(a));
        assert!(
            !Interaction::Move {
                window: a,
                dx: 0.0,
                dy: 0.0,
                origin: None,
                was_snapped: None,
                start: MoveStart::default(),
                press: (0.0, 0.0),
                snap: None,
                armed: false,
                bar: false,
            }
            .is_resize_of(a)
        );
        assert!(
            !Interaction::PressButton {
                window: a,
                button: CaptionButton::Close
            }
            .is_resize_of(a)
        );
    }

    #[test]
    fn resize_rect_takes_client_limits_at_i32_max() {
        let m = Metrics::default();
        let start = Rect::new(100, 50, 208, 126);
        let pad = frame_pad(&m);
        let limits = crate::window::client_limits((i32::MAX, i32::MAX, i32::MAX, i32::MAX));
        let r = resize_rect(start, Edges::LEFT, 10, 0, limits, pad);
        assert_eq!(r.width, 32767 + pad.0);
        assert_eq!(r.right(), start.right());
    }

    #[test]
    fn resize_rect_right_edge_min_width_keeps_x() {
        let m = Metrics::default();
        let start = Rect::new(100, 50, 208, 126);
        let limits = SizeLimits {
            min: (300, 1),
            max: (None, None),
        };
        let r = resize_rect(start, Edges::RIGHT, -100, 0, limits, frame_pad(&m));
        assert_eq!(r.x, start.x);
        assert_eq!(r.width, 300 + 2 * m.frame_width);
    }
}
