//! A user's own Remote Desktop password, set or cleared over the account link once a check
//! worker passes their login password. One change per uid and one store write run at a time.

use std::collections::BTreeMap;

use log::Level;
use repriselogin::account::{
    AccountRefusal, AccountReply, AccountRequest, MIN_REMOTE_PASSWORD, RemoteAccess, nt_hash,
    same_hash,
};
use repriselogin::greeter::{AuthKind, ErrorKind};
use zeroize::Zeroizing;

use super::remote::{ASKED_MORE, NEVER_ASKED};
use super::{
    ACCOUNT_WAIT, Daemon, Effect, LookUpKey, StoredPassword, Timer, WORKER_ENDED, WorkerId,
};
use crate::worker::{FromWorker, Mode, ToWorker};

#[derive(Default)]
pub(super) struct Accounts {
    changes: BTreeMap<u64, Change>,
    /// The store write in flight: a change's, by its link, or `None` for a rewrite of what the
    /// daemon holds.
    storing: Option<Option<u64>>,
    /// The store holds passwords the daemon has dropped since its last write.
    rewrite: bool,
    /// Status links waiting on their group look-up, with the peer's uid and account name.
    statuses: BTreeMap<u64, (u32, String)>,
}

struct Change {
    uid: u32,
    user: String,
    /// The new password's NT hash; `None` clears it.
    nt_hash: Option<Zeroizing<String>>,
    /// Held until the check's first secret prompt takes it.
    login_password: Option<Zeroizing<String>>,
    step: Step,
}

#[derive(Clone, Copy, PartialEq)]
enum Step {
    LookingUp,
    Checking(WorkerId),
    /// The login password passed; the store write waits its turn or runs.
    Store,
}

impl Daemon {
    pub(super) fn is_account_worker(&self, worker: WorkerId) -> bool {
        self.account_checking(worker).is_some()
    }

    fn account_checking(&self, worker: WorkerId) -> Option<u64> {
        (self.remote.accounts.changes.iter())
            .find(|(_, c)| c.step == Step::Checking(worker))
            .map(|(&link, _)| link)
    }

    pub(super) fn account_request(
        &mut self,
        link: u64,
        uid: u32,
        user: String,
        request: AccountRequest,
        out: &mut Vec<Effect>,
    ) {
        let (login_password, nt_hash) = match request {
            AccountRequest::Status => return self.account_status(link, uid, user, out),
            AccountRequest::SetRemotePassword {
                login_password,
                remote_password,
            } => {
                if remote_password.chars().count() < MIN_REMOTE_PASSWORD {
                    return refuse(link, AccountRefusal::TooShort, out);
                }
                let hash = nt_hash(&remote_password);
                if same_hash(&nt_hash(&login_password), &hash) {
                    return refuse(link, AccountRefusal::SameAsLogin, out);
                }
                (login_password, Some(hash))
            }
            AccountRequest::ClearRemotePassword { login_password } => (login_password, None),
        };
        let changes = &mut self.remote.accounts.changes;
        if changes.values().any(|c| c.uid == uid) {
            return refuse(link, AccountRefusal::Busy, out);
        }
        if changes.contains_key(&link) {
            return out.push(Effect::Log(
                Level::Warn,
                format!("a second request on account link {link}; ignoring it"),
            ));
        }
        // Anyone but root may clear their own entry, so a user taken out of the group, or one
        // whose group can't be read, can drop it.
        let clearing = nt_hash.is_none();
        if clearing && uid == 0 {
            return refuse(link, AccountRefusal::NotRemoteUser, out);
        }
        if !clearing {
            out.push(Effect::LookUp {
                key: LookUpKey::Account(link),
                user: user.clone(),
                group: self.config.remote.group.clone(),
            });
        }
        let change = Change {
            uid,
            user,
            nt_hash,
            login_password: Some(login_password),
            step: Step::LookingUp,
        };
        changes.insert(link, change);
        if clearing {
            self.account_check(link, out);
        }
    }

    /// Answers a status at once when root asks or remote logins are off, else after the group
    /// look-up, so the reply says whether a set would be allowed. A look-up that fails or times
    /// out is refused as `Failed`, which User Accounts reads as not answering and asks again.
    fn account_status(&mut self, link: u64, uid: u32, user: String, out: &mut Vec<Effect>) {
        let access = if uid == 0 {
            RemoteAccess::Root
        } else if !self.config.remote.enabled {
            RemoteAccess::Off
        } else {
            out.push(Effect::LookUp {
                key: LookUpKey::Account(link),
                user: user.clone(),
                group: self.config.remote.group.clone(),
            });
            self.remote.accounts.statuses.insert(link, (uid, user));
            return;
        };
        answer(link, self.status(uid, &user, access), out);
    }

    fn status(&self, uid: u32, user: &str, access: RemoteAccess) -> AccountReply {
        let remote_password =
            (self.remote.passwords.iter()).any(|p| p.uid == uid && p.user == user);
        AccountReply::Status {
            remote_password,
            access,
        }
    }

    pub(super) fn account_member(
        &mut self,
        link: u64,
        uid: Option<u32>,
        member: bool,
        failed: bool,
        out: &mut Vec<Effect>,
    ) {
        if let Some((peer, user)) = self.remote.accounts.statuses.remove(&link) {
            if failed {
                return refuse(link, AccountRefusal::Failed, out);
            }
            let access = if member && uid == Some(peer) {
                RemoteAccess::Allowed
            } else {
                RemoteAccess::NotInGroup {
                    group: self.config.remote.group.clone(),
                }
            };
            return answer(link, self.status(peer, &user, access), out);
        }
        let Some(c) = self.remote.accounts.changes.get(&link) else {
            return;
        };
        if c.step != Step::LookingUp {
            return;
        }
        if failed {
            return self.account_failed(link, "the group look-up failed", out);
        }
        match uid {
            Some(uid) if uid != 0 && member && uid == c.uid => {}
            Some(other) if other != 0 && member => {
                let why = format!("the account's uid is {other}, not the peer's {}", c.uid);
                return self.account_failed(link, &why, out);
            }
            _ => {
                self.remote.accounts.changes.remove(&link);
                return refuse(link, AccountRefusal::NotRemoteUser, out);
            }
        }
        self.account_check(link, out);
    }

    /// Spawns the worker that checks change `link`'s login password.
    fn account_check(&mut self, link: u64, out: &mut Vec<Effect>) {
        let Some(c) = self.remote.accounts.changes.get(&link) else {
            return;
        };
        let job = self.job(Mode::Check, &c.user);
        let worker = self.next_worker();
        let c = self.remote.accounts.changes.get_mut(&link).expect("change");
        c.step = Step::Checking(worker);
        out.push(Effect::Spawn { worker, job });
        out.push(Effect::Arm {
            timer: Timer::Account(worker),
            after: ACCOUNT_WAIT,
        });
    }

    pub(super) fn account_deadline(&mut self, worker: WorkerId, out: &mut Vec<Effect>) {
        if let Some(link) = self.account_checking(worker) {
            out.push(Effect::Kill(worker));
            self.account_failed(link, "the login password check took too long", out);
        }
    }

    pub(super) fn account_worker_message(
        &mut self,
        worker: WorkerId,
        msg: FromWorker,
        out: &mut Vec<Effect>,
    ) {
        let Some(link) = self.account_checking(worker) else {
            return;
        };
        let c = self.remote.accounts.changes.get_mut(&link).expect("change");
        match msg {
            FromWorker::Prompt {
                kind: AuthKind::Info | AuthKind::Error,
                ..
            } => out.push(Effect::ToWorker {
                worker,
                msg: ToWorker::Answer(None),
            }),
            FromWorker::Prompt {
                kind: AuthKind::Secret,
                ..
            } if c.login_password.is_some() => out.push(Effect::ToWorker {
                worker,
                msg: ToWorker::Answer(c.login_password.take()),
            }),
            FromWorker::Prompt { .. } => {
                out.push(Effect::Kill(worker));
                self.account_failed(link, ASKED_MORE, out);
            }
            FromWorker::Authenticated { .. } if c.login_password.is_some() => {
                out.push(Effect::Kill(worker));
                self.account_failed(link, NEVER_ASKED, out);
            }
            FromWorker::Authenticated { uid, .. } if uid != c.uid => {
                out.push(Effect::Kill(worker));
                let why = format!("PAM's account {uid} is not the peer's {}", c.uid);
                self.account_failed(link, &why, out);
            }
            FromWorker::Authenticated { .. } => {
                c.step = Step::Store;
                self.store_next(out);
            }
            FromWorker::Failed {
                kind: ErrorKind::Auth,
                ..
            } => {
                out.push(Effect::Log(
                    Level::Info,
                    format!(
                        "{}'s login password was wrong for a Remote Desktop password change",
                        c.user
                    ),
                ));
                self.remote.accounts.changes.remove(&link);
                refuse(link, AccountRefusal::WrongPassword, out);
            }
            FromWorker::Failed { text, .. } => self.account_failed(link, &text, out),
            FromWorker::Started { session_id, .. } => {
                out.push(Effect::TerminateSession(session_id));
                out.push(Effect::Kill(worker));
                self.account_failed(link, "the check opened a session", out);
            }
        }
    }

    pub(super) fn account_worker_exited(&mut self, worker: WorkerId, out: &mut Vec<Effect>) {
        if let Some(link) = self.account_checking(worker) {
            self.account_failed(link, WORKER_ENDED, out);
        }
    }

    fn account_failed(&mut self, link: u64, why: &str, out: &mut Vec<Effect>) {
        let Some(c) = self.remote.accounts.changes.remove(&link) else {
            return;
        };
        out.push(Effect::Log(
            Level::Warn,
            format!("{}'s Remote Desktop password change failed: {why}", c.user),
        ));
        refuse(link, AccountRefusal::Failed, out);
    }

    /// Drops the passwords `gone` picks and writes the store without them.
    pub(super) fn drop_passwords(
        &mut self,
        gone: impl Fn(&StoredPassword) -> bool,
        out: &mut Vec<Effect>,
    ) {
        let before = self.remote.passwords.len();
        let mut unlisted = false;
        let remote = &mut self.remote;
        remote.passwords.retain(|p| {
            let dropped = gone(p);
            unlisted |= dropped && remote.listed.remove(&p.uid);
            !dropped
        });
        if self.remote.passwords.len() == before {
            return;
        }
        if unlisted && self.remote.front {
            out.push(self.passwords_event());
        }
        out.extend(self.rewrite_passwords());
    }

    /// Writes the store with the passwords the daemon holds, after the write in flight.
    pub fn rewrite_passwords(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        self.remote.accounts.rewrite = true;
        self.store_next(&mut out);
        out
    }

    /// The passwords the daemon holds with change `c` made.
    fn with_change(&self, c: &Change) -> Vec<StoredPassword> {
        let mut all: Vec<_> = (self.remote.passwords.iter())
            .filter(|p| p.uid != c.uid)
            .cloned()
            .collect();
        all.extend(c.nt_hash.clone().map(|nt_hash| StoredPassword {
            user: c.user.clone(),
            uid: c.uid,
            nt_hash,
        }));
        all
    }

    /// Writes the next checked change, or else a pending rewrite, unless a write is in flight.
    /// Each write holds the passwords the daemon holds then, so it carries every drop before it.
    fn store_next(&mut self, out: &mut Vec<Effect>) {
        let accounts = &self.remote.accounts;
        if accounts.storing.is_some() {
            return;
        }
        let next = (accounts.changes.iter()).find(|(_, c)| c.step == Step::Store);
        let (link, all) = match next {
            Some((&link, c)) => (Some(link), self.with_change(c)),
            None if accounts.rewrite => (None, self.remote.passwords.clone()),
            None => return,
        };
        self.remote.accounts.rewrite = false;
        out.push(Effect::StorePasswords(all));
        self.remote.accounts.storing = Some(link);
    }

    pub(super) fn stored(&mut self, ok: bool, out: &mut Vec<Effect>) {
        let Some(link) = self.remote.accounts.storing.take() else {
            return;
        };
        let Some(link) = link else {
            if !ok {
                out.push(Effect::Log(
                    Level::Warn,
                    "the Remote Desktop passwords of accounts that are gone stay stored until \
                     the next write"
                        .into(),
                ));
            }
            return self.store_next(out);
        };
        if let Some(c) = self.remote.accounts.changes.remove(&link) {
            if ok {
                let what = if c.nt_hash.is_some() {
                    "set"
                } else {
                    "cleared"
                };
                out.push(Effect::Log(
                    Level::Info,
                    format!("{} {what} their Remote Desktop password", c.user),
                ));
                answer(link, AccountReply::Done, out);
                let all = self.with_change(&c);
                out.extend(self.set_remote_passwords(all));
                // The set's own look-up predates its PAM check, so the group is read again.
                if c.nt_hash.is_some() {
                    self.look_up_listed(out);
                }
            } else {
                out.push(Effect::Log(
                    Level::Warn,
                    format!("{}'s Remote Desktop password was not stored", c.user),
                ));
                refuse(link, AccountRefusal::Failed, out);
            }
        }
        self.store_next(out);
    }
}

fn answer(link: u64, reply: AccountReply, out: &mut Vec<Effect>) {
    out.push(Effect::AccountReply { link, reply });
}

fn refuse(link: u64, reason: AccountRefusal, out: &mut Vec<Effect>) {
    answer(link, AccountReply::Refused { reason }, out);
}
