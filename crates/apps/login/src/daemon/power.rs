//! Turning the computer off or restarting it from the login screen while others are logged on,
//! which an administrator's password allows.

use log::Level;
use repriselogin::greeter::{ErrorKind, GreeterReply, PowerAction};

use super::{BUSY, Conversation, Daemon, Effect, LookUpKey, Phase, Purpose, WorkerId, refused};
use crate::table::Route;
use crate::worker::{AUTH_FAILED, Mode};

/// The group whose members are administrators.
pub const ADMIN_GROUP: &str = "wheel";

impl Daemon {
    /// Checks `user`'s password for `action` as a login's is checked, prompts relayed.
    pub(super) fn power_as(
        &mut self,
        now: std::time::Instant,
        action: PowerAction,
        user: String,
        out: &mut Vec<Effect>,
    ) {
        if self.conversation.is_some() {
            return out.push(refused(BUSY));
        }
        let purpose = Purpose::Power(action);
        self.converse(now, Mode::Check, user, Route::Start, purpose, out);
    }

    /// PAM passed the account: anyone but root is looked up in `ADMIN_GROUP`.
    pub(super) fn admin_authenticated(
        &mut self,
        worker: WorkerId,
        user: String,
        uid: u32,
        out: &mut Vec<Effect>,
    ) {
        if uid == 0 {
            out.push(Effect::Log(
                Level::Warn,
                "root was refused as the administrator to turn the computer off or restart it"
                    .into(),
            ));
            return self.fail_conversation(ErrorKind::Auth, AUTH_FAILED.into(), out);
        }
        let c = self.conversation.as_mut().expect("conversation");
        c.user = user.clone();
        c.phase = Phase::Admin { worker, uid };
        out.push(Effect::LookUp {
            key: LookUpKey::Admin(worker),
            user,
            group: ADMIN_GROUP.into(),
        });
    }

    /// A member with PAM's uid gets `Success`, then the power action; anyone else hears a
    /// wrong password's words, so the reply never tells who is an administrator.
    pub(super) fn admin_member(
        &mut self,
        worker: WorkerId,
        uid: Option<u32>,
        member: bool,
        out: &mut Vec<Effect>,
    ) {
        let Some(Conversation {
            phase:
                Phase::Admin {
                    worker: w,
                    uid: pam_uid,
                },
            purpose: Purpose::Power(action),
            ..
        }) = self.conversation
        else {
            return;
        };
        if w != worker {
            return;
        }
        if !(member && uid == Some(pam_uid)) {
            let user = &self.conversation.as_ref().expect("conversation").user;
            out.push(Effect::Log(
                Level::Info,
                format!("{user} is not an administrator; the computer stays on"),
            ));
            return self.fail_conversation(ErrorKind::Auth, AUTH_FAILED.into(), out);
        }
        let c = self.conversation.take().expect("conversation");
        if let Some(w) = c.worker {
            out.push(Effect::Kill(w));
        }
        let what = match action {
            PowerAction::PowerOff => "turns the computer off",
            PowerAction::Reboot => "restarts the computer",
        };
        out.push(Effect::Log(
            Level::Info,
            format!("{} {what} as an administrator", c.user),
        ));
        if self.greeter_runs(c.greeter) {
            out.push(Effect::Reply(GreeterReply::Success));
        }
        out.push(Effect::Power(action));
    }
}
