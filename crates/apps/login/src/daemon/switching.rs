//! A session moving between the console and remote clients.

use log::Level;

use super::{Daemon, Effect, World};

impl Daemon {
    /// The session on `vt` is the console's from now on.
    pub(super) fn console_on_vt(&mut self, vt: u16, out: &mut Vec<Effect>) {
        let Some(e) = self.sessions.iter().find(|e| e.vt == vt) else {
            return;
        };
        if !e.background {
            return;
        }
        let id = e.id.clone();
        out.push(Effect::Log(
            Level::Info,
            format!("{}'s background session {id} is on the console", e.user),
        ));
        self.sessions.set_background(&id, false);
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
}
