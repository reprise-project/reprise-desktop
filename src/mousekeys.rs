//! MouseKeys (`[accessibility] mouse_keys`): the numeric keypad drives the
//! pointer. [`MouseKeys`] is the pure model (keys and ticks in, moves and
//! presses out); the host below it runs it and ticks it while a direction key is down.

use std::time::Duration;

use smithay::{
    backend::input::{ButtonState, InputTime},
    input::keyboard::ModifiersState,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Point},
};

use crate::compositor::State;
use crate::config::AccessibilityConfig;
use crate::input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};

pub mod code {
    pub const KP7: u32 = 79;
    pub const KP8: u32 = 80;
    pub const KP9: u32 = 81;
    pub const KP_MINUS: u32 = 82;
    pub const KP4: u32 = 83;
    pub const KP5: u32 = 84;
    pub const KP6: u32 = 85;
    pub const KP_PLUS: u32 = 86;
    pub const KP1: u32 = 87;
    pub const KP2: u32 = 88;
    pub const KP3: u32 = 89;
    pub const KP0: u32 = 90;
    pub const KP_DOT: u32 = 91;
    pub const KP_SLASH: u32 = 106;
    pub const KP_STAR: u32 = 63;
}

const GLIDE_AFTER_MS: u64 = 300;
const START_SPEED: f64 = 30.0;
const TICK_MS: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Move(i32, i32),
    Button(u32, bool),
}

fn direction(code: u32) -> Option<(i32, i32)> {
    use code::*;
    Some(match code {
        KP7 => (-1, -1),
        KP8 => (0, -1),
        KP9 => (1, -1),
        KP4 => (-1, 0),
        KP6 => (1, 0),
        KP1 => (-1, 1),
        KP2 => (0, 1),
        KP3 => (1, 1),
        _ => return None,
    })
}

#[derive(Debug)]
pub struct MouseKeys {
    on: bool,
    num_lock: bool,
    speed: u32,
    accel_ms: u32,
    pick: u32,
    dirs: Vec<(u32, (i32, i32))>,
    since: u64,
    last: u64,
    frac: (f64, f64),
    taken: Vec<u32>,
    held: Option<u32>,
}

impl Default for MouseKeys {
    fn default() -> Self {
        let cfg = AccessibilityConfig::default();
        MouseKeys {
            on: cfg.mouse_keys,
            num_lock: cfg.mouse_keys_num_lock,
            speed: cfg.mouse_keys_speed,
            accel_ms: cfg.mouse_keys_acceleration,
            pick: BTN_LEFT,
            dirs: Vec::new(),
            since: 0,
            last: 0,
            frac: (0.0, 0.0),
            taken: Vec::new(),
            held: None,
        }
    }
}

impl MouseKeys {
    pub fn configure(&mut self, cfg: &AccessibilityConfig) -> Vec<Action> {
        self.on = cfg.mouse_keys;
        self.num_lock = cfg.mouse_keys_num_lock;
        self.speed = cfg.mouse_keys_speed;
        self.accel_ms = cfg.mouse_keys_acceleration;
        if self.on {
            return Vec::new();
        }
        self.dirs.clear();
        self.held
            .take()
            .map(|b| Action::Button(b, false))
            .into_iter()
            .collect()
    }

    pub fn on(&self) -> bool {
        self.on
    }

    pub fn pick(&self) -> u32 {
        self.pick
    }

    pub fn held(&self) -> Option<u32> {
        self.held
    }

    /// Whether the keypad drives the pointer: on, Num Lock as asked, and no
    /// modifier held (Alt types a character code, Shift selects).
    pub fn active(&self, num_lock: bool, modifier_held: bool) -> bool {
        self.on && (num_lock || !self.num_lock) && !modifier_held
    }

    pub fn key(&mut self, code: u32, pressed: bool, active: bool, now: u64) -> Option<Vec<Action>> {
        if !pressed {
            let i = self.taken.iter().position(|&c| c == code)?;
            self.taken.swap_remove(i);
            self.dirs.retain(|d| d.0 != code);
            return Some(Vec::new());
        }
        if !active {
            return None;
        }
        let acts = self.press(code, now)?;
        if !self.taken.contains(&code) {
            self.taken.push(code);
        }
        Some(acts)
    }

    fn press(&mut self, code: u32, now: u64) -> Option<Vec<Action>> {
        use code::*;
        if let Some(d) = direction(code) {
            if self.dirs.iter().any(|x| x.0 == code) {
                return Some(Vec::new());
            }
            let first = self.dirs.is_empty();
            self.dirs.push((code, d));
            if !first {
                return Some(Vec::new());
            }
            self.since = now;
            self.last = now;
            self.frac = (0.0, 0.0);
            return Some(vec![Action::Move(d.0, d.1)]);
        }
        let b = self.pick;
        let click = [Action::Button(b, true), Action::Button(b, false)];
        Some(match code {
            KP5 => click.to_vec(),
            KP_PLUS => [click, click].concat(),
            KP_SLASH | KP_STAR | KP_MINUS => {
                self.pick = match code {
                    KP_SLASH => BTN_LEFT,
                    KP_STAR => BTN_MIDDLE,
                    _ => BTN_RIGHT,
                };
                Vec::new()
            }
            KP0 if self.held.is_none() => {
                self.held = Some(b);
                vec![Action::Button(b, true)]
            }
            KP0 => Vec::new(),
            KP_DOT => match self.held.take() {
                Some(h) => vec![Action::Button(h, false)],
                None => Vec::new(),
            },
            _ => return None,
        })
    }

    pub fn deadline(&self) -> Option<u64> {
        (!self.dirs.is_empty()).then_some(self.last + TICK_MS)
    }

    fn speed_at(&self, t: u64) -> f64 {
        let top = self.speed as f64;
        let start = START_SPEED.min(top);
        if self.accel_ms == 0 {
            return top;
        }
        let k = (t as f64 / self.accel_ms as f64).min(1.0);
        start + (top - start) * k
    }

    pub fn tick(&mut self, now: u64) -> (i32, i32) {
        if self.dirs.is_empty() {
            return (0, 0);
        }
        let from = self.last.max(self.since + GLIDE_AFTER_MS);
        self.last = now;
        if now <= from {
            return (0, 0);
        }
        let v = self.speed_at(now - self.since - GLIDE_AFTER_MS);
        let dist = v * (now - from) as f64 / 1000.0;
        let (sx, sy) = self
            .dirs
            .iter()
            .fold((0, 0), |(x, y), (_, (dx, dy))| (x + dx, y + dy));
        let (sx, sy) = (sx.clamp(-1, 1) as f64, sy.clamp(-1, 1) as f64);
        self.frac.0 += sx * dist;
        self.frac.1 += sy * dist;
        let (dx, dy) = (self.frac.0.trunc(), self.frac.1.trunc());
        self.frac.0 -= dx;
        self.frac.1 -= dy;
        (dx as i32, dy as i32)
    }
}

#[derive(Debug, Default)]
pub struct MouseKeysHost {
    pub model: MouseKeys,
    timer: Option<RegistrationToken>,
}

impl State {
    fn mousekeys_now(&self) -> u64 {
        Duration::from(self.clock.now()).as_millis() as u64
    }

    pub fn mousekeys_configured(&mut self) {
        let cfg = self.config.accessibility.clone();
        let mk = &mut self.key_input.mouse_keys;
        let was = mk.model.on();
        let acts = mk.model.configure(&cfg);
        self.mousekeys_do(acts);
        self.arm_mousekeys();
        if was != cfg.mouse_keys {
            self.redraw_taskbars();
        }
    }

    pub fn mousekeys_key(&mut self, code: u32, pressed: bool, mods: &ModifiersState) -> bool {
        let modifier = mods.shift || mods.ctrl || mods.alt || mods.logo;
        let now = self.mousekeys_now();
        let mk = &mut self.key_input.mouse_keys.model;
        let active = mk.active(mods.num_lock, modifier);
        let before = (mk.pick(), mk.held());
        let Some(acts) = mk.key(code, pressed, active, now) else {
            return true;
        };
        let changed = before != (mk.pick(), mk.held());
        self.mousekeys_do(acts);
        self.arm_mousekeys();
        if changed {
            self.redraw_taskbars();
        }
        false
    }

    fn mousekeys_do(&mut self, acts: Vec<Action>) {
        let time = InputTime::now();
        for a in acts {
            match a {
                Action::Move(dx, dy) => {
                    let d: Point<f64, Logical> = (dx as f64, dy as f64).into();
                    self.pointer_moved(d, d, time);
                }
                Action::Button(b, pressed) => {
                    let state = if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    };
                    self.handle_button(b, state, time);
                }
            }
        }
    }

    fn arm_mousekeys(&mut self) {
        let mk = &mut self.key_input.mouse_keys;
        if mk.model.deadline().is_none() {
            if let Some(t) = mk.timer.take() {
                self.handle.remove(t);
            }
            return;
        }
        if mk.timer.is_some() {
            return;
        }
        let token = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(TICK_MS)),
            |_, _, state| {
                let now = state.mousekeys_now();
                let (dx, dy) = state.key_input.mouse_keys.model.tick(now);
                if (dx, dy) != (0, 0) {
                    state.mousekeys_do(vec![Action::Move(dx, dy)]);
                }
                if state.key_input.mouse_keys.model.deadline().is_some() {
                    TimeoutAction::ToDuration(Duration::from_millis(TICK_MS))
                } else {
                    state.key_input.mouse_keys.timer = None;
                    TimeoutAction::Drop
                }
            },
        );
        match token {
            Ok(t) => self.key_input.mouse_keys.timer = Some(t),
            Err(e) => log::warn!("mouse keys timer: {}", e.error),
        }
    }
}

pub fn shown(mk: &MouseKeysHost) -> bool {
    mk.model.on()
}

pub fn tip(mk: &MouseKeysHost) -> String {
    let name = |b: u32| match b {
        BTN_MIDDLE => "middle",
        BTN_RIGHT => "right",
        _ => "left",
    };
    let mut tip = format!(
        "MouseKeys\nThe keys click the {} button",
        name(mk.model.pick())
    );
    if let Some(b) = mk.model.held() {
        tip += &format!("\nThe {} button is held down", name(b));
    }
    tip
}

#[cfg(test)]
mod tests {
    use super::code::*;

    const NUM_LOCK: u32 = 77;
    use super::*;

    fn on() -> MouseKeys {
        let mut m = MouseKeys::default();
        m.configure(&AccessibilityConfig {
            mouse_keys: true,
            mouse_keys_speed: 400,
            mouse_keys_acceleration: 1000,
            ..Default::default()
        });
        m
    }

    fn press(m: &mut MouseKeys, code: u32, now: u64) -> Option<Vec<Action>> {
        m.key(code, true, true, now)
    }

    #[test]
    fn each_direction_key_steps_the_pointer_one_pixel() {
        let want = [
            (KP7, (-1, -1)),
            (KP8, (0, -1)),
            (KP9, (1, -1)),
            (KP4, (-1, 0)),
            (KP6, (1, 0)),
            (KP1, (-1, 1)),
            (KP2, (0, 1)),
            (KP3, (1, 1)),
        ];
        for (code, (dx, dy)) in want {
            let mut m = on();
            assert_eq!(press(&mut m, code, 0), Some(vec![Action::Move(dx, dy)]));
            assert_eq!(m.key(code, false, true, 50), Some(vec![]));
            assert_eq!(m.deadline(), None, "a released key stops the glide");
        }
    }

    #[test]
    fn a_held_key_glides_slowly_then_reaches_the_top_speed() {
        let mut m = on();
        press(&mut m, KP6, 0);
        assert_eq!(m.tick(GLIDE_AFTER_MS), (0, 0));
        let mut moved = 0;
        let mut t = GLIDE_AFTER_MS;
        while t < GLIDE_AFTER_MS + 1000 {
            t += TICK_MS;
            moved += m.tick(t).0;
        }
        assert!(
            (210..=220).contains(&moved),
            "{moved} px in the first second"
        );
        let mut moved = 0;
        for _ in 0..100 {
            t += TICK_MS;
            let (dx, dy) = m.tick(t);
            moved += dx;
            assert_eq!(dy, 0);
        }
        assert_eq!(moved, 400);
    }

    #[test]
    fn no_acceleration_glides_at_the_top_speed_at_once() {
        let mut m = MouseKeys::default();
        m.configure(&AccessibilityConfig {
            mouse_keys: true,
            mouse_keys_speed: 100,
            mouse_keys_acceleration: 0,
            ..Default::default()
        });
        press(&mut m, KP2, 0);
        let t = GLIDE_AFTER_MS;
        m.tick(t);
        assert_eq!(m.tick(t + 500), (0, 50));
    }

    #[test]
    fn two_direction_keys_together_move_diagonally() {
        let mut m = on();
        assert_eq!(press(&mut m, KP8, 0), Some(vec![Action::Move(0, -1)]));
        assert_eq!(
            press(&mut m, KP4, 10),
            Some(vec![]),
            "the second key adds no step"
        );
        m.tick(GLIDE_AFTER_MS);
        let (dx, dy) = m.tick(GLIDE_AFTER_MS + 2000);
        assert!(dx < 0 && dx == dy, "({dx}, {dy}) runs up and left");
        press(&mut m, KP6, 2400);
        let (dx, dy) = m.tick(GLIDE_AFTER_MS + 2500);
        assert_eq!(dx, 0);
        assert!(dy < 0);
    }

    #[test]
    fn five_clicks_plus_double_clicks_and_the_picked_button_is_used() {
        let mut m = on();
        let click = |b| vec![Action::Button(b, true), Action::Button(b, false)];
        assert_eq!(press(&mut m, KP5, 0), Some(click(BTN_LEFT)));
        assert_eq!(
            press(&mut m, KP_PLUS, 0),
            Some([click(BTN_LEFT), click(BTN_LEFT)].concat())
        );
        for (code, b) in [
            (KP_MINUS, BTN_RIGHT),
            (KP_STAR, BTN_MIDDLE),
            (KP_SLASH, BTN_LEFT),
        ] {
            assert_eq!(press(&mut m, code, 0), Some(vec![]));
            assert_eq!(m.pick(), b);
            assert_eq!(press(&mut m, KP5, 0), Some(click(b)));
        }
    }

    #[test]
    fn zero_holds_the_button_and_dot_lets_it_go() {
        let mut m = on();
        press(&mut m, KP_MINUS, 0);
        assert_eq!(
            press(&mut m, KP0, 0),
            Some(vec![Action::Button(BTN_RIGHT, true)])
        );
        assert_eq!(
            press(&mut m, KP0, 0),
            Some(vec![]),
            "a second 0 holds nothing more"
        );
        assert_eq!(m.held(), Some(BTN_RIGHT));
        press(&mut m, KP_SLASH, 0);
        assert_eq!(
            press(&mut m, KP_DOT, 0),
            Some(vec![Action::Button(BTN_RIGHT, false)])
        );
        assert_eq!(press(&mut m, KP_DOT, 0), Some(vec![]));
        press(&mut m, KP0, 0);
        let off = m.configure(&AccessibilityConfig::default());
        assert_eq!(off, vec![Action::Button(BTN_LEFT, false)]);
    }

    #[test]
    fn num_lock_and_modifiers_decide_whether_the_keypad_drives_the_pointer() {
        let m = on();
        assert!(m.active(true, false));
        assert!(
            !m.active(false, false),
            "with Num Lock off the keypad types"
        );
        assert!(
            !m.active(true, true),
            "a held modifier lets the keys through"
        );
        let mut always = MouseKeys::default();
        always.configure(&AccessibilityConfig {
            mouse_keys: true,
            mouse_keys_num_lock: false,
            ..Default::default()
        });
        assert!(always.active(false, false) && always.active(true, false));
        assert!(!MouseKeys::default().active(true, false), "off by default");
    }

    #[test]
    fn inactive_keys_and_other_keys_pass_but_a_taken_release_is_taken() {
        let mut m = on();
        assert_eq!(m.key(KP5, true, false, 0), None);
        assert_eq!(m.key(KP5, false, false, 0), None);
        assert_eq!(press(&mut m, 38, 0), None, "a letter is not MouseKeys'");
        assert_eq!(press(&mut m, NUM_LOCK, 0), None, "Num Lock always toggles");
        press(&mut m, KP8, 0);
        assert_eq!(m.key(KP8, false, false, 100), Some(vec![]));
        assert_eq!(m.deadline(), None);
    }
}
