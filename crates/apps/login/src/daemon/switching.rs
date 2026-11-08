//! A session moving between the console and remote clients: the background mark, the
//! interface the login screen remembers, and a session left without its login worker.

use std::time::Instant;

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

    /// A hand-over to `uid`'s session while the console shows another VT marks it a background
    /// session, armed to end like one until a client shows; an unknown `active_vt` (0) marks none.
    pub(super) fn reached_remotely(
        &mut self,
        now: Instant,
        world: &World,
        uid: u32,
        out: &mut Vec<Effect>,
    ) {
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
        if !self.remote.viewers.contains_key(&uid) {
            self.arm_disconnected(now, id, out);
        }
    }

    /// Ends session `id` when its entry is `uid`'s: without its worker it has no PAM handle
    /// to close. The `SessionRemoved` that follows drops its entry.
    pub(super) fn session_orphaned(&mut self, id: &str, uid: u32, out: &mut Vec<Effect>) {
        let Some(e) = self.sessions.by_id(id).filter(|e| e.uid == uid) else {
            return;
        };
        out.push(Effect::Log(
            Level::Warn,
            format!("{}'s session {id} lost its login worker; ending it", e.user),
        ));
        out.push(Effect::TerminateSession(id.into()));
    }

    fn remember_interface(&mut self, interface: Interface, out: &mut Vec<Effect>) {
        if self.remembered.last_interface != Some(interface) {
            self.remembered.last_interface = Some(interface);
            out.push(Effect::Remember(self.remembered.clone()));
        }
    }
}
