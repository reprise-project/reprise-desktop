//! RemoteDesktop keys and pointer without libei: NotifyKeyboard* and NotifyPointer* through a
//! per-session `reprise_keys_v1` object. A keysym becomes the key that gives it in the seat's keymap.

use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime};
use smithay::input::keyboard::{Keycode, xkb};
use smithay::input::pointer::AxisFrame;
use smithay::utils::{Logical, Point};

use crate::compositor::State;
use crate::eis::{BUTTONS, KEYS, evdev_key, track};
use crate::inputproto::InputSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeysId(u64);

struct Keys {
    id: KeysId,
    /// The app id of the app whose session this serves, "" when unnamed; None for the remote
    /// desktop server's.
    app: Option<String>,
    held: Vec<u32>,
    buttons: Vec<u32>,
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

/// One pointer request on a keys object; distances are logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Pointer {
    Motion(f64, f64),
    /// To a point in the named output's logical coordinates.
    MotionAbsolute(String, f64, f64),
    Button(u32, bool),
    /// Continuous scroll, as wl_pointer.axis values.
    Axis(f64, f64),
    /// Wheel notches, horizontal then vertical, positive right and down.
    Wheel(i32, i32),
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
            buttons: Vec::new(),
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
        self.with_source(source, |s| {
            for b in k.buttons {
                s.handle_button(b, ButtonState::Released, InputTime::now());
            }
        });
        crate::privacy::control_changed(self);
    }

    /// A pointer request through the physical pointer's path, as a libei sender's goes. Under
    /// the lock an app's object only lets go of buttons.
    pub fn keys_pointer(&mut self, id: KeysId, input: Pointer) {
        let release = matches!(input, Pointer::Button(_, false));
        if !release && self.keys_locked_out(id) {
            log::debug!("keys: {input:?} dropped under the lock");
            return;
        }
        let Some(k) = self.inputproto.keys.get_mut(id) else {
            return;
        };
        // A press of a held button or a release of one not held changes nothing.
        if let Pointer::Button(b, pressed) = input
            && (pressed == k.buttons.contains(&b) || !track(&mut k.buttons, BUTTONS, b, pressed))
        {
            log::debug!("keys: ignoring button {b:#x}");
            return;
        }
        let source = k.source();
        let time = InputTime::now();
        self.with_source(source, |s| {
            crate::idle::notify_activity(s);
            match input {
                Pointer::Motion(dx, dy) => {
                    let pos = s.pointer_location() + Point::<f64, Logical>::from((dx, dy));
                    s.pointer_to(pos, time);
                }
                Pointer::MotionAbsolute(output, x, y) => match s.keys_output_point(&output, x, y) {
                    Some(pos) => s.pointer_to(pos, time),
                    None => log::debug!("keys: no output {output:?} on the desktop"),
                },
                Pointer::Button(b, pressed) => {
                    let state = match pressed {
                        true => ButtonState::Pressed,
                        false => ButtonState::Released,
                    };
                    s.handle_button(b, state, time);
                }
                Pointer::Axis(dx, dy) => {
                    let mut frame = AxisFrame::new(time).source(AxisSource::Continuous);
                    for (axis, v) in [(Axis::Horizontal, dx), (Axis::Vertical, dy)] {
                        if v != 0.0 {
                            frame = frame.value(axis, v);
                        }
                    }
                    if frame.axis != (0.0, 0.0) {
                        s.handle_axis(frame);
                    }
                }
                Pointer::Wheel(dx, dy) => {
                    let mut frame = AxisFrame::new(time).source(AxisSource::Wheel);
                    for (axis, steps) in [(Axis::Horizontal, dx), (Axis::Vertical, dy)] {
                        if steps != 0 {
                            frame = frame
                                .value(axis, f64::from(steps) * 15.0)
                                .v120(axis, steps * 120);
                        }
                    }
                    if frame.axis != (0.0, 0.0) {
                        s.handle_axis(frame);
                    }
                }
            }
        });
    }

    /// (x, y) of output `name` on the desktop side, kept on the output, in layout coordinates.
    fn keys_output_point(&self, name: &str, x: f64, y: f64) -> Option<Point<f64, Logical>> {
        let output = (self.scene.outputs().iter())
            .find(|o| o.name() == name && crate::console::on_desk(self, o))?;
        let r = reprisescene::output_rect(output)?;
        let x = x.clamp(0.0, f64::from(r.size.w - 1).max(0.0));
        let y = y.clamp(0.0, f64::from(r.size.h - 1).max(0.0));
        Some(r.loc.to_f64() + Point::from((x, y)))
    }

    /// Let go of every key and button the apps' objects hold, as no user activity.
    pub fn keys_release_apps(&mut self) {
        let apps = (self.inputproto.keys.objects.iter_mut()).filter(|k| k.app.is_some());
        let (mut held, mut buttons) = (Vec::new(), Vec::new());
        for k in apps {
            k.syms.clear();
            held.append(&mut k.held);
            buttons.append(&mut k.buttons);
        }
        self.with_source(InputSource::App, |s| {
            for code in held {
                s.handle_key(evdev_key(code), false, InputTime::now());
            }
            for b in buttons {
                s.handle_button(b, ButtonState::Released, InputTime::now());
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
    use crate::harness::Harness;
    use crate::keys;

    const LEFT: u32 = 0x110;

    fn at(x: f64, y: f64) -> Point<f64, Logical> {
        (x, y).into()
    }

    #[test]
    fn a_keys_object_moves_the_pointer_by_and_onto_an_output() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        h.add_output("TEST-2", 800, 600, Some((1024, 0)));
        h.dispatch();
        let keys = h.state.keys_create(Some(String::new()));
        let to = |o: &str, x, y| Pointer::MotionAbsolute(o.into(), x, y);
        h.state.keys_pointer(keys, to("TEST-2", 100.0, 50.0));
        assert_eq!(h.state.pointer_location(), at(1124.0, 50.0));
        h.state.keys_pointer(keys, Pointer::Motion(10.0, -5.0));
        assert_eq!(h.state.pointer_location(), at(1134.0, 45.0));
        h.state.keys_pointer(keys, to("TEST-2", 5000.0, -10.0));
        assert_eq!(
            h.state.pointer_location(),
            at(1823.0, 0.0),
            "kept on the output"
        );
        h.state.keys_pointer(keys, to("NONE-1", 10.0, 10.0));
        assert_eq!(
            h.state.pointer_location(),
            at(1823.0, 0.0),
            "no such output"
        );
    }

    #[test]
    fn a_keys_object_lets_go_of_its_buttons_when_destroyed() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let (_lock, _surface) = h.map_lock_surface(&mut c);
        let server = h.state.keys_create(None);
        h.state.keys_pointer(
            server,
            Pointer::MotionAbsolute("TEST-1".into(), 500.0, 400.0),
        );
        h.roundtrip(&mut c);
        c.side.buttons.clear();
        h.state.keys_pointer(server, Pointer::Button(LEFT, true));
        h.state.keys_pointer(server, Pointer::Button(LEFT, true));
        h.roundtrip(&mut c);
        assert_eq!(c.side.buttons, [(LEFT, true)], "one press per button");
        h.state.keys_destroy(server);
        h.roundtrip(&mut c);
        assert_eq!(c.side.buttons, [(LEFT, true), (LEFT, false)]);
    }

    #[test]
    fn the_portals_keys_object_drives_the_pointer() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut portal = h.connect_portal();
        let keys = portal.create_app_keys("org.example.Remote");
        keys.pointer_motion_absolute("TEST-1".into(), 300.0, 200.0);
        keys.pointer_motion(-50.0, 25.0);
        h.roundtrip(&mut portal);
        assert_eq!(h.state.pointer_location(), at(250.0, 225.0));
    }

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
