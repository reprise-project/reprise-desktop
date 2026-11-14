//! The computer's own keys in lock mode: a private xkb state decodes them for the console's
//! lock, so no key, modifier or lock key of theirs reaches the seat the remote side types on.

use smithay::backend::input::InputTime;
use smithay::input::keyboard::{KeyboardSource, Keycode, ModifiersState, xkb};

use crate::compositor::State;
use crate::heldkeys::Owner;
use crate::inputproto::InputSource;
use crate::keyroute::{KeyRoute, Modes};
use crate::lockview::Origin;

#[derive(Default)]
pub struct Keys {
    xkb: Option<xkb::State>,
    /// Caps Lock in `xkb`, for the screen's notice.
    pub caps_lock: bool,
}

/// Lock mode begins: the private state takes over the keys the seat holds and its lock keys,
/// then the seat lets go of them.
pub fn begin(state: &mut State) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let pressed = keyboard.pressed_keys();
    let private = private_state(state).map(|mut s| {
        if let Some((m, _)) = state.xkb_mask() {
            s.update_mask(0, 0, m.locked, 0, 0, m.layout_locked);
        }
        for &code in &pressed {
            s.update_key(code, xkb::KeyDirection::Down);
        }
        s
    });
    state.console.keys.caps_lock = private.as_ref().is_some_and(caps_on);
    state.console.keys.xkb = private;
    let_go(state);
}

/// Lock mode ends: the computer's keys held now come back up unheard, as the seat never had
/// them, and the seat lets go of the remote's.
pub fn end(state: &mut State) {
    state.console.keys = Keys::default();
    let_go(state);
}

/// The seat lets go of every key, the typing aids' keys and modifiers, the Ctrl tap and the mouse
/// keys, so a client it focuses later hears none held.
fn let_go(state: &mut State) {
    let Some(keyboard) = state.seat.get_keyboard() else {
        return;
    };
    let pressed = keyboard.pressed_keys();
    keyboard.release_source(state, KeyboardSource::MAIN);
    let now = InputTime::now();
    for code in &pressed {
        if let Some(Owner::Global(hits)) = state.key_input.held.release(code.raw()) {
            for h in &hits {
                state.global_shortcut(h, false, now.millis());
            }
        }
    }
    state.key_input.held.forget_held_back();
    state.stop_field_repeat();
    state.stop_media_repeat();
    state.aids_reset();
    state.key_input.ctrl_tap = Default::default();
    state.mousekeys_forget();
    state.eis_modifiers_changed();
}

/// The keyboard's own keymap: the seat's, or `[keyboard]`'s while a virtual keyboard's is on it.
fn private_state(state: &mut State) -> Option<xkb::State> {
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = if state.ime.vk.foreign() {
        let kb = &state.config.keyboard;
        let options = Some(kb.options.clone()).filter(|o| !o.is_empty());
        xkb::Keymap::new_from_names(
            &ctx,
            &kb.rules,
            &kb.model,
            &kb.layout,
            &kb.variant,
            options,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
    } else {
        let text = state.keymap_text()?;
        xkb::Keymap::new_from_string(
            &ctx,
            text,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
    };
    let keymap = keymap.or_else(|| {
        log::error!("lock mode: the keymap does not compile; the computer's keys type as us");
        xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
    })?;
    Some(xkb::State::new(&keymap))
}

fn caps_on(s: &xkb::State) -> bool {
    s.mod_name_is_active(xkb::MOD_NAME_CAPS, xkb::STATE_MODS_EFFECTIVE)
}

/// One of the computer's own keys in lock mode. The power, sleep, media and VT keys act as at
/// the lock; every other key goes to the console's lock screen.
pub fn key(state: &mut State, code: Keycode, pressed: bool) {
    let Some(mut s) = state.console.keys.xkb.take() else {
        return;
    };
    // The layout the seat has locked decodes here too, as the screen's badge shows it.
    let group = state.key_input.layouts.current;
    s.update_mask(
        s.serialize_mods(xkb::STATE_MODS_DEPRESSED),
        s.serialize_mods(xkb::STATE_MODS_LATCHED),
        s.serialize_mods(xkb::STATE_MODS_LOCKED),
        0,
        0,
        group,
    );
    let syms = s.key_get_syms(code).iter().map(|k| k.raw()).collect();
    let mut mods = ModifiersState::default();
    mods.update_with(&s);
    let press = crate::keyboard::key_press(syms, crate::keys::mask(&mods), pressed);
    let modes = Modes {
        locked: true,
        can_switch_vt: state.backend.is_some(),
        ..Modes::default()
    };
    let route = crate::keyroute::route(&modes, &press, &state.config.keybinds, &[]).route;
    let direction = if pressed {
        xkb::KeyDirection::Down
    } else {
        xkb::KeyDirection::Up
    };
    s.update_key(code, direction);
    let caps = caps_on(&s);
    let caps_changed = std::mem::replace(&mut state.console.keys.caps_lock, caps) != caps;
    // What the lock does next (an unlock, lock mode's end) is no input of the computer's and
    // reaches the seat.
    state.with_source(InputSource::Other, |st| {
        if !pressed {
            st.media_key_up(code.raw());
        }
        match route {
            KeyRoute::PowerKey => {
                let a = st.config.power.power_button;
                crate::power::run_action(st, a, Origin::Console);
            }
            KeyRoute::SleepKey => {
                let a = st.config.power.sleep_button;
                crate::power::run_action(st, a, Origin::Console);
            }
            KeyRoute::Media(a) => st.media_key_down(&a, code.raw()),
            KeyRoute::VtSwitch(n) => {
                if let Some(b) = st.backend.as_mut() {
                    b.change_vt(n as i32);
                }
            }
            _ => {
                if caps_changed {
                    crate::lockview::redraw(st);
                }
                crate::lockview::key_with(st, pressed, &s, code);
            }
        }
    });
    if state.console.keys.xkb.is_none() && crate::console::remote_side(state) {
        state.console.keys.xkb = Some(s);
    }
}
