//! The login screen on the greeter's scene. Under `Host::Greeter` every key and press that no
//! open dialog takes comes here, ahead of the binds (`keyroute`).

use repriselogin::greeter::{GreeterReply, Greeting};
use smithay::backend::input::InputTime;
use smithay::utils::{Logical, Point};

use crate::compositor::State;
use crate::dialogview::DialogKey;

#[derive(Default)]
pub struct GreeterView {
    /// The link to reprise-login closed; the greeter exits 1.
    pub lost: bool,
    #[cfg(test)]
    pub heard: Vec<Heard>,
}

/// What reached the login screen, in order.
#[cfg(test)]
#[derive(Debug, PartialEq)]
pub enum Heard {
    Reply(GreeterReply),
    Key(Vec<u32>),
    Button(u32),
}

/// Starts the login screen from reprise-login's `greeting`.
pub fn setup(_state: &mut State, _greeting: Greeting) {}

/// One reply from reprise-login, in the order it sent them.
pub fn reply(state: &mut State, reply: GreeterReply) {
    #[cfg(test)]
    state.greeter.heard.push(Heard::Reply(reply.clone()));
    let _ = (state, reply);
}

/// The link to reprise-login closed or broke: nothing can log in through this greeter.
pub fn lost(state: &mut State) {
    log::error!("greeter: lost the link to the login manager");
    state.greeter.lost = true;
    state.signal.stop();
}

pub fn key(state: &mut State, key: &DialogKey) {
    #[cfg(test)]
    state.greeter.heard.push(Heard::Key(key.syms.clone()));
    let _ = (state, key);
}

pub fn button(state: &mut State, button: u32, _pos: Point<f64, Logical>) {
    #[cfg(test)]
    state.greeter.heard.push(Heard::Button(button));
    let _ = (state, button);
}

/// The pointer moved over the login screen: no client has it.
pub fn motion(state: &mut State, pos: Point<f64, Logical>, time: InputTime) {
    state.pointer_motion(None, pos, time);
    state.set_cursor("default");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::Host;
    use crate::config::{Action, Keybind};
    use crate::harness::Harness;
    use crate::keys::{KEY_Delete, KEY_r, MOD_ALT, MOD_CTRL};

    const RIGHT: u32 = 0x111;
    const LEFTMETA: u32 = 125;
    const LEFTCTRL: u32 = 29;
    const LEFTALT: u32 = 56;
    const KEY_R: u32 = 19;
    const ENTER: u32 = 28;
    const DELETE: u32 = 111;

    fn greeter() -> Harness {
        let mut config = crate::config::Config {
            base: repriseconf::config::Config {
                terminal: "true".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        config.keybinds.push(Keybind {
            mods: MOD_CTRL | MOD_ALT,
            keysym: KEY_Delete,
            action: Action::Run,
        });
        let mut h = Harness::with_config(config);
        h.state.host = Host::Greeter;
        h.output("TEST-1", 1024, 768);
        h
    }

    fn chord(h: &mut Harness, keys: &[u32]) {
        for &k in keys {
            h.key(k, true);
        }
        for &k in keys.iter().rev() {
            h.key(k, false);
        }
    }

    #[test]
    fn keybinds_never_fire_in_the_greeter() {
        let mut h = greeter();
        chord(&mut h, &[LEFTMETA]);
        chord(&mut h, &[LEFTMETA, KEY_R]);
        chord(&mut h, &[LEFTMETA, ENTER]);
        chord(&mut h, &[LEFTCTRL, LEFTALT, DELETE]);
        assert!(h.state.menus.is_empty(), "no Start menu");
        assert!(h.state.start_panel.is_none(), "no Start panel");
        assert!(h.state.dialogs.views.is_empty(), "no Run dialog");
        assert!(h.state.launcher.is_none(), "no launcher");
        let keys = h
            .state
            .greeter
            .heard
            .iter()
            .filter(|e| matches!(e, Heard::Key(_)))
            .count();
        assert_eq!(keys, 8, "the login screen hears every press");
        assert!(
            h.state.greeter.heard.contains(&Heard::Key(vec![KEY_r])),
            "Super+R reaches the login screen as r"
        );
    }

    #[test]
    fn a_right_press_goes_to_the_login_screen_not_the_desktop() {
        let mut h = greeter();
        h.pointer_to(500.0, 400.0);
        h.button(RIGHT, true);
        h.button(RIGHT, false);
        assert_eq!(h.state.greeter.heard, vec![Heard::Button(RIGHT)]);
        assert!(h.state.menus.is_empty(), "no desktop menu");
    }
}
