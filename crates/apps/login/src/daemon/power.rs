//! Turning the computer off or restarting it from the login screen while others are logged on,
//! which an administrator's password allows.

use std::time::{Duration, Instant};

use log::Level;
use repriselogin::greeter::{ErrorKind, GreeterReply, PowerAction};

use super::{
    BUSY, Conversation, Daemon, Effect, LookUpKey, Phase, Purpose, REFUSE_DELAY, Timer, WorkerId,
    refused,
};
use crate::table::Route;
use crate::worker::{AUTH_FAILED, Mode};

/// The group whose members are administrators.
pub const ADMIN_GROUP: &str = "wheel";
/// A refused check hears its error this long after its last answer, whatever refused it.
pub const ADMIN_REFUSE_DELAY: Duration = REFUSE_DELAY;

impl Daemon {
    /// Opens a check conversation for `user`; its prompts relay as a login's do.
    pub(super) fn power_as(
        &mut self,
        now: Instant,
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
        now: Instant,
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
            return self.refuse_admin(now, worker, out);
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
        now: Instant,
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
            ref user,
            ..
        }) = self.conversation
        else {
            return;
        };
        if w != worker {
            return;
        }
        if !(member && uid == Some(pam_uid)) {
            out.push(Effect::Log(
                Level::Info,
                format!("{user} is not an administrator; the computer stays on"),
            ));
            return self.refuse_admin(now, worker, out);
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

    /// Refuses check worker `worker`'s conversation with a wrong password's error, held until
    /// `ADMIN_REFUSE_DELAY` after its last answer so a refused correct password takes as long
    /// as a wrong one. The worker is killed now.
    pub(super) fn refuse_admin(&mut self, now: Instant, worker: WorkerId, out: &mut Vec<Effect>) {
        let c = self.conversation.as_mut().expect("conversation");
        if let Some(w) = c.worker.take() {
            out.push(Effect::Kill(w));
        }
        c.phase = Phase::Refusing { worker };
        let waited = now.saturating_duration_since(c.heard);
        if waited < ADMIN_REFUSE_DELAY {
            return out.push(Effect::Arm {
                timer: Timer::AdminRefuse(worker),
                after: ADMIN_REFUSE_DELAY - waited,
            });
        }
        out.push(Effect::Log(
            Level::Debug,
            format!(
                "an administrator check was slow: {waited:?}, past the \
                 {ADMIN_REFUSE_DELAY:?} a refusal waits"
            ),
        ));
        self.fail_conversation(ErrorKind::Auth, AUTH_FAILED.into(), out);
    }

    /// Sends check worker `worker`'s held refusal; one cancelled since finds no conversation.
    pub(super) fn admin_refuse_over(
        &mut self,
        now: Instant,
        worker: WorkerId,
        out: &mut Vec<Effect>,
    ) {
        let Some(Conversation {
            phase: Phase::Refusing { worker: w },
            heard,
            ..
        }) = self.conversation
        else {
            return;
        };
        if w != worker {
            return;
        }
        let left = ADMIN_REFUSE_DELAY.saturating_sub(now.saturating_duration_since(heard));
        if !left.is_zero() {
            return out.push(Effect::Arm {
                timer: Timer::AdminRefuse(worker),
                after: left,
            });
        }
        self.fail_conversation(ErrorKind::Auth, AUTH_FAILED.into(), out);
    }
}
