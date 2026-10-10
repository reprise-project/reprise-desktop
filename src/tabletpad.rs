//! Tablet pads: the buttons, rings and strips beside a drawing tablet's
//! surface, handled by Reprise itself. `[[tablet_pads]]` gives each button
//! and each step of a ring or strip an action or a key chord; a chord
//! reaches the focused client as key presses through `handle_key`, so
//! binds and the lock see it as a typed one, and stays down while the
//! button does. Clients see no tablet-v2 pad objects.
//!
//! A pad with modes (libinput reads libwacom's mode-switch buttons) picks
//! its mapping by libinput's mode; any other pad counts its own, advanced by
//! a button bound to `mode`. The connected pads are listed in a `Shared`
//! the settings sheet reads over D-Bus (`org.reprise.Tray.TabletPads`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use smithay::backend::input::InputTime;
use smithay::reexports::input::{
    self as libinput, DeviceCapability,
    event::{
        EventTrait,
        tablet_pad::{ButtonState, TabletPadEvent, TabletPadEventTrait},
    },
};

use crate::compositor::State;
use crate::config::{Action, parse_keybind};
use crate::keys::{KEY_Alt_L, KEY_Control_L, KEY_Shift_L, KEY_Super_L};
use crate::keys::{MOD_ALT, MOD_CTRL, MOD_LOGO, MOD_SHIFT};

/// Degrees of ring travel per step.
const RING_STEP: f64 = 15.0;
/// Strip travel per step, of its whole length.
const STRIP_STEP: f64 = 0.1;

/// What a pad button, or one step of a ring or strip, does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binding {
    /// A compositor action, by its keybind name ("zoom-in").
    Action(Action),
    /// A chord pressed for the focused client: `keys::MOD_*` bits and the
    /// lowercase keysym, as a keybind spells them ("ctrl+plus").
    Keys { mods: u32, keysym: u32 },
    /// The next mode, after the last the first.
    Mode,
}

impl Binding {
    /// `s` as a binding: an action name, "mode", or a key chord. Action
    /// names come first, so "next" is the media action, not the Next key
    /// (spell that "Page_Down"). None for "" and "none".
    pub fn parse(s: &str) -> Result<Option<Binding>, String> {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        if s == "mode" {
            return Ok(Some(Binding::Mode));
        }
        if s != "spawn"
            && let Ok(a) = Action::from_parts(s, None)
        {
            return Ok(Some(Binding::Action(a)));
        }
        let (mods, keysym) =
            parse_keybind(s).map_err(|e| format!("{s:?} is no action, and as keys: {e}"))?;
        Ok(Some(Binding::Keys { mods, keysym }))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mapping {
    /// By button number, the first button 1.
    pub buttons: BTreeMap<u32, Binding>,
    pub ring_up: Option<Binding>,
    pub ring_down: Option<Binding>,
    pub strip_up: Option<Binding>,
    pub strip_down: Option<Binding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadConfig {
    /// A glob over the pad's device name.
    pub name: String,
    /// Every mode's bindings, under `modes`' own.
    pub base: Mapping,
    /// `[[tablet_pads.modes]]`: mode i's bindings over `base`.
    pub modes: Vec<Mapping>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// libinput's button number, the first 0.
    Button(u32),
    RingUp,
    RingDown,
    StripUp,
    StripDown,
}

impl Mapping {
    fn get(&self, c: Control) -> Option<&Binding> {
        match c {
            Control::Button(n) => self.buttons.get(&(n + 1)),
            Control::RingUp => self.ring_up.as_ref(),
            Control::RingDown => self.ring_down.as_ref(),
            Control::StripUp => self.strip_up.as_ref(),
            Control::StripDown => self.strip_down.as_ref(),
        }
    }
}

impl PadConfig {
    /// What `c` does in `mode`: the mode's own binding, else the entry's.
    pub fn binding(&self, mode: usize, c: Control) -> Option<&Binding> {
        (self.modes.get(mode).and_then(|m| m.get(c))).or_else(|| self.base.get(c))
    }

    /// How many modes a `mode` binding cycles through.
    pub fn mode_count(&self) -> usize {
        self.modes.len().max(1)
    }
}

pub fn find<'a>(list: &'a [PadConfig], name: &str) -> Option<&'a PadConfig> {
    list.iter().find(|p| repriseconf::glob::glob(&p.name, name))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePad {
    name: Option<String>,
    buttons: Option<BTreeMap<String, String>>,
    ring_up: Option<String>,
    ring_down: Option<String>,
    strip_up: Option<String>,
    strip_down: Option<String>,
    modes: Option<Vec<FileMapping>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileMapping {
    buttons: Option<BTreeMap<String, String>>,
    ring_up: Option<String>,
    ring_down: Option<String>,
    strip_up: Option<String>,
    strip_down: Option<String>,
}

impl FileMapping {
    fn parse(self, at: &str) -> Result<Mapping, String> {
        let one = |key: &str, v: Option<String>| match v {
            None => Ok(None),
            Some(v) => Binding::parse(&v).map_err(|e| format!("{at}: {key}: {e}")),
        };
        let mut buttons = BTreeMap::new();
        for (k, v) in self.buttons.unwrap_or_default() {
            let n = k.parse::<u32>().ok().filter(|&n| n >= 1).ok_or_else(|| {
                format!("{at}: buttons: {k:?} is no button number (1 is the first)")
            })?;
            if let Some(b) = one(&format!("buttons.{k}"), Some(v))? {
                buttons.insert(n, b);
            }
        }
        Ok(Mapping {
            buttons,
            ring_up: one("ring_up", self.ring_up)?,
            ring_down: one("ring_down", self.ring_down)?,
            strip_up: one("strip_up", self.strip_up)?,
            strip_down: one("strip_down", self.strip_down)?,
        })
    }
}

impl FilePad {
    pub fn parse(self, at: &str) -> Result<PadConfig, String> {
        let name = self.name.unwrap_or_else(|| "*".into());
        let base = FileMapping {
            buttons: self.buttons,
            ring_up: self.ring_up,
            ring_down: self.ring_down,
            strip_up: self.strip_up,
            strip_down: self.strip_down,
        }
        .parse(at)?;
        let modes = (self.modes.unwrap_or_default().into_iter().enumerate())
            .map(|(i, m)| m.parse(&format!("{at} modes #{}", i + 1)))
            .collect::<Result<_, _>>()?;
        Ok(PadConfig { name, base, modes })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Dial {
    last: Option<f64>,
    /// Travel not yet spent on a step, signed as the steps are.
    acc: f64,
}

impl Dial {
    /// A ring at `pos` degrees (clockwise from the top, -1 when the finger
    /// lifts): the steps since the last position, clockwise positive.
    pub fn ring(&mut self, pos: f64) -> i32 {
        self.travel(pos, |d| (d + 180.0).rem_euclid(360.0) - 180.0, RING_STEP)
    }

    /// A strip at `pos` (0 at its top to 1, -1 when the finger lifts): the
    /// steps since the last position, upward positive.
    pub fn strip(&mut self, pos: f64) -> i32 {
        self.travel(pos, |d| -d, STRIP_STEP)
    }

    fn travel(&mut self, pos: f64, delta: impl Fn(f64) -> f64, step: f64) -> i32 {
        if pos < 0.0 {
            *self = Dial::default();
            return 0;
        }
        if let Some(last) = self.last.replace(pos) {
            self.acc += delta(pos - last);
        }
        let steps = (self.acc / step).trunc();
        self.acc -= steps * step;
        steps as i32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PadInfo {
    pub name: String,
    pub buttons: u32,
    pub rings: u32,
    pub strips: u32,
}

pub type Shared = Arc<Mutex<Vec<PadInfo>>>;

struct Pad {
    device: libinput::Device,
    info: PadInfo,
    /// The mode a pad without libinput modes is in.
    mode: usize,
    rings: Vec<Dial>,
    strips: Vec<Dial>,
    /// Each held button's chord: the evdev codes it pressed, in order.
    held: Vec<(u32, Vec<u32>)>,
}

#[derive(Default)]
pub struct Pads {
    pads: Vec<Pad>,
    shared: Shared,
}

impl Pads {
    pub fn shared(&self) -> Shared {
        self.shared.clone()
    }

    fn publish(&self) {
        let list = self.pads.iter().map(|p| p.info.clone()).collect();
        *self.shared.lock().unwrap_or_else(|e| e.into_inner()) = list;
    }
}

pub fn added(state: &mut State, device: &libinput::Device) {
    if !device.has_capability(DeviceCapability::TabletPad) {
        return;
    }
    let count = |n: i32| u32::try_from(n).unwrap_or(0);
    let info = PadInfo {
        name: device.name().into_owned(),
        buttons: count(device.tablet_pad_number_of_buttons()),
        rings: count(device.tablet_pad_number_of_rings()),
        strips: count(device.tablet_pad_number_of_strips()),
    };
    log::info!(
        "new tablet pad {}: {} buttons, {} rings, {} strips",
        info.name,
        info.buttons,
        info.rings,
        info.strips
    );
    let pads = &mut state.devices.pads;
    pads.pads.push(Pad {
        device: device.clone(),
        rings: vec![Dial::default(); info.rings as usize],
        strips: vec![Dial::default(); info.strips as usize],
        info,
        mode: 0,
        held: Vec::new(),
    });
    pads.publish();
}

/// A libinput device went away: forget it if it was a pad, releasing its held keys.
pub fn removed(state: &mut State, device: &libinput::Device) {
    let pads = &mut state.devices.pads;
    let Some(i) = pads.pads.iter().position(|p| &p.device == device) else {
        return;
    };
    let pad = pads.pads.remove(i);
    pads.publish();
    for (_, codes) in pad.held {
        state.pad_chord_up(codes);
    }
}

pub fn event(state: &mut State, e: TabletPadEvent) {
    crate::idle::notify_activity(state);
    let device = e.device();
    let Some(i) = state
        .devices
        .pads
        .pads
        .iter()
        .position(|p| p.device == device)
    else {
        return;
    };
    let group = e.mode_group();
    let libinput_modes = group.number_of_modes() > 1;
    let pad = &mut state.devices.pads.pads[i];
    let mode = if libinput_modes {
        e.mode() as usize
    } else {
        pad.mode
    };
    match e {
        TabletPadEvent::Button(b) => {
            let n = b.button_number();
            if b.button_state() == ButtonState::Released {
                if let Some(h) = pad.held.iter().position(|(btn, _)| *btn == n) {
                    let (_, codes) = pad.held.swap_remove(h);
                    state.pad_chord_up(codes);
                }
                return;
            }
            let Some(binding) = state.pad_binding(i, mode, Control::Button(n)) else {
                return;
            };
            match binding {
                Binding::Keys { mods, keysym } => {
                    let codes = state.pad_chord_down(mods, keysym);
                    state.devices.pads.pads[i].held.push((n, codes));
                }
                Binding::Mode if libinput_modes => {
                    log::debug!(
                        "tablet pad: libinput counts this pad's modes; `mode` does nothing"
                    );
                }
                Binding::Mode => {
                    let count = find(&state.config.tablet_pads, &device.name())
                        .map_or(1, PadConfig::mode_count);
                    let pad = &mut state.devices.pads.pads[i];
                    pad.mode = (pad.mode + 1) % count;
                    log::info!("tablet pad {}: mode {}", pad.info.name, pad.mode + 1);
                }
                Binding::Action(a) => state.pad_action(&a),
            }
        }
        TabletPadEvent::Ring(r) => {
            let Some(dial) = pad.rings.get_mut(r.number() as usize) else {
                return;
            };
            let steps = dial.ring(r.position());
            state.pad_steps(i, mode, steps, Control::RingUp, Control::RingDown);
        }
        TabletPadEvent::Strip(s) => {
            let Some(dial) = pad.strips.get_mut(s.number() as usize) else {
                return;
            };
            let steps = dial.strip(s.position());
            state.pad_steps(i, mode, steps, Control::StripUp, Control::StripDown);
        }
        _ => {}
    }
}

const CHORD_MODS: [(u32, u32); 4] = [
    (MOD_CTRL, KEY_Control_L),
    (MOD_ALT, KEY_Alt_L),
    (MOD_SHIFT, KEY_Shift_L),
    (MOD_LOGO, KEY_Super_L),
];

impl State {
    /// Pad `i`'s binding for `c` in `mode`, cloned off the config.
    fn pad_binding(&self, i: usize, mode: usize, c: Control) -> Option<Binding> {
        let pad = &self.devices.pads.pads[i];
        find(&self.config.tablet_pads, &pad.info.name)?
            .binding(mode, c)
            .cloned()
    }

    /// `steps` of a ring or strip: each one up or down runs its binding,
    /// a chord as one tap.
    fn pad_steps(&mut self, i: usize, mode: usize, steps: i32, up: Control, down: Control) {
        if steps == 0 {
            return;
        }
        let c = if steps > 0 { up } else { down };
        let Some(binding) = self.pad_binding(i, mode, c) else {
            return;
        };
        for _ in 0..steps.unsigned_abs() {
            match &binding {
                Binding::Keys { mods, keysym } => {
                    let codes = self.pad_chord_down(*mods, *keysym);
                    self.pad_chord_up(codes);
                }
                Binding::Action(a) => self.pad_action(a),
                Binding::Mode => {}
            }
        }
    }

    fn pad_action(&mut self, a: &Action) {
        if self.is_locked() || self.inputproto.gated() {
            return;
        }
        self.run_action(a, 0);
    }

    /// Press `mods`, then the key giving `keysym` (with Shift when the
    /// keysym is that key's second level); the evdev codes pressed, in
    /// order.
    fn pad_chord_down(&mut self, mods: u32, keysym: u32) -> Vec<u32> {
        let Some((code, level)) = self.key_for_sym(keysym) else {
            log::warn!("tablet pad: no key gives keysym {keysym:#x}");
            return Vec::new();
        };
        let mods = if level == 1 { mods | MOD_SHIFT } else { mods };
        let mut codes: Vec<u32> = CHORD_MODS
            .iter()
            .filter(|(bit, _)| mods & bit != 0)
            .filter_map(|&(_, sym)| self.key_for_sym(sym).map(|(c, _)| c))
            .collect();
        codes.push(code);
        for &c in &codes {
            self.handle_key(crate::eis::evdev_key(c), true, InputTime::now());
        }
        codes
    }

    fn pad_chord_up(&mut self, codes: Vec<u32>) {
        for c in codes.into_iter().rev() {
            self.handle_key(crate::eis::evdev_key(c), false, InputTime::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{KEY_plus, KEY_z};

    fn parse(text: &str) -> Vec<PadConfig> {
        #[derive(Deserialize)]
        struct F {
            tablet_pads: Vec<FilePad>,
        }
        let f: F = toml::from_str(text).unwrap();
        (f.tablet_pads.into_iter().enumerate())
            .map(|(i, p)| p.parse(&format!("tablet_pads #{}", i + 1)))
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn a_binding_is_an_action_a_mode_switch_or_keys() {
        assert_eq!(
            Binding::parse("zoom-in"),
            Ok(Some(Binding::Action(Action::Zoom(
                crate::magnifier::ZoomStep::In
            ))))
        );
        assert_eq!(Binding::parse("mode"), Ok(Some(Binding::Mode)));
        assert_eq!(
            Binding::parse("ctrl+plus"),
            Ok(Some(Binding::Keys {
                mods: MOD_CTRL,
                keysym: KEY_plus
            }))
        );
        assert_eq!(
            Binding::parse("Ctrl+Shift+Z"),
            Ok(Some(Binding::Keys {
                mods: MOD_CTRL | MOD_SHIFT,
                keysym: KEY_z
            }))
        );
        assert_eq!(Binding::parse(""), Ok(None));
        assert_eq!(Binding::parse("none"), Ok(None));
        assert_eq!(
            Binding::parse("next"),
            Ok(Some(Binding::Action(Action::Next)))
        );
        assert!(Binding::parse("spawn").is_err(), "spawn needs a command");
        assert!(Binding::parse("ctrl+nosuchkey").is_err());
    }

    #[test]
    fn entries_parse_with_buttons_from_one_and_their_modes() {
        let pads = parse(
            r#"
            [[tablet_pads]]
            name = "Wacom Intuos* Pad"
            buttons = { 1 = "zoom-in", 3 = "ctrl+z", 4 = "none" }
            ring_up = "ctrl+plus"
            ring_down = "ctrl+minus"

            [[tablet_pads.modes]]
            [[tablet_pads.modes]]
            ring_up = "volume-up"

            [[tablet_pads]]
            strip_up = "desktop-prev"
            "#,
        );
        assert_eq!(pads.len(), 2);
        assert_eq!(
            pads[0].base.buttons.keys().copied().collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(pads[0].modes.len(), 2);
        assert_eq!(pads[1].name, "*", "no name matches every pad");
        assert_eq!(
            pads[1].base.strip_up,
            Some(Binding::Action(Action::Desktop(
                crate::desktops::Step::Prev
            )))
        );
    }

    #[test]
    fn a_bad_entry_says_where() {
        let err = |text: &str| {
            #[derive(Deserialize)]
            struct F {
                tablet_pads: Vec<FilePad>,
            }
            let f: F = toml::from_str(text).unwrap();
            f.tablet_pads
                .into_iter()
                .next()
                .unwrap()
                .parse("tablet_pads #1")
                .unwrap_err()
        };
        assert!(err("[[tablet_pads]]\nbuttons = { 0 = \"run\" }").contains("buttons: \"0\""));
        assert!(
            err("[[tablet_pads]]\nring_up = \"bogus+x\"").starts_with("tablet_pads #1: ring_up:")
        );
        assert!(
            err("[[tablet_pads]]\n[[tablet_pads.modes]]\nstrip_down = \"x+y\"")
                .starts_with("tablet_pads #1 modes #1: strip_down:")
        );
    }

    #[test]
    fn the_first_matching_entry_maps_a_pad() {
        let pads = parse(
            r#"
            [[tablet_pads]]
            name = "Wacom*"
            buttons = { 1 = "run" }
            [[tablet_pads]]
            buttons = { 1 = "launcher" }
            "#,
        );
        assert_eq!(
            find(&pads, "Wacom Intuos Pro M Pad").unwrap().name,
            "Wacom*"
        );
        assert_eq!(find(&pads, "HUION Kamvas Pad").unwrap().name, "*");
        assert!(find(&pads[..1], "HUION Kamvas Pad").is_none());
    }

    #[test]
    fn a_mode_overrides_only_what_it_binds() {
        let pads = parse(
            r#"
            [[tablet_pads]]
            buttons = { 1 = "run", 2 = "mode" }
            ring_up = "zoom-in"
            [[tablet_pads.modes]]
            [[tablet_pads.modes]]
            buttons = { 1 = "launcher" }
            "#,
        );
        let p = &pads[0];
        assert_eq!(p.mode_count(), 2);
        assert_eq!(
            p.binding(0, Control::Button(0)),
            Some(&Binding::Action(Action::Run))
        );
        assert_eq!(
            p.binding(1, Control::Button(0)),
            Some(&Binding::Action(Action::Launcher))
        );
        assert_eq!(p.binding(1, Control::Button(1)), Some(&Binding::Mode));
        assert_eq!(
            p.binding(1, Control::RingUp),
            Some(&Binding::Action(Action::Zoom(
                crate::magnifier::ZoomStep::In
            )))
        );
        assert_eq!(p.binding(0, Control::RingDown), None);
        assert_eq!(
            p.binding(5, Control::Button(0)),
            Some(&Binding::Action(Action::Run)),
            "a mode past the list takes the entry's"
        );
        assert_eq!(
            PadConfig {
                name: "*".into(),
                base: Mapping::default(),
                modes: vec![]
            }
            .mode_count(),
            1
        );
    }

    #[test]
    fn a_ring_steps_every_fifteen_degrees_across_the_top() {
        let mut d = Dial::default();
        assert_eq!(d.ring(350.0), 0, "a touch only sets the start");
        assert_eq!(d.ring(359.0), 0);
        assert_eq!(d.ring(5.0), 1, "clockwise through 0 is up");
        assert_eq!(d.ring(40.0), 2);
        assert_eq!(d.ring(10.0), -1, "30 degrees back from 5 past a step");
        assert_eq!(d.ring(-1.0), 0, "the finger lifts");
        assert_eq!(d.ring(100.0), 0, "a new touch starts over");
        assert_eq!(d.ring(60.0), -2);
        assert_eq!(d.ring(56.0), 0, "travel under a step waits");
        assert_eq!(d.ring(44.0), -1);
    }

    #[test]
    fn a_strip_steps_up_toward_its_top() {
        let mut d = Dial::default();
        assert_eq!(d.strip(0.5), 0);
        assert_eq!(d.strip(0.29), 2, "up the strip is up");
        assert_eq!(d.strip(0.62), -3);
        assert_eq!(d.strip(-1.0), 0);
        assert_eq!(d.strip(0.0), 0);
    }
}
