//! The keyboard: every key event is decoded once into a `KeyPress`,
//! snapshotted against the compositor's modes and handed to
//! `keyroute::route`, which says who gets it. The route either forwards
//! the key to the seat (the focused client, or whatever grab Smithay's
//! seat holds: a popup, an input method) or takes it for Reprise's own
//! use: a bind, the menu stack, a VT switch, the Super tap.
//!
//! A key is decoded as it was before it acts: its keysyms and the
//! modifiers come from the xkb state ahead of this key's own update, so a
//! bare Super press carries no Super bit and its release still does.
//! A press taken from the client takes its release too: no client hears
//! the release of a key whose press it never saw.

use reprisekit::classic::frame::{Edges, FrameGeometry};
use smithay::{
    backend::input::{Event, InputBackend, InputTime, KeyState, KeyboardKeyEvent},
    input::keyboard::{FilterResult, KeyboardSource, Keycode, ModifiersState, xkb},
    reexports::wayland_server::DisplayHandle,
    utils::SERIAL_COUNTER,
};

use crate::{
    compositor::State,
    config::Action,
    input::Interaction,
    keyroute::{ChordState, GlobalBind, GlobalHit, KeyPress, KeyRoute, Modes},
    keys,
    raster::Rect,
    window::WindowId,
};

/// The keyboard's own state on `State` (`State::key_input`).
#[derive(Debug)]
pub struct KeyInput {
    /// A Super key went down on its own and nothing else has been pressed
    /// since; releasing it now toggles the Start menu (the Super key tap).
    pub super_tap_armed: bool,
    /// The keys (xkb keycodes) whose releases go to no client: those
    /// pressed during a drag and still held after it (no client heard
    /// their presses), and those held when a click closed the Alt+Tab
    /// panel on the same client. A key leaves on its next release or
    /// press.
    pub drag_keys: Vec<u32>,
    /// Keys whose press fired a global shortcut, by xkb keycode: their
    /// release sends deactivated and reaches no client.
    pub shortcut_keys: Vec<(u32, GlobalHit)>,
    /// Whether the last input was a keystroke rather than a click. A
    /// dialog opens with its focus rectangles showing only when it was.
    /// Pointer motion on its own does not count.
    pub last_key_input: bool,
    /// The modifier-only chords (`[[keybinds]]` on a modifier key).
    pub chord: ChordState,
    /// Keys (xkb keycodes) whose press went to no client, so their
    /// release goes to none either.
    taken: Vec<u32>,
    /// Keys (xkb keycodes) whose press went to the input method for
    /// Reprise's own field, so their release goes there too.
    ime_keys: Vec<u32>,
    /// StickyKeys and SlowKeys.
    pub aids: crate::keyfilter::KeyAids,
    /// MouseKeys: the keypad driving the pointer.
    pub mouse_keys: crate::mousekeys::MouseKeysHost,
    /// The keyboard layouts.
    pub layouts: crate::kblayout::Layouts,
    /// Clients inhibiting the compositor's shortcuts.
    pub inhibit: crate::shortcutinhibit::Inhibit,
    /// The media keys' worker and their repeat.
    pub media: crate::media::MediaState,
}

impl KeyInput {
    /// The key state of a new session, with the shortcuts-inhibit global
    /// on `dh`.
    pub fn new(dh: &DisplayHandle) -> KeyInput {
        KeyInput {
            super_tap_armed: false,
            drag_keys: Vec::new(),
            shortcut_keys: Vec::new(),
            last_key_input: false,
            chord: ChordState::default(),
            taken: Vec::new(),
            ime_keys: Vec::new(),
            aids: Default::default(),
            mouse_keys: Default::default(),
            layouts: Default::default(),
            inhibit: crate::shortcutinhibit::Inhibit::new(dh),
            media: Default::default(),
        }
    }
}

/// One key decoded from its keysyms and the modifier mask in force before
/// it: the lowercase keysyms beside them, Caps and Num Lock masked out.
pub fn key_press(syms: Vec<u32>, mods: u32, pressed: bool) -> KeyPress {
    let lower = syms.iter().map(|&s| keys::to_lower(s)).collect();
    KeyPress {
        syms,
        lower,
        mods: mods & !(keys::MOD_CAPS | keys::MOD_MOD2),
        pressed,
    }
}

impl State {
    /// A key event from the backend.
    pub(crate) fn key<B: InputBackend>(&mut self, event: B::KeyboardKeyEvent) {
        let pressed = event.state() == KeyState::Pressed;
        self.handle_key(event.key_code(), pressed, Event::time(&event));
    }

    /// One key event, xkb keycode `code`, from the seat's own keyboard.
    pub(crate) fn handle_key(&mut self, code: Keycode, pressed: bool, time: InputTime) {
        // A virtual keyboard's keymap gives way to the keyboard's own.
        self.vk_restore();
        self.route_key(code, pressed, time, KeyboardSource::MAIN);
    }

    /// One key event from `source`, under the keymap in force: decode it,
    /// route it, and forward it to the seat or act on it. A virtual
    /// keyboard's key skips the typing aids and the input method's grab.
    pub(crate) fn route_key(
        &mut self,
        code: Keycode,
        pressed: bool,
        time: InputTime,
        source: KeyboardSource,
    ) {
        let from_seat = source == KeyboardSource::MAIN;
        self.settle_pointer();
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        self.key_input.last_key_input = true;
        let keycode = code.raw();
        // The keysyms before this key updates the state, as the route
        // reads them, plus F13 to F24 for those keys whatever the keymap
        // gives them.
        let mut syms: Vec<u32> = keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            // SAFETY: the state is only read here; no reference to it
            // outlives the lock.
            let state = unsafe { xkb.state() };
            state.key_get_syms(code).iter().map(|s| s.raw()).collect()
        });
        if let Some(f) = keys::function_key_for(keycode)
            && !syms.contains(&f)
        {
            syms.push(f);
        }
        let mods_before = keyboard.modifier_state();
        let k = key_press(syms, keys::mask(&mods_before), pressed);
        if from_seat && !self.aids_key(keycode, &k) {
            return;
        }
        if from_seat && !self.mousekeys_key(keycode, pressed, &mods_before) {
            self.aids_after_key();
            return;
        }
        crate::access::echo_key(self, code, &k, time.millis());
        // A rename left behind when a window took the keyboard ends with
        // the name typed so far.
        if k.pressed && self.is_renaming() && !self.desktop_has_keyboard() {
            self.commit_rename();
        }
        let modes = self.key_modes();
        let globals = self.global_binds();
        let mut routed = crate::keyroute::route(&modes, &k, &self.config.keybinds, &globals);
        // The release of a key pressed during a drag: its press reached
        // no client, so neither does the release.
        if let Some(i) = self.key_input.drag_keys.iter().position(|&c| c == keycode) {
            self.key_input.drag_keys.swap_remove(i);
            if !k.pressed {
                routed.route = KeyRoute::Swallow;
            }
        }
        // The release of a key that fired a global shortcut: its program
        // hears deactivated, whatever the modifiers are by now, and no
        // client hears the key.
        if !k.pressed {
            let held: Vec<GlobalHit> = take_held(&mut self.key_input.shortcut_keys, keycode);
            if !held.is_empty() {
                for h in &held {
                    self.global_shortcut(h, false, time.millis());
                }
                routed.route = KeyRoute::Swallow;
            }
        }
        let chord = self
            .key_input
            .chord
            .key(keycode, &k, &self.config.keybinds)
            .filter(|a| crate::keyroute::chord_allowed(&modes, a));
        // During a drag a press spoils the chord held so far, and one
        // Reprise's own keys took spends the modifiers it used: the Alt of
        // Alt+Tab picks no drop action.
        let spends = if modes.dragging && k.pressed {
            self.key_input.chord.spoil();
            crate::keyroute::drag_spends(&routed.route, k.mods)
        } else {
            0
        };
        if k.pressed {
            self.stop_media_repeat();
        } else {
            self.media_key_up(keycode);
        }
        if let Some(arm) = routed.arm_super {
            self.key_input.super_tap_armed = arm;
        }
        if routed.fire_super {
            self.key_input.super_tap_armed = false;
            if !self.key_input.aids.filter.config().sticky_keys {
                self.toggle_start_from_keys();
            }
        }

        // The seat follows every key; only the client's route reaches the
        // focused client. A release goes where its press went. Under the
        // lock, `lockview::key` decides each key: a locker's own surface
        // takes it through the seat, else the lock dialog's field does.
        let forward = match routed.route {
            KeyRoute::Client => {
                let taken = self.key_input.taken.iter().position(|&c| c == keycode);
                match (pressed, taken) {
                    (true, Some(i)) => {
                        self.key_input.taken.swap_remove(i);
                        true
                    }
                    (false, Some(i)) => {
                        self.key_input.taken.swap_remove(i);
                        false
                    }
                    (_, None) => true,
                }
            }
            // An X11 program ends its own side of its drag on Escape.
            KeyRoute::DragCancel if self.x11_drag_running() => true,
            _ => {
                if pressed && !self.key_input.taken.contains(&keycode) {
                    self.key_input.taken.push(keycode);
                } else if !pressed {
                    self.key_input.taken.retain(|&c| c != keycode);
                }
                false
            }
        };
        let serial = SERIAL_COUNTER.next_serial();
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        // The lock, the dialogs and the launcher read the key through the
        // seat's own handle, as it stands after the key. A field the input
        // method serves has its keys go to the input method's grab first;
        // what it does not use comes back through a virtual keyboard
        // (`imefield`).
        let mut field_key = None;
        let to_ime = if pressed {
            let to = from_seat
                && crate::imefield::field_route(&routed.route)
                && self.ime_takes_field_keys();
            if to {
                self.key_input.ime_keys.push(keycode);
            }
            to
        } else {
            let held = self.key_input.ime_keys.iter().position(|&c| c == keycode);
            held.map(|i| self.key_input.ime_keys.swap_remove(i))
                .is_some()
        };
        if to_ime {
            routed.route = KeyRoute::Swallow;
        }
        let route = &routed.route;
        let intercepted = keyboard.input_from_source::<(), _>(
            source,
            self,
            code,
            state,
            serial,
            time,
            |st, mods, h| {
                let forward = match route {
                    _ if to_ime => true,
                    KeyRoute::Lock => crate::lockview::key(st, pressed, &h),
                    KeyRoute::Dialog
                    | KeyRoute::Launcher
                    | KeyRoute::Overview
                    | KeyRoute::StartPanel
                    | KeyRoute::ClipView
                    | KeyRoute::EmojiView
                    | KeyRoute::NotifyReply => {
                        field_key = Some(crate::dialogview::DialogKey::of(mods, &h));
                        false
                    }
                    _ => forward,
                };
                if forward {
                    FilterResult::Forward
                } else {
                    FilterResult::Intercept(())
                }
            },
        );
        let forward = intercepted.is_none();
        if let Some(k) = field_key.as_mut() {
            k.edit = crate::editkeys::from_keyboard(&keyboard, self, code);
        }
        // A key kept from the client still changes its modifiers, unless a
        // capture holds the keyboard.
        if !forward && !modes.captured && keyboard.modifier_state() != mods_before {
            keyboard.advertise_modifier_state(self);
        }
        // The input method's grab hears it too, unless the key reached it.
        if !to_ime && keyboard.modifier_state() != mods_before {
            self.ime_grab_modifiers();
        }
        self.follow_layout();
        if keyboard.modifier_state().shift != mods_before.shift
            && let Some(window) = self.moving_window()
        {
            self.track_move(window);
        }
        if modes.dragging {
            let held = keys::mask(&keyboard.modifier_state());
            self.drag_modifiers(held);
        }

        match routed.route {
            KeyRoute::PowerKey => {
                let a = self.config.power.power_button;
                crate::power::run_action(self, a);
            }
            KeyRoute::SleepKey => {
                let a = self.config.power.sleep_button;
                crate::power::run_action(self, a);
            }
            KeyRoute::Media(a) => self.media_key_down(&a, keycode),
            KeyRoute::VtSwitch(n) => {
                // The switch pauses the keyboard with Ctrl and Alt still
                // down on a receiver: release them there first.
                self.release_capture();
                if let Some(b) = self.backend.as_mut() {
                    b.change_vt(n as i32);
                }
            }
            KeyRoute::Lock => {}
            KeyRoute::CaptureEscape => self.release_capture(),
            KeyRoute::Captured => self.capture_key(keycode - 8, pressed),
            KeyRoute::InhibitEscape => self.suspend_inhibitors(),
            KeyRoute::Interaction(sym) => self.interaction_key(sym),
            KeyRoute::BarDragCancel => self.cancel_interaction(),
            KeyRoute::MoveCancel => self.cancel_move(),
            KeyRoute::SwitcherCancel => {
                // The window under a resting pointer takes it back.
                self.switcher_escape();
                let p = self.pointer_location();
                self.repoint(p, time);
            }
            KeyRoute::SwitcherStep { backwards } => self.switcher_step(backwards),
            KeyRoute::DragCancel => self.cancel_drag(),
            KeyRoute::Swallow => {}
            KeyRoute::Menu(sym) => {
                self.menu_key(sym);
                self.redraw_all();
            }
            KeyRoute::Launcher => {
                if let Some(k) = field_key {
                    self.launcher_key(&k);
                }
            }
            KeyRoute::StartPanel => {
                if let Some(k) = field_key {
                    self.start_panel_key(&k);
                }
            }
            KeyRoute::Overview => {
                if let Some(k) = field_key {
                    self.overview_key(&k);
                }
            }
            KeyRoute::Dialog => {
                if let Some(k) = field_key {
                    self.dialog_key(&k);
                }
            }
            KeyRoute::TrayFlyoutClose => crate::trayflyout::close(self),
            KeyRoute::SnapPicker(sym) => crate::snappickerview::key(self, sym),
            KeyRoute::Taskbar(sym) => self.bar_key(sym, k.mods & keys::MOD_SHIFT != 0),
            KeyRoute::NotifyCenter(sym) => crate::notifycenter::key(self, sym, k.mods),
            KeyRoute::NotifyReply => {
                if let Some(k) = field_key {
                    crate::notifyview::key(self, &k);
                }
            }
            KeyRoute::ClipView => {
                if let Some(k) = field_key {
                    self.clipview_key(&k);
                }
            }
            KeyRoute::EmojiView => {
                if let Some(k) = field_key {
                    self.emoji_key(&k);
                }
            }
            KeyRoute::Region(sym) => self.region_key(sym),
            KeyRoute::CalendarClose => crate::calendarview::close(self),
            KeyRoute::Rename => self.rename_key(code, &k.syms),
            KeyRoute::IconMove(dir) => self.navigate(dir),
            KeyRoute::IconOpen(i) => self.open_selection(i),
            KeyRoute::IconProperties(i) => self.properties_key(i),
            KeyRoute::IconTrash(i) => self.trash_selection(i),
            KeyRoute::IconErase(i) => self.erase_selection(i),
            KeyRoute::IconRename(i) => self.begin_rename(i),
            KeyRoute::IconMenu(focus) => self.open_key_menu(focus),
            KeyRoute::IconSelectAll => self.select_all_icons(),
            KeyRoute::IconCut(i) => self.copy_icons(i, true),
            KeyRoute::IconCopy(i) => self.copy_icons(i, false),
            KeyRoute::IconPaste => self.paste_on_desktop(None, false),
            KeyRoute::IconRefresh => self.rescan_desktop(),
            KeyRoute::Binds(actions) => {
                // A bind past an open menu closes it; Start's own toggles it.
                if modes.menus_open && !actions.contains(&Action::StartMenu) {
                    self.close_menu();
                }
                for a in &actions {
                    self.run_action(a, k.mods);
                }
            }
            KeyRoute::Global(hits) => {
                for h in hits {
                    self.global_shortcut(&h, true, time.millis());
                    self.key_input.shortcut_keys.push((keycode, h));
                }
            }
            KeyRoute::Client => {
                if pressed && let Some(w) = self.focused {
                    self.input_reached(w);
                }
            }
        }
        if spends != 0 {
            self.drag_spend(spends);
        }
        if let Some(a) = chord {
            self.run_action(&a, 0);
        }
        self.aids_after_key();
        self.switcher_modifiers();
        crate::access::touch(self);
    }

    /// The modes the key route reads. A host not ported yet reads as
    /// closed.
    pub fn key_modes(&self) -> Modes {
        let desktop = self.desktop.as_ref();
        Modes {
            can_switch_vt: self.backend.is_some(),
            locked: self.is_locked(),
            captured: self.inputproto.gated(),
            saver_preview: crate::saverview::holds_keys(self),
            keyboard_interaction: self.interaction.is_keyboard(),
            bar_drag: matches!(
                self.interaction,
                Interaction::BarMove { .. } | Interaction::BarWidth { .. }
            ),
            pointer_move: matches!(self.interaction, Interaction::Move { origin: None, .. })
                && self.buttons_down > 0,
            switcher_open: self.switcher.is_open(),
            super_tap_armed: self.key_input.super_tap_armed,
            menus_open: !self.menus.is_empty(),
            dialog_has_keyboard: self.dialog_has_keyboard(),
            dialog_modal: self.dialog_is_modal(),
            launcher_open: self.launcher.is_some(),
            overview_open: self.overview.view.is_some(),
            snap_assist_open: self.snap_assist_open(),
            snap_picker_open: self.snap_picker.is_shown(),
            snap_picker_keys: self.snap_picker.has_keys(),
            start_panel_open: self.start_panel.is_some(),
            notify_center_open: crate::notifycenter::is_open(self),
            notify_reply_focused: crate::notifyview::reply_focused(self),
            clipview_open: self.panels.clipview.is_some(),
            emojiview_open: self.panels.emojiview.is_some(),
            region_open: self.panels.region.is_some(),
            calendar_open: crate::calendarview::is_open(self),
            dragging: self.drag_running(),
            tray_flyout_open: crate::trayflyout::open_on(self).is_some(),
            taskbar_keys: self.bar_shared.keys.is_some(),
            renaming: self.is_renaming(),
            shortcuts_inhibited: self.key_input.inhibit.active(),
            desktop_has_keyboard: self.desktop_has_keyboard(),
            has_icons: desktop.is_some_and(|d| !d.entries.is_empty()),
            selected_icon: desktop.and_then(|d| d.selection.focus),
        }
    }

    /// Whether the desktop holds the keyboard: no window and no layer
    /// surface has it.
    pub fn desktop_has_keyboard(&self) -> bool {
        self.focused.is_none() && self.layers_shell.focused.is_none()
    }

    /// The `[[global_shortcuts]]` chords whose program holds a shortcuts
    /// object; the route matches them after the binds.
    pub fn global_binds(&self) -> Vec<GlobalBind> {
        self.inputproto
            .shortcuts
            .live(&self.config.global_shortcuts)
    }

    /// Tell `hit`'s program its shortcut went active (`active`) or
    /// inactive, at `time_msec`.
    pub fn global_shortcut(&mut self, hit: &GlobalHit, active: bool, time_msec: u32) {
        self.inputproto.shortcuts.send(hit, active, time_msec);
    }

    /// A pointer button went down and was delivered: the Super tap and a
    /// modifier chord held now fire no more, the press used up the
    /// StickyKeys latch, and the next dialog opens without focus
    /// rectangles.
    pub fn keys_saw_button(&mut self) {
        self.key_input.super_tap_armed = false;
        self.key_input.last_key_input = false;
        self.key_input.chord.spoil();
        self.aids_button();
    }

    /// The session starts: take `[accessibility]` and the layouts the
    /// keyboard was made with, and start the media keys' worker.
    pub fn keyboard_setup(&mut self) {
        self.aids_configured();
        self.mousekeys_configured();
        self.layouts_configured();
        self.media_setup();
    }

    /// Run `a`, from a bind, a Start menu row, a Quick Launch button or a
    /// jump list. `bind_mods` is the matched keybind's modifier mask, or 0
    /// for an action run outside a keybind (e.g. a Start menu click).
    pub fn run_action(&mut self, a: &Action, bind_mods: u32) {
        match a {
            Action::Spawn(cmd) => {
                self.spawn_command_activated(cmd);
            }
            Action::Exec { argv, app } => self.spawn_app_activated(argv, app),
            Action::Terminal => {
                let cmd = self.config.terminal.clone();
                self.spawn_command_activated(&cmd);
            }
            Action::Exit => {
                crate::sessionend::begin(self, reprisekit::controls::endsession::Goal::LogOff, None)
            }
            Action::Close => {
                if let Some(w) = self.focused {
                    self.close(w);
                }
            }
            Action::Cycle | Action::Flip => {
                // The keys reach here past an open menu or popup
                // (`keyroute::switch_bind`); it closes first.
                self.close_menu();
                self.close_start_panel();
                self.close_launcher();
                self.close_clipview();
                self.close_emoji();
                if *a == Action::Flip && bind_mods != 0 && self.flip_wanted() {
                    self.open_flip(bind_mods);
                } else if bind_mods != 0
                    && self.switcher_start(self.switch_entries().len()).is_some()
                {
                    self.open_switcher(bind_mods);
                } else {
                    self.cycle_focus();
                }
            }
            Action::Maximize => {
                if let Some(w) = self.focused {
                    let maximized = self.win(w).is_some_and(|x| x.maximized);
                    self.set_maximized(w, !maximized);
                }
            }
            Action::Minimize => {
                if let Some(w) = self.focused {
                    self.set_minimized(w, true);
                }
            }
            Action::StartMenu => self.toggle_start_from_keys(),
            Action::WindowMenu => {
                if let Some(w) = self.focused {
                    self.open_window_menu_below(w, false);
                    // Opened from the keyboard: underline the access keys
                    // straight away and highlight the first row.
                    self.enter_menu_keys();
                    self.menu_hover(0, Some(0));
                }
            }
            Action::ShowDesktop => self.toggle_show_desktop(),
            Action::AlwaysOnTop => {
                if let Some(w) = self.focused
                    && let Some(win) = self.win(w)
                    && !win.is_owned()
                    && !win.override_redirect
                {
                    let on = !win.on_top;
                    self.set_on_top(w, on);
                }
            }
            Action::Tab(backwards) => self.step_tab(*backwards),
            Action::Center => {
                if let Some(w) = self.focused {
                    let p = self.pointer_location();
                    let out = self.output_at(p.x, p.y);
                    self.centre_window(w, out.as_ref());
                }
            }
            Action::ToggleFrame => {
                if let Some(w) = self.focused
                    && let Some(on) = self.win(w).map(|x| !x.frameless)
                {
                    self.set_frameless(w, on);
                }
            }
            Action::Lock => crate::lockview::lock(self),
            Action::Run => {
                let d = crate::dialog::Dialog::run(&self.dialogs.run_history);
                self.open_dialog(d);
            }
            Action::LogOff => self.open_dialog(crate::dialog::Dialog::log_off()),
            Action::TurnOff => {
                let d = crate::dialog::Dialog::turn_off(&self.config.power);
                self.open_dialog(d);
            }
            Action::DoNotDisturb => {
                let on = !self.notify.log.dnd;
                crate::notifyview::set_dnd(self, on);
            }
            Action::NightLight => crate::nightlight::toggle_pause(self),
            Action::Snap(dir) => self.key_snap(*dir),
            Action::Desktop(step) => self.step_desktop(*step, false),
            Action::CarryToDesktop(step) => self.step_desktop(*step, true),
            Action::AllDesktops => {
                if let Some(w) = self.focused
                    && let Some(win) = self.win(w)
                    && !win.is_owned()
                {
                    let on = !win.sticky;
                    self.set_all_desktops(w, on);
                }
            }
            Action::Zoom(step) => self.zoom(*step),
            Action::ColorFilter => self.toggle_color_filter(),
            Action::Launcher => self.toggle_launcher(),
            Action::Overview => self.toggle_overview(),
            Action::SnapLayouts => crate::snappickerview::toggle(self),
            Action::FocusTaskbar => self.focus_taskbar(false),
            Action::FocusTray => self.focus_taskbar(true),
            Action::ClipboardHistory => self.toggle_clipview(),
            Action::EmojiPicker => self.toggle_emoji(),
            Action::NextLayout => self.next_layout(),
            Action::Screenshot => self.screenshot(false),
            Action::ScreenshotWindow => self.screenshot(true),
            Action::ScreenshotContent => self.screenshot_content(),
            Action::ScreenshotRegion => self.open_region(),
            Action::VolumeUp
            | Action::VolumeDown
            | Action::Mute
            | Action::MicMute
            | Action::BrightnessUp
            | Action::BrightnessDown
            | Action::KbdBrightnessUp
            | Action::KbdBrightnessDown
            | Action::PlayPause
            | Action::Next
            | Action::Prev
            | Action::Stop
            | Action::Wifi => self.run_media(a),
        }
    }

    /// Rotate focus: bring the bottom-most visible window to the top. When
    /// the desktop has the keyboard, the most recent window takes it back.
    /// Minimized windows stay down.
    pub fn cycle_focus(&mut self) {
        let mut visible = self.switchable_windows();
        visible.retain(|&w| self.win(w).is_some_and(|x| !x.hidden()));
        let next = if self.keyboard_held() {
            visible.first().filter(|_| visible.len() >= 2)
        } else {
            visible.last()
        };
        if let Some(&w) = next {
            self.focus_window(w);
        }
    }
}

/// The seat keyboard's xkb state as `xkb_state_update_mask` takes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct XkbMask {
    pub depressed: u32,
    pub latched: u32,
    pub locked: u32,
    pub layout_depressed: u32,
    pub layout_latched: u32,
    pub layout_locked: u32,
}

/// The xkb modifier names in the order of the `MOD_*` bits.
const MOD_NAMES: [&str; 8] = [
    xkb::MOD_NAME_SHIFT,
    xkb::MOD_NAME_CAPS,
    xkb::MOD_NAME_CTRL,
    xkb::MOD_NAME_ALT,
    xkb::MOD_NAME_NUM,
    xkb::MOD_NAME_MOD3,
    xkb::MOD_NAME_LOGO,
    xkb::MOD_NAME_ISO_LEVEL3_SHIFT,
];

impl State {
    /// The seat keyboard's xkb mask now, and the keymap's index of each
    /// `MOD_*` bit's modifier.
    pub fn xkb_mask(&mut self) -> Option<(XkbMask, [u32; 8])> {
        let keyboard = self.seat.get_keyboard()?;
        Some(keyboard.with_xkb_state(self, |ctx| {
            let x = ctx.xkb().lock().unwrap();
            // SAFETY: the state and keymap are only read; no reference
            // outlives the lock.
            let (s, keymap) = unsafe { (x.state(), x.keymap()) };
            let mask = XkbMask {
                depressed: s.serialize_mods(xkb::STATE_MODS_DEPRESSED),
                latched: s.serialize_mods(xkb::STATE_MODS_LATCHED),
                locked: s.serialize_mods(xkb::STATE_MODS_LOCKED),
                layout_depressed: s.serialize_layout(xkb::STATE_LAYOUT_DEPRESSED),
                layout_latched: s.serialize_layout(xkb::STATE_LAYOUT_LATCHED),
                layout_locked: s.serialize_layout(xkb::STATE_LAYOUT_LOCKED),
            };
            (mask, MOD_NAMES.map(|n| keymap.mod_get_index(n)))
        }))
    }

    /// The seat keyboard's keymap as xkb text.
    pub fn keymap_text(&mut self) -> Option<String> {
        let keyboard = self.seat.get_keyboard()?;
        Some(keyboard.with_xkb_state(self, |ctx| {
            let x = ctx.xkb().lock().unwrap();
            // SAFETY: the keymap is only read; no reference to it outlives
            // the lock.
            unsafe { x.keymap() }.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
        }))
    }

    /// Put `mask` on the seat keyboard's xkb state and tell the focused
    /// client.
    ///
    /// Smithay sets a modifier state only from its named flags, with
    /// nothing latched and the layout dropped, so the flags `mask` makes go
    /// in first (they are what the binds and the client read) and the
    /// exact mask, latched part and layout included, goes on the xkb state
    /// after them.
    pub fn set_xkb_mask(&mut self, m: XkbMask) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let update = |s: &mut xkb::State| {
            s.update_mask(
                m.depressed,
                m.latched,
                m.locked,
                m.layout_depressed,
                m.layout_latched,
                m.layout_locked,
            );
        };
        let flags = keyboard.with_xkb_state(self, |ctx| {
            let x = ctx.xkb().lock().unwrap();
            // SAFETY: the keymap is only read; the scratch state is ours.
            let mut scratch = xkb::State::new(unsafe { x.keymap() });
            update(&mut scratch);
            let mut flags = ModifiersState::default();
            flags.update_with(&scratch);
            flags
        });
        keyboard.set_modifier_state(flags);
        keyboard.with_xkb_state(self, |ctx| {
            let x = ctx.xkb().lock().unwrap();
            // SAFETY: the clone shares the keyboard's own state, as the
            // update must, and is dropped before the lock.
            let mut state = unsafe { x.state() }.clone();
            update(&mut state);
        });
        keyboard.advertise_modifier_state(self);
    }
}

/// How far one arrow press moves the cursor (and with it the window or its
/// edge) in a keyboard move or size.
pub const KEY_STEP: i32 = 9;

/// The edge an arrow key picks in a keyboard size, when that axis has none
/// yet, and the direction it nudges the cursor in: (dx, dy) in units of
/// `KEY_STEP`.
#[expect(non_upper_case_globals)] // xkb keysym names in patterns
pub fn arrow_for(sym: u32) -> Option<(Edges, (i32, i32))> {
    use keys::*;
    Some(match sym {
        KEY_Left | KEY_KP_Left => (Edges::LEFT, (-1, 0)),
        KEY_Right | KEY_KP_Right => (Edges::RIGHT, (1, 0)),
        KEY_Up | KEY_KP_Up => (Edges::TOP, (0, -1)),
        KEY_Down | KEY_KP_Down => (Edges::BOTTOM, (0, 1)),
        _ => return None,
    })
}

/// The edges a keyboard size works on after the arrow `picked` is pressed
/// with `current` edges: an axis with no edge yet takes the arrow's side;
/// an axis that has one keeps it (the arrow then moves that edge).
pub fn pick_edge(current: Edges, picked: Edges) -> Edges {
    let horizontal = Edges::LEFT | Edges::RIGHT;
    let vertical = Edges::TOP | Edges::BOTTOM;
    let axis = if picked.intersects(horizontal) {
        horizontal
    } else {
        vertical
    };
    if current.intersects(axis) {
        current
    } else {
        current | picked
    }
}

/// The edges a pointer move of (dx, dy) picks for a keyboard size that has
/// none yet: the dominant direction's side, None while it has not moved.
pub fn edge_for_motion(dx: f64, dy: f64) -> Option<Edges> {
    if dx == 0.0 && dy == 0.0 {
        return None;
    }
    Some(if dx.abs() >= dy.abs() {
        if dx < 0.0 { Edges::LEFT } else { Edges::RIGHT }
    } else if dy < 0.0 {
        Edges::TOP
    } else {
        Edges::BOTTOM
    })
}

/// The point on `r`'s edge (its side's midpoint, or the corner for two
/// edges) where the cursor rests during a keyboard size of `edges`.
pub fn edge_anchor(r: Rect, edges: Edges) -> (i32, i32) {
    let x = if edges.contains(Edges::LEFT) {
        r.x
    } else if edges.contains(Edges::RIGHT) {
        r.right() - 1
    } else {
        r.x + r.width / 2
    };
    let y = if edges.contains(Edges::TOP) {
        r.y
    } else if edges.contains(Edges::BOTTOM) {
        r.bottom() - 1
    } else {
        r.y + r.height / 2
    };
    (x, y)
}

impl State {
    /// The window menu's Move: the cursor jumps to the middle of the title
    /// bar and the window follows it, from the pointer or the arrow keys,
    /// until a click or Enter commits or Escape puts it back.
    pub fn begin_key_move(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.mapped || win.maximized || win.fullscreen {
            return;
        }
        let r = win.frame_rect();
        let cap = win.geom.caption;
        let m = &self.config.theme.metrics;
        self.warp_cursor(
            r.x + cap.x + cap.width / 2,
            r.y + cap.y + m.caption_height / 2,
        );
        self.begin_move(w, false);
        // The click that picked Move may still be held; the grab is the
        // caption point the cursor jumped to.
        let p = self.pointer_location();
        if let Interaction::Move { origin, press, .. } = &mut self.interaction {
            *origin = Some(r);
            *press = (p.x, p.y);
        }
    }

    /// The window menu's Size: the cursor jumps to the window's middle; the
    /// first arrow key (or pointer move) picks the edge, then the edge
    /// follows the cursor until a click or Enter commits or Escape puts the
    /// old size back.
    pub fn begin_key_size(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.mapped || win.maximized || win.fullscreen {
            return;
        }
        let r = win.frame_rect();
        self.warp_cursor(r.x + r.width / 2, r.y + r.height / 2);
        self.start_resize(w, Edges::NONE, true);
    }

    /// Move the cursor to a layout point, then run the same motion
    /// handling a pointer event would: the interaction follows it and the
    /// client under it hears the motion.
    pub fn warp_cursor(&mut self, x: i32, y: i32) {
        self.pointer_to((f64::from(x), f64::from(y)).into(), InputTime::now());
    }

    /// A key while a keyboard move or size is under way: the arrows nudge
    /// the cursor (picking a side first, for a size with none), Enter
    /// commits, Escape restores the rect it started from. Every other key
    /// is swallowed.
    #[expect(non_upper_case_globals)] // xkb keysym names in patterns
    pub fn interaction_key(&mut self, sym: u32) {
        use keys::*;
        let arrow = arrow_for(sym);
        let p = self.pointer_location();
        let (cx, cy) = (p.x as i32, p.y as i32);
        match self.interaction {
            Interaction::Move {
                origin: Some(_), ..
            } => match sym {
                KEY_Escape => self.cancel_interaction(),
                KEY_Return | KEY_KP_Enter => self.commit_key_interaction(),
                _ => {
                    if let Some((_, (dx, dy))) = arrow {
                        self.warp_cursor(cx + dx * KEY_STEP, cy + dy * KEY_STEP);
                    }
                }
            },
            Interaction::Resize {
                window,
                edges,
                keyboard: Some(_),
                ..
            } => match sym {
                KEY_Escape => self.cancel_interaction(),
                KEY_Return | KEY_KP_Enter => self.commit_key_interaction(),
                _ => {
                    let Some((side, (dx, dy))) = arrow else {
                        return;
                    };
                    let picked = pick_edge(edges, side);
                    if picked != edges {
                        // A new side: the cursor jumps onto it, dragging
                        // nothing, and the drag measures from there.
                        let Some(r) = self.win(window).map(|w| w.frame_rect()) else {
                            return;
                        };
                        let (ax, ay) = edge_anchor(r, picked);
                        if let Some(pointer) = self.seat.get_pointer() {
                            pointer.set_location((f64::from(ax), f64::from(ay)).into());
                        }
                        self.set_resize_edges(picked);
                        self.redraw_all();
                        return;
                    }
                    self.warp_cursor(cx + dx * KEY_STEP, cy + dy * KEY_STEP);
                }
            },
            _ => {}
        }
        self.redraw_all();
    }

    /// Enter on a keyboard move or size: commit it, then hand the pointer
    /// to whatever is under the cursor.
    fn commit_key_interaction(&mut self) {
        self.end_interaction();
        let p = self.pointer_location();
        self.repoint(p, InputTime::now());
    }

    /// Change the edges of the resize under way, re-anchoring the drag at
    /// the cursor's current position and the window's current rect.
    pub fn set_resize_edges(&mut self, new: Edges) {
        let p = self.pointer_location();
        let Interaction::Resize { window, .. } = self.interaction else {
            return;
        };
        let Some(rect) = self.win(window).map(|w| w.frame_rect()) else {
            return;
        };
        if let Interaction::Resize {
            edges,
            start,
            cx,
            cy,
            ..
        } = &mut self.interaction
        {
            *edges = new;
            *start = rect;
            *cx = p.x;
            *cy = p.y;
        }
        // The client's commits anchor on the edges actually dragged.
        if let Some(win) = self.win_mut(window)
            && let Some(a) = win.resize_anchor.as_mut()
        {
            a.edges = new;
            a.right = rect.right();
            a.bottom = rect.bottom();
        }
        self.set_cursor(new.cursor_name());
    }

    /// Put a keyboard move or size `was` back where it started: the rect,
    /// a snap the move had undone with the zone's size, and the snap group
    /// and zone it left (`Snapping::cancel_move`). Anything else is left
    /// alone.
    pub fn restore_keyboard(&mut self, was: Interaction) {
        let (window, rect, snap) = match was {
            Interaction::Move {
                window,
                origin: Some(origin),
                was_snapped,
                ..
            } => {
                self.wm.snap.cancel_move(window);
                // A snap still in place needs nothing but the position.
                let snapped = self.win(window).is_some_and(|w| w.snapped.is_some());
                (window, origin, was_snapped.filter(|_| !snapped))
            }
            Interaction::Resize {
                window,
                keyboard: Some(origin),
                ..
            } => (window, origin, None),
            _ => return,
        };
        let Some(chrome) = self.win(window).map(|w| w.chrome()) else {
            return;
        };
        self.move_to(window, rect.x, rect.y);
        let resize = matches!(was, Interaction::Resize { .. });
        if resize || snap.is_some() {
            let m = &self.config.theme.metrics;
            let (cw, ch) =
                FrameGeometry::client_size_for_total(m, rect.width, rect.height, false, chrome);
            if snap.is_some()
                && let Some(win) = self.win_mut(window)
            {
                win.resize_anchor = None;
            }
            self.request_size(window, cw, ch);
            if snap.is_some()
                && let Some(win) = self.win_mut(window)
            {
                win.snapped = snap;
            }
        }
    }
}

/// Take every global shortcut keycode `code` fired out of `held`.
fn take_held(held: &mut Vec<(u32, GlobalHit)>, code: u32) -> Vec<GlobalHit> {
    let (gone, kept) = std::mem::take(held).into_iter().partition(|h| h.0 == code);
    *held = kept;
    gone.into_iter().map(|h: (u32, GlobalHit)| h.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_press_masks_caps_and_num_lock_and_lowercases() {
        let k = key_press(
            vec![keys::KEY_A],
            keys::MOD_SHIFT | keys::MOD_CAPS | keys::MOD_MOD2,
            true,
        );
        assert_eq!(k.syms, vec![keys::KEY_A]);
        assert_eq!(k.lower, vec![keys::KEY_a]);
        assert_eq!(k.mods, keys::MOD_SHIFT);
        assert!(k.pressed);
    }

    #[test]
    fn keyboard_size_picks_a_side_per_axis_then_keeps_it() {
        let none = Edges::NONE;
        assert_eq!(pick_edge(none, Edges::RIGHT), Edges::RIGHT);
        // The horizontal side is taken: Left now moves the right edge.
        assert_eq!(pick_edge(Edges::RIGHT, Edges::LEFT), Edges::RIGHT);
        // The other axis is still free and adds a corner.
        assert_eq!(
            pick_edge(Edges::RIGHT, Edges::TOP),
            Edges::RIGHT | Edges::TOP
        );
        assert_eq!(edge_for_motion(0.0, 0.0), None);
        assert_eq!(edge_for_motion(-5.0, 2.0), Some(Edges::LEFT));
        assert_eq!(edge_for_motion(1.0, 4.0), Some(Edges::BOTTOM));
        let r = Rect::new(10, 20, 100, 50);
        assert_eq!(edge_anchor(r, Edges::RIGHT), (109, 45));
        assert_eq!(edge_anchor(r, Edges::LEFT | Edges::TOP), (10, 20));
        assert_eq!(arrow_for(keys::KEY_Up), Some((Edges::TOP, (0, -1))));
        assert_eq!(arrow_for(keys::KEY_a), None);
    }

    #[test]
    fn only_a_move_or_size_from_the_menu_owns_the_keyboard() {
        let w = WindowId::next();
        let r = Rect::new(0, 0, 10, 10);
        let mv = |origin| Interaction::Move {
            window: w,
            dx: 0.0,
            dy: 0.0,
            origin,
            was_snapped: None,
            start: crate::input::MoveStart::default(),
            press: (0.0, 0.0),
            snap: None,
            armed: false,
            bar: false,
        };
        let size = |keyboard| Interaction::Resize {
            window: w,
            edges: Edges::NONE,
            start: r,
            cx: 0.0,
            cy: 0.0,
            keyboard,
        };
        assert!(mv(Some(r)).is_keyboard());
        assert!(!mv(None).is_keyboard());
        assert!(size(Some(r)).is_keyboard());
        assert!(!size(None).is_keyboard());
        assert!(!Interaction::Idle.is_keyboard());
    }

    #[test]
    fn a_release_takes_only_its_own_global_shortcuts() {
        let hit = |id: &str| GlobalHit {
            app: "talk".into(),
            id: id.into(),
        };
        let mut held = vec![(40, hit("mute")), (41, hit("deafen")), (40, hit("ptt"))];
        assert_eq!(take_held(&mut held, 40), vec![hit("mute"), hit("ptt")]);
        assert_eq!(held, vec![(41, hit("deafen"))]);
        assert!(take_held(&mut held, 40).is_empty());
    }
}
