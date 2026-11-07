//! A session moving between the console and remote clients: the background mark and the
//! interface the login screen remembers.

use log::Level;
use repriselogin::Interface;

use super::{Daemon, Effect, World};

impl Daemon {
    /// Keeps session `id`'s interface on its entry; only seat0's active session's is remembered.
    pub(super) fn hello(
        &mut self,
        id: &str,
        interface: Interface,
        seat_active: Option<&str>,
        out: &mut Vec<Effect>,
    ) {
        self.sessions.set_interface(id, interface);
        if seat_active == Some(id) {
            self.remember_interface(interface, out);
        }
    }

    /// The session on `vt` is the console's from now on, and its interface is remembered.
    pub(super) fn console_on_vt(&mut self, vt: u16, out: &mut Vec<Effect>) {
        let Some(e) = self.sessions.iter().find(|e| e.vt == vt) else {
            return;
        };
        let (id, interface) = (e.id.clone(), e.interface);
        if e.background {
            out.push(Effect::Log(
                Level::Info,
                format!("{}'s background session {id} is on the console", e.user),
            ));
            self.sessions.set_background(&id, false);
        }
        if let Some(interface) = interface {
            self.remember_interface(interface, out);
        }
    }

    /// A hand-over passed to `uid`'s session while the console shows another VT makes it a
    /// background session. An unknown `active_vt` (0) marks nothing.
    pub(super) fn reached_remotely(&mut self, world: &World, uid: u32, out: &mut Vec<Effect>) {
        let Some(e) = self.sessions.by_uid(uid) else {
            return;
        };
        if e.background || world.active_vt == 0 || world.active_vt == e.vt {
            return;
        }
        out.push(Effect::Log(
            Level::Info,
            format!(
                "{}'s session {} is in the background with a remote client",
                e.user, e.id
            ),
        ));
        let id = e.id.clone();
        self.sessions.set_background(&id, true);
    }

    fn remember_interface(&mut self, interface: Interface, out: &mut Vec<Effect>) {
        if self.remembered.last_interface != Some(interface) {
            self.remembered.last_interface = Some(interface);
            out.push(Effect::Remember(self.remembered.clone()));
        }
    }
}
