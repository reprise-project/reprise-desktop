//! RemoteDesktop keys without libei: NotifyKeyboardKeycode/Keysym through a per-session
//! `reprise_keys_v1` object. A keysym becomes the key that gives it in the seat's keymap.

use smithay::backend::input::InputTime;
use smithay::input::keyboard::{Keycode, xkb};

use crate::compositor::State;
use crate::eis::{KEYS, evdev_key, track};
use crate::inputproto::InputSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeysId(u64);

struct Keys {
    id: KeysId,
    /// The app id of the app whose session this serves, "" when unnamed; None for the remote
    /// desktop server's.
    app: Option<String>,
    held: Vec<u32>,
    /// The key each keysym went down on, so its release finds it after a
    /// layout change.
    syms: Vec<(u32, u32)>,
}

impl Keys {
    fn source(&self) -> InputSource {
        match self.app {
            Some(_) => InputSource::App,
            None => InputSource::Remote,
        }
    }
}

#[derive(Default)]
pub struct KeysState {
    objects: Vec<Keys>,
    next_id: u64,
}

impl KeysState {
    pub fn create(&mut self, app: Option<String>) -> KeysId {
        self.next_id += 1;
        let id = KeysId(self.next_id);
        self.objects.push(Keys {
            id,
            app,
            held: Vec::new(),
            syms: Vec::new(),
        });
        id
    }

    fn get_mut(&mut self, id: KeysId) -> Option<&mut Keys> {
        self.objects.iter_mut().find(|k| k.id == id)
    }

    #[cfg(test)]
    pub fn exists(&self, id: KeysId) -> bool {
        self.objects.iter().any(|k| k.id == id)
    }

    /// The app ids of the apps' objects, "" for an unnamed app.
    pub fn apps(&self) -> impl Iterator<Item = &str> {
        self.objects.iter().filter_map(|k| k.app.as_deref())
    }
}

impl State {
    /// A keys object for `app`, as [`KeysState::create`] takes it.
    pub fn keys_create(&mut self, app: Option<String>) -> KeysId {
        let id = self.inputproto.keys.create(app);
        crate::privacy::control_changed(self);
        id
    }

    /// Forget every object and the keys it holds, releasing none: its later requests do nothing.
    pub fn keys_revoke_all(&mut self) {
        self.inputproto.keys.objects.clear();
        crate::privacy::control_changed(self);
    }

    /// Whether a press on `id` is an app's under the lock, which goes nowhere. The server's keys
    /// type into the lock.
    fn keys_locked_out(&self, id: KeysId) -> bool {
        let app = self
            .inputproto
            .keys
            .objects
            .iter()
            .any(|k| k.id == id && k.app.is_some());
        app && self.app_input_locked()
    }

    pub fn keys_key(&mut self, id: KeysId, code: u32, pressed: bool) {
        if pressed && self.keys_locked_out(id) {
            log::debug!("keys: keycode {code} dropped under the lock");
            return;
        }
        let Some(k) = self.inputproto.keys.get_mut(id) else {
            return;
        };
        if !track(&mut k.held, KEYS, code, pressed) {
            log::debug!("keys: ignoring keycode {code}");
            return;
        }
        let source = k.source();
        self.keys_inject(source, code, pressed);
    }

    pub fn keys_keysym(&mut self, id: KeysId, sym: u32, pressed: bool) {
        if pressed && self.keys_locked_out(id) {
            log::debug!("keys: keysym {sym:#x} dropped under the lock");
            return;
        }
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
        let source = k.source();
        self.keys_inject(source, code, pressed);
    }

    pub fn keys_destroy(&mut self, id: KeysId) {
        let keys = &mut self.inputproto.keys.objects;
        let Some(i) = keys.iter().position(|k| k.id == id) else {
            return;
        };
        let k = keys.swap_remove(i);
        let source = k.source();
        for code in k.held {
            self.keys_inject(source, code, false);
        }
        crate::privacy::control_changed(self);
    }

    /// Let go of every key the apps' objects hold, as no user activity.
    pub fn keys_release_apps(&mut self) {
        let held: Vec<u32> = (self.inputproto.keys.objects.iter_mut())
            .filter(|k| k.app.is_some())
            .flat_map(|k| {
                k.syms.clear();
                std::mem::take(&mut k.held)
            })
            .collect();
        self.with_source(InputSource::App, |s| {
            for code in held {
                s.handle_key(evdev_key(code), false, InputTime::now());
            }
        });
    }

    /// One key through the physical keyboard's path, past an InputCapture
    /// as a libei sender's keys go.
    fn keys_inject(&mut self, source: InputSource, code: u32, pressed: bool) {
        self.with_source(source, |s| {
            crate::idle::notify_activity(s);
            s.handle_key(evdev_key(code), pressed, InputTime::now());
        });
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
