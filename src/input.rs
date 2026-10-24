//! Input events from any backend: the keyboard goes to the focused window
//! (Ctrl+Alt+Fn switches VT first; keys go through `keyboard`), the pointer
//! moves over the outputs, a press focuses and raises the window under it,
//! and a press on a frame moves, sizes or works its caption buttons.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, Device as _, Event, InputBackend, InputEvent,
        PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::pointer::AxisFrame,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use reprisekit::classic::frame::{CaptionButton, Edges, FrameHit};

use reprisescene::TreeId;

use crate::{
    compositor::State,
    raster::Rect,
    taskbar::{TaskKey, TaskbarHit},
    window::WindowId,
};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
pub const BTN_MIDDLE: u32 = 0x112;

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
    /// A left press on empty desktop with a fluid water style: the pointer stirs the fluid.
    Stir,
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
            | Interaction::Stir
            | Interaction::FieldDrag { .. }
            | Interaction::DialogMove { .. }
            | Interaction::DialogResize { .. }
            | Interaction::DialogPress
            | Interaction::NotifyPress
            | Interaction::DisplaysSlide
            | Interaction::Idle => false,
        }
    }

    /// Whether another button's press ends this gesture before it acts
    /// (`State::press`). A move or size from the window menu holds no
    /// button: a press commits it instead.
    pub fn ends_on_press(self) -> bool {
        self != Interaction::Idle && !self.is_keyboard()
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

impl State {
    pub fn process_input<B: InputBackend>(&mut self, event: InputEvent<B>) {
        if !matches!(
            event,
            InputEvent::PointerMotion { .. } | InputEvent::PointerMotionAbsolute { .. }
        ) {
            self.settle_pointer();
            self.flush_motion();
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
                self.pointer_moved(delta, (delta, event.delta_unaccel()), Event::time(&event));
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
                let step = self.pointers.absolute.step(Event::device(&event).id(), pos);
                self.pointer_moved(delta, (step, step), Event::time(&event));
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
}

mod button;
mod move_resize;
mod pointer;
#[cfg(test)]
mod tests;

pub(crate) use button::caption_cancelled;
pub(crate) use move_resize::resize_rect;
