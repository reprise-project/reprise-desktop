//! The seat: keyboard, pointer and input method.

use crate::{Event, Host, KeyInput, MAIN, Mods, PointerInput, Shell, WinId, textinput};
use reprisekit::textedit::{EditKey, key_for};
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3;
use smithay_client_toolkit::{
    dispatch2::Dispatch2,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_keyboard, wl_pointer, wl_seat, wl_surface},
};
use zeroize::Zeroize;

impl Host {
    fn key_input(&self, event: &KeyEvent) -> KeyInput {
        let sym = event.keysym.raw();
        let utf8 = event.utf8.clone();
        let edit = key_for(sym, self.mods.ctrl, self.mods.shift, utf8.as_deref());
        KeyInput {
            sym,
            utf8,
            mods: self.seat_mods(),
            edit,
        }
    }

    fn seat_mods(&self) -> Mods {
        Mods {
            ctrl: self.mods.ctrl,
            shift: self.mods.shift,
            alt: self.mods.alt,
        }
    }

    fn echo_key(&self, event: &KeyEvent, pressed: bool) {
        let Some(echo) = &self.key_echo else {
            return;
        };
        if !self.slots.iter().any(|s| s.access.state.active) {
            return;
        }
        let m = &self.mods;
        let mods = u32::from(m.shift)
            | u32::from(m.caps_lock) << 1
            | u32::from(m.ctrl) << 2
            | u32::from(m.alt) << 3
            | u32::from(m.num_lock) << 4
            | u32::from(m.logo) << 6;
        let typed = event
            .utf8
            .as_deref()
            .filter(|t| t.chars().all(|c| !c.is_control()) && !t.is_empty());
        // What goes into a password field stays in the program; the reader
        // hears the bullets the field reports instead.
        let secret = self
            .focus
            .and_then(|f| self.slots.iter().find(|s| s.id == f))
            .is_some_and(|s| s.access.state.secret);
        if secret && typed.is_some() {
            return;
        }
        let text = match typed {
            Some(t) => t.to_string(),
            None => event
                .keysym
                .name()
                .unwrap_or_default()
                .trim_start_matches("XK_")
                .to_string(),
        };
        echo.send(reprisebus::a11y::KeyNote {
            pressed,
            keysym: event.keysym.raw(),
            keycode: event.raw_code + 8,
            mods,
            time: event.time,
            text: text.into(),
            is_text: typed.is_some(),
        });
    }

    fn key(&mut self, mut event: KeyEvent) {
        self.echo_key(&event, true);
        let input = self.key_input(&event);
        event.utf8.zeroize();
        self.ctx.caret_on = true;
        self.key_last = true;
        let win = self.focus.unwrap_or(MAIN);
        let alt = self.mods.alt || matches!(event.keysym, Keysym::Alt_L | Keysym::Alt_R);
        if let Some(s) = self.slot_mut(win) {
            let was = s.cues.keys;
            s.cues.key(alt);
            s.dirty |= s.cues.keys != was;
        }
        self.send(win, Event::Key(input));
    }

    fn key_up(&mut self, mut event: KeyEvent) {
        self.echo_key(&event, false);
        let input = self.key_input(&event);
        event.utf8.zeroize();
        let win = self.focus.unwrap_or(MAIN);
        self.send(win, Event::KeyUp(input));
    }

    pub(crate) fn pointer(&mut self, win: WinId, input: PointerInput) {
        self.send(win, Event::Pointer(input));
    }

    /// Binds the keyboard `new_capability` left due, once a window has shown its first frame:
    /// the keymap and compose table cost a few milliseconds, and the compositor sends `enter`
    /// to a keyboard bound while one of the client's windows has focus.
    pub(crate) fn bind_keyboard(&mut self, conn: &Connection, qh: &QueueHandle<Self>) {
        if !self.keyboard_due || !self.slots.iter().any(|s| s.mapped) {
            return;
        }
        let Some(seat) = self.seat.clone() else {
            return;
        };
        self.keyboard_due = false;
        let _ = conn.flush();
        let keyboard = self
            .seat_state
            .get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                self.loop_handle.clone(),
                Box::new(|host: &mut Host, _, event| host.key(event)),
            )
            .expect("keyboard");
        self.keyboard = Some(keyboard);
    }

    /// Window `id` lost the keyboard.
    fn lose_keyboard(&mut self, id: WinId) {
        if self.focus == Some(id) {
            self.focus = None;
        }
        if let Some(s) = self.slot_mut(id) {
            if let Some(a) = s.access.adapter.as_mut() {
                a.update_window_focus_state(false);
            }
            s.dirty |= s.cues.blur();
        }
        self.redraw_if_active_shows(id);
        self.send(id, Event::Focus(false));
    }

    /// After a turn with a deferred keyboard leave: the focused window lost the keyboard
    /// unless it went to one of its own surfaces.
    pub(crate) fn settle_keyboard(&mut self) {
        if !std::mem::take(&mut self.kb_settle) {
            return;
        }
        if let Some(f) = self.focus
            && self.kb_on.as_ref().map(|(_, id)| *id) != Some(f)
        {
            self.lose_keyboard(f);
        }
    }

    /// Redraw flyout `id`, which just gained or lost the keyboard, so its
    /// default button follows. A toplevel redraws on the configure that
    /// changes its `activated`.
    fn redraw_if_active_shows(&mut self, id: WinId) {
        let flyout = self
            .slots
            .iter()
            .any(|s| s.id == id && matches!(s.window, Shell::Layer(_)));
        if flyout {
            self.mark_dirty(id);
        }
    }

    fn text_input_event(&mut self, event: zwp_text_input_v3::Event) {
        use zwp_text_input_v3::Event as E;
        let entered = match &event {
            E::Enter { surface } => self.slot_of(surface),
            _ => None,
        };
        let Some(t) = self.text_input.as_mut() else {
            return;
        };
        match event {
            E::Enter { .. } => {
                if let Some(id) = entered {
                    t.enter(id);
                    self.mark_dirty(id);
                }
            }
            E::Leave { .. } => {
                if let Some((id, true)) = t.leave() {
                    self.mark_dirty(id);
                }
            }
            E::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => t.preedit_string(text, cursor_begin, cursor_end),
            E::CommitString { text } => t.commit_string(text),
            E::DeleteSurroundingText {
                before_length,
                after_length,
            } => t.delete_surrounding(before_length, after_length),
            E::Done { .. } => {
                let Some(win) = t.entered else {
                    return;
                };
                let keys = t.done();
                self.ctx.caret_on = true;
                for key in keys {
                    let utf8 = match &key {
                        EditKey::Insert(s) => Some(s.clone()),
                        _ => None,
                    };
                    let input = KeyInput {
                        sym: 0,
                        utf8,
                        mods: Mods::default(),
                        edit: Some(key),
                    };
                    self.send(win, Event::Key(input));
                }
                self.mark_dirty(win);
            }
            _ => {}
        }
    }
}

impl SeatHandler for Host {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.seat = Some(seat.clone());
            self.keyboard_due = true;
            if let Some(manager) = &self.text_input_manager
                && self.text_input.is_none()
            {
                let ti = manager.get_text_input(&seat, qh, TextInputOf);
                self.text_input = Some(textinput::TextInput::new(ti));
            }
            if let Some(manager) = &self.data_device_manager
                && self.data_device.is_none()
            {
                self.data_device = Some(manager.get_data_device(qh, &seat));
            }
            self.primary.attach(qh, &seat);
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            self.keyboard_due = false;
        }
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
            if let Some(t) = self.text_input.take() {
                t.destroy();
            }
        }
        if capability == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.release();
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for Host {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        serial: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.last_serial = serial;
        let Some(id) = self.slot_of(surface).or_else(|| self.menu_owner(surface)) else {
            return;
        };
        self.kb_on = Some((surface.clone(), id));
        // From a window to its menu or back: the window keeps the keyboard.
        if self.focus == Some(id) {
            return;
        }
        self.focus = Some(id);
        if let Some(s) = self.slot_mut(id)
            && let Some(a) = s.access.adapter.as_mut()
        {
            a.update_window_focus_state(true);
        }
        self.redraw_if_active_shows(id);
        self.send(id, Event::Focus(true));
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let was_on = match self.kb_on.take() {
            Some((on, id)) if &on == surface => Some(id),
            other => {
                self.kb_on = other;
                None
            }
        };
        let window = self.slot_of(surface);
        // A menu popup, open or just destroyed, or a window handing the keyboard to its menu:
        // where it went is known once the turn's events are in.
        let to_menu = window.is_some_and(|id| self.has_menu(id));
        if window.is_none() && was_on.is_some() || to_menu {
            self.kb_settle = true;
            return;
        }
        if let Some(id) = window {
            self.lose_keyboard(id);
        }
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.last_serial = serial;
        self.input_serial = serial;
        self.key(event);
    }
    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key(event);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.key_up(event);
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.mods = modifiers;
    }
}

impl PointerHandler for Host {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let local = (event.position.0 as i32, event.position.1 as i32);
            let Some((id, (x, y))) = self
                .slot_of(&event.surface)
                .map(|id| (id, local))
                .or_else(|| self.menu_point(&event.surface, local))
            else {
                continue;
            };
            self.pointer_pos = (x, y);
            match event.kind {
                PointerEventKind::Enter { .. } => self.pointer_in = Some(id),
                PointerEventKind::Leave { .. } if self.pointer_in == Some(id) => {
                    self.pointer_in = None
                }
                _ => {}
            }
            let input = match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    PointerInput::Motion { x, y }
                }
                PointerEventKind::Leave { .. } => PointerInput::Leave,
                PointerEventKind::Press {
                    button,
                    serial,
                    time,
                } => {
                    self.last_serial = serial;
                    self.press_serial = serial;
                    self.input_serial = serial;
                    self.press_time = time;
                    self.key_last = false;
                    PointerInput::Press {
                        x,
                        y,
                        button,
                        time,
                        mods: self.seat_mods(),
                    }
                }
                PointerEventKind::Release { button, .. } => PointerInput::Release {
                    x,
                    y,
                    button,
                    mods: self.seat_mods(),
                },
                PointerEventKind::Axis { vertical, .. } => {
                    let rows = if vertical.discrete != 0 {
                        vertical.discrete * reprisekit::pointer::WHEEL_ROWS
                    } else {
                        (vertical.absolute / 5.0) as i32
                    };
                    if rows == 0 {
                        continue;
                    }
                    PointerInput::Scroll { x, y, rows }
                }
            };
            self.pointer(id, input);
        }
    }
}

struct TextInputOf;

impl Dispatch2<zwp_text_input_v3::ZwpTextInputV3, Host> for TextInputOf {
    fn event(
        &self,
        host: &mut Host,
        _: &zwp_text_input_v3::ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        host.text_input_event(event);
    }
}
