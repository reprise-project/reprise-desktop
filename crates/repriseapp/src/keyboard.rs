//! The wl_keyboard through xkb. The keymap compiles at the first key or held modifier and the
//! compose table at the first press, so a program used only with the pointer builds neither.

use crate::Host;
use smithay_client_toolkit::{
    dispatch2::Dispatch2,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    seat::keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
};
use std::{os::fd::OwnedFd, time::Duration};
use wayland_client::{
    Connection, Proxy, QueueHandle, WEnum,
    protocol::{wl_keyboard, wl_surface},
};
use xkbcommon::xkb;

/// The wl_keyboard's user data; its state is [`Host::xkb`].
pub(crate) struct KeyboardData;

/// The seat keyboard's xkb state, repeat rate and repeating key.
#[derive(Default)]
pub(crate) struct Xkb {
    /// The compositor's last keymap, until a key needs it.
    keymap: Option<(OwnedFd, usize)>,
    state: Option<xkb::State>,
    /// None until the first press; then the locale's table, if it has one.
    compose: Option<Option<xkb::compose::State>>,
    /// Delay in ms and interval; None while the compositor disables repeat.
    repeat: Option<(u32, Duration)>,
    focus: Option<wl_surface::WlSurface>,
    repeating: Option<Repeating>,
}

struct Repeating {
    key: KeyEvent,
    surface: wl_surface::WlSurface,
    token: RegistrationToken,
    first: bool,
}

impl Xkb {
    fn take_keymap(&mut self, fd: OwnedFd, size: usize) {
        self.keymap = Some((fd, size));
        if self.state.is_some() {
            self.state = None;
            self.compile();
        }
    }

    fn compile(&mut self) -> Option<&mut xkb::State> {
        if let Some((fd, size)) = self.keymap.take() {
            let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
            // SAFETY: `fd` is the compositor's keymap of `size` bytes; xkbcommon maps it
            // private and read-only, as wl_keyboard asks.
            let keymap = unsafe {
                xkb::Keymap::new_from_fd(
                    &context,
                    fd,
                    size,
                    xkb::KEYMAP_FORMAT_TEXT_V1,
                    xkb::COMPILE_NO_FLAGS,
                )
            };
            match keymap {
                Ok(Some(keymap)) => self.state = Some(xkb::State::new(&keymap)),
                Ok(None) => log::error!("the compositor's keymap does not compile"),
                Err(e) => log::error!("the compositor's keymap: {e}"),
            }
        }
        self.state.as_mut()
    }

    /// The modifiers after a wl_keyboard.modifiers. All-zero masks on a keymap not yet
    /// compiled read as no modifiers, which is what the compiled state would say.
    fn modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) -> Modifiers {
        if self.state.is_none() && depressed | latched | locked | group == 0 {
            return Modifiers::default();
        }
        self.compile();
        let Some(state) = self.state.as_mut() else {
            return Modifiers::default();
        };
        state.update_mask(depressed, latched, locked, 0, 0, group);
        let on = |name| state.mod_name_is_active(name, xkb::STATE_MODS_EFFECTIVE);
        let mods = Modifiers {
            ctrl: on(xkb::MOD_NAME_CTRL),
            alt: on(xkb::MOD_NAME_ALT),
            shift: on(xkb::MOD_NAME_SHIFT),
            caps_lock: on(xkb::MOD_NAME_CAPS),
            logo: on(xkb::MOD_NAME_LOGO),
            num_lock: on(xkb::MOD_NAME_NUM),
        };
        if let Some(r) = self.repeating.as_mut() {
            r.key.utf8 = typed(state, &mut self.compose, r.key.keysym, r.key.raw_code);
        }
        mods
    }

    /// The key event for wl_keyboard key `code`, text only on a press; None without a keymap.
    fn key(&mut self, time: u32, code: u32, pressed: bool) -> Option<KeyEvent> {
        self.compile()?;
        let state = self.state.as_ref()?;
        let keysym = state.key_get_one_sym(xkb::Keycode::new(code + 8));
        let utf8 = pressed
            .then(|| typed(state, &mut self.compose, keysym, code))
            .flatten();
        Some(KeyEvent {
            time,
            raw_code: code,
            keysym,
            utf8,
        })
    }

    fn repeats(&self, code: u32) -> bool {
        self.state
            .as_ref()
            .is_some_and(|s| s.get_keymap().key_repeats(xkb::Keycode::new(code + 8)))
    }
}

/// The text key `code` types, through the compose table, loaded here the first time.
fn typed(
    state: &xkb::State,
    compose: &mut Option<Option<xkb::compose::State>>,
    keysym: Keysym,
    code: u32,
) -> Option<String> {
    let compose = compose.get_or_insert_with(compose_table);
    match compose {
        Some(c) => match c.feed(keysym) {
            xkb::FeedResult::Ignored => None,
            xkb::FeedResult::Accepted => match c.status() {
                xkb::Status::Composed => c.utf8(),
                xkb::Status::Nothing => Some(state.key_get_utf8(xkb::Keycode::new(code + 8))),
                _ => None,
            },
        },
        None => Some(state.key_get_utf8(xkb::Keycode::new(code + 8))),
    }
}

fn compose_table() -> Option<xkb::compose::State> {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|k| std::env::var_os(k).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "C".into());
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let table =
        xkb::compose::Table::new_from_locale(&context, &locale, xkb::compose::COMPILE_NO_FLAGS)
            .ok()?;
    Some(xkb::compose::State::new(
        &table,
        xkb::compose::STATE_NO_FLAGS,
    ))
}

impl Host {
    fn stop_repeat(&mut self) {
        if let Some(r) = self.xkb.repeating.take() {
            self.loop_handle.remove(r.token);
        }
    }

    fn start_repeat(&mut self, key: KeyEvent) {
        self.stop_repeat();
        let (Some((delay, _)), Some(surface)) = (self.xkb.repeat, self.xkb.focus.clone()) else {
            return;
        };
        let timer = Timer::from_duration(Duration::from_millis(delay.into()));
        let Ok(token) = self
            .loop_handle
            .insert_source(timer, |_, _, host: &mut Host| host.repeat_tick())
        else {
            return;
        };
        self.xkb.repeating = Some(Repeating {
            key,
            surface,
            token,
            first: true,
        });
    }

    fn repeat_tick(&mut self) -> TimeoutAction {
        let (Some(r), Some((delay, gap))) = (self.xkb.repeating.as_mut(), self.xkb.repeat) else {
            self.xkb.repeating = None;
            return TimeoutAction::Drop;
        };
        if !r.surface.is_alive() {
            self.xkb.repeating = None;
            return TimeoutAction::Drop;
        }
        r.key.time += if std::mem::take(&mut r.first) {
            delay
        } else {
            gap.as_millis() as u32
        };
        let key = r.key.clone();
        self.key(key);
        TimeoutAction::ToDuration(gap)
    }
}

impl Dispatch2<wl_keyboard::WlKeyboard, Host> for KeyboardData {
    fn event(
        &self,
        host: &mut Host,
        keyboard: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        conn: &Connection,
        qh: &QueueHandle<Host>,
    ) {
        use wl_keyboard::{Event, KeyState, KeymapFormat};
        match event {
            Event::Keymap {
                format: WEnum::Value(KeymapFormat::XkbV1),
                fd,
                size,
            } => host.xkb.take_keymap(fd, size as usize),
            Event::Keymap { .. } => log::warn!("the compositor sent a keymap that is not xkb"),
            Event::Enter {
                serial,
                surface,
                keys,
            } => {
                let raw: Vec<u32> = keys
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| u32::from_le_bytes(*c))
                    .collect();
                host.xkb.focus = Some(surface.clone());
                host.enter(conn, qh, keyboard, &surface, serial, &raw, &[]);
            }
            Event::Leave { serial, surface } => {
                host.stop_repeat();
                host.xkb.focus = None;
                host.leave(conn, qh, keyboard, &surface, serial);
            }
            Event::Key {
                serial,
                time,
                key,
                state: WEnum::Value(state),
            } => {
                let Some(event) = host.xkb.key(time, key, state == KeyState::Pressed) else {
                    return;
                };
                match state {
                    KeyState::Released => {
                        if host.xkb.repeating.as_ref().map(|r| r.key.raw_code) == Some(key) {
                            host.stop_repeat();
                        }
                        host.release_key(conn, qh, keyboard, serial, event);
                    }
                    KeyState::Repeated => host.repeat_key(conn, qh, keyboard, serial, event),
                    _ => {
                        host.press_key(conn, qh, keyboard, serial, event.clone());
                        if host.xkb.repeats(key) {
                            host.start_repeat(event);
                        }
                    }
                }
            }
            Event::Modifiers {
                serial,
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
            } => {
                let mods = host
                    .xkb
                    .modifiers(mods_depressed, mods_latched, mods_locked, group);
                let raw = RawModifiers {
                    depressed: mods_depressed,
                    latched: mods_latched,
                    locked: mods_locked,
                };
                host.update_modifiers(conn, qh, keyboard, serial, mods, raw, group);
            }
            Event::RepeatInfo { rate, delay } => {
                host.xkb.repeat = (rate > 0).then(|| {
                    let gap = Duration::from_micros(1_000_000 / rate as u64);
                    (delay.max(0) as u32, gap)
                });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn keymap_fd() -> (OwnedFd, usize) {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap =
            xkb::Keymap::new_from_names(&context, "", "", "us", "", None, xkb::COMPILE_NO_FLAGS)
                .expect("the us keymap compiles");
        let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(text.as_bytes()).unwrap();
        file.write_all(b"\0").unwrap();
        (file.into(), text.len() + 1)
    }

    #[test]
    fn the_keymap_compiles_at_the_first_key_and_compose_loads_at_the_first_press() {
        let mut xkb = Xkb::default();
        let (fd, size) = keymap_fd();
        xkb.take_keymap(fd, size);
        let mods = xkb.modifiers(0, 0, 0, 0);
        assert!(!(mods.ctrl || mods.alt || mods.shift || mods.caps_lock || mods.num_lock));
        assert!(xkb.state.is_none(), "no modifiers held: the keymap waits");
        let up = xkb.key(1, 30, false).expect("a keymap");
        assert_eq!(up.keysym, Keysym::a);
        assert!(xkb.state.is_some());
        assert!(
            xkb.compose.is_none(),
            "a release types nothing: no compose table"
        );
        let down = xkb.key(2, 30, true).expect("a keymap");
        assert_eq!(down.utf8.as_deref(), Some("a"));
        assert!(xkb.compose.is_some());
    }

    #[test]
    fn a_held_modifier_compiles_the_keymap() {
        let mut xkb = Xkb::default();
        let (fd, size) = keymap_fd();
        xkb.take_keymap(fd, size);
        let mods = xkb.modifiers(1 << 2, 0, 0, 0);
        assert!(
            mods.ctrl && !mods.shift,
            "Control is real modifier 2: {mods:?}"
        );
        assert!(xkb.state.is_some());
    }

    #[test]
    fn a_new_keymap_after_use_compiles_at_once() {
        let mut xkb = Xkb::default();
        let (fd, size) = keymap_fd();
        xkb.take_keymap(fd, size);
        xkb.key(1, 30, false);
        let (fd, size) = keymap_fd();
        xkb.take_keymap(fd, size);
        assert!(xkb.keymap.is_none() && xkb.state.is_some());
    }
}
