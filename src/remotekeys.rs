//! RemoteDesktop keys without libei: NotifyKeyboardKeycode/Keysym through a per-session
//! `reprise_keys_v1` object. A keysym becomes the key that gives it in the seat's keymap.

use smithay::backend::input::InputTime;
use smithay::input::keyboard::{Keycode, xkb};

use crate::compositor::State;
use crate::eis::{KEYS, evdev_key, track};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeysId(u64);

struct Keys {
    id: KeysId,
    held: Vec<u32>,
    /// The key each keysym went down on, so its release finds it after a
    /// layout change.
    syms: Vec<(u32, u32)>,
}

#[derive(Default)]
pub struct KeysState {
    objects: Vec<Keys>,
    next_id: u64,
}

impl KeysState {
    pub fn create(&mut self) -> KeysId {
        self.next_id += 1;
        let id = KeysId(self.next_id);
        self.objects.push(Keys {
            id,
            held: Vec::new(),
            syms: Vec::new(),
        });
        id
    }

    fn get_mut(&mut self, id: KeysId) -> Option<&mut Keys> {
        self.objects.iter_mut().find(|k| k.id == id)
    }
}

impl State {
    pub fn keys_key(&mut self, id: KeysId, code: u32, pressed: bool) {
        let Some(k) = self.inputproto.keys.get_mut(id) else {
            return;
        };
        if !track(&mut k.held, KEYS, code, pressed) {
            log::debug!("keys: ignoring keycode {code}");
            return;
        }
        self.keys_inject(code, pressed);
    }

    pub fn keys_keysym(&mut self, id: KeysId, sym: u32, pressed: bool) {
        let Some(k) = self.inputproto.keys.get_mut(id) else {
            return;
        };
        let down = k.syms.iter().position(|&(s, _)| s == sym);
        let code = match (pressed, down) {
            (false, Some(i)) => k.syms.swap_remove(i).1,
            (false, None) => return,
            (true, Some(i)) => k.syms[i].1,
            (true, None) => {
                let Some(code) = self.keycode_for_sym(sym) else {
                    log::debug!("keys: no key gives keysym {sym:#x}");
                    return;
                };
                code
            }
        };
        let Some(k) = self.inputproto.keys.get_mut(id) else {
            return;
        };
        if !track(&mut k.held, KEYS, code, pressed) {
            return;
        }
        if pressed && down.is_none() {
            k.syms.push((sym, code));
        }
        self.keys_inject(code, pressed);
    }

    pub fn keys_destroy(&mut self, id: KeysId) {
        let keys = &mut self.inputproto.keys.objects;
        let Some(i) = keys.iter().position(|k| k.id == id) else {
            return;
        };
        for code in keys.swap_remove(i).held {
            self.keys_inject(code, false);
        }
    }

    /// One key through the physical keyboard's path, past an InputCapture
    /// as a libei sender's keys go.
    fn keys_inject(&mut self, code: u32, pressed: bool) {
        crate::idle::notify_activity(self);
        let was = std::mem::replace(&mut self.inputproto.injecting, true);
        self.handle_key(evdev_key(code), pressed, InputTime::now());
        self.inputproto.injecting = was;
    }

    fn keycode_for_sym(&mut self, sym: u32) -> Option<u32> {
        self.key_for_sym(sym).map(|(code, _)| code)
    }

    pub(crate) fn key_for_sym(&mut self, sym: u32) -> Option<(u32, u32)> {
        let keyboard = self.seat.get_keyboard()?;
        keyboard.with_xkb_state(self, |ctx| {
            let xkb = ctx.xkb().lock().unwrap();
            // SAFETY: the state is only read here; no reference to it
            // outlives the lock.
            let layout = unsafe { xkb.state() }.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE);
            // SAFETY: as for the state.
            let keymap = unsafe { xkb.keymap() };
            key_in(keymap, layout, sym)
        })
    }
}

#[cfg(test)]
fn keycode_in(keymap: &xkb::Keymap, layout: u32, sym: u32) -> Option<u32> {
    key_in(keymap, layout, sym).map(|(code, _)| code)
}

fn key_in(keymap: &xkb::Keymap, layout: u32, sym: u32) -> Option<(u32, u32)> {
    let codes = keymap.min_keycode().raw()..=keymap.max_keycode().raw();
    let levels = codes
        .clone()
        .map(|c| keymap.num_levels_for_key(Keycode::new(c), layout))
        .max()?;
    (0..levels).find_map(|level| {
        codes.clone().find_map(|c| {
            let kc = Keycode::new(c);
            (keymap.key_get_syms_by_level(kc, layout, level).iter())
                .any(|s| s.raw() == sym)
                .then(|| c.checked_sub(8))
                .flatten()
                .map(|code| (code, level))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    fn us() -> xkb::Keymap {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(&ctx, "evdev", "pc105", "us", "", None, 0)
            .expect("the us keymap compiles")
    }

    #[test]
    fn a_keysym_finds_its_key_at_the_lowest_level() {
        let km = us();
        assert_eq!(keycode_in(&km, 0, keys::KEY_v), Some(47));
        assert_eq!(keycode_in(&km, 0, keys::KEY_Control_L), Some(29));
        assert_eq!(keycode_in(&km, 0, keys::KEY_Shift_L), Some(42));
        assert_eq!(keycode_in(&km, 0, keys::KEY_V), Some(47));
        assert_eq!(keycode_in(&km, 0, keys::KEY_exclam), Some(2));
    }

    #[test]
    fn a_keysym_says_the_level_its_key_gives_it_at() {
        let km = us();
        assert_eq!(key_in(&km, 0, keys::KEY_plus), Some((13, 1)));
        assert_eq!(key_in(&km, 0, keys::KEY_minus), Some((12, 0)));
    }

    #[test]
    fn a_keysym_no_key_gives_has_no_key() {
        assert_eq!(keycode_in(&us(), 0, keys::KEY_Greek_alpha), None);
    }
}
