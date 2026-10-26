use smithay::{
    backend::input::{ButtonState, InputTime},
    input::pointer::ButtonEvent,
    utils::{Logical, Point, SERIAL_COUNTER},
};

use reprisekit::classic::frame::FrameHit;
use reprisekit::fluid::Source;
use reprisekit::pointer::{Click, DoubleClick};

use crate::{compositor::State, keys, menuview::MenuRoot, taskbar::TaskbarHit, window::WindowId};

use super::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Hit, Interaction};

/// Whether a press on `hit` closes the Glass Start menu: anywhere but on
/// the panel, a popup over it (a menu) and Start, which toggles it itself.
pub(super) fn press_closes_start_panel(hit: &Hit) -> bool {
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

impl State {
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
            if self.buttons_down == 0 && !self.over_x11(pos) {
                crate::xwayland::menus::dismiss(self);
            }
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
            if state == ButtonState::Pressed {
                self.seat_buttons.push(button);
            } else {
                self.seat_buttons.retain(|&b| b != button);
            }
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

    /// Let go of every held button when the seat loses the device (a session
    /// pause, the lock): the gesture they hold is cancelled, not finished, and
    /// the client under the pointer hears each release.
    pub(crate) fn release_all_buttons(&mut self, time: InputTime) {
        if self.interaction != Interaction::Idle {
            self.cancel_interaction();
        }
        for button in self.seat_buttons.clone() {
            self.handle_button(button, ButtonState::Released, time);
        }
        self.buttons_down = 0;
        self.seat_buttons.clear();
    }

    /// Whether `pos` is on an X11 program's surface, where Xwayland gets the press itself.
    fn over_x11(&self, pos: Point<f64, Logical>) -> bool {
        matches!(
            self.scene_hit(pos),
            Hit::Surface { window: Some(w), .. } if self.win(w).is_some_and(|w| w.x11().is_some())
        )
    }

    pub(crate) fn press(&mut self, button: u32, pos: Point<f64, Logical>, time: InputTime) {
        if self.is_locked() {
            crate::lockview::button(self, button, pos);
            return;
        }
        if self.drag_running() {
            return;
        }
        // A second button ends the gesture another button holds before it
        // acts, so a surface it opens or a gesture it starts never runs
        // beside a held move, size, drag or press.
        if self.interaction.ends_on_press() {
            self.cancel_interaction();
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
        // A press on a field's edit menu leaves the panel or reply field under it open for the
        // pick.
        let on_edit_menu = matches!(hit, Hit::Menu { .. }) && self.edit_menu_open();
        // A press off the balloon takes the keyboard from its reply field,
        // which stays open.
        if !matches!(hit, Hit::Notification) && !on_edit_menu {
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
        if !matches!(hit, Hit::ClipView { .. }) && !on_edit_menu {
            self.close_clipview();
        }
        if !matches!(hit, Hit::EmojiView { .. }) && !on_edit_menu {
            self.close_emoji();
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
                } else if button == BTN_RIGHT {
                    self.dialog_right_press(index, dialog_button, at);
                } else if let Some(nav) = reprisekit::pointer::nav_button(button) {
                    self.dialog_nav(index, nav);
                }
            }
            // The desktop takes the keyboard from the windows. Any button on
            // an icon may become a drag (middle drops as left does); a right
            // press that stays put opens the icon's menu on the release.
            Hit::Desktop { index } => {
                self.focus_desktop();
                if button == BTN_RIGHT && self.rename_right_press(pos.x, pos.y) {
                    self.open_edit_menu(Some(at));
                    return;
                }
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
                        let double = self.desktop_press(index, at, time.millis(), mods);
                        match desktop_left(self.stir_takes(pos), index, double) {
                            DesktopLeft::Icon => {}
                            DesktopLeft::Marquee => {
                                self.interaction = Interaction::Marquee;
                                self.water_click(pos);
                            }
                            DesktopLeft::Stir => {
                                self.interaction = Interaction::Stir;
                                self.stir_down(Source::Pointer, pos);
                            }
                            DesktopLeft::Clear => {
                                self.end_marquee();
                                self.stir_clear(pos);
                            }
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
            Hit::Launcher {
                hit: reprisekit::controls::launcher::Hit::Field,
            }
            | Hit::ClipView {
                hit: reprisekit::controls::cliphistory::Hit::Field,
            }
            | Hit::EmojiView {
                hit: reprisekit::controls::emojipicker::Hit::Field,
            } if button == BTN_RIGHT => {
                self.restart_caret();
                self.open_edit_menu(Some(at));
            }
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
}

/// What a left press on the desktop starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DesktopLeft {
    Icon,
    Marquee,
    Stir,
    Clear,
}

/// A left press on icon `index` (None: empty desktop), `double` when it pairs with the last one
/// there. Where the fluid takes a stir, empty desktop stirs and a double click clears the fluid.
pub(super) fn desktop_left(stirs: bool, index: Option<usize>, double: bool) -> DesktopLeft {
    match (index, stirs, double) {
        (Some(_), _, _) => DesktopLeft::Icon,
        (None, false, _) => DesktopLeft::Marquee,
        (None, true, false) => DesktopLeft::Stir,
        (None, true, true) => DesktopLeft::Clear,
    }
}

/// Whether a press on frame part `hit` completes a double click on the caption or icon. Only left
/// presses there pair; another button forgets the last.
pub(super) fn caption_double(
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
