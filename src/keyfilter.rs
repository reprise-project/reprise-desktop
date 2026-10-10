//! StickyKeys and SlowKeys (`[accessibility]`): the filter every key passes before `keyroute`, so the aids reach clients and the compositor's binds alike.
//!
//! [`KeyFilter`] is the pure part. SlowKeys holds a press back until it has been down for the delay; a key let go sooner never happens. StickyKeys turns a lone tap of Shift, Ctrl, Alt or Super into a latch the next key picks up (a second tap locks it, with `sticky_lock`). The shortcuts: Shift pressed five times and right Shift held eight seconds offer either aid or turn it off; left Alt+left Shift+Num Lock is MouseKeys' shortcut (MouseKeys itself is `mousekeys`).
//!
//! The host puts the latched and locked modifiers into the seat keyboard's xkb *latched* mask, so binds and the focused client (`wl_keyboard.modifiers`) see them (`State::sync_aids`).

use std::time::Duration;

use smithay::{
    backend::input::InputTime,
    input::keyboard::Keycode,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
};

use crate::compositor::State;
use crate::config::AccessibilityConfig;
use crate::keyroute::{KeyPress, modifier_key};
use crate::keys::*;

/// Shift presses in a row that make the StickyKeys shortcut.
const SHIFT_PRESSES: u32 = 5;
/// The longest gap between two of them, in ms.
const SHIFT_GAP_MS: u64 = 1000;
/// How long right Shift is held for the SlowKeys shortcut, in ms.
const RIGHT_SHIFT_HOLD_MS: u64 = 8000;

/// One of the aids with a shortcut.
#[expect(clippy::enum_variant_names, reason = "the aids are named so")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aid {
    StickyKeys,
    SlowKeys,
    MouseKeys,
}

impl Aid {
    pub fn key(self) -> &'static str {
        match self {
            Aid::StickyKeys => "sticky_keys",
            Aid::SlowKeys => "slow_keys",
            Aid::MouseKeys => "mouse_keys",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Aid::StickyKeys => "StickyKeys",
            Aid::SlowKeys => "SlowKeys",
            Aid::MouseKeys => "MouseKeys",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: u32,
    /// The `MOD_*` bit of Shift, Ctrl, Alt or Super; None for any
    /// other key.
    pub modifier: Option<u32>,
    pub right_shift: bool,
    /// Left Shift or left Alt, which with Num Lock make the MouseKeys
    /// shortcut.
    pub left: bool,
    pub num_lock: bool,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    /// Nobody sees it, the keyboard's xkb state included.
    Swallow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The shortcut of an aid that is off: ask whether to turn it on.
    Prompt(Aid),
    /// The filter turned this aid off (its shortcut, or two keys at once).
    Off(Aid),
}

/// What the host does besides the verdict.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    /// Presses SlowKeys accepts now, in order: each is delivered as a
    /// fresh press and comes back through [`KeyFilter::key`].
    pub accept: Vec<u32>,
    pub notice: Option<Notice>,
    pub mods_changed: bool,
}

/// The filter's state; it keeps its own copy of the settings, since it
/// turns aids off itself.
#[derive(Debug, Default)]
pub struct KeyFilter {
    cfg: AccessibilityConfig,
    down: Vec<u32>,
    shift_run: (u32, u64),
    /// Right Shift went down alone at this time and is still down.
    right_shift_since: Option<u64>,
    /// Left Shift and left Alt keys down: (keycode, `MOD_*` bit).
    left_down: Vec<(u32, u32)>,
    /// The Num Lock that made the MouseKeys shortcut, whose release is
    /// swallowed with it.
    eaten: Option<u32>,
    /// SlowKeys: presses waiting out the delay, with when they went down.
    /// A release of one of these is swallowed with it, SlowKeys on or not.
    pending: Vec<(u32, u64)>,
    delivering: Vec<u32>,
    /// StickyKeys: `MOD_*` bits held for the next key, and locked.
    latched: u32,
    locked: u32,
    /// Modifier keys down that StickyKeys saw go down: (keycode, bit).
    held: Vec<(u32, u32)>,
    /// A key went down while a modifier key was held, so letting go of
    /// it latches nothing.
    used: bool,
}

impl KeyFilter {
    /// Take new settings; an aid that went off forgets its state. Returns
    /// whether the latched or locked modifiers changed.
    pub fn configure(&mut self, cfg: &AccessibilityConfig) -> bool {
        self.cfg = cfg.clone();
        if !cfg.slow_keys {
            self.delivering.clear();
        }
        if !cfg.sticky_keys {
            return self.clear_sticky();
        }
        if !cfg.sticky_lock && self.locked != 0 {
            self.locked = 0;
            return true;
        }
        false
    }

    pub fn config(&self) -> &AccessibilityConfig {
        &self.cfg
    }

    pub fn latched(&self) -> u32 {
        self.latched
    }

    pub fn locked(&self) -> u32 {
        self.locked
    }

    pub fn key(&mut self, k: Key, now: u64) -> (Verdict, Effects) {
        let mut fx = Effects::default();
        if k.pressed
            && let Some(i) = self.delivering.iter().position(|&c| c == k.code)
        {
            self.delivering.remove(i);
            self.sticky(k, &mut fx);
            return (Verdict::Pass, fx);
        }
        if self.shortcuts(k, now, &mut fx) {
            return (Verdict::Swallow, fx);
        }
        if !k.pressed
            && let Some(i) = self.pending.iter().position(|p| p.0 == k.code)
        {
            self.pending.remove(i);
            return (Verdict::Swallow, fx);
        }
        if k.pressed && self.cfg.slow_keys {
            if !self.pending.iter().any(|p| p.0 == k.code) {
                self.pending.push((k.code, now));
            }
            return (Verdict::Swallow, fx);
        }
        self.sticky(k, &mut fx);
        (Verdict::Pass, fx)
    }

    /// A pointer button went down: it takes the latched modifiers with
    /// it, as a key would. Returns whether that changed anything.
    pub fn button(&mut self) -> bool {
        let had = self.latched != 0;
        self.latched = 0;
        had
    }

    pub fn deadline(&self) -> Option<u64> {
        let slow = self
            .cfg
            .slow_keys
            .then(|| self.pending.iter().map(|p| p.1 + self.delay()).min())
            .flatten();
        let hold = self
            .cfg
            .shortcuts
            .then(|| self.right_shift_since.map(|t| t + RIGHT_SHIFT_HOLD_MS))
            .flatten();
        [slow, hold].into_iter().flatten().min()
    }

    pub fn tick(&mut self, now: u64) -> Effects {
        let mut fx = Effects::default();
        if self.cfg.slow_keys {
            let delay = self.delay();
            let (due, waiting): (Vec<_>, Vec<_>) =
                self.pending.iter().partition(|p| p.1 + delay <= now);
            self.pending = waiting;
            for (code, _) in due {
                self.delivering.push(code);
                fx.accept.push(code);
            }
        }
        if self.cfg.shortcuts
            && self
                .right_shift_since
                .is_some_and(|t| t + RIGHT_SHIFT_HOLD_MS <= now)
        {
            self.right_shift_since = None;
            self.shortcut(Aid::SlowKeys, &mut fx);
        }
        fx
    }

    fn delay(&self) -> u64 {
        self.cfg.slow_keys_delay as u64
    }

    /// The shortcuts watch the keys as they are pressed, before SlowKeys.
    /// True: the key made the MouseKeys shortcut (or is the release of
    /// the one that did) and is swallowed, so Num Lock stays as it was.
    fn shortcuts(&mut self, k: Key, now: u64, fx: &mut Effects) -> bool {
        if !k.pressed {
            self.down.retain(|&c| c != k.code);
            self.left_down.retain(|l| l.0 != k.code);
            if k.right_shift {
                self.right_shift_since = None;
            }
            if self.eaten == Some(k.code) {
                self.eaten = None;
                return true;
            }
            return false;
        }
        let alone = self.down.iter().all(|&c| c == k.code);
        if !self.down.contains(&k.code) {
            self.down.push(k.code);
        }
        if k.left
            && let Some(bit) = k.modifier
            && !self.left_down.iter().any(|l| l.0 == k.code)
        {
            self.left_down.push((k.code, bit));
        }
        if k.num_lock && self.cfg.shortcuts && self.mouse_chord(k.code) {
            self.shift_run = (0, now);
            self.eaten = Some(k.code);
            self.shortcut(Aid::MouseKeys, fx);
            return true;
        }
        self.right_shift_since = (k.right_shift && alone).then_some(now);
        let shift = k.modifier == Some(MOD_SHIFT);
        if !shift || !alone {
            self.shift_run = (0, now);
            return false;
        }
        let (count, last) = self.shift_run;
        let count = if count > 0 && now - last <= SHIFT_GAP_MS {
            count + 1
        } else {
            1
        };
        self.shift_run = (count, now);
        if count == SHIFT_PRESSES {
            self.shift_run = (0, now);
            self.shortcut(Aid::StickyKeys, fx);
        }
        false
    }

    /// Whether Num Lock (`num_lock`) going down now makes the MouseKeys
    /// shortcut: left Alt and left Shift are down and nothing else.
    fn mouse_chord(&self, num_lock: u32) -> bool {
        let held = |bit| self.left_down.iter().any(|l| l.1 == bit);
        held(MOD_ALT)
            && held(MOD_SHIFT)
            && self
                .down
                .iter()
                .all(|&c| c == num_lock || self.left_down.iter().any(|l| l.0 == c))
    }

    fn shortcut(&mut self, aid: Aid, fx: &mut Effects) {
        if !self.cfg.shortcuts {
            return;
        }
        let on = match aid {
            Aid::StickyKeys => self.cfg.sticky_keys,
            Aid::SlowKeys => self.cfg.slow_keys,
            Aid::MouseKeys => self.cfg.mouse_keys,
        };
        fx.notice = Some(if on {
            self.turn_off(aid, fx);
            Notice::Off(aid)
        } else {
            Notice::Prompt(aid)
        });
    }

    fn turn_off(&mut self, aid: Aid, fx: &mut Effects) {
        match aid {
            Aid::StickyKeys => {
                self.cfg.sticky_keys = false;
                fx.mods_changed |= self.clear_sticky();
            }
            Aid::SlowKeys => {
                self.cfg.slow_keys = false;
                self.delivering.clear();
            }
            // `mousekeys` lets go of its held button once the config says
            // it is off.
            Aid::MouseKeys => self.cfg.mouse_keys = false,
        }
    }

    fn clear_sticky(&mut self) -> bool {
        let had = self.latched | self.locked != 0;
        self.latched = 0;
        self.locked = 0;
        self.held.clear();
        self.used = false;
        had
    }

    fn sticky(&mut self, k: Key, fx: &mut Effects) {
        if !self.cfg.sticky_keys {
            return;
        }
        if k.pressed {
            if self.held.iter().any(|h| h.0 != k.code) {
                self.used = true;
                if self.cfg.sticky_two_keys_off {
                    self.turn_off(Aid::StickyKeys, fx);
                    fx.notice = Some(Notice::Off(Aid::StickyKeys));
                    return;
                }
            }
            match k.modifier {
                Some(bit) => {
                    if !self.held.iter().any(|h| h.0 == k.code) {
                        self.held.push((k.code, bit));
                    }
                }
                None if self.latched != 0 => {
                    self.latched = 0;
                    fx.mods_changed = true;
                }
                None => {}
            }
            return;
        }
        let Some(i) = self.held.iter().position(|h| h.0 == k.code) else {
            return;
        };
        let (_, bit) = self.held.remove(i);
        if !self.used {
            if self.locked & bit != 0 {
                self.locked &= !bit;
            } else if self.latched & bit != 0 {
                self.latched &= !bit;
                if self.cfg.sticky_lock {
                    self.locked |= bit;
                }
            } else {
                self.latched |= bit;
            }
            fx.mods_changed = true;
        }
        if self.held.is_empty() {
            self.used = false;
        }
    }
}

/// The filter as the session runs it: the model, its timer, and the xkb
/// latched bits last put on the seat keyboard for it.
#[derive(Debug, Default)]
pub struct KeyAids {
    pub filter: KeyFilter,
    timer: Option<RegistrationToken>,
    applied: u32,
    /// The latched or locked modifiers changed on a key that is still
    /// being routed: `aids_after_key` hands them over once it has been.
    resync: bool,
    /// A shortcut's prompt, asked by `aids_after_key` so the key that
    /// asked for it does not land in it.
    prompt: Option<Aid>,
}

impl State {
    fn aids_now(&self) -> u64 {
        Duration::from(self.clock.now()).as_millis() as u64
    }

    /// The config changed (or the session starts): follow
    /// `[accessibility]`.
    pub fn aids_configured(&mut self) {
        let cfg = self.config.accessibility.clone();
        let aids = &mut self.key_input.aids;
        let was_sticky = aids.filter.config().sticky_keys;
        aids.filter.configure(&cfg);
        self.sync_aids();
        self.arm_aids();
        if was_sticky != cfg.sticky_keys {
            self.redraw_taskbars();
        }
    }

    /// Run key `code` (decoded as `k`) through the filter. False means it
    /// is swallowed: the caller routes nothing and the keyboard's xkb
    /// state skips it too. True: route it, then call `aids_after_key`.
    pub fn aids_key(&mut self, code: u32, k: &KeyPress) -> bool {
        let key = Key {
            code,
            modifier: k.syms.iter().find_map(|&s| modifier_key(s)).map(|m| m.1),
            right_shift: k.syms.contains(&KEY_Shift_R),
            left: k.syms.iter().any(|&s| s == KEY_Shift_L || s == KEY_Alt_L),
            num_lock: k.syms.contains(&KEY_Num_Lock),
            pressed: k.pressed,
        };
        let now = self.aids_now();
        let (verdict, fx) = self.key_input.aids.filter.key(key, now);
        self.key_input.aids.resync |= fx.mods_changed;
        self.aids_effects(fx);
        self.arm_aids();
        if verdict == Verdict::Swallow {
            // Nothing is routed, so there is no after.
            self.aids_after_key();
            return false;
        }
        true
    }

    /// A key went through its route: hand over the modifiers it latched
    /// or used up, and ask what its shortcut asks.
    pub fn aids_after_key(&mut self) {
        if std::mem::take(&mut self.key_input.aids.resync) {
            self.sync_aids();
            self.redraw_taskbars();
        }
        if let Some(aid) = self.key_input.aids.prompt.take()
            && !self.is_locked()
        {
            self.open_dialog(crate::dialog::Dialog::key_aid_prompt(aid));
        }
    }

    /// A pointer press was delivered: it used up the latch.
    pub fn aids_button(&mut self) {
        if self.key_input.aids.filter.button() {
            self.sync_aids();
            self.redraw_taskbars();
        }
    }

    /// Put the filter's modifiers in the seat keyboard's latched mask,
    /// next to whatever xkb latched itself, and tell the focused client.
    /// Returns whether that changed the keyboard.
    pub fn sync_aids(&mut self) -> bool {
        let f = &self.key_input.aids.filter;
        let bits = f.latched() | f.locked();
        let Some((mut mask, indexes)) = self.xkb_mask() else {
            return false;
        };
        let want = xkb_mask(&indexes, bits);
        let latched = (mask.latched & !self.key_input.aids.applied) | want;
        self.key_input.aids.applied = want;
        if latched == mask.latched {
            return false;
        }
        mask.latched = latched;
        self.set_xkb_mask(mask);
        true
    }

    fn arm_aids(&mut self) {
        if let Some(t) = self.key_input.aids.timer.take() {
            self.handle.remove(t);
        }
        let Some(deadline) = self.key_input.aids.filter.deadline() else {
            return;
        };
        let ms = deadline.saturating_sub(self.aids_now()).max(1);
        let token = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(ms)),
            |_, _, state| {
                state.key_input.aids.timer = None;
                let now = state.aids_now();
                let fx = state.key_input.aids.filter.tick(now);
                state.key_input.aids.resync |= fx.mods_changed;
                state.aids_effects(fx);
                state.aids_after_key();
                state.arm_aids();
                TimeoutAction::Drop
            },
        );
        match token {
            Ok(t) => self.key_input.aids.timer = Some(t),
            Err(e) => log::warn!("key aids timer: {}", e.error),
        }
    }

    fn aids_effects(&mut self, fx: Effects) {
        for code in fx.accept {
            // The press comes back through the key handler; its release
            // comes from the keyboard as usual.
            self.handle_key(Keycode::new(code), true, InputTime::now());
        }
        match fx.notice {
            Some(Notice::Prompt(aid)) => self.key_input.aids.prompt = Some(aid),
            Some(Notice::Off(aid)) => {
                log::info!("{} turned off from the keyboard", aid.name());
                self.store_aid(aid, false);
                self.mousekeys_configured();
            }
            None => {}
        }
    }

    /// Turn `aid` on or off: live at once, and in config.toml.
    pub fn set_aid(&mut self, aid: Aid, on: bool) {
        self.store_aid(aid, on);
        self.aids_configured();
        self.mousekeys_configured();
    }

    /// Write `aid`'s switch into the live config and the file.
    fn store_aid(&mut self, aid: Aid, on: bool) {
        let a = &mut self.config.accessibility;
        match aid {
            Aid::StickyKeys => a.sticky_keys = on,
            Aid::SlowKeys => a.slow_keys = on,
            Aid::MouseKeys => a.mouse_keys = on,
        }
        let key = repriseconf::key("accessibility", aid.key());
        crate::configwrite::save_flag(&self.config_path, key, on);
        self.redraw_taskbars();
    }
}

fn xkb_mask(indexes: &[u32], bits: u32) -> u32 {
    indexes
        .iter()
        .enumerate()
        .filter(|&(i, &idx)| bits & (1 << i) != 0 && idx < 32)
        .fold(0, |m, (_, &idx)| m | 1 << idx)
}

/// Whether the tray shows the StickyKeys cell: while StickyKeys is on.
pub fn shown(aids: &KeyAids) -> bool {
    aids.filter.config().sticky_keys
}

pub fn tip(aids: &KeyAids) -> String {
    let f = &aids.filter;
    let names = |bits: u32| {
        MODIFIERS
            .iter()
            .filter(|(b, _)| bits & b != 0)
            .map(|(_, n)| *n)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut tip = "StickyKeys".to_string();
    if f.latched() != 0 {
        tip += &format!("\nHeld for the next key: {}", names(f.latched()));
    }
    if f.locked() != 0 {
        tip += &format!("\nLocked: {}", names(f.locked()));
    }
    tip
}

const MODIFIERS: [(u32, &str); 4] = [
    (MOD_SHIFT, "Shift"),
    (MOD_CTRL, "Ctrl"),
    (MOD_ALT, "Alt"),
    (MOD_LOGO, "Super"),
];

#[cfg(test)]
mod tests {
    use super::*;

    const LSHIFT: u32 = 50;
    const RSHIFT: u32 = 62;
    const CTRL: u32 = 37;
    const ALT: u32 = 64;
    const SUPER: u32 = 133;
    const A: u32 = 38;
    const B: u32 = 56;
    const NUMLOCK: u32 = 77;

    fn key(code: u32, pressed: bool) -> Key {
        let modifier = match code {
            LSHIFT | RSHIFT => Some(MOD_SHIFT),
            CTRL => Some(MOD_CTRL),
            ALT => Some(MOD_ALT),
            SUPER => Some(MOD_LOGO),
            _ => None,
        };
        Key {
            code,
            modifier,
            right_shift: code == RSHIFT,
            left: code == LSHIFT || code == ALT,
            num_lock: code == NUMLOCK,
            pressed,
        }
    }

    fn sticky() -> KeyFilter {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig {
            sticky_keys: true,
            ..Default::default()
        });
        f
    }

    fn slow(delay: u32) -> KeyFilter {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig {
            slow_keys: true,
            slow_keys_delay: delay,
            ..Default::default()
        });
        f
    }

    fn tap(f: &mut KeyFilter, code: u32, t: u64) -> Option<Notice> {
        let a = f.key(key(code, true), t).1.notice;
        let b = f.key(key(code, false), t + 50).1.notice;
        a.or(b)
    }

    #[test]
    fn a_modifier_tap_latches_it_for_the_next_key_only() {
        let mut f = sticky();
        let (v, fx) = f.key(key(LSHIFT, true), 0);
        assert_eq!((v, fx.mods_changed), (Verdict::Pass, false));
        let fx = f.key(key(LSHIFT, false), 50).1;
        assert!(fx.mods_changed);
        assert_eq!(f.latched(), MOD_SHIFT);
        // The next key goes through with Shift and takes the latch.
        let (v, fx) = f.key(key(A, true), 100);
        assert_eq!(v, Verdict::Pass);
        assert!(fx.mods_changed);
        assert_eq!(f.latched(), 0, "the latch goes with the key");
        f.key(key(A, false), 150);
        f.key(key(B, true), 200);
        assert_eq!(f.latched(), 0, "b is plain");
    }

    #[test]
    fn taps_of_different_modifiers_latch_together() {
        let mut f = sticky();
        tap(&mut f, CTRL, 0);
        tap(&mut f, ALT, 100);
        assert_eq!(f.latched(), MOD_CTRL | MOD_ALT);
    }

    #[test]
    fn a_second_tap_locks_and_a_third_unlocks() {
        let mut f = sticky();
        tap(&mut f, SUPER, 0);
        tap(&mut f, SUPER, 100);
        assert_eq!((f.latched(), f.locked()), (0, MOD_LOGO));
        f.key(key(A, true), 200);
        f.key(key(A, false), 250);
        assert_eq!(f.locked(), MOD_LOGO, "a key leaves a lock");
        tap(&mut f, SUPER, 300);
        assert_eq!((f.latched(), f.locked()), (0, 0));
    }

    #[test]
    fn without_the_lock_a_second_tap_lets_go() {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig {
            sticky_keys: true,
            sticky_lock: false,
            ..Default::default()
        });
        tap(&mut f, ALT, 0);
        tap(&mut f, ALT, 100);
        assert_eq!((f.latched(), f.locked()), (0, 0));
    }

    #[test]
    fn two_keys_at_once_turn_sticky_keys_off() {
        let mut f = sticky();
        tap(&mut f, CTRL, 0);
        f.key(key(LSHIFT, true), 100);
        let (v, fx) = f.key(key(A, true), 150);
        assert_eq!(v, Verdict::Pass, "the key itself still goes through");
        assert_eq!(fx.notice, Some(Notice::Off(Aid::StickyKeys)));
        assert!(fx.mods_changed, "the Ctrl latch is dropped");
        assert!(!f.config().sticky_keys);
        assert_eq!(f.latched(), 0);
        f.key(key(A, false), 200);
        f.key(key(LSHIFT, false), 250);
        assert_eq!(f.latched(), 0, "letting go of Shift latches nothing now");
    }

    #[test]
    fn a_chord_latches_nothing_when_two_keys_are_allowed() {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig {
            sticky_keys: true,
            sticky_two_keys_off: false,
            ..Default::default()
        });
        f.key(key(CTRL, true), 0);
        let fx = f.key(key(A, true), 50).1;
        assert_eq!(fx.notice, None);
        f.key(key(A, false), 100);
        f.key(key(CTRL, false), 150);
        assert_eq!(f.latched(), 0, "Ctrl was used with a");
        assert!(f.config().sticky_keys);
    }

    #[test]
    fn a_pointer_button_takes_the_latch() {
        let mut f = sticky();
        tap(&mut f, CTRL, 0);
        tap(&mut f, SUPER, 100);
        tap(&mut f, SUPER, 200);
        assert!(f.button());
        assert_eq!((f.latched(), f.locked()), (0, MOD_LOGO));
        assert!(!f.button(), "nothing left to take");
    }

    #[test]
    fn turning_sticky_keys_off_drops_every_latch_and_lock() {
        let mut f = sticky();
        tap(&mut f, CTRL, 0);
        tap(&mut f, ALT, 100);
        tap(&mut f, ALT, 200);
        assert!(f.configure(&AccessibilityConfig::default()));
        assert_eq!((f.latched(), f.locked()), (0, 0));
        tap(&mut f, CTRL, 300);
        assert_eq!(f.latched(), 0, "off means off");
    }

    #[test]
    fn slow_keys_drops_a_short_tap_and_accepts_a_held_key() {
        let mut f = slow(500);
        assert_eq!(f.key(key(A, true), 0).0, Verdict::Swallow);
        assert_eq!(f.deadline(), Some(500));
        assert_eq!(f.key(key(A, false), 300).0, Verdict::Swallow);
        assert_eq!(f.deadline(), None, "the tap is gone");
        assert!(f.tick(600).accept.is_empty());

        assert_eq!(f.key(key(B, true), 1000).0, Verdict::Swallow);
        assert!(f.tick(1499).accept.is_empty());
        assert_eq!(f.tick(1500).accept, vec![B]);
        // The accepted press comes back in and goes through.
        assert_eq!(f.key(key(B, true), 1500).0, Verdict::Pass);
        assert_eq!(f.key(key(B, false), 1800).0, Verdict::Pass);
    }

    #[test]
    fn slow_keys_then_sticky_keys_latch_an_accepted_modifier() {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig {
            sticky_keys: true,
            slow_keys: true,
            slow_keys_delay: 300,
            ..Default::default()
        });
        f.key(key(CTRL, true), 0);
        assert_eq!(f.tick(300).accept, vec![CTRL]);
        f.key(key(CTRL, true), 300);
        assert_eq!(f.key(key(CTRL, false), 400).0, Verdict::Pass);
        assert_eq!(f.latched(), MOD_CTRL);
        // A rejected tap of Shift neither latches nor takes the Ctrl latch.
        f.key(key(LSHIFT, true), 500);
        f.key(key(LSHIFT, false), 600);
        assert_eq!(f.latched(), MOD_CTRL);
    }

    #[test]
    fn a_release_of_a_held_back_press_is_swallowed_after_slow_keys_goes_off() {
        let mut f = slow(1000);
        f.key(key(A, true), 0);
        f.configure(&AccessibilityConfig::default());
        assert_eq!(f.deadline(), None);
        assert_eq!(f.key(key(A, false), 100).0, Verdict::Swallow);
        assert_eq!(f.key(key(A, true), 200).0, Verdict::Pass);
    }

    #[test]
    fn five_shift_presses_offer_sticky_keys_and_turn_it_off_when_on() {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig::default());
        for i in 0..4 {
            assert_eq!(tap(&mut f, LSHIFT, i * 200), None);
        }
        assert_eq!(
            tap(&mut f, RSHIFT, 800),
            Some(Notice::Prompt(Aid::StickyKeys))
        );
        let mut f = sticky();
        let notices: Vec<_> = (0..5)
            .filter_map(|i| tap(&mut f, LSHIFT, i * 200))
            .collect();
        assert_eq!(notices, vec![Notice::Off(Aid::StickyKeys)]);
        assert!(!f.config().sticky_keys);
        assert_eq!((f.latched(), f.locked()), (0, 0));
    }

    #[test]
    fn the_shift_run_breaks_on_another_key_a_held_key_or_a_pause() {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig::default());
        let run = |f: &mut KeyFilter, start: u64| {
            (0..4)
                .filter_map(|i| tap(f, LSHIFT, start + i * 200))
                .collect::<Vec<_>>()
        };
        assert!(run(&mut f, 0).is_empty());
        tap(&mut f, A, 700);
        assert!(run(&mut f, 800).is_empty(), "a restarted the count");
        assert_eq!(tap(&mut f, LSHIFT, 2500), None, "over a second since");
        // Shift tapped under a held key is not the shortcut.
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig::default());
        f.key(key(A, true), 0);
        assert!((0..5).all(|i| tap(&mut f, LSHIFT, 10 + i * 100).is_none()));
    }

    #[test]
    fn right_shift_held_eight_seconds_offers_slow_keys() {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig::default());
        f.key(key(RSHIFT, true), 1000);
        assert_eq!(f.deadline(), Some(9000));
        assert_eq!(f.tick(8999).notice, None);
        assert_eq!(f.tick(9000).notice, Some(Notice::Prompt(Aid::SlowKeys)));
        assert_eq!(f.deadline(), None, "once per hold");
        // Let go early, or with another key down: nothing.
        f.key(key(RSHIFT, false), 9500);
        f.key(key(RSHIFT, true), 10_000);
        f.key(key(RSHIFT, false), 12_000);
        assert_eq!(f.deadline(), None);
        f.key(key(A, true), 13_000);
        f.key(key(RSHIFT, true), 13_100);
        assert_eq!(f.deadline(), None);
    }

    #[test]
    fn the_hold_turns_slow_keys_off_when_on() {
        let mut f = slow(300);
        f.key(key(RSHIFT, true), 0);
        assert_eq!(f.deadline(), Some(300), "the acceptance comes first");
        assert_eq!(f.tick(300).accept, vec![RSHIFT]);
        f.key(key(RSHIFT, true), 300);
        assert_eq!(f.tick(8000).notice, Some(Notice::Off(Aid::SlowKeys)));
        assert!(!f.config().slow_keys);
        assert_eq!(f.key(key(RSHIFT, false), 8100).0, Verdict::Pass);
    }

    #[test]
    fn the_xkb_mask_follows_the_keymaps_indexes() {
        // Shift, Caps, Ctrl, Alt, Num, Mod3, Super, Mod5 as xkb numbers them.
        let idx = [0, 1, 2, 3, 4, 5, 6, 7];
        assert_eq!(xkb_mask(&idx, MOD_SHIFT | MOD_LOGO), 1 | 1 << 6);
        let odd = [0, 1, 2, 5, 4, 5, 3, u32::MAX];
        assert_eq!(xkb_mask(&odd, MOD_ALT | MOD_LOGO), 1 << 5 | 1 << 3);
        assert_eq!(xkb_mask(&odd, MOD_MOD5), 0, "no index, no bit");
    }

    #[test]
    fn the_tip_names_what_is_held_and_locked() {
        let mut aids = KeyAids::default();
        aids.filter.configure(&AccessibilityConfig {
            sticky_keys: true,
            ..Default::default()
        });
        assert_eq!(tip(&aids), "StickyKeys");
        tap(&mut aids.filter, CTRL, 0);
        tap(&mut aids.filter, LSHIFT, 100);
        tap(&mut aids.filter, SUPER, 200);
        tap(&mut aids.filter, SUPER, 300);
        assert_eq!(
            tip(&aids),
            "StickyKeys\nHeld for the next key: Shift, Ctrl\nLocked: Super"
        );
    }

    #[test]
    fn shortcuts_off_keeps_both_quiet() {
        let mut f = KeyFilter::default();
        f.configure(&AccessibilityConfig {
            shortcuts: false,
            ..Default::default()
        });
        assert!((0..5).all(|i| tap(&mut f, LSHIFT, i * 100).is_none()));
        f.key(key(RSHIFT, true), 1000);
        assert_eq!(f.deadline(), None);
        assert_eq!(f.tick(20_000).notice, None);
        // Nor does left Alt+left Shift+Num Lock, which then toggles Num
        // Lock as usual.
        f.key(key(ALT, true), 2000);
        f.key(key(LSHIFT, true), 2010);
        let (v, fx) = f.key(key(NUMLOCK, true), 2020);
        assert_eq!((v, fx.notice), (Verdict::Pass, None));
    }

    #[test]
    fn left_alt_left_shift_num_lock_offers_mouse_keys_and_turns_it_off_when_on() {
        let mut f = KeyFilter::default();
        f.key(key(ALT, true), 0);
        f.key(key(LSHIFT, true), 10);
        let (v, fx) = f.key(key(NUMLOCK, true), 20);
        assert_eq!(v, Verdict::Swallow, "Num Lock stays as it was");
        assert_eq!(fx.notice, Some(Notice::Prompt(Aid::MouseKeys)));
        assert_eq!(f.key(key(NUMLOCK, false), 60).0, Verdict::Swallow);
        f.key(key(LSHIFT, false), 70);
        f.key(key(ALT, false), 80);
        // A Num Lock on its own is just Num Lock.
        assert_eq!(
            f.key(key(NUMLOCK, true), 200),
            (Verdict::Pass, Effects::default())
        );
        f.key(key(NUMLOCK, false), 250);
        // Right Shift, or another key held as well, is not the shortcut.
        f.key(key(ALT, true), 300);
        f.key(key(RSHIFT, true), 310);
        assert_eq!(f.key(key(NUMLOCK, true), 320).1.notice, None);
        f.key(key(NUMLOCK, false), 330);
        f.key(key(RSHIFT, false), 340);
        f.key(key(LSHIFT, true), 350);
        f.key(key(A, true), 360);
        assert_eq!(f.key(key(NUMLOCK, true), 370).1.notice, None);
        for c in [NUMLOCK, A, LSHIFT, ALT] {
            f.key(key(c, false), 400);
        }

        f.configure(&AccessibilityConfig {
            mouse_keys: true,
            ..Default::default()
        });
        f.key(key(LSHIFT, true), 1000);
        f.key(key(ALT, true), 1010);
        let fx = f.key(key(NUMLOCK, true), 1020).1;
        assert_eq!(fx.notice, Some(Notice::Off(Aid::MouseKeys)));
        assert!(!f.config().mouse_keys);
    }
}
